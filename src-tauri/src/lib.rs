//! memTools — rsrs 桌面快捷记忆小工具
//!
//! P0+P1：Tauri 2 壳 + 全局热键小窗 + 三层菜单 + 手动路径提交（client-only HTTP）
//! 红线：不启停 rsrs 服务、不使用 --direct、不 shell 拼接调用 rsrs。

pub mod config;
pub mod rsrs_client;

pub mod agent;
pub mod automation;
pub mod graph;
pub mod graph_vectors;
pub mod llm;
pub mod processor;
pub mod processor_settings;
pub mod quick;
pub mod recent;
pub mod reminders;
pub mod selection;
pub mod server;
pub mod targets;
pub mod extension_setup;
pub mod updater;
pub mod update_http;
mod window_position;

use std::sync::Arc;

use automation::CandidateOut;
use rsrs_client::{RememberRequest, RsrsClient, TaxonomyItem};
use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 全局状态
struct AppState {
    config: config::AppConfig,
    client: Arc<RsrsClient>,
    /// 快捷目标（P9）：主窗、划词面板、扩展端点共用同一份数据
    targets: Arc<std::sync::Mutex<targets::TargetStore>>,
    /// 梳理来源（P10）：小窗下拉选择，叠加在 config.toml 之上，立即生效
    processor: Arc<processor_settings::ProcessorSettings>,
}

// ---------------------------------------------------------------------------
// 命令层
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct TaxonomyOut {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) domain: String,
}

impl TaxonomyOut {
    /// rsrs 分类树根 → 前端 DTO
    pub(crate) fn from_item(t: TaxonomyItem) -> Self {
        Self {
            id: t.id,
            name: t.name,
            domain: t.domain.unwrap_or_default(),
        }
    }
}

/// 拉取 23 分类树根（人域 / AI域）
#[tauri::command]
fn get_taxonomy(state: tauri::State<'_, AppState>) -> Result<Vec<TaxonomyOut>, String> {
    let items: Vec<TaxonomyItem> = state
        .client
        .list_taxonomy()
        .map_err(|e| format_rsrs_err(&e))?;
    Ok(items.into_iter().map(TaxonomyOut::from_item).collect())
}

#[derive(Serialize)]
struct SubmitResult {
    ok: bool,
    action: Option<String>,
    id: Option<String>,
    parent: Option<String>,
    title: Option<String>,
    message: String,
    /// 候选悬空时回传候选，供 UI 择一（改/并/挂/存）
    candidates: Vec<CandidateOut>,
    /// 是否需要用户择一
    needs_confirm: bool,
    /// 是否需要用户补 `【触发】`（task 类型且未填）
    needs_trigger: bool,
    warnings: Vec<String>,
}

impl SubmitResult {
    fn fail(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            action: None,
            id: None,
            parent: None,
            title: None,
            message: message.into(),
            candidates: Vec::new(),
            needs_confirm: false,
            needs_trigger: false,
            warnings: Vec::new(),
        }
    }
}

/// 单条正文长度上限（超长请拆分；防止误粘整篇文档）
const MAX_CONTENT_CHARS: usize = 8000;

/// 手动路径提交：
/// - target = "journal" → 日记一步落库（--importance trivial，不判树不挂树）
/// - target = "main"   → 主区带 parent 挂树根（--title --parent --type --importance important）
/// - 主区自动补三标（无标记时 recall 会截断）
/// - type = task 必须带【触发】：缺失时回 needs_trigger，由 UI 询问，禁猜日期
/// - 判重命中（候选悬空）→ 回传候选供 UI 择一，禁止当作成功
#[tauri::command]
fn submit_memory(
    state: tauri::State<'_, AppState>,
    target: String,
    content: String,
    tree: Option<String>,
    r#type: Option<String>,
    trigger: Option<String>,
) -> Result<SubmitResult, String> {
    let mut content = content.trim().to_string();
    if content.is_empty() {
        return Ok(SubmitResult::fail("内容为空，请先输入要记录的内容。"));
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Ok(SubmitResult::fail(format!(
            "内容过长（{} 字，上限 {} 字）：请拆分后分条记录。",
            content.chars().count(),
            MAX_CONTENT_CHARS
        )));
    }

    let mut warnings: Vec<String> = Vec::new();
    let mut req = RememberRequest::default();

    let summary_hint = if target == "journal" {
        req.importance = Some("trivial".into());
        "日记".to_string()
    } else {
        // 主区：必须带树根
        let tree_name = match tree {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return Ok(SubmitResult::fail("主区需要选择一棵分类树。")),
        };
        let ty = r#type
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "context".into());

        // 三标补全（本地降级：不改写事实，只按段落结构补标记）
        let (norm, w) = automation::normalize_sections(&content);
        content = norm;
        warnings.extend(w);

        // task 必须带【触发】：日期/前提缺失时回 UI 询问，不推断
        if ty == "task" && !content.contains("【触发】") {
            let t = trigger.unwrap_or_default().trim().to_string();
            if t.is_empty() {
                let mut r = SubmitResult::fail(
                    "task 类型必须写【触发】行：请填写确认的日期（YYYY-MM-DD）或可核实前提，不要推测。",
                );
                r.needs_trigger = true;
                return Ok(r);
            }
            let t = t.strip_prefix("【触发】").unwrap_or(&t).trim().to_string();
            if !has_verifiable_date(&t) {
                warnings.push("【触发】未含 YYYY-MM-DD 日期，请确认其前提可核实。".into());
            }
            content.push_str(&format!("\n【触发】{t}"));
        }

        let title = build_title(&content);
        req.title = Some(title);
        req.parent = Some(tree_name.clone());
        req.r#type = Some(ty);
        req.importance = Some("important".into());
        format!("主区（挂 {}", tree_name)
    };

    req.content = content.clone();
    let res = state.client.remember(req).map_err(|e| format_rsrs_err(&e))?;

    // 候选悬空：返回判重候选但未写入 —— 回传候选供 UI 择一，禁止当作成功
    let candidates: Vec<CandidateOut> = res
        .pending
        .iter()
        .map(|c| CandidateOut {
            id: c.id.clone(),
            title: c.title.clone(),
            score: c.similarity.map(|s| s / 100.0).unwrap_or(0.0),
            source: "remember".into(),
        })
        .collect();

    if res.is_dangling() {
        return Ok(SubmitResult {
            ok: false,
            action: None,
            id: None,
            parent: None,
            title: None,
            message: if candidates.is_empty() {
                "rsrs 返回判重候选但未写入（候选悬空）。请改/并/挂或存新条。".into()
            } else {
                format!(
                    "判重命中 {} 条相似记忆但未写入，请择一：改 / 并 / 挂 / 存新条。",
                    candidates.len()
                )
            },
            candidates,
            needs_confirm: true,
            needs_trigger: false,
            warnings: res.warnings.clone(),
        });
    }

    let summary = res.summary.clone().unwrap_or_default();
    let mut msg = format!("{}）已落库，共 {} 条。", summary_hint, res.count.unwrap_or(1));
    if let Some(t) = summary.title.as_deref() {
        if !t.is_empty() {
            msg.push_str(&format!("\n标题：{t}"));
        }
    }
    for w in &warnings {
        msg.push_str(&format!("\n· {w}"));
    }
    if !res.warnings.is_empty() {
        msg.push_str("\n（提示：rsrs 插件 hook 告警为非致命，已忽略）");
    }
    Ok(SubmitResult {
        ok: true,
        action: summary.action,
        id: summary.id,
        parent: summary.parent,
        title: summary.title,
        message: msg,
        candidates: Vec::new(),
        needs_confirm: false,
        needs_trigger: false,
        warnings: res.warnings,
    })
}

/// 是否含可核实日期（YYYY-MM-DD）
fn has_verifiable_date(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 10 {
        return false;
    }
    chars.windows(10).any(|w| {
        w[0..4].iter().all(|c| c.is_ascii_digit())
            && w[4] == '-'
            && w[5..7].iter().all(|c| c.is_ascii_digit())
            && w[7] == '-'
            && w[8..10].iter().all(|c| c.is_ascii_digit())
    })
}

