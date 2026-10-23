//! 浏览器扩展本机端点（P6-②）+ 快捷目标接线（P9）
//!
//! - 只绑定 `127.0.0.1`，供本机 Chrome/Edge 扩展提交网页选中文字；
//! - 所有接口要求 `X-MemTools-Token`（或 `Authorization: Bearer <token>`）；
//! - 令牌默认自动生成到 `{app_config_dir}/extension_token.txt`，不写进仓库；
//! - 落库复用 `RsrsClient` + `automation`，不 shell 拼接、不启停 rsrs 服务。
//!
//! 路由：
//!   GET  /health    → { ok, name, version }
//!   GET  /targets   → { ok, targets, taxonomy }
//!   PUT  /targets   → { ok, targets, taxonomy, message }
//!   POST /open      → { ok, message }（唤醒 memTools，可带选区文字预填）
//!   POST /remember  → { text, mode: "journal"|"auto" | target_id | process, url?, page_title?, title? }
//!   POST /process   → { content, task, lang? }（梳理：LLM 或 agent，不落库）
//!   GET  /agent     → { ok, enabled, kind, bin, version }（探测 agent CLI 可用性）
//!
//! `mode=auto` 的冲突处理：判重命中高相似条目时**不执行破坏性分支**（改/并），
//! 退化为「挂到候选之下」并在响应里标记 `needs_confirm`，由用户回小窗核对。
//! `process` 字段（P10）：给了就先梳理再落库，失败不静默落原文（502/503/504 明确回传）。
//! 梳理来源：`[processor] agent_endpoint` / `[processor.agent] enabled` 决定走 agent
//! （opencode / Qoder），否则走 OpenAI 兼容 LLM。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use tauri::AppHandle;

use crate::automation::{self, CandidateOut};
use crate::config::AppConfig;
use crate::processor::{self, Task};
use crate::processor_settings::ProcessorSettings;
use crate::quick::{self, QuickOutcome};
use crate::rsrs_client::{RememberRequest, RememberResponse, RsrsClient};
use crate::targets::{QuickTarget, TargetStore};
use crate::TaxonomyOut;

const MAX_BODY_BYTES: u64 = 256 * 1024;
const MAX_TEXT_CHARS: usize = 8000;

/// 端点启动结果（端口可能因 port=0 被系统分配）
pub struct Started {
    pub port: u16,
    pub token: String,
}

/// 后台线程持有的依赖集合（避免路由处理函数携带一长串参数）
struct Ctx<'a> {
    client: &'a RsrsClient,
    cfg: &'a AppConfig,
    /// 梳理来源的用户选择（小窗下拉写入，叠加在 config.toml 之上）
    processor: &'a ProcessorSettings,
    token: &'a str,
    backup_dir: &'a Path,
    /// 唤醒主窗的回调（无界面模式为 None，此时 `/open` 不可用）
    open_main: Option<&'a OpenMain>,
    targets: &'a Mutex<TargetStore>,
}

/// 唤醒主窗并预填正文的回调（由调用方注入，端点本身不依赖 tauri 窗口）
type OpenMain = Box<dyn Fn(Option<&str>) + Send + 'static>;

#[derive(Debug, Deserialize)]
struct RememberIn {
    text: String,
    #[serde(default)]
    title: Option<String>,
    /// journal | auto（默认 auto）
    #[serde(default)]
    mode: Option<String>,
    /// 快捷目标 id（P9）：给了就按目标落库，忽略 mode
    #[serde(default)]
    target_id: Option<String>,
    /// 梳理任务（P10）：给了就先调 LLM 梳理，再用结果落库
    #[serde(default)]
    process: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    page_title: Option<String>,
}

/// `POST /process` 请求体（只梳理，不落库）
#[derive(Debug, Deserialize)]
struct ProcessIn {
    content: String,
    task: String,
    #[serde(default)]
    lang: Option<String>,
}

/// `POST /process` 响应体
#[derive(Debug, Serialize)]
struct ProcessResponse {
    ok: bool,
    task: Option<String>,
    content: String,
    /// 实际来源：`llm` 或 agent 的 kind（`opencode` / `qoder`）
    source: Option<String>,
    message: String,
}

impl ProcessResponse {
    fn fail(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            task: None,
            content: String::new(),
            source: None,
            message: message.into(),
        }
    }
}

/// 梳理错误码：未启用 / 未配置 / 找不到 agent → 503；超时 → 504；其余（模型或 agent 报错）→ 502
fn process_error_code(e: &str) -> u16 {
    if e.contains("未启用")
        || e.contains("未配置")
        || e.contains("未找到")
        || e.contains("找不到")
        || e.contains("无法启动")
    {
        503
    } else if e.contains("超时") {
        504
    } else {
        502
    }
}

/// `POST /open` 请求体（唤醒 memTools）
#[derive(Debug, Default, Deserialize)]
struct OpenIn {
    /// 预填正文（为空则只唤醒不预填）
    #[serde(default)]
    text: Option<String>,
    /// 来源页面地址（仅用于回显来源，不写入预填正文）
    #[serde(default)]
    url: Option<String>,
    /// 来源页面标题（仅用于回显来源，不写入预填正文）
    #[serde(default)]
    page_title: Option<String>,
}

/// `PUT /targets` 请求体
#[derive(Debug, Default, Deserialize)]
struct TargetsIn {
    #[serde(default)]
    targets: Vec<QuickTarget>,
}

/// `GET/PUT /targets` 响应体
#[derive(Debug, Serialize)]
struct TargetsResponse {
    ok: bool,
    targets: Vec<QuickTarget>,
    taxonomy: Vec<TaxonomyOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiResponse {
    ok: bool,
    branch: Option<String>,
    action: Option<String>,
    id: Option<String>,
    parent: Option<String>,
    title: Option<String>,
    message: String,
    warnings: Vec<String>,
    candidates: Vec<CandidateOut>,
    needs_confirm: bool,
}

impl ApiResponse {
    fn fail(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            branch: None,
            action: None,
            id: None,
            parent: None,
            title: None,
            message: message.into(),
            warnings: Vec::new(),
            candidates: Vec::new(),
            needs_confirm: false,
        }
    }
}

