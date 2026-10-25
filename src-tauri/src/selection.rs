//! 全局划词 + 悬浮小按钮（P8，仿 STranslate）
//!
//! 流程（2026-10-25 改版：拖选不再取词，点击才取词）：
//! 1. 专用线程装 `WH_MOUSE_LL` 低级鼠标钩子，回调只做轻量判定（不碰剪贴板/UI），
//!    命中「拖选」与「双击选词」后经 `mpsc` 投给取词线程；判定要求按下与抬起在
//!    同一窗口、且两点都落在该窗口客户区内（标题栏/边框拖动不算划词）；
//! 2. 取词线程命中后**先不注入**，只把悬浮图标弹到光标旁；终端类窗口与黑名单进程
//!    （VS Code 等内嵌终端的宿主）连图标都不弹，改为等应用自己复制；
//! 3. 同一线程建 message-only 窗口挂 `AddClipboardFormatListener`：只认「刚拖选过的
//!    那个前台窗口、在 `clipboard_arm_ms` 内」发生的剪贴板更新，作为取词来源——给终端
//!    一条零注入通道，也让用户自己按 Ctrl+C 后不必再点图标（文字已就位，点击即展开）；
//! 4. 用户点图标才真正取词：核对前台仍是来源窗口 → 按物理键状态决定发不发 Ctrl
//!    （只在没按下时自己按下、只释放自己按过的键）→ 轮询剪贴板 → 比对序列号后才还原；
//! 5. 光标右下显示 `selector` 小窗（40×40，透明置顶），
//!    `SetWindowPos(SWP_NOACTIVATE|SWP_SHOWWINDOW)` 定位，不抢焦点。
//!
//! 红线：不启停 rsrs 服务；不 shell；不改动剪贴板以外的系统状态。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::SelectionConfig;

/// 收起态图标尺寸（CSS 逻辑像素；显示时按目标屏幕 DPI 转换）
const ICON_W: i32 = 40;
const ICON_H: i32 = 40;
/// 展开态面板尺寸（与 selection.css 约定一致）
const PANEL_W: i32 = 280;
const PANEL_H: i32 = 168;
/// VK_C（`VK_C` 常量位于键盘模块，单独写数值避免额外导入）
const VK_C_KEY: u16 = 0x43;
/// 双击判定：两次抬起间隔与位置容差
const DOUBLE_CLICK_MS: u128 = 420;
const CLICK_TOLERANCE_PX: i32 = 4;
/// 剪贴板更新消息
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
/// message-only 窗口类名
const CLIP_CLASS: &str = "memToolsClipboardSink";

/// 终端类窗口：往里注入 Ctrl+C 等于给 TUI 发 0x03，故一律不注入
const TERMINAL_CLASSES: &[&str] = &[
    "ConsoleWindowClass",
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "mintty",
    "org.wezfurlong.wezterm",
    "wezterm",
    "Alacritty",
    "alacritty",
    "PuTTY",
];

#[derive(Clone, Serialize)]
pub struct SelectionPayload {
    pub text: String,
    /// 来源窗口标题（可能为空）
    pub source: String,
}

/// 悬浮窗共享状态（托盘开关 / 命令 / 钩子线程共用）
pub struct SelectionState {
    enabled: AtomicBool,
    visible: AtomicBool,
    expanded: AtomicBool,
    payload: Mutex<Option<SelectionPayload>>,
    /// 当前窗口矩形（物理像素，屏幕坐标）
    rect: Mutex<Option<(i32, i32, i32, i32)>>,
    /// 本次悬浮的来源窗口（HWND 转 isize）与是否零注入通道
    source: Mutex<Option<(isize, bool)>>,
}

impl SelectionState {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            visible: AtomicBool::new(false),
            expanded: AtomicBool::new(false),
            payload: Mutex::new(None),
            rect: Mutex::new(None),
            source: Mutex::new(None),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    fn set_visible(&self, on: bool) {
        self.visible.store(on, Ordering::Relaxed);
    }

    fn set_expanded(&self, on: bool) {
        self.expanded.store(on, Ordering::Relaxed);
    }

    pub fn payload(&self) -> Option<SelectionPayload> {
        self.payload.lock().ok().and_then(|p| p.clone())
    }

    fn set_payload(&self, p: Option<SelectionPayload>) {
        if let Ok(mut slot) = self.payload.lock() {
            *slot = p;
        }
    }

    fn rect(&self) -> Option<(i32, i32, i32, i32)> {
        self.rect.lock().ok().and_then(|r| *r)
    }

