//! memTools 配置：读取 src-tauri/config.toml（热键/端口/自动化模式/窗口行为）

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub hotkey: HotkeyConfig,
    pub window: WindowConfig,
    pub rsrs: RsrsConfig,
    pub automation: AutomationConfig,
    /// 浏览器扩展本机端点（P6-②）；旧配置文件缺该段时用默认值
    #[serde(default)]
    pub extension: ExtensionConfig,
    /// 自动更新（P7）；旧配置文件缺该段时用默认值
    #[serde(default)]
    pub update: UpdateConfig,
    /// 全局划词悬浮（P8）；旧配置文件缺该段时用默认值
    #[serde(default)]
    pub selection: SelectionConfig,
    /// 提醒机制（P5）；旧配置文件缺该段时用默认值
    #[serde(default)]
    pub reminder: ReminderConfig,
    /// 内容梳理 / 分析（P10 层次一：直接调 LLM）；旧配置文件缺该段时用默认值
    #[serde(default)]
    pub processor: ProcessorConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotkeyConfig {
    pub register: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WindowConfig {
    pub width: f64,
    pub height: f64,
    pub always_on_top: bool,
    pub decorations: bool,
    pub transparent: bool,
    pub remember_position: bool,
    pub submit_close: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RsrsConfig {
    pub base_url: String,
    /// MCP over HTTP 端点路径（POST {base_url}/{mcp_path}）
    pub mcp_path: String,
    pub timeout_secs: u64,
    /// 慢操作超时（remember/recall/update/attach/show/tree）—— 插件 hook 可能耗时 >5s
    #[serde(default = "default_op_timeout")]
    pub op_timeout_secs: u64,
    pub tools: RsrsToolNames,
}

fn default_op_timeout() -> u64 {
    30
}

impl RsrsConfig {
    pub fn port(&self) -> u16 {
        // 从 base_url 提取端口（http://127.0.0.1:15169 -> 15169）
        self.base_url
            .split(':')
            .last()
            .and_then(|s| s.trim_end_matches('/').parse::<u16>().ok())
            .unwrap_or(15169)
    }
}

/// memory_* MCP 工具名映射（rsrs runtime 暴露的 15 个工具）
#[derive(Debug, Clone, Deserialize)]
pub struct RsrsToolNames {
    pub status: String,
    pub taxonomy: String,
    pub remember: String,
    pub recall: String,
    pub tree: String,
    pub show: String,
    pub update: String,
    pub attach: String,
    pub list: String,
    /// memory_query_log_mark：自评召回候选（存后 --good/--bad）
    #[serde(default = "default_query_log_mark")]
    pub query_log_mark: String,
}

fn default_query_log_mark() -> String {
    "memory_query_log_mark".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct AutomationConfig {
    pub enabled: bool,
    pub mode: String,
    pub recall_rounds: u32,
    pub recall_limit: u32,
    /// 疑似重复阈值：>= 需用户确认（改/并）
    #[serde(default = "default_dup_threshold")]
    pub dup_threshold: f64,
    /// 同题阈值：>= 可自动「挂」其下
    #[serde(default = "default_attach_threshold")]
    pub attach_threshold: f64,
    /// 模型判档 / 判树（P2 模型路径）；旧配置文件缺该段时用默认值（关闭）
    #[serde(default)]
    pub llm: LlmConfig,
}

fn default_dup_threshold() -> f64 {
    0.85
}
fn default_attach_threshold() -> f64 {
    0.70
}

/// 模型判档路径：任意 OpenAI 兼容 `/v1/chat/completions`（memory-gateway / Ollama / vLLM 等）。
/// `automation.mode = local-fallback` 时完全不调用；失败自动回退本地并告警。
#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_timeout")]
    pub timeout_secs: u64,
}

fn default_llm_timeout() -> u64 {
    60
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://127.0.0.1:8125/v1/chat/completions".into(),
            model: "qwen3:8b".into(),
            api_key: String::new(),
            timeout_secs: default_llm_timeout(),
        }
    }
}

/// 提醒机制：扫描含 `【触发】` 的 task 条目，UI 侧定时核对（CLI 无后台提醒）。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ReminderConfig {
    pub enabled: bool,
    /// 后台扫描间隔（分钟）
    pub interval_minutes: u32,
    /// 「即将到期」窗口（天）
    pub soon_days: i64,
}