/// 启动端点；`cfg.extension.enabled=false` 时不启动
///
/// `app` 用于唤醒主窗（`POST /open`）并预填选区文字。
/// `client` 与主窗共用同一个实例：分类树缓存（P9 预热线程）因此对 `/targets` 生效。
pub fn start(
    cfg: AppConfig,
    token_path: PathBuf,
    backup_dir: PathBuf,
    app: AppHandle,
    targets: Arc<Mutex<TargetStore>>,
    client: Arc<RsrsClient>,
    processor: Arc<ProcessorSettings>,
) -> Result<Started, String> {
    let open_main: OpenMain = Box::new(move |text| crate::open_main_window(&app, text));
    spawn_server(
        cfg,
        token_path,
        backup_dir,
        Some(open_main),
        targets,
        Some(client),
        processor,
    )
}

/// 启动端点（无界面模式：只提供 HTTP 路由，`/open` 无窗口可唤醒）
///
/// 供冒烟脚本与无 GUI 的自测使用；正常运行时走 [`start`]。
pub fn start_without_window(
    cfg: AppConfig,
    token_path: PathBuf,
    backup_dir: PathBuf,
    targets: Arc<Mutex<TargetStore>>,
    processor: Arc<ProcessorSettings>,
) -> Result<Started, String> {
    spawn_server(cfg, token_path, backup_dir, None, targets, None, processor)
}

fn spawn_server(
    cfg: AppConfig,
    token_path: PathBuf,
    backup_dir: PathBuf,
    open_main: Option<OpenMain>,
    targets: Arc<Mutex<TargetStore>>,
    client: Option<Arc<RsrsClient>>,
    processor: Arc<ProcessorSettings>,
) -> Result<Started, String> {
    if !cfg.extension.enabled {
        return Err("浏览器扩展端点未启用".into());
    }
    let token = resolve_token(&cfg, &token_path);
    let thread_token = token.clone();
    let addr = format!("127.0.0.1:{}", cfg.extension.port);
    let server = Server::http(&addr)
        .map_err(|e| format!("扩展端点 {addr} 启动失败：{e}（端口被占用？）"))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(cfg.extension.port);

    std::thread::spawn(move || {
        let client = client.unwrap_or_else(|| {
            Arc::new(RsrsClient::new(
                cfg.rsrs.base_url.clone(),
                cfg.rsrs.mcp_path.clone(),
                cfg.rsrs.tools.clone(),
                cfg.rsrs.timeout_secs,
                cfg.rsrs.op_timeout_secs,
            ))
        });
        for request in server.incoming_requests() {
            let ctx = Ctx {
                client: client.as_ref(),
                cfg: &cfg,
                processor: processor.as_ref(),
                token: &thread_token,
                backup_dir: &backup_dir,
                open_main: open_main.as_ref(),
                targets: &targets,
            };
            handle(request, &ctx);
        }
    });

    Ok(Started { port, token })
}

/// 令牌优先级：config.toml 固定值 > 已有令牌文件 > 新生成
pub fn resolve_token(cfg: &AppConfig, path: &Path) -> String {
    let fixed = cfg.extension.token.trim();
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    if let Ok(text) = std::fs::read_to_string(path) {
        let text = text.trim();
        if !text.is_empty() {
            return text.to_string();
        }
    }
    let token = gen_token();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, &token) {
        eprintln!("[memTools] 写入扩展令牌失败（{}）：{e}", path.display());
    }
    token
}