    fn set_rect(&self, r: Option<(i32, i32, i32, i32)>) {
        if let Ok(mut slot) = self.rect.lock() {
            *slot = r;
        }
    }

    fn source(&self) -> Option<(isize, bool)> {
        self.source.lock().ok().and_then(|s| *s)
    }

    fn set_source(&self, s: Option<(isize, bool)>) {
        if let Ok(mut slot) = self.source.lock() {
            *slot = s;
        }
    }
}

enum Trigger {
    /// 命中划词（拖选 / 双击），带目标窗口 HWND（isize）
    Selection(isize),
    /// 点击悬浮窗外部 → 隐藏
    Outside,
    /// 剪贴板有新增（终端通道 / 用户自己复制）
    Clipboard,
}

/// 按下起点（划词判定：位置、时间、窗口、是否客户区）
#[derive(Clone, Copy)]
struct DownInfo {
    x: i32,
    y: i32,
    at: Instant,
    hwnd: isize,
    in_client: bool,
}

/// 刚发生的划选（剪贴板归因窗口）
#[derive(Clone, Copy)]
struct Arm {
    hwnd: isize,
    at: Instant,
}

static HOOK_STATE: OnceLock<Arc<SelectionState>> = OnceLock::new();
static HOOK_TX: OnceLock<Sender<Trigger>> = OnceLock::new();
static HOOK_CFG: OnceLock<SelectionConfig> = OnceLock::new();

static DOWN: Mutex<Option<DownInfo>> = Mutex::new(None);
/// 上一次「原地抬起」（用于双击判定）
static LAST_CLICK: Mutex<Option<(i32, i32, Instant)>> = Mutex::new(None);
/// 划选后的剪贴板归因窗口
static ARM: Mutex<Option<Arm>> = Mutex::new(None);
/// 最后一次由自己造成的剪贴板序列号（事件到达时据此忽略自家写入）
static SELF_SEQ: AtomicU32 = AtomicU32::new(0);
/// 正在注入取词（期间的剪贴板事件一律忽略）
static INJECTING: AtomicBool = AtomicBool::new(false);

/// 拖选判定：位移达到阈值且不是长时间按住（避免拖窗口/拖滚动条误判为划词）
fn drag_counts_as_selection(dist: i32, elapsed: Duration, threshold: i32) -> bool {
    dist >= threshold.max(1) && elapsed <= Duration::from_secs(10)
}

/// 终端类窗口判定（忽略大小写）
fn is_terminal_class(class: &str) -> bool {
    !class.is_empty() && TERMINAL_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(class))
}

/// 取路径最后一段（`C:\...\Code.exe` → `Code.exe`），去掉可能的引号
fn process_basename(path: &str) -> &str {
    let trimmed = path.trim().trim_matches('"');
    match trimmed.rsplit(|c| c == '\\' || c == '/').next() {
        Some(name) if !name.is_empty() => name,
        _ => trimmed,
    }
}

/// 归因窗口是否已过期
fn arm_expired(elapsed: Duration, arm_ms: u64) -> bool {
    elapsed > Duration::from_millis(arm_ms.max(200))
}

/// 取词文本是否在 `[min_chars, max_chars]` 区间内
fn accept_text(text: &str, min_chars: usize, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    let chars = trimmed.chars().count();
    if chars < min_chars.max(1) || chars > max_chars.max(1) {
        return None;
    }
    Some(trimmed.to_string())
}

/// 注入方案：按键守卫决定发哪些键
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KeyPlan {
    /// 有其它修饰键按下 → 本次不取词
    Skip,
    /// 用户已按住 Ctrl → 只发 C 的按下/抬起，绝不碰 Ctrl
    OnlyC,
    /// 常规：Ctrl↓ C↓ C↑ Ctrl↑
    FullCtrlC,
}

fn plan_for(ctrl_down: bool, other_modifier_down: bool) -> KeyPlan {
    if other_modifier_down {
        KeyPlan::Skip
    } else if ctrl_down {
        KeyPlan::OnlyC
    } else {
        KeyPlan::FullCtrlC
    }
}

fn clear_down() {
    if let Ok(mut slot) = DOWN.lock() {
        *slot = None;
    }
}

fn clear_arm() {
    if let Ok(mut slot) = ARM.lock() {
        *slot = None;
    }
}

fn set_arm(hwnd: isize) {
    if let Ok(mut slot) = ARM.lock() {
        *slot = Some(Arm {
            hwnd,
            at: Instant::now(),
        });
    }
}

fn post(t: Trigger) {
    if let Some(tx) = HOOK_TX.get() {
        let _ = tx.send(t);
    }
}

