//! memTools 配置：读取 config.toml（热键/端口/自动化模式/窗口行为）
//!
//! 查找顺序（第一个存在的文件生效）：
//! 1. 环境变量 `MEMTOOLS_CONFIG`（绝对路径）
//! 2. 可执行文件同目录的 `config.toml`（绿色便携）
//! 3. 用户配置目录：Windows `%APPDATA%\com.memtools.desktop\config.toml`，
//!    macOS `~/Library/Application Support/com.memtools.desktop/config.toml`，
//!    Linux `$XDG_CONFIG_HOME/com.memtools.desktop/config.toml`（否则 `~/.config/...`）
//! 4. 仅 debug 构建：`{CARGO_MANIFEST_DIR}/config.toml`（开发便利，release 不参与）
//! 5. 编译进二进制内的默认模板（`include_str!`）
//!
//! 因此二进制不再绑定构建机路径；release 用户可通过 exe 旁的 config.toml
//! 或用户配置目录覆盖默认值。

use std::path::PathBuf;

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
    /// 可选鉴权令牌：非空时作为 `Authorization: Bearer <token>` 发送
    #[serde(default)]
    pub auth_token: String,
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

/// 模型判档路径：任意 OpenAI 兼容 `/v1/chat/completions`。
/// `automation.mode = local-fallback` 时完全不调用；失败自动回退本地并告警。
#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
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
            endpoint: String::new(),
            model: String::new(),
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
    /// update.json 地址（公网 HTTPS 或对象存储均可）
    pub endpoint: String,
    pub check_on_start: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint:
                "https://github.com/risense-ai/respire-memtool/releases/latest/download/update.json"
                    .into(),
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
        }
    }
}

/// 用户配置目录下的 config.toml（按平台约定）
pub fn user_config_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let path = {
        let base = std::env::var_os("APPDATA")?;
        PathBuf::from(base)
            .join("com.memtools.desktop")
            .join("config.toml")
    };
    #[cfg(target_os = "macos")]
    let path = {
        let home = std::env::var_os("HOME")?;
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("com.memtools.desktop")
            .join("config.toml")
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let path = {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        base.join("com.memtools.desktop").join("config.toml")
    };
    Some(path)
}

/// 首次运行时把内置模板写到用户配置目录（exe 旁或用户目录已有配置则不动）
pub fn ensure_user_config() {
    let Some(user) = user_config_path() else {
        return;
    };
    if user.is_file() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("config.toml").is_file() {
                return;
            }
        }
    }
    if let Some(dir) = user.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&user, include_str!("../config.toml")) {
        Ok(_) => eprintln!("[memTools] 已生成用户配置：{}", user.display()),
        Err(e) => eprintln!("[memTools] 写入用户配置失败（{}）：{e}", user.display()),
    }
}

impl AppConfig {
    /// 按查找顺序加载第一个存在的配置文件；全部缺失/解析失败时用内置默认值。
    pub fn load() -> Self {
        let Some(path) = Self::candidate_paths().into_iter().find(|p| p.is_file()) else {
            eprintln!("[memTools] 未找到 config.toml，使用内置默认配置");
            return Self::parse_embedded();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<AppConfig>(&text) {
                Ok(cfg) => {
                    eprintln!("[memTools] 使用配置文件：{}", path.display());
                    cfg.normalized()
                }
                Err(e) => {
                    eprintln!(
                        "[memTools] 配置文件解析失败（{}）：{e}，使用内置默认配置",
                        path.display()
                    );
                    Self::parse_embedded()
                }
            },
            Err(e) => {
                eprintln!(
                    "[memTools] 读取配置文件失败（{}）：{e}，使用内置默认配置",
                    path.display()
                );
                Self::parse_embedded()
            }
        }
    }

    fn candidate_paths() -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(p) = std::env::var_os("MEMTOOLS_CONFIG") {
            if !p.is_empty() {
                out.push(PathBuf::from(p));
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                out.push(dir.join("config.toml"));
            }
        }
        // 开发构建优先读源码目录，避免首次运行生成的用户配置盖住正在编辑的模板
        #[cfg(debug_assertions)]
        out.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config.toml"));
        if let Some(p) = user_config_path() {
            out.push(p);
        }
        out
    }

    fn parse_embedded() -> Self {
        match toml::from_str::<AppConfig>(include_str!("../config.toml")) {
            Ok(cfg) => cfg.normalized(),
            Err(e) => {
                eprintln!("[memTools] 内置默认配置解析失败：{e}，使用代码默认值");
                Self::default_config()
            }
        }
    }

    /// 兼容旧配置：memory-gateway 已弃用，等价于 llm
    fn normalized(mut self) -> Self {
        if self
            .automation
            .mode
            .trim()
            .eq_ignore_ascii_case("memory-gateway")
        {
            eprintln!(
                "[memTools] automation.mode=memory-gateway 已弃用，按 llm 处理；请改用 local-fallback | high-quality | llm"
            );
            self.automation.mode = "llm".into();
        }
        self
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
                auth_token: String::new(),
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
        }
    }
}