/// 本地令牌：128 bit 伪随机（时间 + PID 派生），足够本机端点使用
pub fn gen_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos ^ ((std::process::id() as u64) << 32) ^ 0x9E37_79B9_7F4A_7C15;
    let mut out = String::new();
    for _ in 0..2 {
        x = splitmix64(x);
        out.push_str(&format!("{x:016x}"));
    }
    out
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn handle(mut req: Request, ctx: &Ctx<'_>) {
    let method = req.method().clone();
    let path = req.url().split('?').next().unwrap_or("").to_string();

    if method == Method::Options {
        respond(req, 204, "");
        return;
    }

    if !authorized(&req, ctx.token) {
        respond(
            req,
            401,
            &serde_json::to_string(&ApiResponse::fail(
                "令牌无效：请把 memTools 的 extension_token.txt 内容填入扩展选项。",
            ))
            .unwrap_or_default(),
        );
        return;
    }

    match (&method, path.as_str()) {
        (Method::Get, "/health") => {
            let body = serde_json::json!({
                "ok": true,
                "name": "memTools",
                "version": env!("CARGO_PKG_VERSION"),
            });
            respond(req, 200, &body.to_string());
        }
        (Method::Get, "/targets") => {
            let body = TargetsResponse {
                ok: true,
                targets: ctx.targets.lock().unwrap().list(),
                taxonomy: taxonomy_out(ctx.client),
                message: None,
            };
            respond(req, 200, &serde_json::to_string(&body).unwrap_or_default());
        }
        (Method::Put, "/targets") => {
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err(msg) => {
                    respond(
                        req,
                        413,
                        &serde_json::to_string(&ApiResponse::fail(msg)).unwrap_or_default(),
                    );
                    return;
                }
            };
            let input: TargetsIn = if body.trim().is_empty() {
                TargetsIn::default()
            } else {
                match serde_json::from_str(&body) {
                    Ok(v) => v,
                    Err(e) => {
                        respond(
                            req,
                            400,
                            &serde_json::to_string(&ApiResponse::fail(format!("请求 JSON 无效：{e}")))
                                .unwrap_or_default(),
                        );
                        return;
                    }
                }
            };
            let (code, resp) = match ctx.targets.lock().unwrap().replace(input.targets) {
                Ok(list) => {
                    let count = list.len();
                    (
                        200,
                        TargetsResponse {
                            ok: true,
                            targets: list,
                            taxonomy: taxonomy_out(ctx.client),
                            message: Some(format!("已保存 {count} 个快捷目标")),
                        },
                    )
                }
                Err(msg) => (
                    400,
                    TargetsResponse {
                        ok: false,
                        targets: Vec::new(),
                        taxonomy: Vec::new(),
                        message: Some(msg),
                    },
                ),
            };
            respond(req, code, &serde_json::to_string(&resp).unwrap_or_default());
        }
        (Method::Post, "/open") => {
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err(msg) => {
                    respond(
                        req,
                        413,
                        &serde_json::to_string(&ApiResponse::fail(msg)).unwrap_or_default(),
                    );
                    return;
                }
            };
            let input: OpenIn = if body.trim().is_empty() {
                OpenIn::default()
            } else {
                match serde_json::from_str(&body) {
                    Ok(v) => v,
                    Err(e) => {
                        respond(
                            req,
                            400,
                            &serde_json::to_string(&ApiResponse::fail(format!("请求 JSON 无效：{e}")))
                                .unwrap_or_default(),
                        );
                        return;
                    }
                }
            };
            let text = input.text.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let source = source_line(input.page_title.as_deref(), input.url.as_deref());
            if !source.is_empty() {
                eprintln!("[memTools] 扩展请求唤醒主窗（{source}）");
            }
            let Some(open_main) = ctx.open_main else {
                respond(
                    req,
                    503,
                    &serde_json::to_string(&ApiResponse::fail("memTools 主窗口不可用（无界面模式）"))
                        .unwrap_or_default(),
                );
                return;
            };
            open_main(text);
            let body = serde_json::json!({ "ok": true, "message": "已打开 memTools" });
            respond(req, 200, &body.to_string());
        }
        (Method::Post, "/remember") => {
            let body = read_body(&mut req);
            let body = match body {
                Ok(b) => b,
                Err(msg) => {
                    respond(
                        req,
                        413,
                        &serde_json::to_string(&ApiResponse::fail(msg)).unwrap_or_default(),
                    );
                    return;
                }
            };
            let input: RememberIn = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(e) => {
                    respond(
                        req,
                        400,
                        &serde_json::to_string(&ApiResponse::fail(format!("请求 JSON 无效：{e}")))
                            .unwrap_or_default(),
                    );
                    return;
                }
            };
            let (code, resp) = remember(ctx, input);
            respond(req, code, &serde_json::to_string(&resp).unwrap_or_default());
        }
        (Method::Post, "/process") => {
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err(msg) => {
                    respond(
                        req,
                        413,
                        &serde_json::to_string(&ApiResponse::fail(msg)).unwrap_or_default(),
                    );
                    return;
                }
            };
            let input: ProcessIn = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(e) => {
                    respond(
                        req,
                        400,
                        &serde_json::to_string(&ApiResponse::fail(format!("请求 JSON 无效：{e}")))
                            .unwrap_or_default(),
                    );
                    return;
                }
            };
            let task = match Task::parse(&input.task) {
                Some(t) => t,
                None => {
                    respond(
                        req,
                        400,
                        &serde_json::to_string(&ApiResponse::fail(format!(
                            "未知梳理任务：{}",
                            input.task.trim()
                        )))
                        .unwrap_or_default(),
                    );
                    return;
                }
            };
            let proc_cfg = ctx.processor.effective(&ctx.cfg.processor);
            match processor::process(&proc_cfg, &input.content, &task, input.lang.as_deref()) {
                Ok(content) => {
                    let body = ProcessResponse {
                        ok: true,
                        task: Some(task.as_str().to_string()),
                        content,
                        source: Some(proc_cfg.source().to_string()),
                        message: format!(
                            "已{}（{} · {}）",
                            task.label(),
                            task.as_str(),
                            proc_cfg.source()
                        ),
                    };
                    respond(req, 200, &serde_json::to_string(&body).unwrap_or_default());
                }
                Err(e) => {
                    let code = process_error_code(&e);
                    respond(
                        req,
                        code,
                        &serde_json::to_string(&ProcessResponse::fail(e)).unwrap_or_default(),
                    );
                }
            }
        }
        (Method::Get, "/agent") => {
            let proc_cfg = ctx.processor.effective(&ctx.cfg.processor);
            let cfg = &proc_cfg;
            let enabled = cfg.agent_enabled();
            let (ok, kind, bin, version, message) = if !enabled {
                (
                    false,
                    cfg.agent.kind.clone(),
                    String::new(),
                    String::new(),
                    "未启用 agent 路径（[processor] agent_endpoint 为空且 [processor.agent] enabled = false）"
                        .to_string(),
                )
            } else {
                match crate::agent::probe(&cfg.agent) {
                    Ok((kind, bin, version)) => (
                        true,
                        kind.as_str().to_string(),
                        bin.clone(),
                        version,
                        format!("{} 可用：{bin}", kind.as_str()),
                    ),
                    Err(e) => (false, cfg.agent.kind.clone(), String::new(), String::new(), e),
                }
            };
            let body = serde_json::json!({
                "ok": ok,
                "enabled": enabled,
                "source": cfg.source(),
                "kind": kind,
                "bin": bin,
                "version": version,
                "message": message,
            });
            respond(req, 200, &body.to_string());
        }
        (Method::Get, "/") => {
            let body = ApiResponse {
                ok: true,
                branch: None,
                action: None,
                id: None,
                parent: None,
                title: None,
                message: "memTools 扩展端点。POST /remember，GET /health。".into(),
                warnings: Vec::new(),
                candidates: Vec::new(),
                needs_confirm: false,
            };
            respond(req, 200, &serde_json::to_string(&body).unwrap_or_default());
        }
        _ => respond(
            req,
            404,
            &serde_json::to_string(&ApiResponse::fail("未知路由")).unwrap_or_default(),
        ),
    }
}

fn read_body(req: &mut Request) -> Result<String, String> {
    let mut body = String::new();
    let mut reader = req.as_reader().take(MAX_BODY_BYTES);
    reader
        .read_to_string(&mut body)
        .map_err(|e| format!("读取请求体失败：{e}"))?;
    if body.len() as u64 >= MAX_BODY_BYTES {
        return Err("请求体过大（上限 256KB）".into());
    }
    Ok(body)
}

