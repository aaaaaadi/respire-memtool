//! rsrs-client — MCP JSON-RPC over HTTP 客户端
//!
//! rsrs runtime 的真实接口为 MCP over HTTP：POST {base_url}/mcp，
//! JSON-RPC 2.0（initialize + tools/call），暴露 15 个 memory_* 工具。
//! 本模块负责：initialize 握手、tools/call 调用、JSON-RPC 响应解析、
//! rsrs envelope（actions/errors/items/status/summary）解析与错误分类。
//!
//! 红线：不启停 rsrs 服务、不使用 --direct、不 shell 拼接命令。

use std::sync::Mutex;
use std::io::{BufRead, BufReader, Cursor};

use serde::{Deserialize, Serialize};

use crate::config::RsrsToolNames;

// ---------------------------------------------------------------------------
// 错误分类
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum RsrsError {
    /// 连接失败（runtime 不可达 / HTTP transport 错误）
    Unreachable(String),
    /// 鉴权失败（HTTP 401/403 或 JSON-RPC 鉴权错误）
    AuthFailed(String),
    /// 候选悬空（判重候选已返回但未写入）
    CandidatesDangling(String),
    /// MCP / JSON-RPC 层错误（error 对象、isError、envelope.errors、status != ok）
    Mcp(String),
    /// 响应 JSON 解析失败
    Parse(String),
    /// 其他
    Other(String),
}

impl std::fmt::Display for RsrsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RsrsError::Unreachable(m) => write!(f, "runtime 不可达（连接失败）: {m}"),
            RsrsError::AuthFailed(m) => write!(f, "鉴权失败: {m}"),
            RsrsError::CandidatesDangling(m) => write!(f, "候选悬空（返回判重候选但未写入）: {m}"),
            RsrsError::Mcp(m) => write!(f, "MCP 错误: {m}"),
            RsrsError::Parse(m) => write!(f, "响应解析失败: {m}"),
            RsrsError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for RsrsError {}

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 分类树根（memory_taxonomy 返回的 items）
#[derive(Debug, Clone, Deserialize)]
pub struct TaxonomyItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub domain: Option<String>,
    /// 分类描述（value 中 "名 - 描述" 的 "描述" 部分）
    #[serde(default)]
    pub description: Option<String>,
}

/// memory_remember 入参（arguments 直映射）
#[derive(Debug, Clone, Default, Serialize)]
pub struct RememberRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// parent id 或分类目录标题（catalog title）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// important | trivial
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_ids: Option<String>,
}

/// remember 结果摘要（rsrs envelope.summary）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RememberSummary {
    pub action: Option<String>,
    pub id: Option<String>,
    pub parent: Option<String>,
    pub title: Option<String>,
    pub count: Option<u64>,
    /// 判重候选态：candidates
    pub state: Option<String>,
    /// 是否已写入（候选悬空时 false）
    pub written: Option<bool>,
    pub merge_count: Option<u64>,
    pub parent_count: Option<u64>,
}

/// 判重候选（remember 悬空时 items[] 中的待决条目）
#[derive(Debug, Clone, Default, Serialize)]
pub struct PendingCandidate {
    /// 8 位 id 前缀（rsrs 接受前缀）
    pub id: String,
    pub title: String,
    pub similarity: Option<f64>,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RememberResponse {
    /// rsrs envelope.status
    pub status: Option<String>,
    pub summary: Option<RememberSummary>,
    /// 写入条数（summary.count）
    pub count: Option<u64>,
    /// 候选悬空：items/actions 中的候选（原始字符串，兼容旧逻辑）
    pub candidates: Vec<String>,
    /// 候选悬空：结构化候选条目
    pub pending: Vec<PendingCandidate>,
    /// summary.state（candidates 表示悬空）
    pub state: Option<String>,
    /// summary.written（false 表示未写入）
    pub written: Option<bool>,
    /// 非致命告警（hook continuing 等）
    pub warnings: Vec<String>,
    /// 原始 envelope（便于排查）
    pub raw: Option<String>,
}

impl RememberResponse {
    /// 候选悬空判定：返回了判重候选但未写入，必须由程序侧择一执行
    pub fn is_dangling(&self) -> bool {
        if self.written == Some(false) {
            return true;
        }
        if !self.pending.is_empty() || !self.candidates.is_empty() {
            return true;
        }
        if let Some(st) = &self.status {
            if st.to_lowercase() == "pending" {
                return true;
            }
        }
        if let Some(st) = &self.state {
            if st.to_lowercase().contains("candidate") {
                return true;
            }
        }
        if let Some(sum) = &self.summary {
            if let Some(action) = &sum.action {
                let a = action.to_lowercase();
                if a.contains("candidate") || a.contains("候选") {
                    return true;
                }
            }
        }
        false
    }