/// 前端需要的展示配置
#[derive(Serialize)]
struct AppConfigOut {
    hotkey: String,
    submit_close: bool,
    port: u16,
    automation_enabled: bool,
    automation_mode: String,
    llm_enabled: bool,
    reminder_enabled: bool,
    /// 梳理（P10）是否启用 / 端点 / 模型
    processor_enabled: bool,
    processor_endpoint: String,
    processor_model: String,
    /// 梳理来源：`llm` 或 agent kind（`opencode` / `qoder`）
    processor_source: String,
    /// agent 路径是否启用（层次二）
    processor_agent_enabled: bool,
    /// agent kind / 可执行文件 / 模型（界面显示用）
    processor_agent_kind: String,
    processor_agent_bin: String,
    processor_agent_model: String,
    /// 界面选择落盘位置（`processor.json`）
    processor_choice_path: String,
}

#[tauri::command]
fn get_app_config(state: tauri::State<'_, AppState>) -> AppConfigOut {
    let proc_cfg = state.processor.effective(&state.config.processor);
    AppConfigOut {
        hotkey: state.config.hotkey.register.clone(),
        submit_close: state.config.window.submit_close,
        port: state.config.rsrs.port(),
        automation_enabled: state.config.automation.enabled,
        automation_mode: state.config.automation.mode.clone(),
        llm_enabled: state.config.automation.llm.enabled,
        reminder_enabled: state.config.reminder.enabled,
        processor_enabled: proc_cfg.enabled,
        processor_endpoint: proc_cfg.endpoint.clone(),
        processor_model: proc_cfg.model.clone(),
        processor_source: proc_cfg.source().to_string(),
        processor_agent_enabled: proc_cfg.agent_enabled(),
        processor_agent_kind: proc_cfg.agent.kind.clone(),
        processor_agent_bin: proc_cfg.agent.bin.clone(),
        processor_agent_model: proc_cfg.agent.model.clone(),
        processor_choice_path: state.processor.path().display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// 梳理来源（P10 层次二）：界面下拉切换，立即生效、无需改 config.toml / 重启
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SourceOptionOut {
    id: String,
    label: String,
    hint: String,
}

#[derive(Serialize)]
struct LlmPartOut {
    endpoint: String,
    model: String,
    /// 端点是否已配置（仅看配置，不发起请求）
    configured: bool,
}

#[derive(Serialize)]
struct AgentPartOut {
    kind: String,
    bin: String,
    model: String,
    timeout_secs: u64,
}

#[derive(Serialize)]
struct ProcessorStatusOut {
    /// 梳理总开关（关闭时点任务会明确提示）
    enabled: bool,
    /// `off` | `llm` | `opencode` | `qoder`
    source: String,
    source_label: String,
    /// true = 界面选择；false = 跟随 config.toml
    overridden: bool,
    llm: LlmPartOut,
    agent: AgentPartOut,
    /// 可选来源（按钮顺序）
    options: Vec<SourceOptionOut>,
    choice_path: String,
    config_path: String,
}

fn source_label(id: &str) -> &'static str {
    match id {
        "off" => "关闭",
        "llm" => "模型",
        "opencode" => "opencode",
        "qoder" => "Qoder",
        _ => "未知",
    }
}

/// 界面来源：显式选择优先；否则按 config.toml（enabled=false 视为 off）
fn effective_source_id(
    choice: &processor_settings::ProcessorChoice,
    cfg: &config::ProcessorConfig,
) -> String {
    if !choice.source.is_empty() {
        return choice.source.clone();
    }
    if !cfg.enabled {
        return "off".into();
    }
    cfg.source().to_string()
}

fn build_processor_status(state: &AppState) -> ProcessorStatusOut {
    let choice = state.processor.get();
    let proc_cfg = state.processor.effective(&state.config.processor);
    let source = effective_source_id(&choice, &state.config.processor);
    let options = vec![
        SourceOptionOut {
            id: "llm".into(),
            label: "模型".into(),
            hint: "直连 OpenAI 兼容端点（config.toml [processor] endpoint）".into(),
        },
        SourceOptionOut {
            id: "opencode".into(),
            label: "opencode".into(),
            hint: "本地 opencode headless，可读项目上下文，较慢（20–40s）".into(),
        },
        SourceOptionOut {
            id: "qoder".into(),
            label: "Qoder".into(),
            hint: "本地 Qoder CLI，需先执行 qodercli /login 登录".into(),
        },
        SourceOptionOut {
            id: "off".into(),
            label: "关闭".into(),
            hint: "关闭梳理，任务按钮会提示未启用".into(),
        },
    ];
    ProcessorStatusOut {
        enabled: proc_cfg.enabled,
        source_label: source_label(&source).to_string(),
        source,
        overridden: !choice.source.is_empty(),
        llm: LlmPartOut {
            configured: !proc_cfg.endpoint.trim().is_empty(),
            endpoint: proc_cfg.endpoint.clone(),
            model: proc_cfg.model.clone(),
        },
        agent: AgentPartOut {
            kind: proc_cfg.agent.kind.clone(),
            bin: proc_cfg.agent.bin.clone(),
            model: proc_cfg.agent.model.clone(),
            timeout_secs: proc_cfg.agent.timeout_secs,
        },
        options,
        choice_path: state.processor.path().display().to_string(),
        config_path: config::AppConfig::path().display().to_string(),
    }
}

#[tauri::command]
fn processor_status(state: tauri::State<'_, AppState>) -> ProcessorStatusOut {
    build_processor_status(&state)
}

/// 切换来源（`off` / `llm` / `opencode` / `qoder`）；可选覆盖模型与 bin
#[tauri::command]
fn processor_set_source(
    state: tauri::State<'_, AppState>,
    source: String,
    model: Option<String>,
    agent_model: Option<String>,
    agent_bin: Option<String>,
) -> Result<ProcessorStatusOut, String> {
    let mut choice = state.processor.get();
    choice.source = source;
    if let Some(v) = model {
        choice.model = v;
    }
    if let Some(v) = agent_model {
        choice.agent_model = v;
    }
    if let Some(v) = agent_bin {
        choice.agent_bin = v;
    }
    state.processor.set(choice)?;
    Ok(build_processor_status(&state))
}

/// 回到 config.toml 的默认来源
#[tauri::command]
fn processor_reset_source(
    state: tauri::State<'_, AppState>,
) -> Result<ProcessorStatusOut, String> {
    state.processor.reset()?;
    Ok(build_processor_status(&state))
}

#[derive(Serialize)]
struct AgentProbeOut {
    ok: bool,
    kind: String,
    bin: String,
    version: String,
    message: String,
}

/// 探测当前来源的 agent CLI 是否可用（本地执行，约 1s；不会发起梳理请求）
#[tauri::command]
fn processor_probe(state: tauri::State<'_, AppState>) -> AgentProbeOut {
    let proc_cfg = state.processor.effective(&state.config.processor);
    let kind = proc_cfg.agent.kind.clone();
    if !proc_cfg.agent_enabled() {
        return AgentProbeOut {
            ok: false,
            kind,
            bin: String::new(),
            version: String::new(),
            message: "当前来源不是 agent（模型 / 关闭）".into(),
        };
    }
    match agent::probe(&proc_cfg.agent) {
        Ok((kind, bin, version)) => AgentProbeOut {
            ok: true,
            kind: kind.as_str().to_string(),
            bin: bin.clone(),
            version,
            message: format!("{} 可用：{bin}", kind.as_str()),
        },
        Err(e) => AgentProbeOut {
            ok: false,
            kind,
            bin: String::new(),
            version: String::new(),
            message: e,
        },
    }
}

/// 浏览器扩展接入信息（端口 / 令牌 / 令牌文件路径）
#[derive(Serialize)]
struct ExtensionInfo {
    enabled: bool,
    port: u16,
    token: String,
    token_path: String,
}

#[tauri::command]
fn extension_info(app: AppHandle, state: tauri::State<'_, AppState>) -> ExtensionInfo {
    let token_path = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("extension_token.txt");
    let endpoint = app.try_state::<server::Started>();
    let configured = state.config.extension.token.trim();
    let token = if !configured.is_empty() {
        configured.to_string()
    } else {
        std::fs::read_to_string(&token_path)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    ExtensionInfo {
        enabled: endpoint.is_some(),
        port: endpoint.as_ref().map(|s| s.port).unwrap_or(state.config.extension.port),
        token: endpoint.as_ref().map(|s| s.token.clone()).unwrap_or(token),
        token_path: token_path.display().to_string(),
    }
}

#[derive(Serialize)]
struct ExtensionInstallOut {
    /// 私有扩展目录：来自本机用户配置目录，完整显示以便 Chrome 选择，不包含令牌值。
    directory: String,
    /// 安装指引：来自打开安装页的结果；失败保留可操作说明，不返回空提示。
    message: String,
}

/// 用户点击后导出私有扩展并打开安装页；不执行静默安装。
#[tauri::command]
fn extension_install(app: AppHandle) -> Result<ExtensionInstallOut, String> {
    // 1. 必须使用本次已成功监听的端点和令牌，避免端口占用时导出错误配置。
    let endpoint = app
        .try_state::<server::Started>()
        .ok_or("扩展端点未启动：请检查配置中的 extension.enabled 和端口占用。")?;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("chrome-extension");
    let directory = extension_setup::prepare(&dir, endpoint.port, &endpoint.token)?;
    // 2. 浏览器打开失败仍返回准备好的目录，用户可手动完成 Chrome 的加载确认。
    let result = extension_setup::open_install_ui(&directory);
    Ok(ExtensionInstallOut {
        directory: directory.display().to_string(),
        message: result.err().unwrap_or_else(|| "开启开发者模式，点击加载已解压的扩展并选择此目录；已有扩展请点重新加载。配对会自动完成。".into()),
    })
}

/// runtime 连通性 + 离线/同步状态（只读探测，不启停服务）
#[derive(Serialize)]
struct RuntimeStatus {
    online: bool,
    message: String,
    offline: Option<bool>,
    autosync: Option<bool>,
    pending: u64,
    conflicts: u64,
    last_sync: Option<i64>,
    phase: Option<String>,
}

#[tauri::command]
fn runtime_status(state: tauri::State<'_, AppState>) -> RuntimeStatus {
    match state.client.status_info() {
        Ok(s) => {
            let offline = s.offline.unwrap_or(false);
            let pending = s.pending;
            let message = if offline {
                format!("rsrs runtime 离线：{pending} 条待同步（离线不丢，下次 sync 补传）")
            } else if pending > 0 {
                format!("rsrs runtime 在线：{pending} 条待同步")
            } else {
                "rsrs runtime 在线".into()
            };
            RuntimeStatus {
                online: true,
                message,
                offline: s.offline,
                autosync: s.autosync,
                pending,
                conflicts: s.conflicts,
                last_sync: s.last_sync,
                phase: s.phase,
            }
        }
        Err(e) => RuntimeStatus {
            online: false,
            message: format_rsrs_err(&e),
            offline: None,
            autosync: None,
            pending: 0,
            conflicts: 0,
            last_sync: None,
            phase: None,
        },
    }
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    save_window_position(&app);
    app.exit(0);
}

// ---------------------------------------------------------------------------
// 全局划词悬浮（P8）
// ---------------------------------------------------------------------------

/// 读取划词结果；文字未就位时当场取词（点击图标才注入，最长阻塞约 capture_timeout_ms）
#[tauri::command]
async fn selection_payload(
    sel: tauri::State<'_, Arc<selection::SelectionState>>,
) -> Result<Option<selection::SelectionPayload>, String> {
    let state = sel.inner().clone();
    Ok(
        tauri::async_runtime::spawn_blocking(move || selection::capture_on_click(&state))
            .await
            .ok()
            .flatten(),
    )
}

#[tauri::command]
fn selection_expand(
    app: AppHandle,
    sel: tauri::State<'_, Arc<selection::SelectionState>>,
    expanded: bool,
    height: Option<i32>,
) -> Result<(), String> {
    selection::resize_selector(&app, sel.inner(), expanded)
}

#[tauri::command]
fn selection_hide(app: AppHandle, sel: tauri::State<'_, Arc<selection::SelectionState>>) {
    selection::hide_selector(&app, sel.inner());
}

/// 打开主窗并聚焦；带文字时预填正文（划词面板与扩展 `/open` 共用）
pub fn open_main_window(app: &AppHandle, text: Option<&str>) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let _ = win.show();
    let _ = win.set_focus();
    if let Some(t) = text.map(str::trim).filter(|s| !s.is_empty()) {
        let _ = win.emit("selection-prefill", t);
    }
}

#[tauri::command]
fn selection_open_main(
    app: AppHandle,
    sel: tauri::State<'_, Arc<selection::SelectionState>>,
    text: Option<String>,
    memory_id: Option<String>,
) {
    let target = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    open_main_window(&app, target.as_deref());
    // 保存回执的实际 ID 只用于定位已存记录，不预填或覆盖未提交输入。
    if let Some(id) = memory_id.filter(|id| id.len() >= 8 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')) {
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.emit("selection-view-memory", id);
        }
    }
    selection::hide_selector(&app, sel.inner());
}