fn authorized(req: &Request, token: &str) -> bool {
    for h in req.headers() {
        if h.field.equiv("x-memtools-token") && h.value.as_str() == token {
            return true;
        }
        if h.field.equiv("authorization") {
            if let Some(v) = h.value.as_str().strip_prefix("Bearer ") {
                if v == token {
                    return true;
                }
            }
        }
    }
    false
}

fn respond(req: Request, status: u16, body: &str) {
    let mut resp = Response::from_string(body).with_status_code(StatusCode(status));
    for (k, v) in [
        ("Content-Type", "application/json; charset=utf-8"),
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, POST, PUT, OPTIONS"),
        (
            "Access-Control-Allow-Headers",
            "content-type, x-memtools-token, authorization",
        ),
        // Chrome 私有网络访问预检（扩展 → localhost）
        ("Access-Control-Allow-Private-Network", "true"),
        ("Access-Control-Max-Age", "86400"),
    ] {
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp.add_header(h);
        }
    }
    let _ = req.respond(resp);
}

fn remember(ctx: &Ctx<'_>, input: RememberIn) -> (u16, ApiResponse) {
    let raw_text = input.text.trim();
    if raw_text.is_empty() {
        return (400, ApiResponse::fail("选区内容为空"));
    }
    // 内容梳理（P10）：先调模型 / agent 梳理，失败不静默存原文
    let proc_cfg = ctx.processor.effective(&ctx.cfg.processor);
    let processed;
    let text: &str = match input
        .process
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(p) => {
            let task = match Task::parse(p) {
                Some(t) => t,
                None => return (400, ApiResponse::fail(format!("未知梳理任务：{p}"))),
            };
            match processor::process(&proc_cfg, raw_text, &task, None) {
                Ok(out) => {
                    processed = out;
                    processed.as_str()
                }
                Err(e) => {
                    let code = process_error_code(&e);
                    return (
                        code,
                        ApiResponse::fail(format!(
                            "梳理失败：{e}。可改用「存到主区」直接保存原文。"
                        )),
                    );
                }
            }
        }
        None => raw_text,
    };
    if text.is_empty() {
        return (400, ApiResponse::fail("选中内容为空"));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return (
            400,
            ApiResponse::fail(format!(
                "内容过长（{} 字，上限 {} 字），请缩短后重试或改用小窗分段记录。",
                text.chars().count(),
                MAX_TEXT_CHARS
            )),
        );
    }

    // 判定顺序：先看 target_id（快捷目标），没有才回退 mode（保持既有扩展行为）
    if let Some(target_id) = input
        .target_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return remember_by_target(
            ctx,
            target_id,
            text,
            input.page_title.as_deref(),
            input.url.as_deref(),
        );
    }

    let cfg = ctx.cfg;
    let client = ctx.client;
    let backup_dir = ctx.backup_dir;

    let mode = match input.mode.as_deref().unwrap_or("auto") {
        "journal" => "journal",
        "auto" => "auto",
        other => return (400, ApiResponse::fail(format!("未知 mode：{other}"))),
    };

    let source = source_line(input.page_title.as_deref(), input.url.as_deref());

    if mode == "journal" {
        let mut content = text.to_string();
        if !source.is_empty() {
            content.push_str("\n\n");
            content.push_str(&source);
        }
        let req = RememberRequest {
            content,
            importance: Some("trivial".into()),
            ..Default::default()
        };
        return match client.remember(req) {
            Ok(res) => (200, from_remember(res, "journal")),
            Err(e) => (502, ApiResponse::fail(format!("rsrs 写入失败：{e}"))),
        };
    }

    // auto：判档 → 判重 → 判树 → 非破坏性执行
    let taxonomy = match client.list_taxonomy() {
        Ok(t) => t,
        Err(e) => return (502, ApiResponse::fail(format!("读取分类失败：{e}"))),
    };

    let mut content = text.to_string();
    let mut warnings = Vec::new();
    let mut verdict = None;
    if cfg.automation.mode != "local-fallback" && cfg.automation.llm.enabled {
        let trees: Vec<String> = taxonomy.iter().map(|t| t.name.clone()).collect();
        match crate::llm::judge(
            &cfg.automation.llm,
            &content,
            input.page_title.as_deref(),
            &trees,
        ) {
            Ok(v) => verdict = Some(v),
            Err(e) => warnings.push(format!("模型判档失败，已回退本地：{e}")),
        }
    } else if cfg.automation.mode != "local-fallback" {
        warnings.push(format!(
            "automation.mode={} 但 [automation.llm] enabled=false，已用本地判档。",
            cfg.automation.mode
        ));
    }
    let prelim = verdict
        .as_ref()
        .map(|v| v.importance.clone())
        .filter(|s| s == "important" || s == "trivial")
        .unwrap_or_else(|| automation::judge_importance(&content).to_string());
    if prelim != "trivial" {
        let (norm, w) = automation::normalize_sections(&content);
        content = norm;
        warnings.extend(w);
    }
    if !source.is_empty() {
        content.push_str("\n\n");
        content.push_str(&source);
    }

    let mut plan = match automation::plan_with(
        client,
        &content,
        input.page_title.as_deref(),
        &taxonomy,
        &cfg.automation,
        verdict.as_ref(),
    ) {
        Ok(p) => p,
        Err(e) => return (502, ApiResponse::fail(format!("生成计划失败：{e}"))),
    };
    warnings.append(&mut plan.warnings);

    let needs_confirm = plan.needs_confirm;
    let custom_title = input
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let title = custom_title.unwrap_or_else(|| plan.title.clone());
    // 扩展侧传入的标题可能含换行/超长：压平并截断，避免破坏 rsrs 标题
    let title: String = title
        .replace(['\n', '\r'], " ")
        .trim()
        .chars()
        .take(60)
        .collect();
    let (branch, candidate, tree) = if needs_confirm {
        // 破坏性分支（改/并）不在扩展侧静默执行；退化为「挂」到最相似条目下
        match plan.candidates.first() {
            Some(top) => ("attach".to_string(), Some(top.id.clone()), None),
            None => {
                // 判重无候选、判树也无匹配：不静默猜分类，交回用户选择
                return (
                    200,
                    ApiResponse::fail(
                        "未匹配到分类树，也未找到相似记忆；请打开 memTools 小窗手动选择分类后保存，或补充更明确的内容再试。",
                    ),
                );
            }
        }
    } else {
        (
            plan.branch.clone(),
            plan.suggested_parent.clone(),
            plan.suggested_tree.clone(),
        )
    };

    if needs_confirm {
        if let Some(top) = plan.candidates.first() {
            warnings.push(format!(
                "疑似与已有记忆重复（{:.0}%：{}），已挂到该条之下；请回 memTools 核对。",
                top.score * 100.0,
                top.title
            ));
        }
    }

    match automation::apply(
        client,
        &content,
        &title,
        &branch,
        candidate.as_deref(),
        tree.as_deref(),
        &plan.importance,
        backup_dir,
    ) {
        Ok(outcome) => {
            let chosen: Vec<String> = if branch == "attach" {
                candidate.clone().into_iter().collect()
            } else {
                Vec::new()
            };
            if let Err(e) = crate::grade_candidates(client, &plan.candidates, &chosen) {
                warnings.push(format!("query-log 自评失败：{e}"));
            }
            let mut resp = from_outcome(outcome);
            // rsrs 的 attach/create 回执可能不带 title；用本地生成的标题补齐，便于扩展通知展示
            if resp.title.as_deref().unwrap_or("").trim().is_empty() {
                resp.title = Some(title.clone());
            }
            resp.warnings.extend(warnings);
            resp.needs_confirm = needs_confirm || resp.needs_confirm;
            resp.candidates = plan.candidates.clone();
            (200, resp)
        }
        Err(e) => (502, ApiResponse::fail(format!("rsrs 写入失败：{e}"))),
    }
}