    /// 合并类候选的 id 前缀列表（用于 --merge-ids）
    pub fn candidate_ids(&self) -> Vec<String> {
        self.pending
            .iter()
            .map(|c| c.id.clone())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// recall 候选（memory_recall 的 details[] 元素）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RecallCandidate {
    pub entry: RecallEntry,
    pub score: f64,
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RecallEntry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub kind: Option<String>,
    pub importance: Option<String>,
    pub parent_id: Option<String>,
}

/// show 结果（memory_show 的 details）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ShownMemory {
    pub entry: RecallEntry,
    pub ancestors: Vec<NodeRef>,
    pub children: Vec<NodeRef>,
}

/// memory_list 返回的最近条目（details[]，含时间戳）
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ListedEntry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub kind: Option<String>,
    pub importance: Option<String>,
    pub parent_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct NodeRef {
    pub id: String,
    pub title: String,
}

/// 扁平化的树节点（memory_tree）
#[derive(Debug, Clone, Default, Serialize)]
pub struct TreeNode {
    pub id: String,
    pub title: String,
    pub kind: Option<String>,
    pub depth: u32,
    pub is_leaf: bool,
}

/// outline 模式的树节点（memory_tree --outline）：只含层级、id、标题
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OutlineNode {
    pub depth: u32,
    pub id: String,
    pub short_id: String,
    pub title: String,
}

/// runtime 状态摘要（离线/待同步/数据目录）
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusInfo {
    pub online: bool,
    /// rsrs 自报离线（本地可用，等待 sync 补传）
    pub offline: Option<bool>,
    pub autosync: Option<bool>,
    /// 待上传条数（sync_live.pending，取不到时回退 recall_statistics.pending）
    pub pending: u64,
    pub conflicts: u64,
    pub data_dir: Option<String>,
    /// sync_scheduler.last_success_unix
    pub last_sync: Option<i64>,
    pub phase: Option<String>,
    /// 语义检索索引状态（summary.retrieval_index.state）
    pub index_state: Option<String>,
    /// 语义索引错误（summary.retrieval_index.error，可为 null）
    pub index_error: Option<String>,
    /// 索引可用（旧版 rsrs 无 retrieval_index 时视为可用）
    pub index_ready: bool,
    /// 索引构建线程是否运行中
    pub index_worker_active: bool,
}

/// rsrs envelope：所有 memory_* 工具统一返回的包装结构
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RsrsEnvelope {
    #[serde(default)]
    pub actions: Vec<serde_json::Value>,
    #[serde(default)]
    pub errors: Vec<serde_json::Value>,
    #[serde(default)]
    pub items: Vec<RsrsItem>,
    #[serde(default)]
    pub related: Vec<serde_json::Value>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub summary: Option<serde_json::Value>,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    /// 非致命告警（如 rsrs 插件 hook 超时 "(continuing)"）——不视为失败
    #[serde(skip)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RsrsItem {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP 客户端
// ---------------------------------------------------------------------------

pub struct RsrsClient {
    base_url: String,
    mcp_path: String,
    tools: RsrsToolNames,
    /// 可选鉴权令牌（非空时发送 Authorization: Bearer）
    token: String,
    timeout: std::time::Duration,
    /// 慢操作超时（remember/recall/update/attach/show/tree）—— 插件 hook 可能耗时 >5s
    slow_timeout: std::time::Duration,
    /// 连接内部状态：JSON-RPC id 递增 + initialize 握手标记
    inner: Mutex<Inner>,
}

impl Clone for RsrsClient {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            mcp_path: self.mcp_path.clone(),
            tools: self.tools.clone(),
            token: self.token.clone(),
            timeout: self.timeout,
            slow_timeout: self.slow_timeout,
            inner: Mutex::new(Inner {
                id: 0,
                initialized: false,
            }),
        }
    }
}

#[derive(Debug)]
struct Inner {
    id: u64,
    initialized: bool,
}