#[derive(Serialize)]
struct QuickSave {
    ok: bool,
    id: Option<String>,
    message: String,
    /// 落库标题（快捷目标挂树时由正文生成）
    title: Option<String>,
    /// 实际执行的分支：journal | create | auto
    branch: Option<String>,
    /// 需用户核对（判重候选悬空 / 破坏性分支降级为挂）
    needs_confirm: bool,
}

impl QuickSave {
    fn from_quick(o: quick::QuickOutcome) -> Self {
        let message = if o.needs_confirm {
            format!("{}已挂到相似条目下，请到 memTools 核对。", o.message)
        } else {
            o.message
        };
        Self {
            ok: o.ok,
            id: o.id,
            message,
            title: o.title,
            branch: Some(o.branch),
            needs_confirm: o.needs_confirm,
        }
    }
}

#[tauri::command]
fn selection_journal(state: tauri::State<'_, AppState>, text: String) -> Result<QuickSave, String> {
    let content = text.trim().to_string();
    if content.is_empty() {
        return Err("选中内容为空".into());
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err(format!(
            "内容过长（{} 字，上限 {} 字），请到 memTools 中分段记录。",
            content.chars().count(),
            MAX_CONTENT_CHARS
        ));
    }
    let req = RememberRequest {
        content,
        importance: Some("trivial".into()),
        ..Default::default()
    };
    let res = state.client.remember(req).map_err(|e| format_rsrs_err(&e))?;
    if res.is_dangling() {
        return Ok(QuickSave {
            ok: false,
            id: None,
            message: "rsrs 返回判重候选但未写入（候选悬空），请到 memTools 中择一处理。".into(),
            title: None,
            branch: Some("journal".into()),
            needs_confirm: true,
        });
    }
    let id = res.summary.as_ref().and_then(|s| s.id.clone());
    Ok(QuickSave {
        ok: true,
        id,
        message: format!("已存为日记（{} 条）", res.count.unwrap_or(1)),
        title: None,
        branch: Some("journal".into()),
        needs_confirm: false,
    })
}

// ---------------------------------------------------------------------------
// 快捷目标（P9）
// ---------------------------------------------------------------------------

/// 快捷目标（出参）：`type` 字段序列化后为 "type"
#[derive(Serialize, Clone)]
pub(crate) struct TargetOut {
    id: String,
    label: String,
    kind: String,
    parent: Option<String>,
    r#type: Option<String>,
    importance: Option<String>,
}