fn source_line(page_title: Option<&str>, url: Option<&str>) -> String {
    let title = page_title.unwrap_or("").trim();
    let url = url.unwrap_or("").trim();
    if url.is_empty() {
        return String::new();
    }
    let url = if url.chars().count() > 2000 {
        url.chars().take(2000).collect::<String>()
    } else {
        url.to_string()
    };
    if title.is_empty() {
        format!("来源：{url}")
    } else {
        format!("来源：《{title}》{url}")
    }
}

/// 来源行（复用 `source_line`；没有 URL 时不追加来源）
fn source_note(page_title: Option<&str>, url: Option<&str>) -> Option<String> {
    let line = source_line(page_title, url);
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// 按快捷目标落库（P9）：目标不存在 → 400；写入失败 → 502
fn remember_by_target(
    ctx: &Ctx<'_>,
    target_id: &str,
    text: &str,
    page_title: Option<&str>,
    url: Option<&str>,
) -> (u16, ApiResponse) {
    let Some(target) = ctx.targets.lock().unwrap().get(target_id) else {
        return (400, ApiResponse::fail(format!("目标不存在：{target_id}")));
    };
    let source_note = source_note(page_title, url);
    match quick::save_to_target(
        ctx.client,
        ctx.cfg,
        ctx.backup_dir,
        &target,
        text,
        source_note.as_deref(),
    ) {
        Ok(outcome) => (200, from_quick(outcome)),
        Err(e) => (502, ApiResponse::fail(e)),
    }
}

/// 快捷目标落库结果 → 既有 ApiResponse 形状
fn from_quick(o: QuickOutcome) -> ApiResponse {
    ApiResponse {
        ok: o.ok,
        branch: Some(o.branch),
        action: None,
        id: o.id,
        parent: o.parent,
        title: o.title,
        message: o.message,
        warnings: o.warnings,
        candidates: Vec::new(),
        needs_confirm: o.needs_confirm,
    }
}

/// 分类树根（走缓存 + 有界超时；取不到时返回空数组，不让整个请求失败）
fn taxonomy_out(client: &RsrsClient) -> Vec<TaxonomyOut> {
    client
        .list_taxonomy()
        .unwrap_or_default()
        .into_iter()
        .map(TaxonomyOut::from_item)
        .collect()
}

fn from_remember(res: RememberResponse, branch: &str) -> ApiResponse {
    if res.is_dangling() {
        let candidates = pending_candidates(&res);
        return ApiResponse {
            ok: false,
            branch: Some(branch.into()),
            action: None,
            id: None,
            parent: None,
            title: None,
            message: "rsrs 返回判重候选但未写入（候选悬空），请回 memTools 择一处理。".into(),
            warnings: res.warnings.clone(),
            candidates,
            needs_confirm: true,
        };
    }
    let summary = res.summary.clone().unwrap_or_default();
    ApiResponse {
        ok: true,
        branch: Some(branch.into()),
        action: summary.action,
        id: summary.id,
        parent: summary.parent,
        title: summary.title,
        message: "已保存到日记（trivial）。".into(),
        warnings: res.warnings.clone(),
        candidates: Vec::new(),
        needs_confirm: false,
    }
}

fn from_outcome(o: automation::Outcome) -> ApiResponse {
    ApiResponse {
        ok: o.ok,
        branch: Some(o.branch),
        action: o.action,
        id: o.id,
        parent: o.parent,
        title: o.title,
        message: o.message,
        warnings: o.warnings,
        candidates: o.candidates,
        needs_confirm: o.needs_confirm,
    }
}

fn pending_candidates(res: &RememberResponse) -> Vec<CandidateOut> {
    res.pending
        .iter()
        .map(|c| CandidateOut {
            id: c.id.clone(),
            title: c.title.clone(),
            score: c.similarity.unwrap_or(0.0) / 100.0,
            source: "remember".into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_32_hex_and_unique() {
        let a = gen_token();
        let b = gen_token();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn source_line_includes_title_and_url() {
        assert_eq!(
            source_line(Some("示例页"), Some("https://example.com/a")),
            "来源：《示例页》https://example.com/a"
        );
        assert_eq!(
            source_line(None, Some("https://x.test")),
            "来源：https://x.test"
        );
        assert_eq!(source_line(Some("t"), None), "");
    }

    #[test]
    fn source_line_caps_url() {
        let long = "h".repeat(3000);
        let line = source_line(None, Some(&long));
        assert_eq!(line.chars().count(), "来源：".chars().count() + 2000);
    }

    #[test]
    fn source_note_reuses_source_line_format() {
        assert_eq!(
            source_note(Some("示例页"), Some("https://example.com/a")).as_deref(),
            Some("来源：《示例页》https://example.com/a")
        );
        assert_eq!(source_note(None, None), None);
        assert_eq!(source_note(Some("示例页"), None), None);
    }

    #[test]
    fn remember_in_parses_target_id() {
        let v: RememberIn =
            serde_json::from_str(r#"{"text":"正文","target_id":"journal"}"#).expect("应能解析");
        assert_eq!(v.text, "正文");
        assert_eq!(v.target_id.as_deref(), Some("journal"));
        assert!(v.mode.is_none());
    }

    #[test]
    fn remember_in_without_target_id_keeps_mode() {
        let v: RememberIn =
            serde_json::from_str(r#"{"text":"正文","mode":"journal"}"#).expect("应能解析");
        assert_eq!(v.mode.as_deref(), Some("journal"));
        assert!(v.target_id.is_none());
    }

    #[test]
    fn open_in_parses_optional_fields() {
        let v: OpenIn = serde_json::from_str(
            r#"{"text":"选区文字","url":"https://x.test","page_title":"页面"}"#,
        )
        .expect("应能解析");
        assert_eq!(v.text.as_deref(), Some("选区文字"));
        assert_eq!(v.url.as_deref(), Some("https://x.test"));
        assert_eq!(v.page_title.as_deref(), Some("页面"));

        let empty: OpenIn = serde_json::from_str("{}").expect("应能解析");
        assert!(empty.text.is_none());
    }

    #[test]
    fn targets_in_parses_quick_targets() {
        let v: TargetsIn = serde_json::from_str(
            r#"{"targets":[{"id":"t1","label":"AI","kind":"tree","parent":"AI-domain","type":"decision","importance":"important"}]}"#,
        )
        .expect("应能解析");
        assert_eq!(v.targets.len(), 1);
        assert_eq!(v.targets[0].id, "t1");
        assert_eq!(v.targets[0].r#type.as_deref(), Some("decision"));
        assert_eq!(v.targets[0].parent.as_deref(), Some("AI-domain"));
    }

    #[test]
    fn targets_response_serializes_type_and_omits_absent_message() {
        let body = TargetsResponse {
            ok: true,
            targets: vec![QuickTarget {
                id: "t1".into(),
                label: "AI".into(),
                kind: "tree".into(),
                parent: Some("AI-domain".into()),
                r#type: Some("decision".into()),
                importance: Some("important".into()),
            }],
            taxonomy: Vec::new(),
            message: None,
        };
        let v = serde_json::to_value(&body).expect("应能序列化");
        assert_eq!(v["ok"], serde_json::json!(true));
        assert_eq!(v["targets"][0]["type"], serde_json::json!("decision"));
        assert!(v.get("message").is_none());
    }

    #[test]
    fn targets_response_reports_failure_message() {
        let body = TargetsResponse {
            ok: false,
            targets: Vec::new(),
            taxonomy: Vec::new(),
            message: Some("目标 id 重复：t1".into()),
        };
        let v = serde_json::to_value(&body).expect("应能序列化");
        assert_eq!(v["ok"], serde_json::json!(false));
        assert_eq!(v["message"], serde_json::json!("目标 id 重复：t1"));
        assert_eq!(v["targets"].as_array().map(|a| a.len()), Some(0));
    }

    // -----------------------------------------------------------------------
    // HTTP 路由实测（契约 §2.4）：无界面端点，不依赖 rsrs runtime
    // -----------------------------------------------------------------------

    /// 测试用临时目录（进程号 + 纳秒，避免撞车）
    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "memtools-server-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("创建测试临时目录失败");
        dir
    }

    /// 起一个无界面端点（端口 0 = 系统分配）+ 独立的目标存储
    fn start_test_server(tag: &str) -> (String, String, PathBuf) {
        start_test_server_with(tag, |_| {})
    }

    /// 同上，允许在启动前调整配置（例如关闭梳理）
    fn start_test_server_with(
        tag: &str,
        tweak: impl FnOnce(&mut AppConfig),
    ) -> (String, String, PathBuf) {
        let dir = temp_dir(tag);
        let mut cfg = AppConfig::load();
        cfg.extension.enabled = true;
        cfg.extension.port = 0;
        cfg.extension.token = String::new();
        // 测试不依赖仓库里的 config.toml：处理器配置先归一，再交给 tweak 调整
        cfg.processor.enabled = true;
        cfg.processor.endpoint = "http://127.0.0.1:1/v1/chat/completions".into();
        cfg.processor.model = "mock".into();
        cfg.processor.agent_endpoint = String::new();
        cfg.processor.agent.enabled = false;
        cfg.processor.agent.kind = "opencode".into();
        cfg.processor.agent.bin = String::new();
        cfg.processor.agent.model = String::new();
        tweak(&mut cfg);
        let store = Arc::new(std::sync::Mutex::new(TargetStore::load(dir.join("targets.json"))));
        let processor = Arc::new(ProcessorSettings::load(dir.join("processor.json")));
        let started = start_without_window(
            cfg,
            dir.join("token.txt"),
            dir.join("backups"),
            store,
            processor,
        )
        .expect("端点应能启动");
        (
            format!("http://127.0.0.1:{}", started.port),
            started.token,
            dir,
        )
    }

    /// 无人监听的端口（runtime 不可达）
    fn dead_base_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("应能绑定本地端口");
        let port = listener.local_addr().expect("应有本地地址").port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    /// 起一个 rsrs runtime 不可达的无界面端点
    fn start_offline_server(tag: &str) -> (String, String, PathBuf) {
        let dir = temp_dir(tag);
        let mut cfg = AppConfig::load();
        cfg.extension.enabled = true;
        cfg.extension.port = 0;
        cfg.extension.token = String::new();
        cfg.rsrs.base_url = dead_base_url();
        let store = Arc::new(std::sync::Mutex::new(TargetStore::load(dir.join("targets.json"))));
        let processor = Arc::new(ProcessorSettings::load(dir.join("processor.json")));
        let started = start_without_window(
            cfg,
            dir.join("token.txt"),
            dir.join("backups"),
            store,
            processor,
        )
        .expect("端点应能启动");
        (
            format!("http://127.0.0.1:{}", started.port),
            started.token,
            dir,
        )
    }

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build()
    }

    #[test]
    fn http_health_and_root_return_ok() {
        let (base, token, dir) = start_test_server("health");
        let agent = agent();
        let health: serde_json::Value = agent
            .get(&format!("{base}/health"))
            .set("X-MemTools-Token", &token)
            .call()
            .expect("/health 应返回 200")
            .into_json()
            .expect("应返回 JSON");
        assert_eq!(health["ok"], serde_json::json!(true));
        assert_eq!(health["name"], serde_json::json!("memTools"));

        let root: serde_json::Value = agent
            .get(&format!("{base}/"))
            .set("X-MemTools-Token", &token)
            .call()
            .expect("/ 应返回 200")
            .into_json()
            .expect("应返回 JSON");
        assert_eq!(root["ok"], serde_json::json!(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_requires_token_and_answers_preflight() {
        let (base, _token, dir) = start_test_server("auth");
        let agent = agent();
        match agent.get(&format!("{base}/health")).call() {
            Err(ureq::Error::Status(401, _)) => {}
            other => panic!("无令牌应返回 401，实际：{other:?}"),
        }
        let resp = agent
            .request("OPTIONS", &format!("{base}/targets"))
            .call()
            .expect("预检应返回 204");
        assert_eq!(resp.status(), 204);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_targets_get_and_put_round_trip() {
        let (base, token, dir) = start_test_server("targets");
        let agent = agent();
        let listed: serde_json::Value = agent
            .get(&format!("{base}/targets"))
            .set("X-MemTools-Token", &token)
            .call()
            .expect("GET /targets 应返回 200")
            .into_json()
            .expect("应返回 JSON");
        assert_eq!(listed["ok"], serde_json::json!(true));
        assert!(listed["targets"].is_array());
        assert!(listed["taxonomy"].is_array());

        let saved: serde_json::Value = agent
            .put(&format!("{base}/targets"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({
                "targets": [{
                    "id": "t1", "label": "AI 挂点", "kind": "tree",
                    "parent": "AI-domain", "type": "decision", "importance": "important"
                }]
            }))
            .expect("PUT /targets 应返回 200")
            .into_json()
            .expect("应返回 JSON");
        assert_eq!(saved["ok"], serde_json::json!(true));
        assert_eq!(saved["targets"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(saved["targets"][0]["id"], serde_json::json!("t1"));
        assert_eq!(saved["targets"][0]["type"], serde_json::json!("decision"));

        let listed: serde_json::Value = agent
            .get(&format!("{base}/targets"))
            .set("X-MemTools-Token", &token)
            .call()
            .expect("GET /targets 应返回 200")
            .into_json()
            .expect("应返回 JSON");
        assert_eq!(listed["targets"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(listed["targets"][0]["label"], serde_json::json!("AI 挂点"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_targets_put_rejects_invalid_target() {
        let (base, token, dir) = start_test_server("targets-bad");
        let agent = agent();
        let resp = agent
            .put(&format!("{base}/targets"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({
                "targets": [{"id": "t1", "label": "坏目标", "kind": "bogus"}]
            }))
            .expect_err("非法目标应返回 400");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 400);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["ok"], serde_json::json!(false));
        assert!(body["message"].as_str().unwrap_or("").contains("目标"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_remember_with_unknown_target_returns_400() {
        let (base, token, dir) = start_test_server("remember-target");
        let agent = agent();
        let resp = agent
            .post(&format!("{base}/remember"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({"text": "正文", "target_id": "nope"}))
            .expect_err("目标不存在应返回 400");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 400);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["ok"], serde_json::json!(false));
        assert_eq!(body["message"], serde_json::json!("目标不存在：nope"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_remember_with_unknown_mode_returns_400() {
        let (base, token, dir) = start_test_server("remember-mode");
        let agent = agent();
        let resp = agent
            .post(&format!("{base}/remember"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({"text": "正文", "mode": "bogus"}))
            .expect_err("未知 mode 应返回 400");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 400);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["message"], serde_json::json!("未知 mode：bogus"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_open_without_window_reports_unavailable() {
        let (base, token, dir) = start_test_server("open");
        let agent = agent();
        let resp = agent
            .post(&format!("{base}/open"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({"text": "选区文字"}))
            .expect_err("无界面模式应返回 503");
        match resp {
            ureq::Error::Status(code, _) => assert_eq!(code, 503),
            other => panic!("应返回状态错误，实际：{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_process_unknown_task_returns_400() {
        let (base, token, dir) = start_test_server("process-task");
        let agent = agent();
        let resp = agent
            .post(&format!("{base}/process"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({"content": "正文", "task": "bogus"}))
            .expect_err("未知任务应返回 400");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 400);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["ok"], serde_json::json!(false));
        assert!(body["message"]
            .as_str()
            .unwrap_or("")
            .contains("未知梳理任务"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_process_disabled_returns_503() {
        let (base, token, dir) = start_test_server_with("process-off", |cfg| {
            cfg.processor.enabled = false;
        });
        let agent = agent();
        let resp = agent
            .post(&format!("{base}/process"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({"content": "正文", "task": "summarize"}))
            .expect_err("梳理未启用应返回 503");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 503);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["ok"], serde_json::json!(false));
        assert!(body["message"].as_str().unwrap_or("").contains("未启用"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_remember_with_disabled_process_returns_503() {
        let (base, token, dir) = start_test_server_with("remember-process", |cfg| {
            cfg.processor.enabled = false;
        });
        let agent = agent();
        let resp = agent
            .post(&format!("{base}/remember"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({"text": "正文", "process": "summarize"}))
            .expect_err("梳理未启用时不应静默落原文");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 503);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["ok"], serde_json::json!(false));
        assert!(body["message"].as_str().unwrap_or("").contains("梳理失败"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 起一个 mock LLM 端点，固定返回给定正文，返回端口
    fn spawn_mock_llm(reply: &'static str) -> u16 {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("mock LLM 应能启动");
        let port = server
            .server_addr()
            .to_ip()
            .expect("mock 地址应为 IP")
            .port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let body = serde_json::json!({
                    "choices": [{ "message": { "content": reply } }]
                })
                .to_string();
                let _ = req.respond(tiny_http::Response::from_string(body));
            }
        });
        port
    }

    #[test]
    fn process_error_codes_distinguish_disabled_and_unreachable() {
        assert_eq!(process_error_code("梳理功能未启用（...）"), 503);
        assert_eq!(process_error_code("未配置模型端点（...）"), 503);
        assert_eq!(process_error_code("未找到 qodercli：请把它装好"), 503);
        assert_eq!(process_error_code("opencode 调用超时（240s）"), 504);
        assert_eq!(process_error_code("模型端点不可达: connection refused"), 502);
        assert_eq!(process_error_code("模型端点 HTTP 500: boom"), 502);
    }

    #[test]
    fn http_process_with_mock_llm_returns_summary() {
        let port = spawn_mock_llm("【行为】mock 摘要");
        let (base, token, dir) = start_test_server_with("process-mock", |cfg| {
            cfg.processor.enabled = true;
            cfg.processor.endpoint = format!("http://127.0.0.1:{port}/v1/chat/completions");
            cfg.processor.model = "mock".into();
            cfg.processor.timeout_secs = 10;
        });
        let agent = agent();
        let body: serde_json::Value = agent
            .post(&format!("{base}/process"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({
                "content": "一段需要总结的网页文字",
                "task": "summarize"
            }))
            .expect("/process 应返回 200")
            .into_json()
            .expect("应返回 JSON");
        assert_eq!(body["ok"], serde_json::json!(true));
        assert_eq!(body["task"], serde_json::json!("summarize"));
        assert_eq!(body["content"], serde_json::json!("【行为】mock 摘要"));
        assert_eq!(body["source"], serde_json::json!("llm"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_agent_reports_disabled_by_default() {
        let (base, token, dir) = start_test_server("agent-off");
        let body: serde_json::Value = agent()
            .get(&format!("{base}/agent"))
            .set("X-MemTools-Token", &token)
            .call()
            .expect("/agent 应返回 200")
            .into_json()
            .expect("应返回 JSON");
        assert_eq!(body["ok"], serde_json::json!(false));
        assert_eq!(body["enabled"], serde_json::json!(false));
        assert_eq!(body["source"], serde_json::json!("llm"));
        assert!(body["message"]
            .as_str()
            .unwrap_or("")
            .contains("未启用 agent"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_process_with_agent_route_reports_unknown_kind() {
        let (base, token, dir) = start_test_server_with("process-agent", |cfg| {
            cfg.processor.enabled = true;
            cfg.processor.endpoint = String::new(); // agent 路径不需要 LLM 端点
            cfg.processor.agent.enabled = true;
            cfg.processor.agent.kind = "claude".into();
        });
        let resp = agent()
            .post(&format!("{base}/process"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({ "content": "正文", "task": "summarize" }))
            .expect_err("未知 agent 类型应返回 502");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 502);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["ok"], serde_json::json!(false));
        assert!(body["message"]
            .as_str()
            .unwrap_or("")
            .contains("未知 agent 类型"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_remember_process_then_unknown_target_returns_400_without_writing() {
        let port = spawn_mock_llm("【行为】mock 摘要");
        let (base, token, dir) = start_test_server_with("remember-process-mock", |cfg| {
            cfg.processor.enabled = true;
            cfg.processor.endpoint = format!("http://127.0.0.1:{port}/v1/chat/completions");
            cfg.processor.model = "mock".into();
            cfg.processor.timeout_secs = 10;
        });
        let agent = agent();
        let resp = agent
            .post(&format!("{base}/remember"))
            .set("X-MemTools-Token", &token)
            .send_json(serde_json::json!({
                "text": "一段需要总结的网页文字",
                "process": "summarize",
                "target_id": "nope"
            }))
            .expect_err("目标不存在应返回 400（说明梳理已成功、随后才查目标）");
        let body: serde_json::Value = match resp {
            ureq::Error::Status(code, resp) => {
                assert_eq!(code, 400);
                resp.into_json().expect("应返回 JSON")
            }
            other => panic!("应返回状态错误，实际：{other:?}"),
        };
        assert_eq!(body["message"], serde_json::json!("目标不存在：nope"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn http_targets_stays_fast_when_rsrs_unreachable() {
        let (base, token, dir) = start_offline_server("targets-offline");
        let agent = agent();
        let started_at = std::time::Instant::now();
        let listed: serde_json::Value = agent
            .get(&format!("{base}/targets"))
            .set("X-MemTools-Token", &token)
            .call()
            .expect("runtime 不可达时 GET /targets 也应返回 200")
            .into_json()
            .expect("应返回 JSON");
        let elapsed = started_at.elapsed();
        eprintln!("[memTools] runtime 不可达时 GET /targets 耗时 {elapsed:?}");
        assert_eq!(listed["ok"], serde_json::json!(true));
        assert!(
            listed["targets"].is_array(),
            "targets 应完整返回，不受 runtime 影响"
        );
        assert_eq!(
            listed["taxonomy"].as_array().map(|a| a.len()),
            Some(0),
            "runtime 不可达时分类树应降级为空数组"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "GET /targets 不应被 runtime 拖住，实际耗时 {elapsed:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
