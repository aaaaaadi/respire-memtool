//! 全局划词 + 悬浮小按钮（P8，仿 STranslate）
//!
//! 流程：
//! 1. 专用线程装 `WH_MOUSE_LL` 低级鼠标钩子，回调只做轻量判定（不碰剪贴板/UI），
//!    检测「拖选」与「双击选词」，命中后经 `mpsc` 投给取词线程；
//! 2. 取词线程：跳过 memTools 自身前台窗口 → 等鼠标事件落地 → `SendInput` 模拟 Ctrl+C
//!    → 按剪贴板序列号轮询 → `arboard` 读文本 → 还原原剪贴板文本；
//! 3. 光标右下显示 `selector` 小窗（40×40，透明置顶），
//!    用 `SetWindowPos(SWP_NOACTIVATE|SWP_SHOWWINDOW)` 定位，不抢焦点；
//! 4. 点击悬浮窗外部由同一钩子判定并隐藏。
//!
//! 红线：不启停 rsrs 服务；不 shell；不改动剪贴板以外的系统状态。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
#[cfg(windows)]
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
#[cfg(windows)]
use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, SetWindowLongPtrW, SetWindowPos,
    SetWindowsHookExW, ShowWindow, TranslateMessage, UnhookWindowsHookEx, GWL_EXSTYLE,
    HWND_TOPMOST, MSLLHOOKSTRUCT, MSG, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WH_MOUSE_LL,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use crate::config::SelectionConfig;

/// 收起态图标窗口尺寸（物理像素）
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
}

impl SelectionState {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            visible: AtomicBool::new(false),
            expanded: AtomicBool::new(false),
            payload: Mutex::new(None),
            rect: Mutex::new(None),
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
}

enum Trigger {
    /// 命中划词（拖选 / 双击）
    Selection,
    /// 点击悬浮窗外部 → 隐藏
    Outside,
}

static HOOK_STATE: OnceLock<Arc<SelectionState>> = OnceLock::new();
static HOOK_TX: OnceLock<Sender<Trigger>> = OnceLock::new();
static HOOK_CFG: OnceLock<SelectionConfig> = OnceLock::new();

/// 按下起点（仅用于划词判定）
static DOWN: Mutex<Option<(i32, i32, Instant)>> = Mutex::new(None);
/// 上一次「原地抬起」（用于双击判定）
static LAST_CLICK: Mutex<Option<(i32, i32, Instant)>> = Mutex::new(None);

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
            let mut msg: MSG = std::mem::zeroed();
            // GetMessageW 返回 0 表示 WM_QUIT，-1 表示错误
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            UnhookWindowsHookEx(hook);
        });
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
    if let Ok(mut slot) = DOWN.lock() {
        *slot = Some((pt.x, pt.y, Instant::now()));
    }
}

fn on_button_up(state: &SelectionState, cfg: &SelectionConfig, pt: POINT) {
    let down = DOWN.lock().ok().and_then(|mut d| d.take());
    let Some((x0, y0, t0)) = down else {
        return;
    };
    if !state.is_enabled() {
        return;
    }
    let dist = (pt.x - x0).abs().max((pt.y - y0).abs());
    let elapsed = t0.elapsed();

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
            post(Trigger::Selection);
        }
        return;
    }

    if drag_counts_as_selection(dist, elapsed, cfg.drag_threshold_px) {
        post(Trigger::Selection);
    }
}

/// 拖选判定：位移达到阈值且不是长时间按住（避免拖窗口/拖滚动条误判为划词）
fn drag_counts_as_selection(dist: i32, elapsed: Duration, threshold: i32) -> bool {
    dist >= threshold.max(1) && elapsed <= Duration::from_secs(10)
}

fn clear_down() {
    if let Ok(mut slot) = DOWN.lock() {
        *slot = None;
    }
}

fn post(t: Trigger) {
    if let Some(tx) = HOOK_TX.get() {
        let _ = tx.send(t);
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
                    Trigger::Selection => {
                        if !state.is_enabled() {
                            continue;
                        }
                        if state.is_visible() {
                            hide_selector(&app, &state);
                        }
if let Some(payload) = grab_selection(&cfg) {
                            show_selector(&app, &state, payload);
                        }
                    }
                }
            }
        });
}

#[cfg(not(windows))]
/// 非 Windows 平台：划词功能不可用，所有公共 API 做空实现
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