impl TargetOut {
    fn from_target(t: &targets::QuickTarget) -> Self {
        Self {
            id: t.id.clone(),
            label: t.label.clone(),
            kind: t.kind.clone(),
            parent: t.parent.clone(),
            r#type: t.r#type.clone(),
            importance: t.importance.clone(),
        }
    }
}

/// 快捷目标（入参）
#[derive(Deserialize)]
pub(crate) struct TargetIn {
    #[serde(default)]
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    importance: Option<String>,
}

impl TargetIn {
    fn into_target(self) -> targets::QuickTarget {
        targets::QuickTarget {
            id: self.id,
            label: self.label,
            kind: self.kind,
            parent: self.parent,
            r#type: self.r#type,
            importance: self.importance,
        }
    }
}

/// 快捷目标列表响应
#[derive(Serialize)]
struct TargetsOut {
    ok: bool,
    targets: Vec<TargetOut>,
    taxonomy: Vec<TaxonomyOut>,
    message: String,
}

/// 分类树根（走缓存 + 有界超时；取不到时给空数组，不让整个命令失败）
fn taxonomy_out(state: &AppState) -> Vec<TaxonomyOut> {
    state
        .client
        .list_taxonomy()
        .unwrap_or_default()
        .into_iter()
        .map(TaxonomyOut::from_item)
        .collect()
}

/// 读取快捷目标列表
#[tauri::command]
fn quick_targets_list(state: tauri::State<'_, AppState>) -> TargetsOut {
    TargetsOut {
        ok: true,
        targets: state.targets.lock().unwrap().list().iter().map(TargetOut::from_target).collect(),
        taxonomy: taxonomy_out(&state),
        message: String::new(),
    }
}

/// 保存快捷目标列表（失败不回异常，把原因带给前端显示）
#[tauri::command]
fn quick_targets_save(state: tauri::State<'_, AppState>, targets: Vec<TargetIn>) -> TargetsOut {
    let incoming: Vec<targets::QuickTarget> = targets.into_iter().map(TargetIn::into_target).collect();
    match state.targets.lock().unwrap().replace(incoming) {
        Ok(list) => {
            let count = list.len();
            TargetsOut {
                ok: true,
                targets: list.iter().map(TargetOut::from_target).collect(),
                taxonomy: taxonomy_out(&state),
                message: format!("已保存 {count} 个快捷目标"),
            }
        }
        Err(e) => TargetsOut {
            ok: false,
            targets: Vec::new(),
            taxonomy: Vec::new(),
            message: e,
        },
    }
}

/// 恢复默认快捷目标（日记 → 自动判档）
#[tauri::command]
fn quick_targets_reset(state: tauri::State<'_, AppState>) -> TargetsOut {
    match state.targets.lock().unwrap().reset() {
        Ok(list) => TargetsOut {
            ok: true,
            targets: list.iter().map(TargetOut::from_target).collect(),
            taxonomy: taxonomy_out(&state),
            message: "已恢复默认快捷目标".into(),
        },
        Err(e) => TargetsOut {
            ok: false,
            targets: Vec::new(),
            taxonomy: Vec::new(),
            message: e,
        },
    }
}

/// 划词面板用的快捷目标列表（竖排一列，第 1 个最优先）
#[tauri::command]
fn selection_targets(state: tauri::State<'_, AppState>) -> Vec<TargetOut> {
    state.targets.lock().unwrap().list().iter().map(TargetOut::from_target).collect()
}

/// 按快捷目标保存划词内容
#[tauri::command]
fn selection_save_target(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
    target_id: String,
) -> Result<QuickSave, String> {
    let content = text.trim().to_string();
    if content.is_empty() {
        return Err("选中内容为空".into());
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err(format!(
            "内容过长（{} 字，上限 {} 字），请到 memTools 中分段记录。",
            content.chars().count(),
            MAX_CONTENT_CHARS
        ));
    }
    let target = state
        .targets
        .lock()
        .unwrap()
        .get(&target_id)
        .ok_or_else(|| format!("目标不存在：{target_id}"))?;
    let outcome = quick::save_to_target(
        &state.client,
        &state.config,
        &automation_backup_dir(&app),
        &target,
        &content,
        None,
    )
    .map_err(|e| e)?;
    Ok(QuickSave::from_quick(outcome))
}

// ---------------------------------------------------------------------------
// 自动化路径（P2）
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AutomationDto {
    /// plan | done
    phase: String,
    /// journal | attach | create | update | merge | confirm
    branch: String,
    needs_confirm: bool,
    ok: bool,
    importance: String,
    title: Option<String>,
    action: Option<String>,
    id: Option<String>,
    parent: Option<String>,
    message: String,
    warnings: Vec<String>,
    candidates: Vec<CandidateOut>,
    /// model | local（判档来源）
    source: String,
}

impl AutomationDto {
    /// 把告警（补三标提示、hook 非致命告警等）并入展示信息
    fn with_warnings(mut message: String, warnings: &[String]) -> String {
        for w in warnings {
            message.push_str(&format!("\n· {w}"));
        }
        message
    }

    fn from_plan(p: automation::Plan) -> Self {
        let message = Self::with_warnings(p.message, &p.warnings);
        Self {
            phase: "plan".into(),
            branch: p.branch,
            needs_confirm: p.needs_confirm,
            ok: !p.needs_confirm,
            importance: p.importance,
            title: Some(p.title),
            action: None,
            id: None,
            parent: p.suggested_parent,
            message,
            warnings: p.warnings,
            candidates: p.candidates,
            source: p.source,
        }
    }

    fn from_outcome(o: automation::Outcome) -> Self {
        let message = Self::with_warnings(o.message, &o.warnings);
        Self {
            phase: "done".into(),
            branch: o.branch,
            needs_confirm: o.needs_confirm,
            ok: o.ok,
            importance: o.importance,
            title: o.title,
            action: o.action,
            id: o.id,
            parent: o.parent,
            message,
            warnings: o.warnings,
            candidates: o.candidates,
            source: "local".into(),
        }
    }
}

/// 备份目录（改/并 前落盘，防不可逆）：{app_config_dir}/backups
fn automation_backup_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("backups")
}

/// 自动化提交：判档 → 判重（两轮）→ 判树 → 非破坏性分支自动执行；破坏性分支回 plan 待确认
#[tauri::command]
fn automation_submit(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
    content: String,
    hint: Option<String>,
) -> Result<AutomationDto, String> {
    let mut content = content.trim().to_string();
    if content.is_empty() {
        return Err("内容为空，请先输入要记录的内容。".into());
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err(format!(
            "内容过长（{} 字，上限 {} 字）：请拆分后分条记录。",
            content.chars().count(),
            MAX_CONTENT_CHARS
        ));
    }
    let taxonomy = state
        .client
        .list_taxonomy()
        .map_err(|e| format_rsrs_err(&e))?;
    let cfg = state.config.automation.clone();

    // 模型判档 / 判树（P2 模型路径）：失败回退本地并告警，不静默
    let mut model_warnings = Vec::new();
    let verdict = if cfg.mode != "local-fallback" && cfg.llm.enabled {
        let trees: Vec<String> = taxonomy.iter().map(|t| t.name.clone()).collect();
        match llm::judge(&cfg.llm, &content, hint.as_deref(), &trees) {
            Ok(v) => Some(v),
            Err(e) => {
                model_warnings.push(format!("模型判档失败，已回退本地：{e}"));
                None
            }
        }
    } else {
        if cfg.mode != "local-fallback" {
            model_warnings.push(format!(
                "automation.mode={} 但 [automation.llm] enabled=false，已用本地判档。",
                cfg.mode
            ));
        }
        None
    };

    // 判档在规范化之前（避免自由文本被补成【行为】后一律判 important）；
    // 判为 important 才补三标，日记保持原文。
    let prelim = verdict
        .as_ref()
        .map(|v| v.importance.clone())
        .filter(|s| s == "important" || s == "trivial")
        .unwrap_or_else(|| automation::judge_importance(&content).to_string());
    let mut norm_warnings = Vec::new();
    if prelim != "trivial" {
        let (norm, w) = automation::normalize_sections(&content);
        content = norm;
        norm_warnings = w;
    }

    let mut plan = automation::plan_with(
        &state.client,
        &content,
        hint.as_deref(),
        &taxonomy,
        &cfg,
        verdict.as_ref(),
    )
    .map_err(|e| format_rsrs_err(&e))?;
    plan.warnings.extend(norm_warnings);
    plan.warnings.extend(model_warnings);

    if plan.needs_confirm {
        return Ok(AutomationDto::from_plan(plan));
    }
    let outcome = automation::apply(
        &state.client,
        &content,
        &plan.title,
        &plan.branch,
        plan.suggested_parent.as_deref(),
        plan.suggested_tree.as_deref(),
        &plan.importance,
        &automation_backup_dir(&app),
    )
    .map_err(|e| format_rsrs_err(&e))?;

    // 存后自评：被采纳的候选标 good，其余候选标 bad（best-effort）
    let chosen: Vec<String> = match plan.branch.as_str() {
        "attach" | "update" => plan.suggested_parent.clone().into_iter().collect(),
        _ => Vec::new(),
    };
    let mut dto = AutomationDto::from_outcome(outcome);
    dto.source = plan.source.clone();
    if let Err(w) = grade_candidates(&state.client, &plan.candidates, &chosen) {
        dto.warnings.push(w);
    }
    Ok(dto)
}