/// CSS 尺寸转为屏幕物理像素；窗口、工作区避让和鼠标命中共用转换结果。
fn selector_size(expanded: bool, scale_factor: f64) -> (i32, i32) {
    let (w, h) = if expanded {
        (PANEL_W, PANEL_H)
    } else {
        (ICON_W, ICON_H)
    };
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    (
        (w as f64 * scale).ceil() as i32,
        (h as f64 * scale).ceil() as i32,
    )
}

fn point_inside(rect: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    let (rx, ry, rw, rh) = rect;
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::System::DataExchange::{
        AddClipboardFormatListener, GetClipboardSequenceNumber,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
        VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetAncestor,
        GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow, GetMessageW,
        GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, RegisterClassW,
        SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow,
        TranslateMessage, UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, GWL_EXSTYLE, HWND_MESSAGE,
        HWND_TOPMOST, MSLLHOOKSTRUCT, MSG, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WH_MOUSE_LL,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    use arboard;

    /// 启动钩子与取词线程（只启动一次；`enabled=false` 时仍安装钩子，但内部直接忽略）
    pub fn start(app: AppHandle, cfg: SelectionConfig, state: Arc<SelectionState>) {
        if HOOK_TX.get().is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel::<Trigger>();
        if HOOK_TX.set(tx).is_err() {
            return;
        }
        let _ = HOOK_STATE.set(state.clone());
        let _ = HOOK_CFG.set(cfg.clone());

        spawn_worker(app, cfg, state, rx);
        spawn_hook_thread();
    }

    fn spawn_hook_thread() {
        let _ = std::thread::Builder::new()
            .name("memtools-mouse-hook".into())
            .spawn(|| unsafe {
                let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), std::ptr::null_mut(), 0);
                if hook.is_null() {
                    eprintln!(
                        "[memTools] 全局鼠标钩子安装失败：{}",
                        std::io::Error::last_os_error()
                    );
                    return;
                }
                // 剪贴板监听与本线程共用同一消息循环（取词线程只做重活）
                if !install_clipboard_listener() {
                    eprintln!(
                        "[memTools] 剪贴板监听安装失败：{}",
                        std::io::Error::last_os_error()
                    );
                }
                let mut msg: MSG = std::mem::zeroed();
                // GetMessageW 返回 0 表示 WM_QUIT，-1 表示错误
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                UnhookWindowsHookEx(hook);
            });
    }

    /// message-only 窗口 + `AddClipboardFormatListener`
    fn install_clipboard_listener() -> bool {
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let class: Vec<u16> = CLIP_CLASS.encode_utf16().chain(std::iter::once(0)).collect();
            let title: Vec<u16> = "memTools clipboard".encode_utf16().chain(std::iter::once(0)).collect();

            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(clip_proc);
            wc.hInstance = hinst;
            wc.lpszClassName = class.as_ptr();
            if RegisterClassW(&wc) == 0 {
                // 1410 = 类已存在，重装/热重启时会出现，忽略
                if std::io::Error::last_os_error().raw_os_error() != Some(1410) {
                    return false;
                }
            }
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return false;
            }
            AddClipboardFormatListener(hwnd) != 0
        }
    }

    unsafe extern "system" fn clip_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CLIPBOARDUPDATE {
            post(Trigger::Clipboard);
            return 0;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            if let (Some(state), Some(cfg)) = (HOOK_STATE.get(), HOOK_CFG.get()) {
                let info = &*(lparam as *const MSLLHOOKSTRUCT);
                match wparam as u32 {
                    WM_LBUTTONDOWN => on_button_down(state, info.pt),
                    WM_LBUTTONUP => on_button_up(state, cfg, info.pt),
                    _ => {}
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    fn on_button_down(state: &SelectionState, pt: POINT) {
        // 悬浮窗可见时：点在窗外 = 隐藏，点在窗内 = 与面板交互，均不参与划词
        if state.is_visible() {
            if let Some(r) = state.rect() {
                if point_inside(r, pt.x, pt.y) {
                    clear_down();
                    return;
                }
            }
            post(Trigger::Outside);
        }
        if !state.is_enabled() {
            clear_down();
            return;
        }
        // 用光标下的顶层窗口判断，而不是 GetForegroundWindow：首次点在非活动窗口时，
        // 钩子先于激活触发，前台还停在旧窗口上
        let hwnd = root_window_at(pt);
        if hwnd.is_null() {
            clear_down();
            return;
        }
        let in_client = client_area_contains(hwnd, pt);
        if let Ok(mut slot) = DOWN.lock() {
            *slot = Some(DownInfo {
                x: pt.x,
                y: pt.y,
                at: Instant::now(),
                hwnd: hwnd as isize,
                in_client,
            });
        }
    }

    fn on_button_up(state: &SelectionState, cfg: &SelectionConfig, pt: POINT) {
        let down = DOWN.lock().ok().and_then(|mut d| d.take());
        let Some(down) = down else {
            return;
        };
        if !state.is_enabled() {
            return;
        }
        // 按下与抬起必须在同一窗口，且都在客户区内（标题栏/边框/移出窗口不算划词）
        let hwnd = root_window_at(pt);
        if hwnd.is_null() || hwnd as isize != down.hwnd {
            return;
        }
        if !down.in_client || !client_area_contains(hwnd, pt) {
            return;
        }
        let dist = (pt.x - down.x).abs().max((pt.y - down.y).abs());
        let elapsed = down.at.elapsed();


        // 原地点击：可能是双击选词
        if dist <= CLICK_TOLERANCE_PX {
            let double = LAST_CLICK
                .lock()
                .ok()
                .and_then(|last| *last)
                .map(|(lx, ly, lt)| {
                    lt.elapsed().as_millis() <= DOUBLE_CLICK_MS
                        && (pt.x - lx).abs() <= CLICK_TOLERANCE_PX
                        && (pt.y - ly).abs() <= CLICK_TOLERANCE_PX
                })
                .unwrap_or(false);
            if let Ok(mut last) = LAST_CLICK.lock() {
                *last = Some((pt.x, pt.y, Instant::now()));
            }
            if cfg.double_click && double {
                post(Trigger::Selection(hwnd as isize));
            }
            return;
        }

        if drag_counts_as_selection(dist, elapsed, cfg.drag_threshold_px) {
            post(Trigger::Selection(hwnd as isize));
        }
    }

    fn spawn_worker(
        app: AppHandle,
        cfg: SelectionConfig,
        state: Arc<SelectionState>,
        rx: Receiver<Trigger>,
    ) {
        let _ = std::thread::Builder::new()
            .name("memtools-selection".into())
            .spawn(move || {
                while let Ok(trigger) = rx.recv() {
                    match trigger {
                        Trigger::Outside => hide_selector(&app, &state),
                        Trigger::Selection(hwnd) => on_selection(&app, &cfg, &state, hwnd),
                        Trigger::Clipboard => on_clipboard(&app, &cfg, &state),
                    }
                }
            });
    }

    /// 命中划选：只弹图标，不取词（终端/黑名单连图标都不弹，等剪贴板事件）
    fn on_selection(app: &AppHandle, cfg: &SelectionConfig, state: &SelectionState, raw: isize) {
        // 用拖选命中的真实窗口，不用 GetForegroundWindow：有些窗口不激活（或拖选发生在非活动窗口）时前台会指向别处
        let hwnd = raw as HWND;
        if hwnd.is_null() || pid_of(hwnd) == unsafe { GetCurrentProcessId() } {
            return;
        }
        let no_inject = is_no_inject(hwnd, cfg);
        if state.is_visible() {
            hide_selector(app, state);
        }
        set_arm(hwnd as isize);
        if no_inject {
            // 终端与内嵌终端宿主：绝不注入，等应用自己复制后由剪贴板事件唤出
            return;
        }
        let payload = SelectionPayload {
            text: String::new(),
            source: window_title(hwnd),
        };
        show_selector(app, state, payload, hwnd, false);
    }

    /// 剪贴板有新增：只认「刚拖选过的那个前台窗口」在归因窗口内的更新
    fn on_clipboard(app: &AppHandle, cfg: &SelectionConfig, state: &SelectionState) {
        if !state.is_enabled() || INJECTING.load(Ordering::SeqCst) {
            return;
        }
        let seq = unsafe { GetClipboardSequenceNumber() };
        if seq == 0 || seq <= SELF_SEQ.load(Ordering::SeqCst) {
            return;
        }
        let arm = ARM.lock().ok().and_then(|mut a| a.take());
        let Some(arm) = arm else {
            return;
        };
        if arm_expired(arm.at.elapsed(), cfg.clipboard_arm_ms) {
            return;
        }
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() || hwnd as isize != arm.hwnd || pid_of(hwnd) == unsafe { GetCurrentProcessId() } {
            return;
        }
        // 给来源应用一点时间把剪贴板写完
        std::thread::sleep(Duration::from_millis(40));
        let Some(text) = read_clipboard_text() else {
            return;
        };
        let Some(text) = accept_text(&text, cfg.min_chars, cfg.max_chars) else {
            return;
        };
        let payload = SelectionPayload {
            text,
            source: window_title(hwnd),
        };
        if state.is_visible() {
            // 普通窗口已弹图标：补全文字，点击时无需再注入
            state.set_payload(Some(payload.clone()));
            if let Some(win) = app.get_webview_window("selector") {
                let _ = win.emit("selection://text", payload);
            }
        } else {
            show_selector(app, state, payload, hwnd, false);
        }
        SELF_SEQ.store(unsafe { GetClipboardSequenceNumber() }, Ordering::SeqCst);
        clear_arm();
    }

    /// 写剪贴板的文本（只读，不改动用户内容）
    fn read_clipboard_text() -> Option<String> {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        clipboard.get_text().ok()
    }

    /// 窗口是否终端类，或进程名落在黑名单里（内嵌终端的宿主只能按进程名判）
    fn is_no_inject(hwnd: HWND, cfg: &SelectionConfig) -> bool {
        if is_terminal_class(&window_class(hwnd)) {
            return true;
        }
        let pid = pid_of(hwnd);
        if pid == 0 {
            return false;
        }
        let exe = process_name(pid);
        if exe.is_empty() {
            return false;
        }
        cfg.no_inject_processes
            .iter()
            .any(|p| !p.trim().is_empty() && p.trim().eq_ignore_ascii_case(&exe))
    }

    /// 点击图标时取词：已有文字直接返回；否则核对来源窗口后注入一次
    pub fn capture_on_click(state: &SelectionState) -> Option<SelectionPayload> {
        // 没有可见的悬浮窗就没有可取的划词（同时挡住启动时的主动拉取）
        if !state.is_visible() {
            return None;
        }
        if let Some(p) = state.payload() {
            if !p.text.trim().is_empty() {
                return Some(p);
            }
        }
        let (source, no_inject) = state.source()?;
        if no_inject {
            return None;
        }
        let cfg = HOOK_CFG.get()?;
        let hwnd = source as HWND;
        if !ensure_foreground(hwnd) {
            return None;
        }
        let plan = plan_for(
            key_down(VK_CONTROL),
            key_down(VK_SHIFT) || key_down(VK_MENU) || key_down(VK_LWIN) || key_down(VK_RWIN),
        );
        let text = capture_selected_text(
            Duration::from_millis(cfg.capture_timeout_ms.max(50)),
            plan,
        )?;
        let text = accept_text(&text, cfg.min_chars, cfg.max_chars)?;
        let payload = SelectionPayload {
            text,
            source: window_title(hwnd),
        };
        state.set_payload(Some(payload.clone()));
        clear_arm();
        Some(payload)
    }

    /// 前台必须是来源窗口；若被自身窗口抢到前台，尝试拉回来源窗口再核对
    fn ensure_foreground(hwnd: HWND) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        if fg as isize == hwnd as isize {
            return true;
        }
        if fg.is_null() || pid_of(fg) == unsafe { GetCurrentProcessId() } {
            unsafe { SetForegroundWindow(hwnd) };
            std::thread::sleep(Duration::from_millis(30));
            return unsafe { GetForegroundWindow() } as isize == hwnd as isize;
        }
        false
    }

    /// 模拟复制并读回选中文本；只在剪贴板仍是本次结果时才还原此前的文本。
    fn capture_selected_text(timeout: Duration, plan: KeyPlan) -> Option<String> {
        if plan == KeyPlan::Skip {
            return None;
        }
        let mut clipboard = arboard::Clipboard::new().ok()?;
        let before = clipboard.get_text().ok();
        let seq_before = unsafe { GetClipboardSequenceNumber() };

        INJECTING.store(true, Ordering::SeqCst);
        send_copy(plan);

        let deadline = Instant::now() + timeout;
        let mut grabbed: Option<String> = None;
        let mut seq_after = seq_before;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(30));
            let seq = unsafe { GetClipboardSequenceNumber() };
            if seq == seq_before {
                continue;
            }
            seq_after = seq;
            match clipboard.get_text() {
                Ok(t) if !t.trim().is_empty() => {
                    grabbed = Some(t);
                    break;
                }
                _ => continue,
            }
        }

        // 只有「本次取词确实改了剪贴板、且此后没人再写」才还原，避免冲掉用户自己的复制
        let last_seq = unsafe { GetClipboardSequenceNumber() };
        if last_seq != seq_before && last_seq == seq_after {
            if let Some(prev) = before {
                if grabbed.as_deref() != Some(prev.as_str()) {
                    let _ = clipboard.set_text(prev);
                }
            }
        }
        SELF_SEQ.store(unsafe { GetClipboardSequenceNumber() }, Ordering::SeqCst);
        INJECTING.store(false, Ordering::SeqCst);
        grabbed
    }

    /// 按方案注入：只按下自己需要的键，也只释放自己按下过的键
    fn send_copy(plan: KeyPlan) {
        if plan == KeyPlan::Skip {
            return;
        }
        let mut inputs: Vec<INPUT> = Vec::with_capacity(4);
        if plan == KeyPlan::FullCtrlC {
            inputs.push(key_input(VK_CONTROL, false));
        }
        inputs.push(key_input(VK_C_KEY, false));
        inputs.push(key_input(VK_C_KEY, true));
        if plan == KeyPlan::FullCtrlC {
            inputs.push(key_input(VK_CONTROL, true));
        }
        if inputs.is_empty() {
            return;
        }
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    fn key_input(vk: u16, up: bool) -> INPUT {
        let mut input: INPUT = unsafe { std::mem::zeroed() };
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki = KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dwExtraInfo: 0,
        };
        input
    }

    fn key_down(vk: u16) -> bool {
        unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
    }

    fn pid_of(hwnd: HWND) -> u32 {
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    fn window_class(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn process_name(pid: u32) -> String {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 512];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
            CloseHandle(handle);
            if ok == 0 {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            process_basename(&path).to_string()
        }
    }

    fn window_title(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
            .trim()
            .chars()
            .take(64)
            .collect()
    }

    /// 屏幕坐标下最顶层窗口的根（顶层）窗口；钩子先于激活触发时用它取真实目标
    fn root_window_at(pt: POINT) -> HWND {
        unsafe {
            let child = WindowFromPoint(pt);
            if child.is_null() {
                return child;
            }
            let root = GetAncestor(child, GA_ROOT);
            if root.is_null() {
                child
            } else {
                root
            }
        }
    }

    /// 屏幕坐标是否落在窗口客户区内（标题栏、边框、调整边框都不算）
    fn client_area_contains(hwnd: HWND, pt: POINT) -> bool {
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            if GetClientRect(hwnd, &mut rc) == 0 {
                return false;
            }
            let mut tl = POINT {
                x: rc.left,
                y: rc.top,
            };
            let mut br = POINT {
                x: rc.right,
                y: rc.bottom,
            };
            if ClientToScreen(hwnd, &mut tl) == 0 || ClientToScreen(hwnd, &mut br) == 0 {
                return false;
            }
            pt.x >= tl.x && pt.x < br.x && pt.y >= tl.y && pt.y < br.y
        }
    }

    /// 光标右下显示悬浮图标（`text` 为空表示等用户点击才取词）
    fn show_selector(
        app: &AppHandle,
        state: &SelectionState,
        payload: SelectionPayload,
        hwnd: HWND,
        no_inject: bool,
    ) {
        let Some(win) = app.get_webview_window("selector") else {
            return;
        };
        state.set_payload(Some(payload.clone()));
        state.set_source(Some((hwnd as isize, no_inject)));
        // 先渲染内容再显示，避免闪一帧空白
        let _ = win.emit("selection://show", payload);

        let offset = HOOK_CFG.get().map(|c| c.offset_px.max(0)).unwrap_or(14);
        let (cx, cy) = cursor_pos();
        let (w, h) = selector_size_at(&win, cx, cy, false);
        let (x, y) = clamp_to_work_area(cx + offset, cy + offset, w, h, (cx, cy));
        apply_rect(&win, x, y, w, h, true);
        state.set_rect(Some((x, y, w, h)));
        state.set_expanded(false);
        state.set_visible(true);
    }

    /// 展开 / 收起（前端点击图标后调用；面板尺寸变化由这里统一处理）
    pub fn resize_selector(
        app: &AppHandle,
        state: &SelectionState,
        expanded: bool,
    ) -> Result<(), String> {
        let win = app
            .get_webview_window("selector")
            .ok_or_else(|| "悬浮窗不存在".to_string())?;
        let (cx, cy) = state.rect().map(|r| (r.0, r.1)).unwrap_or_else(cursor_pos);
        let (w, h) = selector_size_at(&win, cx, cy, expanded);
        let (x, y) = clamp_to_work_area(cx, cy, w, h, (cx, cy));
        apply_rect(&win, x, y, w, h, true);
        state.set_rect(Some((x, y, w, h)));
        state.set_expanded(expanded);
        Ok(())
    }

    pub fn hide_selector(app: &AppHandle, state: &SelectionState) {
        state.set_visible(false);
        state.set_expanded(false);
        state.set_rect(None);
        state.set_source(None);
        clear_arm();
        if let Some(win) = app.get_webview_window("selector") {
            if let Some(hwnd) = raw_hwnd(&win) {
                unsafe {
                    ShowWindow(hwnd as HWND, SW_HIDE);
                }
            } else {
                let _ = win.hide();
            }
            let _ = win.emit("selection://hide", ());
        }
    }

    /// 托盘开关：关闭时立即隐藏图标
    pub fn set_enabled(app: &AppHandle, state: &SelectionState, on: bool) {
        state.set_enabled(on);
        if !on {
            hide_selector(app, state);
        }
    }

    /// 按即将显示的位置取 DPI，不能沿用隐藏窗口上一次所在屏幕的缩放。
    fn selector_size_at(win: &tauri::WebviewWindow, x: i32, y: i32, expanded: bool) -> (i32, i32) {
        let scale = win
            .monitor_from_point(x as f64, y as f64)
            .ok()
            .flatten()
            .map(|monitor| monitor.scale_factor())
            .or_else(|| win.scale_factor().ok())
            .unwrap_or(1.0);
        selector_size(expanded, scale)
    }

    /// 窗口置顶显示（不激活）。`show=false` 时仅调整位置/尺寸。
    fn apply_rect(win: &tauri::WebviewWindow, x: i32, y: i32, w: i32, h: i32, show: bool) {
        let Some(hwnd) = raw_hwnd(win) else {
            // 兜底：走 Tauri API（可能短暂抢焦点）
            let _ = win.set_size(tauri::PhysicalSize::new(w.max(1) as u32, h.max(1) as u32));
            let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
            if show {
                let _ = win.show();
            }
            return;
        };
        // 不抢焦点、不进 Alt+Tab
        unsafe {
            let style = GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE);
            let want = style | (WS_EX_NOACTIVATE as isize) | (WS_EX_TOOLWINDOW as isize);
            if style != want {
                SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, want);
            }
            let mut flags = SWP_NOACTIVATE;
            if show {
                flags |= SWP_SHOWWINDOW;
            }
            SetWindowPos(hwnd as HWND, HWND_TOPMOST, x, y, w, h, flags);
        }
    }

    /// Tauri 的 HWND 类型随 windows crate 版本变化（isize 或裸指针），统一转成 isize
    fn raw_hwnd(win: &tauri::WebviewWindow) -> Option<isize> {
        win.hwnd().ok().map(|h| h.0 as isize)
    }

    fn cursor_pos() -> (i32, i32) {
        let mut p = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut p);
        }
        (p.x, p.y)
    }

    /// 把窗口限制在光标所在显示器的工作区内（多屏 / 任务栏 / DPI 均已由系统给物理像素）
    fn clamp_to_work_area(x: i32, y: i32, w: i32, h: i32, anchor: (i32, i32)) -> (i32, i32) {
        let Some((left, top, right, bottom)) = work_area(anchor.0, anchor.1) else {
            return (x, y);
        };
        let nx = x.clamp(left, (right - w).max(left));
        let ny = y.clamp(top, (bottom - h).max(top));
        (nx, ny)
    }

    fn work_area(x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
        unsafe {
            let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
            if monitor.is_null() {
                return None;
            }
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            Some((
                info.rcWork.left,
                info.rcWork.top,
                info.rcWork.right,
                info.rcWork.bottom,
            ))
        }
    }
}