/// 取词：跳过自身前台窗口 → 等鼠标事件落地 → 模拟 Ctrl+C → 读剪贴板 → 还原
fn grab_selection(cfg: &SelectionConfig) -> Option<SelectionPayload> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return None;
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == unsafe { GetCurrentProcessId() } {
        return None;
    }
    let source = window_title(hwnd);

    // 给来源应用一点时间处理 mouse-up（拖选结束时选区才稳定）
    std::thread::sleep(Duration::from_millis(80));
    // 期间用户已切换窗口则放弃
    if unsafe { GetForegroundWindow() } != hwnd {
        return None;
    }

    let text = capture_selected_text(Duration::from_millis(cfg.capture_timeout_ms.max(50)))?;
    let trimmed = text.trim();
    let chars = trimmed.chars().count();
    if chars < cfg.min_chars.max(1) || chars > cfg.max_chars.max(1) {
        return None;
    }
    Some(SelectionPayload {
        text: trimmed.to_string(),
        source,
    })
}

/// 模拟 Ctrl+C 并读回选中文本；完成后还原此前的剪贴板文本。
#[cfg(windows)]
fn capture_selected_text(timeout: Duration) -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let before = clipboard.get_text().ok();
    let seq_before = unsafe { GetClipboardSequenceNumber() };

    send_ctrl_c();

    let deadline = Instant::now() + timeout;
    let mut grabbed: Option<String> = None;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(30));
        if unsafe { GetClipboardSequenceNumber() } == seq_before {
            continue;
        }
        match clipboard.get_text() {
            Ok(t) if !t.trim().is_empty() => {
                grabbed = Some(t);
                break;
            }
            _ => continue,
        }
    }

    // 还原原剪贴板（仅文本；原本没有文本时保留复制结果，避免清空用户内容）
    if let Some(prev) = before {
        if grabbed.as_deref() != Some(prev.as_str()) {
            let _ = clipboard.set_text(prev);
        }
    }
    grabbed
}

#[cfg(not(windows))]
fn capture_selected_text(_timeout: Duration) -> Option<String> {
    None
}

fn send_ctrl_c() {
    let inputs = [key_input(VK_CONTROL, false), key_input(VK_C_KEY, false), key_input(VK_C_KEY, true), key_input(VK_CONTROL, true)];
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

/// 光标右下显示悬浮图标
fn show_selector(app: &AppHandle, state: &SelectionState, payload: SelectionPayload) {
    let Some(win) = app.get_webview_window("selector") else {
        return;
    };
    state.set_payload(Some(payload.clone()));
    // 先渲染内容再显示，避免闪一帧空白
    let _ = win.emit("selection://show", payload);

    let offset = HOOK_CFG.get().map(|c| c.offset_px.max(0)).unwrap_or(14);
    let (cx, cy) = cursor_pos();
    let (x, y) = clamp_to_work_area(cx + offset, cy + offset, ICON_W, ICON_H);
    apply_rect(&win, x, y, ICON_W, ICON_H, true);
    state.set_rect(Some((x, y, ICON_W, ICON_H)));
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
    let (w, h) = if expanded {
        (PANEL_W, PANEL_H)
    } else {
        (ICON_W, ICON_H)
    };
    let (cx, cy) = state.rect().map(|r| (r.0, r.1)).unwrap_or_else(cursor_pos);
    let (x, y) = clamp_to_work_area(cx, cy, w, h);
    apply_rect(&win, x, y, w, h, true);
    state.set_rect(Some((x, y, w, h)));
    state.set_expanded(expanded);
    Ok(())
}

pub fn hide_selector(app: &AppHandle, state: &SelectionState) {
    state.set_visible(false);
    state.set_expanded(false);
    state.set_rect(None);
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
        let want = style
            | (WS_EX_NOACTIVATE as isize)
            | (WS_EX_TOOLWINDOW as isize);
        if style != want {
            SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, want);
        }
        let mut flags = SWP_NOACTIVATE;
        if show {
            flags |= SWP_SHOWWINDOW;
        }
        SetWindowPos(
            hwnd as HWND,
            HWND_TOPMOST,
            x,
            y,
            w,
            h,
            flags,
        );
    }
}

/// Tauri 的 HWND 类型随 windows crate 版本变化（isize 或裸指针），统一转成 isize
#[cfg(windows)]
fn raw_hwnd(win: &tauri::WebviewWindow) -> Option<isize> {
    win.hwnd().ok().map(|h| h.0 as isize)
}

#[cfg(not(windows))]
fn raw_hwnd(_win: &tauri::WebviewWindow) -> Option<isize> {
    None
}

fn cursor_pos() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// 把窗口限制在光标所在显示器的工作区内（多屏 / 任务栏 / DPI 均已由系统给物理像素）
fn clamp_to_work_area(x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    let Some((left, top, right, bottom)) = work_area(x, y) else {
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

fn point_inside(rect: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    let (rx, ry, rw, rh) = rect;
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}