impl Default for ReminderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 30,
            soon_days: 7,
        }
    }
}

/// 浏览器扩展端点配置（P6-②）
#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionConfig {
    /// 是否启动本机端点
    pub enabled: bool,
    /// 只绑 127.0.0.1
    pub port: u16,
    /// 固定令牌；留空则首次启动自动生成并写入 {app_config_dir}/extension_token.txt
    #[serde(default)]
    pub token: String,
}

impl Default for ExtensionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 17650,
            token: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateConfig {
    pub enabled: bool,
    /// update.json 地址（本机 HTTP 通道或对象存储均可）
    pub endpoint: String,
    pub check_on_start: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: "http://127.0.0.1:17651/update.json".into(),
            check_on_start: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SelectionConfig {
    pub enabled: bool,
    pub min_chars: usize,
    pub max_chars: usize,
    pub drag_threshold_px: i32,
    pub capture_timeout_ms: u64,
    pub double_click: bool,
    pub offset_px: i32,
    /// 剪贴板归因窗口：拖选后多久内的复制算本次划词（毫秒）
    #[serde(default = "default_clipboard_arm_ms")]
    pub clipboard_arm_ms: u64,
    /// 不注入取词的进程名黑名单（内嵌终端的宿主如 VS Code，只能按进程名判）
    #[serde(default = "default_no_inject_processes")]
    pub no_inject_processes: Vec<String>,
}

fn default_clipboard_arm_ms() -> u64 {
    1200
}

fn default_no_inject_processes() -> Vec<String> {
    [
        "Code.exe",
        "Code - Insiders.exe",
        "Cursor.exe",
        "Windsurf.exe",
        "Trae.exe",
        "Qoder.exe",
        "WindowsTerminal.exe",
        "wt.exe",
        "OpenConsole.exe",
        "conhost.exe",
        "wezterm-gui.exe",
        "alacritty.exe",
        "mintty.exe",
        "Hyper.exe",
        "Tabby.exe",
        "FluentTerminal.exe",
        "ConEmu.exe",
        "ConEmu64.exe",
        "Terminus.exe",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for SelectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_chars: 2,
            max_chars: 4000,
            drag_threshold_px: 8,
            capture_timeout_ms: 600,
            double_click: true,
            offset_px: 14,
            clipboard_arm_ms: default_clipboard_arm_ms(),
            no_inject_processes: default_no_inject_processes(),
        }
    }
}

/// 内容梳理（P10 层次一）：直接调用 LLM 把内容总结 / 结构化 / 起标题 / 翻译 / 提取 / 润色。
///
/// 与 `[automation.llm]` 相互独立：判档走 automation，梳理走 processor。
/// `prompts` 为空时使用内置模板；可在 config.toml 里逐条覆盖。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProcessorConfig {
    /// 是否启用梳理（关闭时 /process 直接 503，不调用模型）
    pub enabled: bool,
    /// 任意 OpenAI 兼容 `/v1/chat/completions`（memory-gateway / Ollama / vLLM）
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_processor_timeout")]
    pub timeout_secs: u64,
    /// 送入模型的内容上限（超长直接拒绝，避免误粘整篇文档）
    #[serde(default = "default_processor_max_chars")]
    pub max_chars: usize,
    /// 各任务的 prompt 模板（空 = 用内置默认）；`{content}` / `{lang}` 会被替换
    #[serde(default)]
    pub prompts: ProcessorPrompts,
    /// 层次二：agent 服务地址（opencode 的 `--attach`，如 http://127.0.0.1:4096）。
    /// 留空 = 只看 `[processor.agent] enabled`；非空 = 直接用 agent，忽略 enabled。
    #[serde(default)]
    pub agent_endpoint: String,
    /// 层次二：agent headless 调用配置（opencode / Qoder）
    #[serde(default)]
    pub agent: AgentConfig,
}

fn default_processor_timeout() -> u64 {
    120
}
fn default_processor_max_chars() -> usize {
    8000
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: "http://127.0.0.1:8125/v1/chat/completions".into(),
            model: "qwen3:8b".into(),
            api_key: String::new(),
            timeout_secs: default_processor_timeout(),
            max_chars: default_processor_max_chars(),
            prompts: ProcessorPrompts::default(),
            agent_endpoint: String::new(),
            agent: AgentConfig::default(),
        }
    }
}

