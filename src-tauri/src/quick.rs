//! 快捷目标保存逻辑（P9）—— 最小存根实现，等待完整同步。

use crate::config::AppConfig;
use crate::rsrs_client::RsrsClient;
use std::path::Path;

/// 保存到快捷目标的结果
#[derive(Debug, Clone)]
pub struct QuickOutcome {
    pub ok: bool,
    pub id: Option<String>,
    pub message: String,
    pub title: Option<String>,
    pub branch: String,
    pub needs_confirm: bool,
    pub parent: Option<String>,
    pub warnings: Vec<String>,
}

/// 按快捷目标保存（存根：实际实现需完整同步 memTools 代码库）
pub fn save_to_target(
    _client: &RsrsClient,
    _config: &AppConfig,
    _backup_dir: &Path,
    _target: &crate::targets::QuickTarget,
    _content: &str,
    _trigger: Option<&str>,
) -> Result<QuickOutcome, String> {
    Err("quick 模块尚未完整同步，请等待 memTools 完整代码同步".into())
}