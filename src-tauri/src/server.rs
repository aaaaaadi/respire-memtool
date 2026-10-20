//! 浏览器扩展本机端点（P6-②）
//!
//! - 只绑定 `127.0.0.1`，供本机 Chrome/Edge 扩展提交网页选中文字；
//! - 所有接口要求 `X-MemTools-Token`（或 `Authorization: Bearer <token>`）；
//! - 令牌默认自动生成到 `{app_config_dir}/extension_token.txt`，不写进仓库；
//! - 落库复用 `RsrsClient` + `automation`，不 shell 拼接、不启停 rsrs 服务。
//!
//! 路由：
//!   GET  /health    → { ok, name, version }
//!   POST /remember  → { text, mode: "journal"|"auto", url?, page_title?, title? }
//!
//! `mode=auto` 的冲突处理：判重命中高相似条目时**不执行破坏性分支**（改/并），
//! 退化为「挂到候选之下」并在响应里标记 `needs_confirm`，由用户回小窗核对。

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::automation::{self, CandidateOut};
use crate::config::AppConfig;
use crate::rsrs_client::{RememberRequest, RememberResponse, RsrsClient};

const MAX_BODY_BYTES: u64 = 256 * 1024;
const MAX_TEXT_CHARS: usize = 8000;

/// 端点启动结果（端口可能因 port=0 被系统分配）
pub struct Started {
    pub port: u16,
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct RememberIn {
    text: String,
    #[serde(default)]
    title: Option<String>,
    /// journal | auto（默认 auto）
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    page_title: Option<String>,
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
pub fn start(cfg: AppConfig, token_path: PathBuf, backup_dir: PathBuf) -> Result<Started, String> {
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
        let client = RsrsClient::new(
            cfg.rsrs.base_url.clone(),
            cfg.rsrs.mcp_path.clone(),
            cfg.rsrs.tools.clone(),
            cfg.rsrs.timeout_secs,
            cfg.rsrs.op_timeout_secs,
        )
        .with_token(cfg.rsrs.auth_token.clone());
        for request in server.incoming_requests() {
            handle(request, &client, &cfg, &thread_token, &backup_dir);
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

/// 本地令牌：CSPRNG 生成的 128 bit（UUID v4），用于回环端点鉴权
pub fn gen_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn handle(mut req: Request, client: &RsrsClient, cfg: &AppConfig, token: &str, backup_dir: &Path) {
    let method = req.method().clone();
    let path = req.url().split('?').next().unwrap_or("").to_string();

    if method == Method::Options {
        respond(req, 204, "");
        return;
    }

    if !authorized(&req, token) {
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
            let (code, resp) = remember(client, cfg, backup_dir, input);
            respond(req, code, &serde_json::to_string(&resp).unwrap_or_default());
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
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
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

fn remember(
    client: &RsrsClient,
    cfg: &AppConfig,
    backup_dir: &Path,
    input: RememberIn,
) -> (u16, ApiResponse) {
    let text = input.text.trim();
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
        if cfg.automation.llm.endpoint.trim().is_empty() {
            warnings.push(
                "未配置 [automation.llm] endpoint，已用本地判档；如需模型判档请填写任意 OpenAI 兼容端点。"
                    .into(),
            );
        } else {
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
        Err(e) => {
            return (
                502,
                ApiResponse::fail(crate::with_index_hint(
                    client,
                    format!("生成计划失败：{e}"),
                )),
            )
        }
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
}