impl ProcessorConfig {
    /// 是否走 agent（层次二）：`agent_endpoint` 非空或 `[processor.agent] enabled = true`
    pub fn agent_enabled(&self) -> bool {
        !self.agent_endpoint.trim().is_empty() || self.agent.enabled
    }

    /// 当前梳理来源：`"llm"` 或 agent 的 kind（`opencode` / `qoder`）
    pub fn source(&self) -> &str {
        if self.agent_enabled() {
            let kind = self.agent.kind.trim();
            if kind.is_empty() {
                "opencode"
            } else {
                kind
            }
        } else {
            "llm"
        }
    }

    /// 复用 `llm.rs` 的 OpenAI 兼容调用（把 processor 配置映射成 LlmConfig）
    pub fn as_llm(&self) -> LlmConfig {
        LlmConfig {
            enabled: self.enabled,
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            api_key: self.api_key.clone(),
            timeout_secs: self.timeout_secs,
        }
    }
}

/// 层次二：agent headless 调用配置（opencode / Qoder）。
///
/// memTools 通过**子进程直调 agent CLI**（不经 shell 拼接，避免注入），
/// 让一个真实 agent（带工具/skill）来梳理内容，再落库。
/// 层次一（直调 LLM）由 `[processor] endpoint` 提供，两者按 `enabled` 二选一。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// 是否启用 agent 路径（`[processor] agent_endpoint` 非空时同样启用）
    pub enabled: bool,
    /// `opencode` | `qoder`
    pub kind: String,
    /// 可执行文件路径；留空自动探测（PATH / 已知安装位置）
    pub bin: String,
    /// 模型：opencode 用 `provider/model`；qoder 用 auto/efficient/qmodel 等档位
    pub model: String,
    /// opencode 的 `--agent`（用自定义 agent 限定工具/权限）
    pub agent: String,
    /// 单次调用超时（秒）；agent 冷启动比直连 LLM 慢
    pub timeout_secs: u64,
    /// 工作目录；留空用 `{temp}/memtools-agent`，避免污染当前项目与会话历史
    pub workdir: String,
    /// 是否 `--pure`（opencode：跳过外部插件，防止 hook 拖慢/额外写记忆）
    pub pure: bool,
    /// qoder 的 `--max-turns`（0 = 不传）
    pub max_turns: u32,
    /// qoder 的 `--allowed-tools`（逗号分隔；留空不传）
    pub allowed_tools: String,
    /// qoder 的 `--disallowed-tools`（逗号分隔；留空不传）
    pub disallowed_tools: String,
    /// 追加参数（原样传给 CLI）
    pub extra_args: Vec<String>,
    /// 令牌：非空则注入到 `token_env` 指定的环境变量（不落日志）
    pub token: String,
    /// 令牌环境变量名；留空按 kind 取默认（qoder → QODER_PERSONAL_ACCESS_TOKEN）
    pub token_env: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: "opencode".into(),
            bin: String::new(),
            model: String::new(),
            agent: String::new(),
            timeout_secs: 240,
            workdir: String::new(),
            pure: true,
            max_turns: 0,
            allowed_tools: String::new(),
            disallowed_tools: String::new(),
            extra_args: Vec::new(),
            token: String::new(),
            token_env: String::new(),
        }
    }
}

impl AgentConfig {
    /// 令牌注入的目标环境变量名（默认值按 kind）
    pub fn token_env_name(&self) -> String {
        let custom = self.token_env.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        match self.kind.trim().to_lowercase().as_str() {
            "qoder" => "QODER_PERSONAL_ACCESS_TOKEN".into(),
            _ => "OPENCODE_API_KEY".into(),
        }
    }
}

/// 内置 prompt 模板：`{content}` 必填，`{lang}` 仅翻译用
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProcessorPrompts {
    pub summarize: String,
    pub structure: String,
    pub title: String,
    pub translate: String,
    pub extract: String,
    pub polish: String,
}