impl RsrsClient {
    pub fn new(
        base_url: String,
        mcp_path: String,
        tools: RsrsToolNames,
        timeout_secs: u64,
        op_timeout_secs: u64,
    ) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            mcp_path: mcp_path.trim_start_matches('/').to_string(),
            tools,
            token: String::new(),
            timeout: std::time::Duration::from_secs(timeout_secs.max(1)),
            slow_timeout: std::time::Duration::from_secs(op_timeout_secs.max(1)),
            inner: Mutex::new(Inner {
                id: 0,
                initialized: false,
            }),
        }
    }

    /// 设置 rsrs 鉴权令牌（空白视为未配置）
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = token.into().trim().to_string();
        self
    }

    fn with_auth(&self, req: ureq::Request) -> ureq::Request {
        let req = req.set("Accept", "application/json, text/event-stream");
        if self.token.is_empty() {
            req
        } else {
            req.set("Authorization", &format!("Bearer {}", self.token))
        }
    }

    fn mcp_url(&self) -> String {
        format!("{}/{}", self.base_url, self.mcp_path)
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout(self.timeout)
            .build()
    }

    fn agent_slow(&self) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout(self.slow_timeout)
            .build()
    }

    /// MCP initialize 握手（幂等；失败不阻塞，tools/call 仍会尝试）
    fn ensure_initialized(&self, inner: &mut Inner, agent: &ureq::Agent) {
        if inner.initialized {
            return;
        }
        inner.initialized = true; // 只尝试一次
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": inner.id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "memTools", "version": env!("CARGO_PKG_VERSION") }
            }
        });
        inner.id += 1;
        // 握手用极短超时（500ms），避免不可达时拖慢只读探测
        let handshake_agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout(std::time::Duration::from_millis(500))
            .build();
        match self.with_auth(handshake_agent.post(&self.mcp_url())).send_json(&req) {
            Ok(resp) => {
                let _ = Self::parse_http(resp, &req["id"]);
                // 保持原有兼容行为：握手失败不阻塞后续调用，但 SSE 不再等连接关闭。
            }
            Err(_) => {}
        }
        // 通知 initialized（无响应期待）
        let _ = self
            .with_auth(agent.post(&self.mcp_url()))
            .send_json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {}
            }));
    }

    /// 调用 MCP 工具（快速超时）：status/taxonomy 等只读探测
    fn call_tool(&self, tool: &str, arguments: &impl Serialize) -> Result<RsrsEnvelope, RsrsError> {
        let agent = self.agent();
        self.call_tool_with(&agent, tool, arguments)
    }

    /// 调用 MCP 工具（慢超时）：remember/recall/update 等可能触发插件 hook 者
    fn call_tool_slow(
        &self,
        tool: &str,
        arguments: &impl Serialize,
    ) -> Result<RsrsEnvelope, RsrsError> {
        let agent = self.agent_slow();
        self.call_tool_with(&agent, tool, arguments)
    }

    fn call_tool_with(
        &self,
        agent: &ureq::Agent,
        tool: &str,
        arguments: &impl Serialize,
    ) -> Result<RsrsEnvelope, RsrsError> {
        let url = self.mcp_url();

        let mut inner = self
            .inner
            .lock()
            .map_err(|_| RsrsError::Other("连接状态锁失效".into()))?;
        self.ensure_initialized(&mut inner, agent);

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": inner.id,
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": arguments
            }
        });
        inner.id += 1;
        drop(inner);

        let resp: serde_json::Value = match self.with_auth(agent.post(&url)).send_json(&req) {
            Ok(r) | Err(ureq::Error::Status(_, r)) => Self::parse_http(r, &req["id"])?,
            Err(ureq::Error::Transport(t)) => {
                return Err(RsrsError::Unreachable(t.to_string()));
            }
        };

        // JSON-RPC error 对象
        if let Some(e) = resp.get("error") {
            return Err(Self::jsonrpc_error(e));
        }
        // result
        let result = resp
            .get("result")
            .ok_or_else(|| RsrsError::Mcp("响应缺少 result".into()))?;

        if result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false) {
            let text = extract_text(result);
            return Err(RsrsError::Mcp(format!(
                "工具 {tool} 执行失败: {}",
                truncate(&text, 600)
            )));
        }

        // 拼接 content[].text 后解析 envelope
        let text = extract_text(result);
        if text.trim().is_empty() {
            return Err(RsrsError::Mcp(format!("工具 {tool} 返回空内容")));
        }
        let mut env: RsrsEnvelope = serde_json::from_str(&text)
            .map_err(|e| RsrsError::Parse(format!("envelope 解析失败: {e}（原文: {}）", truncate(&text, 300))))?;

        // 失败语义：envelope.errors 与 status。注意：
        //  - rsrs 插件 hook 超时是「非致命告警」(on_error=warn、continuing)，不得判失败（见记忆 ae2b3499）；
        //  - remember 判重候选时 status="pending" 是正常语义，必须继续解析，不能当错误。
        let mut fatal: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for e in &env.errors {
            let msg = e
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| e.to_string());
            if is_nonfatal_warning(&msg) {
                warnings.push(msg);
            } else {
                fatal.push(msg);
            }
        }
        if !fatal.is_empty() {
            let msg = fatal.join("; ");
            if looks_like_auth(&msg) {
                return Err(RsrsError::AuthFailed(msg));
            }
            if looks_like_candidate(&msg) {
                return Err(RsrsError::CandidatesDangling(msg));
            }
            return Err(RsrsError::Mcp(format!("工具 {tool}: {msg}")));
        }
        let st = env.status.clone().unwrap_or_default();
        let st_lc = st.to_lowercase();
        // recall 的零命中使用 skip，不是提交失败。仅放行明确的空召回；
        // 写入工具、缺少计数或自相矛盾的结果仍报错，不能把“跳过写入”当成功。
        let empty_recall = tool == self.tools.recall
            && st_lc == "skip"
            && env.summary.as_ref().and_then(|s| s.get("count")).and_then(|v| v.as_u64()) == Some(0)
            && env.details.as_ref().map_or(true, |v| v.as_array().map_or(false, |items| items.is_empty()));
        if !st.is_empty() && st_lc != "ok" && st_lc != "pending" && !empty_recall {
            let msg = format!("工具 {tool} status={st}");
            if looks_like_auth(&msg) {
                return Err(RsrsError::AuthFailed(msg));
            }
            return Err(RsrsError::Mcp(msg));
        }
        // 非致命告警已抽出到 warnings；errors 清空（致命项在上方已 return Err）
        env.warnings = warnings;
        env.errors = Vec::new();
        Ok(env)
    }

    fn parse_http(
        resp: ureq::Response,
        expected_id: &serde_json::Value,
    ) -> Result<serde_json::Value, RsrsError> {
        let status = resp.status();
        // 状态码已足以判定鉴权失败，正文中断不能把它误报为解析错误。
        if status == 401 || status == 403 {
            return Err(RsrsError::AuthFailed(format!("HTTP {status}")));
        }
        let is_sse = resp
            .header("Content-Type")
            .unwrap_or("")
            .split(';')
            .next()
            .map(str::trim)
            .map(|s| s.eq_ignore_ascii_case("text/event-stream"))
            .unwrap_or(false);
        // 1. SSE 收到本次请求的完整事件即可返回，不等待长连接关闭。
        if (200..300).contains(&status) && is_sse {
            return Self::read_sse(BufReader::new(resp.into_reader()), expected_id);
        }
        let text = resp
            .into_string()
            .map_err(|e| RsrsError::Parse(format!("读取响应失败：{e}")))?;
        // 2. 无 SSE 标头的兼容端点按正文识别；错误报文仍保留原有 JSON-RPC 分类。
        let trimmed = text.trim_start_matches('\u{feff}').trim_start();
        let parsed = if trimmed.starts_with('{') || trimmed.starts_with('[') {
            serde_json::from_str(trimmed).map_err(|e| RsrsError::Parse(e.to_string()))
        } else {
            Self::read_sse(Cursor::new(trimmed.as_bytes()), expected_id)
        };
        if !(200..300).contains(&status) {
            if let Ok(value) = &parsed {
                if let Some(error) = value.get("error") {
                    return Err(Self::jsonrpc_error(error));
                }
            }
            return Err(RsrsError::Mcp(format!("HTTP {status}")));
        }
        let value = parsed?;
        if value.get("id") != Some(expected_id) {
            return Err(RsrsError::Parse("JSON-RPC 响应 id 与请求不匹配".into()));
        }
        Ok(value)
    }

    /// 只返回当前请求的 result/error；心跳、通知和其他请求的事件不能冒充本次响应。
    fn read_sse(
        mut reader: impl BufRead,
        expected_id: &serde_json::Value,
    ) -> Result<serde_json::Value, RsrsError> {
        let mut line = String::new();
        let mut data = String::new();
        let mut first_line = true;
        let mut total = 0usize;
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| RsrsError::Parse(format!("读取 SSE 失败：{e}")))?;
            total += n;
            if total > 32 * 1024 * 1024 {
                return Err(RsrsError::Parse("SSE 响应超过 32MB".into()));
            }
            let mut field = line.trim_end_matches(['\r', '\n']);
            if first_line {
                field = field.trim_start_matches('\u{feff}');
                first_line = false;
            }
            // 1. 空行提交一个事件；多行 data 按 SSE 规范用换行连接。
            if n == 0 || field.is_empty() {
                if !data.is_empty() {
                    let value: serde_json::Value = serde_json::from_str(&data)
                        .map_err(|e| RsrsError::Parse(format!("SSE data JSON 无效：{e}")))?;
                    if value.get("id") == Some(expected_id)
                        && (value.get("result").is_some() || value.get("error").is_some())
                    {
                        return Ok(value);
                    }
                    data.clear();
                }
                if n == 0 {
                    return Err(RsrsError::Parse(
                        "SSE 中未收到匹配本次请求的 JSON-RPC 响应".into(),
                    ));
                }
            } else if field == "data" || field.starts_with("data:") {
                // 2. 忽略 event/id/retry 和注释，data 字段只去掉冒号后的一个可选空格。
                let value = field.strip_prefix("data:").unwrap_or("");
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
        }
    }

    fn jsonrpc_error(e: &serde_json::Value) -> RsrsError {
        let code = e.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let message = e.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
        let msg = format!("JSON-RPC code={code}: {message}");
        if code == -32001 || code == 401 || code == 403 || looks_like_auth(&message) {
            RsrsError::AuthFailed(msg)
        } else {
            RsrsError::Mcp(msg)
        }
    }

    // -----------------------------------------------------------------------
    // 业务方法
    // -----------------------------------------------------------------------

    /// memory_status：连通性探测（只读）
    pub fn status(&self) -> Result<RsrsEnvelope, RsrsError> {
        self.call_tool(&self.tools.status, &serde_json::json!({}))
    }

    /// memory_status → 离线 / 待同步摘要（连接失败时由调用方按不可达处理）
    pub fn status_info(&self) -> Result<StatusInfo, RsrsError> {
        let env = self.status()?;
        let s = env.summary.unwrap_or(serde_json::Value::Null);
        let pending = s
            .get("sync_live")
            .and_then(|v| v.get("pending"))
            .and_then(|v| v.as_u64())
            .or_else(|| {
                s.get("recall_statistics")
                    .and_then(|v| v.get("pending"))
                    .and_then(|v| v.as_u64())
            })
            .unwrap_or(0);
        let index = s.get("retrieval_index").filter(|v| !v.is_null());
        let (index_state, index_error, index_worker_active, index_ready) = match index {
            None => (None, None, false, true),
            Some(v) => {
                let state = v
                    .get("state")
                    .and_then(|x| x.as_str())
                    .map(|x| x.to_string());
                let error = v
                    .get("error")
                    .and_then(|x| x.as_str())
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty());
                let worker_active = v
                    .get("worker_active")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let ready = state.as_deref().map(|x| x.eq_ignore_ascii_case("ready"))
                    == Some(true)
                    && error.is_none();
                (state, error, worker_active, ready)
            }
        };
        Ok(StatusInfo {
            online: true,
            offline: s.get("offline").and_then(|v| v.as_bool()),
            autosync: s.get("autosync").and_then(|v| v.as_bool()),
            pending,
            conflicts: s
                .get("sync_live")
                .and_then(|v| v.get("conflicts"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            data_dir: s
                .get("data_dir")
                .and_then(|v| v.as_str())
                .map(|x| x.to_string()),
            last_sync: s
                .get("sync_scheduler")
                .and_then(|v| v.get("last_success_unix"))
                .and_then(|v| v.as_i64()),
            phase: s
                .get("sync_live")
                .and_then(|v| v.get("phase"))
                .and_then(|v| v.as_str())
                .map(|x| x.to_string()),
            index_state,
            index_error,
            index_ready,
            index_worker_active,
        })
    }

    /// memory_query_log_mark：自评召回候选（good = 被采纳 / bad = 被拒绝）
    ///
    /// 候选已滚出 query_log 时 rsrs 返回 status=warn（unmatched）；自评是尽力而为，
    /// 这类告警不视为失败。
    pub fn query_log_mark(&self, ids: &[String], good: bool) -> Result<(), RsrsError> {
        let ids: Vec<String> = ids
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        match self.call_tool(
            &self.tools.query_log_mark,
            &serde_json::json!({ "ids": ids.join(","), "good": good, "bad": !good }),
        ) {
            Ok(_) => Ok(()),
            Err(RsrsError::Mcp(msg)) if msg.contains("status=warn") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// memory_tree（outline=true）：目标树 / 挂点路径（depth 为相对层级）
    pub fn tree_outline(&self, from: &str, depth: u32) -> Result<Vec<OutlineNode>, RsrsError> {
        let env = self.call_tool_slow(
            &self.tools.tree,
            &serde_json::json!({ "from": from, "depth": depth, "outline": true }),
        )?;
        let mut out = Vec::new();
        if let Some(serde_json::Value::Array(arr)) = &env.details {
            for v in arr {
                if let Ok(n) = serde_json::from_value::<OutlineNode>(v.clone()) {
                    out.push(n);
                }
            }
        }
        Ok(out)
    }

    /// 当前激活账户的数据目录（status.summary.data_dir）。
    /// 图谱需要直接只读 SQLite / core-index；runtime 不可达时由调用方兜底。
    pub fn data_dir(&self) -> Result<String, RsrsError> {
        let env = self.status()?;
        let dd = env
            .summary
            .as_ref()
            .and_then(|s| s.get("data_dir"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if dd.is_empty() {
            Err(RsrsError::Parse("status 响应缺少 summary.data_dir".into()))
        } else {
            Ok(dd)
        }
    }

    /// memory_taxonomy：23 分类树根全量
    pub fn list_taxonomy(&self) -> Result<Vec<TaxonomyItem>, RsrsError> {
        let env = self.call_tool(&self.tools.taxonomy, &serde_json::json!({}))?;
        env.items
            .iter()
            .map(|item| {
                let domain = item.name.clone().unwrap_or_default();
                let value = item.value.clone().unwrap_or_default();
                // value 形如 "编程开发 - 编程开发之事：代码项目..."
                let (name, description) = match value.split_once(" - ") {
                    Some((n, d)) => (n.trim().to_string(), Some(d.trim().to_string())),
                    None => (value.clone(), None),
                };
                let name = if name.is_empty() { value.clone() } else { name };
                Ok(TaxonomyItem {
                    id: name.clone(),
                    name,
                    domain: Some(domain),
                    description,
                })
            })
            .collect()
    }

    /// 快速 taxonomy 调用（1s 超时），用于只读探测不阻塞 UI
    pub fn list_taxonomy_fast(&self) -> Result<Vec<TaxonomyItem>, RsrsError> {
        let client = self.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .try_proxy_from_env(false)
                .timeout(std::time::Duration::from_secs(1))
                .build();
            let result: Result<Vec<TaxonomyItem>, RsrsError> = client.call_tool_with(&agent, &client.tools.taxonomy, &serde_json::json!({}))
                .and_then(|env| {
                    env.items
                        .iter()
                        .map(|item| {
                            let domain = item.name.clone().unwrap_or_default();
                            let value = item.value.clone().unwrap_or_default();
                            let (name, description) = match value.split_once(" - ") {
                                Some((n, d)) => (n.trim().to_string(), Some(d.trim().to_string())),
                                None => (value.clone(), None),
                            };
                            let name = if name.is_empty() { value.clone() } else { name };
                            Ok(TaxonomyItem {
                                id: name.clone(),
                                name,
                                domain: Some(domain),
                                description,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                });
            let _ = tx.send(result);
        });
        match rx.recv_timeout(std::time::Duration::from_millis(1500)) {
            Ok(items) => items,
            Err(_) => Err(RsrsError::Unreachable("taxonomy 快速调用超时".into())),
        }
    }

    /// memory_remember：手动/自动路径提交
    /// - 日记：importance=trivial（一步落库，不挂树）
    /// - 主区：title/parent/type/importance=important（挂树根）
    /// - 判重命中：status=pending、summary.written=false、items[].status=pending（候选悬空）
    pub fn remember(&self, req: RememberRequest) -> Result<RememberResponse, RsrsError> {
        let args = serde_json::to_value(&req)
            .map_err(|e| RsrsError::Other(format!("序列化入参失败: {e}")))?;
        let env = self.call_tool_slow(&self.tools.remember, &args)?;

        let mut resp = RememberResponse {
            status: env.status.clone(),
            summary: None,
            count: None,
            candidates: Vec::new(),
            pending: Vec::new(),
            state: None,
            written: None,
            warnings: env.warnings.clone(),
            raw: Some(serde_json::to_string(&env).unwrap_or_default()),
        };

        if let Some(sum) = &env.summary {
            let parsed = serde_json::from_value::<RememberSummary>(sum.clone()).ok();
            resp.count = parsed.as_ref().and_then(|s| s.count);
            resp.state = parsed.as_ref().and_then(|s| s.state.clone());
            resp.written = parsed.as_ref().and_then(|s| s.written);
            resp.summary = parsed;
        }

        // 结构化候选：items[] 中 status=pending
        for it in &env.items {
            let status_pending = it
                .status
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case("pending"))
                .unwrap_or(false);
            let name = it.name.clone().unwrap_or_default();
            let value = it.value.clone().unwrap_or_default();
            if status_pending || looks_like_candidate(&name) {
                let (sim, title) = parse_candidate_value(&value);
                resp.pending.push(PendingCandidate {
                    id: name.clone(),
                    title,
                    similarity: sim,
                    action: None,
                });
                resp.candidates.push(if value.is_empty() {
                    name
                } else {
                    format!("{name} {value}")
                });
            }
        }
        // actions 中的 merge-ids 提示也视为悬空信号
        for a in &env.actions {
            let s = a.as_str().map(|x| x.to_string()).unwrap_or_else(|| a.to_string());
            if s.contains("merge-ids") {
                resp.candidates.push(s);
            }
        }
        Ok(resp)
    }

    /// memory_recall：语义检索候选（结构化）
    pub fn recall_candidates(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<RecallCandidate>, RsrsError> {
        let env = self.call_tool_slow(
            &self.tools.recall,
            &serde_json::json!({ "query": query, "limit": limit }),
        )?;
        let mut out: Vec<RecallCandidate> = Vec::new();
        if let Some(details) = &env.details {
            if let Ok(list) = serde_json::from_value::<Vec<RecallCandidate>>(details.clone()) {
                out = list;
            }
        }
        Ok(out)
    }

    /// memory_show：单条全文（含 ancestors/children）
    pub fn show_entry(&self, id: &str) -> Result<ShownMemory, RsrsError> {
        let env = self.call_tool_slow(&self.tools.show, &serde_json::json!({ "id": id }))?;
        let details = env
            .details
            .ok_or_else(|| RsrsError::Parse("show 响应缺少 details".into()))?;
        serde_json::from_value::<ShownMemory>(details)
            .map_err(|e| RsrsError::Parse(format!("show details 解析失败: {e}")))
    }

    /// memory_tree：从 from 起 depth 层，扁平化为节点列表（from 为空则返回森林）
    pub fn tree_flat(&self, from: &str, depth: u32) -> Result<Vec<TreeNode>, RsrsError> {
        let env = self.call_tool_slow(
            &self.tools.tree,
            &serde_json::json!({ "from": from, "depth": depth }),
        )?;
        let mut out: Vec<TreeNode> = Vec::new();
        if let Some(details) = &env.details {
            if let Some(arr) = details.as_array() {
                for node in arr {
                    flatten_node(node, 0, &mut out);
                }
            }
        }
        Ok(out)
    }

    // ---- 以下为 P2 自动化路径预留（MCP 工具名映射，保证 client 模块完整）----

    /// memory_recall：语义判重检索
    pub fn recall(&self, query: &str, limit: u32) -> Result<RsrsEnvelope, RsrsError> {
        self.call_tool_slow(
            &self.tools.recall,
            &serde_json::json!({ "query": query, "limit": limit }),
        )
    }

    /// memory_tree：因果树（forest / subtree / outline）
    pub fn tree(&self, from: &str, depth: u32) -> Result<RsrsEnvelope, RsrsError> {
        self.call_tool_slow(
            &self.tools.tree,
            &serde_json::json!({ "from": from, "depth": depth }),
        )
    }

    /// memory_show：单条全文
    pub fn show(&self, id: &str) -> Result<RsrsEnvelope, RsrsError> {
        self.call_tool_slow(&self.tools.show, &serde_json::json!({ "id": id }))
    }

    /// memory_update：更新 title/content/kind/importance
    pub fn update(
        &self,
        id: &str,
        content: Option<&str>,
        title: Option<&str>,
    ) -> Result<RsrsEnvelope, RsrsError> {
        let mut args = serde_json::json!({ "id": id });
        if let Some(c) = content {
            args["content"] = serde_json::json!(c);
        }
        if let Some(t) = title {
            args["title"] = serde_json::json!(t);
        }
        self.call_tool_slow(&self.tools.update, &args)
    }

    /// memory_attach：显式挂子节点
    pub fn attach(&self, id: &str, parent: &str) -> Result<RsrsEnvelope, RsrsError> {
        self.call_tool_slow(
            &self.tools.attach,
            &serde_json::json!({ "id": id, "parent": parent }),
        )
    }

    /// memory_list：最近条目（备份/排查用）
    pub fn list(&self, limit: u32) -> Result<RsrsEnvelope, RsrsError> {
        self.call_tool_slow(&self.tools.list, &serde_json::json!({ "limit": limit }))
    }

    /// 读取全部有效记忆（含日记）；用运行时有效条数确定窗口，不把 200/2000 条冒充全部。
    pub fn list_all(&self) -> Result<Vec<ListedEntry>, RsrsError> {
        // 1. 取当前账号有效条数；旧 Runtime 无计数时从 1000 开始逐步扩大。
        let status = self.status()?;
        let count = status
            .summary
            .as_ref()
            .and_then(|s| s.get("local_alive"))
            .and_then(|v| v.as_u64());
        let expected = count
            .map(u32::try_from)
            .transpose()
            .map_err(|_| RsrsError::Other("记忆数量超过客户端可请求范围".into()))?;
        if expected == Some(0) {
            return Ok(Vec::new());
        }
        let mut window = expected.unwrap_or(1000).saturating_add(1);
        loop {
            let items = self.list_recent(window)?;
            // 2. 返回少于请求窗口代表到达库尾；若少于自报有效数，明确提示重试而不声称完整。
            if items.len() < window as usize {
                if expected.map(|n| items.len() < n as usize).unwrap_or(false) {
                    return Err(RsrsError::Mcp(
                        "有效记忆数量与返回结果不一致，请刷新重试。".into(),
                    ));
                }
                return Ok(items);
            }
            window = window
                .checked_mul(2)
                .ok_or_else(|| RsrsError::Other("记忆数量超过客户端可请求范围".into()))?;
        }
    }

    /// memory_list：最近条目（结构化，server 按 created_at 倒序）
    pub fn list_recent(&self, limit: u32) -> Result<Vec<ListedEntry>, RsrsError> {
        let env =
            self.call_tool_slow(&self.tools.list, &serde_json::json!({ "limit": limit }))?;
        match &env.details {
            Some(serde_json::Value::Array(arr)) => {
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    let entry = serde_json::from_value::<ListedEntry>(v.clone()).map_err(|e| {
                        RsrsError::Parse(format!("list details 解析失败: {e}"))
                    })?;
                    out.push(entry);
                }
                Ok(out)
            }
            _ => Err(RsrsError::Parse("list 响应缺少 details 数组".into())),
        }
    }
}

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

/// 解析候选 value："similarity=88% title=xxx" → (Some(88.0), "xxx")
fn parse_candidate_value(value: &str) -> (Option<f64>, String) {
    let mut sim = None;
    let mut title = String::new();
    for token in value.split_whitespace() {
        if let Some(num) = token.strip_prefix("similarity=") {
            let cleaned = num.trim_end_matches('%');
            sim = cleaned.parse::<f64>().ok();
        } else if let Some(t) = token.strip_prefix("title=") {
            title = t.to_string();
        }
    }
    if title.is_empty() {
        // 兜底：整串作为标题
        title = value.to_string();
    }
    (sim, title)
}

/// 递归扁平化 tree 节点
fn flatten_node(node: &serde_json::Value, depth: u32, out: &mut Vec<TreeNode>) {
    let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if id.is_empty() {
        return;
    }
    let title = node
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let kind = node.get("kind").and_then(|v| v.as_str()).map(|s| s.to_string());
    let is_leaf = node
        .get("is_leaf")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    out.push(TreeNode {
        id,
        title,
        kind,
        depth,
        is_leaf,
    });
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for c in children {
            flatten_node(c, depth + 1, out);
        }
    }
}

fn extract_text(result: &serde_json::Value) -> String {
    let mut buf = String::new();
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        for item in content {
            if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                buf.push_str(t);
            }
        }
    }
    buf
}

fn truncate(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
#[path = "rsrs_client_snapshot_tests.rs"]
mod snapshot_regressions;

fn looks_like_auth(s: &str) -> bool {
    let s = s.to_lowercase();
    s.contains("401") || s.contains("403") || s.contains("unauthorized") || s.contains("auth")
}

fn looks_like_candidate(s: &str) -> bool {
    let s = s.to_lowercase();
    s.contains("candidate") || s.contains("候选") || s.contains("pending")
}

/// rsrs 插件 hook 的「非致命告警」：on_error=warn、continuing，写入其实成功。
/// 可靠标记为 "(continuing)"（见记忆 ae2b3499 实测报文）。
fn is_nonfatal_warning(s: &str) -> bool {
    s.to_lowercase().contains("continuing")
}

#[cfg(test)]
#[path = "rsrs_client_recall_tests.rs"]
mod recall_regressions;