/// query-log 自评：chosen → good，其余候选 → bad；失败只告警
pub(crate) fn grade_candidates(
    client: &RsrsClient,
    candidates: &[CandidateOut],
    chosen: &[String],
) -> Result<(), String> {
    if candidates.is_empty() {
        return Ok(());
    }
    let mut good = Vec::new();
    let mut bad = Vec::new();
    for c in candidates {
        if chosen.iter().any(|x| x == &c.id) {
            good.push(c.id.clone());
        } else {
            bad.push(c.id.clone());
        }
    }
    if let Err(e) = client.query_log_mark(&good, true) {
        return Err(format!("query-log 自评（good）失败：{e}"));
    }
    if let Err(e) = client.query_log_mark(&bad, false) {
        return Err(format!("query-log 自评（bad）失败：{e}"));
    }
    Ok(())
}

/// 用户择一执行：update（改）/ merge（并）/ attach（挂）/ create（存）
#[tauri::command]
fn automation_resolve(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
    content: String,
    choice: String,
    candidate: Option<String>,
    tree: Option<String>,
) -> Result<AutomationDto, String> {
    let mut content = content.trim().to_string();
    if content.is_empty() {
        return Err("内容为空，请先输入要记录的内容。".into());
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err(format!(
            "内容过长（{} 字，上限 {} 字）：请拆分后分条记录。",
            content.chars().count(),
            MAX_CONTENT_CHARS
        ));
    }
    let branch = match choice.as_str() {
        "update" | "merge" | "attach" | "create" => choice.clone(),
        other => return Err(format!("未知选择: {other}")),
    };
    let importance = automation::judge_importance(&content).to_string();
    if importance != "trivial" {
        let (norm, _) = automation::normalize_sections(&content);
        content = norm;
    }
    let title = automation::build_title(&content);
    let mut outcome = automation::apply(
        &state.client,
        &content,
        &title,
        &branch,
        candidate.as_deref(),
        tree.as_deref(),
        &importance,
        &automation_backup_dir(&app),
    )
    .map_err(|e| format_rsrs_err(&e))?;

    // 存后自评：用户选中的候选标 good（其余候选在 plan 阶段已不可得，交由下次召回更新）
    let chosen: Vec<String> = match branch.as_str() {
        "merge" => candidate
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        "update" | "attach" => candidate.clone().into_iter().collect(),
        _ => Vec::new(),
    };
    if let Err(e) = state.client.query_log_mark(&chosen, true) {
        outcome
            .warnings
            .push(format!("query-log 自评（good）失败：{e}"));
    }
    Ok(AutomationDto::from_outcome(outcome))
}

// ---------------------------------------------------------------------------
// 内容梳理（P10 层次一：直接调 LLM）
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProcessResult {
    ok: bool,
    task: Option<String>,
    /// 处理后的正文（标题任务为压平后的标题）
    content: String,
    /// 实际来源：`llm` 或 agent kind（`opencode` / `qoder`）
    source: String,
    message: String,
}

/// 梳理内容：总结 / 结构化 / 起标题 / 翻译 / 提取 / 润色。
///
/// 只返回处理结果，不写库；由前端回填输入框后再走自动化 / 手动落库。
/// 来源由配置决定：`[processor] agent_endpoint` / `[processor.agent] enabled` → agent，
/// 否则直连 OpenAI 兼容 LLM。
#[tauri::command]
fn process_memory(
    state: tauri::State<'_, AppState>,
    content: String,
    task: String,
    lang: Option<String>,
) -> Result<ProcessResult, String> {
    let task = processor::Task::parse(&task)
        .ok_or_else(|| format!("未知任务：{}", task.trim()))?;
    let proc_cfg = state.processor.effective(&state.config.processor);
    let out = processor::process(&proc_cfg, &content, &task, lang.as_deref())?;
    let source = proc_cfg.source().to_string();
    Ok(ProcessResult {
        ok: true,
        task: Some(task.as_str().to_string()),
        content: out,
        message: format!("已{}（{} · {source}）", task.label(), task.as_str()),
        source,
    })
}

// ---------------------------------------------------------------------------
// 最近记忆浏览（P3.5）
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RecentItemOut {
    id: String,
    title: String,
    content: String,
    kind: Option<String>,
    importance: Option<String>,
    parent_id: Option<String>,
    created_at: Option<String>,
    /// server 按 updated_at 倒序（最近活跃），UI 以它为主时间
    updated_at: Option<String>,
}

impl From<rsrs_client::ListedEntry> for RecentItemOut {
    fn from(e: rsrs_client::ListedEntry) -> Self {
        Self {
            id: e.id,
            title: e.title,
            content: e.content,
            kind: e.kind,
            importance: e.importance,
            parent_id: e.parent_id,
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}

#[derive(Serialize)]
struct RecentOut {
    items: Vec<RecentItemOut>,
    /// 完整快照中的有效记忆数（含日记）
    scanned: u32,
    /// 兼容字段：完整读取成功时固定 false；不完整读取会明确报错
    truncated: bool,
    /// 实际生效的分类（None = 全部）
    category: Option<String>,
}

/// 最近记忆：limit=0 为全部有效记忆；分类按已读取的真实父链筛选，空分类返回空状态。
#[tauri::command]
async fn list_recent(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
    category: Option<String>,
) -> Result<RecentOut, String> {
    let client = state.client.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 1. 只读取得完整账号快照，网络等待不占用桌面界面线程。
        let entries = client.list_all().map_err(|e| format_rsrs_err(&e))?;
        let scanned = entries.len() as u32;
        let category = category
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty() && name != "全部" && name != "all");
        let requested = limit.unwrap_or(20);
        // 2. 先按真实祖先筛选，再限制最近数量；0 不截断，扫描完整时不存在采样截断。
        let items = recent::filter_category(
            entries,
            category.as_deref(),
            (requested != 0).then_some(requested as usize),
        );
        Ok(RecentOut {
            items: items.into_iter().map(Into::into).collect(),
            scanned,
            truncated: false,
            category,
        })
    })
    .await
    .map_err(|e| format!("记忆读取线程失败：{e}"))?
}