// 非 Windows 平台：划词功能不可用，所有公共 API 做空实现
#[cfg(not(windows))]
pub fn start(_app: AppHandle, _cfg: SelectionConfig, _state: Arc<SelectionState>) {}

#[cfg(not(windows))]
pub fn set_enabled(_app: &AppHandle, _state: &SelectionState, _on: bool) {}

#[cfg(not(windows))]
pub fn resize_selector(
    _app: &AppHandle,
    _state: &SelectionState,
    _expanded: bool,
) -> Result<(), String> {
    Err("划词功能仅支持 Windows".into())
}

#[cfg(not(windows))]
pub fn hide_selector(_app: &AppHandle, _state: &SelectionState) {}

#[cfg(not(windows))]
pub fn capture_on_click(state: &SelectionState) -> Option<SelectionPayload> {
    state.payload()
}

// Windows 平台：重导出公共 API
#[cfg(windows)]
pub use windows_impl::{capture_on_click, hide_selector, resize_selector, set_enabled, start};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_scales_icon_and_panel_to_physical_pixels() {
        assert_eq!(selector_size(false, 1.0), (40, 40));
        assert_eq!(selector_size(false, 1.25), (50, 50));
        assert_eq!(selector_size(false, 1.5), (60, 60));
        assert_eq!(selector_size(false, 2.0), (80, 80));
        assert_eq!(selector_size(true, 1.5), (420, 252));
        assert_eq!(selector_size(true, 2.0), (560, 336));
    }

    #[test]
    fn scaled_selector_hit_area_includes_visible_right_and_bottom_edges() {
        let (w, h) = selector_size(false, 1.5);
        assert!(point_inside((100, 100, w, h), 159, 159));
        assert!(!point_inside((100, 100, w, h), 160, 160));
    }

    #[test]
    fn drag_threshold() {
        let ok = Duration::from_millis(300);
        assert!(!drag_counts_as_selection(3, ok, 8));
        assert!(drag_counts_as_selection(8, ok, 8));
        assert!(drag_counts_as_selection(80, Duration::from_millis(1500), 8));
        // 长时间按住（拖窗口/滚动条）不算划词
        assert!(!drag_counts_as_selection(80, Duration::from_secs(11), 8));
        // 阈值 0 也至少要求 1px
        assert!(!drag_counts_as_selection(0, ok, 0));
        assert!(drag_counts_as_selection(1, ok, 0));
    }

    #[test]
    fn rect_hit_test() {
        let r = (100, 100, 40, 40);
        assert!(point_inside(r, 100, 100));
        assert!(point_inside(r, 139, 139));
        assert!(!point_inside(r, 140, 120));
        assert!(!point_inside(r, 99, 120));
        assert!(!point_inside(r, 120, 140));
    }

    #[test]
    fn state_flags() {
        let st = SelectionState::new(true);
        assert!(st.is_enabled());
        st.set_enabled(false);
        assert!(!st.is_enabled());
        assert!(st.payload().is_none());
        st.set_payload(Some(SelectionPayload {
            text: "hi".into(),
            source: "记事本".into(),
        }));
        assert_eq!(st.payload().map(|p| p.text), Some("hi".into()));
        assert!(st.source().is_none());
        st.set_source(Some((42, true)));
        assert_eq!(st.source(), Some((42, true)));
    }

    #[test]
    fn terminal_window_classes() {
        assert!(is_terminal_class("CASCADIA_HOSTING_WINDOW_CLASS"));
        assert!(is_terminal_class("consolewindowclass"));
        assert!(is_terminal_class("mintty"));
        assert!(!is_terminal_class("Chrome_WidgetWin_1"));
        assert!(!is_terminal_class("Notepad"));
        assert!(!is_terminal_class(""));
    }

    #[test]
    fn basename_of_process_path() {
        assert_eq!(
            process_basename(r"C:\Program Files\Microsoft VS Code\Code.exe"),
            "Code.exe"
        );
        assert_eq!(process_basename("code.exe"), "code.exe");
        assert_eq!(process_basename("\"C:\\Tools\\wt.exe\""), "wt.exe");
        assert_eq!(process_basename(""), "");
    }

    #[test]
    fn modifier_guard() {
        assert_eq!(plan_for(false, false), KeyPlan::FullCtrlC);
        // 用户已按住 Ctrl：只发 C，绝不碰 Ctrl
        assert_eq!(plan_for(true, false), KeyPlan::OnlyC);
        // 其它修饰键按下：本次不取词
        assert_eq!(plan_for(false, true), KeyPlan::Skip);
        assert_eq!(plan_for(true, true), KeyPlan::Skip);
    }

    #[test]
    fn text_bounds() {
        assert_eq!(accept_text("  hi  ", 2, 4000).as_deref(), Some("hi"));
        assert!(accept_text("a", 2, 4000).is_none());
        assert!(accept_text("", 2, 4000).is_none());
        assert!(accept_text(&"x".repeat(4001), 2, 4000).is_none());
        // 阈值 0 也至少 1 字
        assert!(accept_text("", 0, 0).is_none());
    }

    #[test]
    fn arm_window_expiry() {
        assert!(!arm_expired(Duration::from_millis(100), 1200));
        assert!(!arm_expired(Duration::from_millis(1200), 1200));
        assert!(arm_expired(Duration::from_millis(1300), 1200));
        // 配置异常时兜底为 200ms
        assert!(!arm_expired(Duration::from_millis(150), 0));
        assert!(arm_expired(Duration::from_millis(250), 0));
    }
}