impl Default for ProcessorPrompts {
    fn default() -> Self {
        Self {
            summarize: "你是记忆整理助手。分析下面的内容，总结成一条可长期保存的记忆条目：\n\
1. 用【前因】【行为】【后果】三标分段（某段缺失可省略）；\n\
2. 保留关键事实、实体、日期、结论与可复用步骤；\n\
3. 去掉广告、导航、客套等噪音，不编造原文没有的信息；\n\
4. 语言简洁，总长不超过 400 字。\n\
只输出整理后的正文。\n\n内容：\n{content}"
                .into(),
            structure: "你是记忆整理助手。把下面的内容结构化，用【前因】【行为】【后果】三标分段；\n\
只调整结构，不增删事实、不编造信息；原文已有标记则保留并整理。\n\
只输出整理后的正文。\n\n内容：\n{content}"
                .into(),
            title: "你是记忆标题助手。为下面的内容起一个 12-24 字的事件事实标题，以「做了什么」为主；\n\
禁止只写主题分类名，禁止日期前缀，不要解释。\n只输出标题本身。\n\n内容：\n{content}"
                .into(),
            translate: "你是翻译助手。把下面的内容翻译成{lang}，保持事实、数字、专有名词准确，不增删信息。\n\
只输出译文。\n\n内容：\n{content}"
                .into(),
            extract: "你是记忆整理助手。提取下面内容的要点，用简洁条目列表（每条一行，以 - 开头），\n\
保留关键事实、数字、实体与结论，不编造。\n只输出要点列表。\n\n内容：\n{content}"
                .into(),
            polish: "你是文字润色助手。润色下面的内容，修正错别字与病句、让表达更清晰，\n\
但不改变事实与含义、不加新信息。\n只输出润色后的正文。\n\n内容：\n{content}"
                .into(),
        }
    }
}

impl ProcessorPrompts {
    /// 取某任务的模板（空则回退内置默认）；未知任务返回空串
    pub fn get(&self, task: &str) -> String {
        let d = ProcessorPrompts::default();
        let raw = match task {
            "summarize" => pick(&self.summarize, &d.summarize),
            "structure" => pick(&self.structure, &d.structure),
            "title" => pick(&self.title, &d.title),
            "translate" => pick(&self.translate, &d.translate),
            "extract" => pick(&self.extract, &d.extract),
            "polish" => pick(&self.polish, &d.polish),
            _ => return String::new(),
        };
        raw.to_string()
    }
}

/// 模板为空时回退默认（避免未知任务导致递归）
fn pick<'a>(custom: &'a str, default: &'a str) -> &'a str {
    if custom.trim().is_empty() {
        default
    } else {
        custom
    }
}

impl AppConfig {
    /// `config.toml` 的固定路径（编译期 `CARGO_MANIFEST_DIR`，换目录放 exe 仍读这里）
    pub fn path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.toml")
    }

    /// 从 src-tauri/config.toml 加载；失败时返回内置默认值
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<AppConfig>(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("[memTools] config.toml 解析失败，使用默认配置: {e}");
                    Self::default_config()
                }
            },
            Err(_) => Self::default_config(),
        }
    }

    fn default_config() -> Self {
        Self {
            hotkey: HotkeyConfig {
                register: "Ctrl+Alt+R".into(),
            },
            window: WindowConfig {
                width: 480.0,
                height: 560.0,
                always_on_top: true,
                decorations: false,
                transparent: true,
                remember_position: true,
                submit_close: true,
            },
            rsrs: RsrsConfig {
                base_url: "http://127.0.0.1:15169".into(),
                mcp_path: "mcp".into(),
                timeout_secs: 5,
                op_timeout_secs: 30,
                tools: RsrsToolNames {
                    status: "memory_status".into(),
                    taxonomy: "memory_taxonomy".into(),
                    remember: "memory_remember".into(),
                    recall: "memory_recall".into(),
                    tree: "memory_tree".into(),
                    show: "memory_show".into(),
                    update: "memory_update".into(),
                    attach: "memory_attach".into(),
                    list: "memory_list".into(),
                    query_log_mark: "memory_query_log_mark".into(),
                },
            },
            automation: AutomationConfig {
                enabled: false,
                mode: "local-fallback".into(),
                recall_rounds: 2,
                recall_limit: 3,
                dup_threshold: 0.85,
                attach_threshold: 0.70,
                llm: LlmConfig::default(),
            },
            extension: ExtensionConfig::default(),
            update: UpdateConfig::default(),
            selection: SelectionConfig::default(),
            reminder: ReminderConfig::default(),
            processor: ProcessorConfig::default(),
        }
    }
}