// ---------------------------------------------------------------------------
// 记忆搜索（id 精确 / recall 语义）
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SearchItemOut {
    id: String,
    title: String,
    content: String,
    kind: Option<String>,
    importance: Option<String>,
    parent_id: Option<String>,
    /// recall 语义相似度 0..1（id 精确命中为 None）
    score: Option<f64>,
    /// 已被新条取代时的新条 id
    superseded_by: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct SearchOut {
    /// id | recall
    mode: String,
    query: String,
    items: Vec<SearchItemOut>,
    /// 附加说明（未找到 / 祖先路径 / 空查询提示）
    message: Option<String>,
}

/// id 形态：8 位前缀或完整 UUID（十六进制 + 连字符）
fn looks_like_memory_id(s: &str) -> bool {
    let t = s.trim();
    (8..=36).contains(&t.len())
        && t.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && t.chars().filter(|c| c.is_ascii_hexdigit()).count() >= 8
}

/// 搜索记忆：
/// - 输入像 id（如 `bd22ad3a`）→ memory_show 精确命中（支持 8 位前缀），附祖先路径；
/// - 否则 → memory_recall 语义匹配（rsrs recall 原则），返回相似度与 superseded 标记。
#[tauri::command]
fn search_memory(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<SearchOut, String> {
    let q = query.trim().to_string();
    let limit = limit.unwrap_or(20).clamp(1, 100);
    if q.is_empty() {
        return Ok(SearchOut {
            mode: "recall".into(),
            query: q,
            items: Vec::new(),
            message: Some("请输入要搜索的内容，或粘贴 8 位 id。".into()),
        });
    }

    if looks_like_memory_id(&q) {
        return match state.client.show_entry(&q) {
            Ok(shown) => {
                let path = shown
                    .ancestors
                    .iter()
                    .map(|n| n.title.clone())
                    .collect::<Vec<_>>()
                    .join(" / ");
                let item = SearchItemOut {
                    id: shown.entry.id,
                    title: shown.entry.title,
                    content: shown.entry.content,
                    kind: shown.entry.kind,
                    importance: shown.entry.importance,
                    parent_id: shown.entry.parent_id,
                    score: None,
                    superseded_by: None,
                    created_at: None,
                    updated_at: None,
                };
                Ok(SearchOut {
                    mode: "id".into(),
                    query: q,
                    items: vec![item],
                    message: Some(if path.is_empty() {
                        "id 精确命中".into()
                    } else {
                        format!("id 精确命中 · 路径：{path}")
                    }),
                })
            }
            Err(e) => {
                let msg = e.to_string();
                let lower = msg.to_lowercase();
                if lower.contains("no such entry")
                    || lower.contains("not found")
                    || lower.contains("not_exist")
                {
                    Ok(SearchOut {
                        mode: "id".into(),
                        query: q.clone(),
                        items: Vec::new(),
                        message: Some(format!("未找到 id 为 {q} 的记忆（支持 8 位前缀）。")),
                    })
                } else {
                    Err(format_rsrs_err(&e))
                }
            }
        };
    }

    let candidates = state
        .client
        .recall_candidates(&q, limit)
        .map_err(|e| format_rsrs_err(&e))?;
    let items = candidates
        .into_iter()
        .map(|c| SearchItemOut {
            id: c.entry.id,
            title: c.entry.title,
            content: c.entry.content,
            kind: c.entry.kind,
            importance: c.entry.importance,
            parent_id: c.entry.parent_id,
            score: Some(c.score),
            superseded_by: c.superseded_by,
            created_at: None,
            updated_at: None,
        })
        .collect();
    Ok(SearchOut {
        mode: "recall".into(),
        query: q,
        items,
        message: None,
    })
}

// ---------------------------------------------------------------------------
// 挂点可视化（P5）
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MemoryPathOut {
    id: String,
    title: String,
    /// 根 → 本条（标题链）
    path: Vec<String>,
    path_text: String,
}

/// 目标条目的挂点路径（memory_show 的 ancestors 根在前）
#[tauri::command]
fn memory_path(state: tauri::State<'_, AppState>, id: String) -> Result<MemoryPathOut, String> {
    let shown = state
        .client
        .show_entry(&id)
        .map_err(|e| format_rsrs_err(&e))?;
    let mut path: Vec<String> = shown.ancestors.iter().map(|n| n.title.clone()).collect();
    path.push(shown.entry.title.clone());
    Ok(MemoryPathOut {
        id: shown.entry.id,
        title: shown.entry.title,
        path_text: path.join(" / "),
        path,
    })
}

/// 目标树的 outline（memory_tree --outline）
#[tauri::command]
fn memory_outline(
    state: tauri::State<'_, AppState>,
    id: String,
    depth: Option<u32>,
) -> Result<Vec<rsrs_client::OutlineNode>, String> {
    state
        .client
        .tree_outline(&id, depth.unwrap_or(2).clamp(1, 6))
        .map_err(|e| format_rsrs_err(&e))
}

// ---------------------------------------------------------------------------
// 提醒（P5）
// ---------------------------------------------------------------------------

fn reminder_cache_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("reminders")
}

/// 扫描含【触发】的 task，返回逾期 / 今天 / 即将
#[tauri::command]
fn list_reminders(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<reminders::ReminderOut, String> {
    reminders::scan(
        &state.client,
        state.config.reminder.soon_days,
        &reminder_cache_dir(&app),
    )
}

/// 忽略一条提醒（同 id 的触发日期变更后会重新出现）
#[tauri::command]
fn reminder_dismiss(app: AppHandle, id: String, due: String) -> Result<(), String> {
    reminders::dismiss(
        &reminder_cache_dir(&app),
        &id,
        &due,
        &reminders::today_local(),
    )
}

/// 完成一条提醒：备份原文 → 追加【完成】YYYY-MM-DD → 回读校验
///
/// 完成后扫描器直接跳过该条（不写本地忽略，移除【完成】行即可重新提醒）。
#[tauri::command]
fn reminder_complete(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let shown = state
        .client
        .show_entry(&id)
        .map_err(|e| format_rsrs_err(&e))?;
    let today = reminders::today_local();
    let Some(new_content) = reminders::with_completion(&shown.entry.content, &today) else {
        return Ok(());
    };

    let tag: String = id.chars().take(8).collect();
    automation::backup_json(
        &automation_backup_dir(&app),
        &format!("complete-{tag}"),
        &serde_json::json!({
            "id": shown.entry.id,
            "title": shown.entry.title,
            "content": shown.entry.content,
        }),
    )
    .map_err(|e| e.to_string())?;

    let full_id = shown.entry.id.clone();
    state
        .client
        .update(&full_id, Some(&new_content), None)
        .map_err(|e| format_rsrs_err(&e))?;
    let verify = state
        .client
        .show_entry(&full_id)
        .map_err(|e| format_rsrs_err(&e))?;
    if !reminders::is_completed(&verify.entry.content) {
        return Err("完成标记回读校验失败：正文未包含【完成】行。".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 3D 图谱（P6-③）
// ---------------------------------------------------------------------------

/// 构建全量 3D 图谱快照。
///
/// 数据目录优先取 rsrs runtime 的 `status.summary.data_dir`（支持多账户/多空间），
/// runtime 不可达时回退到 ~/.rsrs / accounts 下最近修改的库。
#[tauri::command]
async fn graph_snapshot(
    state: tauri::State<'_, AppState>,
    graph: tauri::State<'_, Arc<graph::GraphService>>,
) -> Result<graph::GraphSnapshot, String> {
    let data_dir = match state.client.data_dir() {
        Ok(d) => std::path::PathBuf::from(d),
        Err(_) => graph::guess_data_dir(),
    };
    let svc = graph.inner().clone();
    tauri::async_runtime::spawn_blocking(move || svc.snapshot(&data_dir))
        .await
        .map_err(|e| format!("图谱构建线程失败: {e}"))?
}

/// 从游标取增量事件（本地/其它宿主的写入、云端同步、recall 点亮）。
#[tauri::command]
async fn graph_events_since(
    graph: tauri::State<'_, Arc<graph::GraphService>>,
    seq: i64,
    qid: i64,
) -> Result<graph::GraphEventsOut, String> {
    let svc = graph.inner().clone();
    tauri::async_runtime::spawn_blocking(move || svc.events_since(seq, qid))
        .await
        .map_err(|e| format!("图谱同步线程失败: {e}"))?
}

/// 无视冻结基座重新计算 PCA 布局（用户主动「重排」时用）。
#[tauri::command]
async fn graph_rebuild_layout(
    graph: tauri::State<'_, Arc<graph::GraphService>>,
) -> Result<graph::GraphSnapshot, String> {
    let svc = graph.inner().clone();
    tauri::async_runtime::spawn_blocking(move || svc.rebuild_layout())
        .await
        .map_err(|e| format!("图谱重排线程失败: {e}"))?
}

/// 图谱视图放大/还原主窗（绕过 resizable:false 的程序化尺寸切换）。
#[tauri::command]
fn set_graph_window(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    expanded: bool,
) -> Result<(), String> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    if expanded {
        let (w, h) = match win.current_monitor().map_err(|e| e.to_string())? {
            Some(m) => {
                let s = m.size();
                (
                    (s.width as f64 * 0.92).min(1180.0).max(520.0),
                    (s.height as f64 * 0.90).min(860.0).max(400.0),
                )
            }
            None => (1080.0, 760.0),
        };
        win.set_size(PhysicalSize::new(w as u32, h as u32))
            .map_err(|e| e.to_string())?;
        win.center().map_err(|e| e.to_string())?;
    } else {
        win.set_size(PhysicalSize::new(
            state.config.window.width as u32,
            state.config.window.height as u32,
        ))
        .map_err(|e| e.to_string())?;
        win.center().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

/// 标题生成（12-24 字事件事实，禁主题名当标题）—— 统一走 automation 模块
fn build_title(content: &str) -> String {
    automation::build_title(content)
}

/// 错误分类 → 用户可读文案
fn format_rsrs_err(e: &rsrs_client::RsrsError) -> String {
    use rsrs_client::RsrsError;
    match e {
        RsrsError::Unreachable(msg) => {
            format!("runtime 不可达（连接失败）：{}。\n请由宿主启动 rsrs runtime（client-only 端口 15169）后重试。", msg)
        }
        RsrsError::AuthFailed(msg) => {
            format!("鉴权失败：{}。\n请检查 rsrs token 配置；loopback 默认免 token。", msg)
        }
        RsrsError::CandidatesDangling(msg) => {
            format!("rsrs 返回判重候选但未写入（候选悬空）：{}。\n需择一执行，禁止当作成功。", msg)
        }
        RsrsError::Mcp(msg) => format!("MCP 错误：{}", msg),
        RsrsError::Parse(msg) => format!("响应解析失败：{}", msg),
        RsrsError::Other(msg) => msg.clone(),
    }
}

// ---------------------------------------------------------------------------
// 窗口位置记忆
// ---------------------------------------------------------------------------

fn window_state_path(app: &AppHandle) -> std::path::PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    dir.join("window_state.json")
}

/// 换屏或分辨率变化后，在当前工作区内恢复主窗口，避免托盘唤起到屏幕外。
fn ensure_window_on_screen(win: &tauri::WebviewWindow) -> tauri::Result<()> {
    let position = win.outer_position()?;
    let size = win.outer_size()?;
    let areas = win.available_monitors()?.iter().map(|monitor| {
        let area = monitor.work_area();
        (area.position, area.size)
    }).collect::<Vec<_>>();
    let visible = window_position::visible_position(position, size, &areas);
    if visible != position {
        win.set_position(visible)?;
    }
    Ok(())
}

fn save_window_position(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_minimized().unwrap_or(true) {
            return;
        }
        if let Ok(pos) = win.outer_position() {
            if let Ok(data) = serde_json::to_string(&(pos.x, pos.y)) {
                let _ = std::fs::write(window_state_path(app), data);
            }
        }
    }
}

fn restore_window_position(app: &AppHandle) {
    let path = window_state_path(app);
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok((x, y)) = serde_json::from_str::<(i32, i32)>(&data) {
            if let Some(win) = app.get_webview_window("main") {
                if win.set_position(PhysicalPosition::new(x, y)).is_ok() {
                    if let Err(error) = ensure_window_on_screen(&win) {
                        eprintln!("恢复窗口位置失败: {error}");
                    } else {
                        save_window_position(app);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 热键
// ---------------------------------------------------------------------------

fn register_hotkey(app: &AppHandle, spec: &str) -> Result<(), Box<dyn std::error::Error>> {
    let shortcut: Shortcut = spec.parse().map_err(|e| {
        format!("热键格式无效 {}: {:?}", spec, e)
    })?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| format!("注册热键失败: {:?}", e))?;
    Ok(())
}

/// 显示并聚焦小窗（托盘菜单用）
fn show_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        if let Err(error) = ensure_window_on_screen(&win) {
            eprintln!("检查窗口可见范围失败: {error}");
        } else {
            save_window_position(app);
        }
        let _ = win.show();
        let _ = win.set_focus();
        let _ = app.emit("memtools-shown", ());
    }
}

/// 唤起/隐藏小窗
fn toggle_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) && !win.is_minimized().unwrap_or(false) {
            save_window_position(app);
            let _ = win.hide();
        } else {
            show_window(app);
        }
    }
}

// ---------------------------------------------------------------------------
// 系统托盘
// ---------------------------------------------------------------------------

/// 常驻托盘：左键唤起/隐藏小窗；右键菜单「显示面板 / 退出 memTools」。
/// 关闭窗口（✕）只隐藏，进程常驻，热键与托盘持续可用。
fn setup_tray(app: &AppHandle, selection_enabled: bool) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "显示面板", true, None::<&str>)?;
    let selection_item = CheckMenuItem::with_id(
        app,
        "selection",
        "划词悬浮",
        true,
        selection_enabled,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 memTools", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &selection_item, &quit_item])?;
    let selection_item_for_event = selection_item.clone();

    let mut builder = TrayIconBuilder::with_id("memtools-tray")
        .tooltip("memTools · rsrs 记忆")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "selection" => {
                let state = app.state::<Arc<selection::SelectionState>>();
                let on = !state.is_enabled();
                selection::set_enabled(app, state.inner(), on);
                let _ = selection_item_for_event.set_checked(on);
            }
            "quit" => {
                save_window_position(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 自动更新（P7）
// ---------------------------------------------------------------------------

#[tauri::command]
async fn update_check(state: tauri::State<'_, AppState>) -> Result<updater::CheckOut, String> {
    if !state.config.update.enabled {
        return Err("自动更新已关闭（config.toml [update] enabled = false）".into());
    }
    let endpoint = state.config.update.endpoint.clone();
    tauri::async_runtime::spawn_blocking(move || updater::check(&endpoint).map(|(out, _)| out))
        .await
        .map_err(|e| format!("检查更新线程失败：{e}"))?
}

#[tauri::command]
async fn update_install(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<updater::CheckOut, String> {
    if !state.config.update.enabled {
        return Err("自动更新已关闭".into());
    }
    let endpoint = state.config.update.endpoint.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<updater::CheckOut, String> {
        let (out, manifest) = updater::check(&endpoint)?;
        if !out.has_update {
            return Err(format!("已是最新版本 v{}", out.current));
        }
        updater::install(&handle, &manifest)?;
        let _ = handle.emit("update://restarting", ());
        std::thread::sleep(std::time::Duration::from_millis(400));
        handle.exit(0);
        Ok(out)
    })
    .await
    .map_err(|e| format!("更新线程失败：{e}"))?
}

pub fn run() {
    let default_config = config::AppConfig::load();

    tauri::Builder::default()
        // 单例：重复双击 exe 不再起第二个进程/托盘，改为唤起已有小窗（须最先注册）
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_window(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            // 全局状态：config 已在 Builder 阶段载入；快捷目标存储需等应用配置目录可解析
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir());
            let target_store = Arc::new(std::sync::Mutex::new(targets::TargetStore::load(config_dir.join("targets.json"))));
            // 梳理来源（P10）：界面下拉写入 processor.json，叠加在 config.toml 之上
            let processor_settings = Arc::new(processor_settings::ProcessorSettings::load(
                config_dir.join("processor.json"),
            ));
            let client = Arc::new(RsrsClient::new(
                default_config.rsrs.base_url.clone(),
                default_config.rsrs.mcp_path.clone(),
                default_config.rsrs.tools.clone(),
                default_config.rsrs.timeout_secs,
                default_config.rsrs.op_timeout_secs,
            ));
            app.manage(AppState {
                config: default_config.clone(),
                client: client.clone(),
                targets: target_store.clone(),
                processor: processor_settings.clone(),
            });
            // 分类树缓存预热：后台线程每 120s 刷新一次，让首次 /targets 通常直接命中缓存
            {
                let warm_client = client.clone();
                let spawned = std::thread::Builder::new()
                    .name("memtools-taxonomy-refresh".into())
                    .spawn(move || {
                        // 启动期延迟 2s，避开启动阶段的其它 rsrs 调用
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        loop {
                            warm_client.list_taxonomy();
                            std::thread::sleep(std::time::Duration::from_secs(120));
                        }
                    });
                if let Err(e) = spawned {
                    eprintln!("[memTools] 分类树预热线程启动失败：{e}");
                }
            }
            // 应用 config.toml 窗口尺寸（默认 480x560）
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_size(PhysicalSize::new(
                    default_config.window.width as u32,
                    default_config.window.height as u32,
                ));
            }
            // 恢复上次窗口位置
            if default_config.window.remember_position {
                restore_window_position(app.handle());
            }
            // 注册全局热键（配置化）
            let spec = default_config.hotkey.register.clone();
            if let Err(e) = register_hotkey(app.handle(), &spec) {
                eprintln!("[memTools] {}", e);
            }
            // 系统托盘（常驻；窗口隐藏后仍可经托盘唤起/退出）
            if let Err(e) = setup_tray(app.handle(), default_config.selection.enabled) {
                eprintln!("[memTools] 建立系统托盘失败: {e}");
            }
            // 全局划词悬浮（P8）：钩子常驻安装，enabled=false 时内部直接忽略（托盘可随时开）
            let selection_state = Arc::new(selection::SelectionState::new(
                default_config.selection.enabled,
            ));
            app.manage(selection_state.clone());
            selection::start(
                app.handle().clone(),
                default_config.selection.clone(),
                selection_state,
            );
            updater::cleanup(app.handle());
            // 3D 图谱服务（P6-③）：缓存放在应用配置目录，不写 rsrs 数据目录
            let graph_cache = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("graph");
            app.manage(Arc::new(graph::GraphService::new(graph_cache)));
            // 提醒扫描线程（P5）：启动后 8s 首扫，之后每 interval_minutes 一次
            if default_config.reminder.enabled {
                let client = app.state::<AppState>().client.clone();
                let handle = app.handle().clone();
                let cache = app
                    .path()
                    .app_config_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join("reminders");
                let soon_days = default_config.reminder.soon_days;
                let interval = std::time::Duration::from_secs(
                    (default_config.reminder.interval_minutes.max(1) as u64) * 60,
                );
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(8));
                    loop {
                        if let Ok(out) = reminders::scan(&client, soon_days, &cache) {
                            let due = out.overdue + out.today;
                            if let Some(tray) = handle.tray_by_id("memtools-tray") {
                                let tip = if due > 0 {
                                    format!("memTools · {due} 条任务到期")
                                } else {
                                    "memTools · rsrs 记忆".to_string()
                                };
                                let _ = tray.set_tooltip(Some(tip));
                            }
                            let _ = handle.emit(
                                "reminders://due",
                                serde_json::json!({
                                    "overdue": out.overdue,
                                    "today": out.today,
                                    "soon": out.soon,
                                }),
                            );
                        }
                        std::thread::sleep(interval);
                    }
                });
            }
            // 浏览器扩展本机端点（P6-②）：只绑 127.0.0.1，失败不阻塞小窗
            if default_config.extension.enabled {
                let token_path = app
                    .path()
                    .app_config_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join("extension_token.txt");
                match server::start(
                    default_config.clone(),
                    token_path.clone(),
                    automation_backup_dir(app.handle()),
                    app.handle().clone(),
                    target_store.clone(),
                    client.clone(),
                    processor_settings.clone(),
                ) {
                    Ok(s) => {
                        eprintln!(
                            "[memTools] 浏览器扩展端点已启动 http://127.0.0.1:{}（令牌文件：{}）",
                            s.port,
                            token_path.display()
                        );
                        app.manage(s);
                    }
                    Err(e) => eprintln!("[memTools] {e}"),
                }
            }
            Ok(())
        })
        .on_window_event(|_window, event| {
            if let WindowEvent::Moved(_) = event {
                // 位置变化实时记录（由 quit/隐藏时落盘亦可）
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_taxonomy,
            submit_memory,
            get_app_config,
            extension_info,
            extension_install,
            runtime_status,
            hide_window,
            quit_app,
            selection_payload,
            selection_expand,
            selection_hide,
            selection_open_main,
            selection_journal,
            selection_targets,
            selection_save_target,
            quick_targets_list,
            quick_targets_save,
            quick_targets_reset,
            automation_submit,
            automation_resolve,
            process_memory,
            processor_status,
            processor_set_source,
            processor_reset_source,
            processor_probe,
            list_recent,
            search_memory,
            memory_path,
            memory_outline,
            list_reminders,
            reminder_dismiss,
            reminder_complete,
            graph_snapshot,
            graph_events_since,
            graph_rebuild_layout,
            set_graph_window,
            update_check,
            update_install
        ])
        .run(tauri::generate_context!())
        .expect("error while running memTools");
}

#[cfg(test)]
mod tests {
    use super::looks_like_memory_id;
    use super::{TargetIn, TargetOut};

    #[test]
    fn id_prefix_and_uuid() {
        assert!(looks_like_memory_id("bd22ad3a"));
        assert!(looks_like_memory_id("bd22ad3a-7dbe-47ff-bb07-e32139ed18fc"));
        assert!(looks_like_memory_id("  bd22ad3a  "));
    }

    #[test]
    fn content_is_not_id() {
        assert!(!looks_like_memory_id("修复 window 拖拽"));
        assert!(!looks_like_memory_id("hello"));
        assert!(!looks_like_memory_id("bd22ad3g"));
        assert!(!looks_like_memory_id("1234"));
    }

    #[test]
    fn target_out_serializes_type_field() {
        let out = TargetOut {
            id: "t1".into(),
            label: "AI 挂点".into(),
            kind: "tree".into(),
            parent: Some("AI-domain".into()),
            r#type: Some("decision".into()),
            importance: Some("important".into()),
        };
        let v = serde_json::to_value(&out).expect("应能序列化");
        assert_eq!(v["id"], serde_json::json!("t1"));
        assert_eq!(v["type"], serde_json::json!("decision"));
        assert_eq!(v["parent"], serde_json::json!("AI-domain"));
        assert_eq!(v["importance"], serde_json::json!("important"));
    }

    #[test]
    fn target_in_parses_type_field() {
        let v: TargetIn = serde_json::from_str(
            r#"{"id":"t1","label":"AI 挂点","kind":"tree","parent":"AI-domain","type":"decision","importance":"important"}"#,
        )
        .expect("应能解析");
        assert_eq!(v.id, "t1");
        assert_eq!(v.r#type.as_deref(), Some("decision"));
        assert_eq!(v.parent.as_deref(), Some("AI-domain"));
    }

    #[test]
    fn target_in_tolerates_missing_fields() {
        let v: TargetIn = serde_json::from_str(r#"{"id":"t1"}"#).expect("应能解析");
        assert_eq!(v.id, "t1");
        assert!(v.label.is_empty());
        assert!(v.kind.is_empty());
        assert!(v.r#type.is_none());
    }
}

