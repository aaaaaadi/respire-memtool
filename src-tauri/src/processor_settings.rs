//! 梳理来源的运行时设置（P10 层次二 UI 化）
//!
//! `config.toml` 仍是默认值与高级参数（超时、workdir、工具白名单…）的来源；
//! 小窗里的「来源」下拉把用户选择存到 `{app_config_dir}/processor.json`，
//! 叠加在 `config.toml` 之上并**立即生效**（不用改文件、不用重启）。
//!
//! 这样普通用户只需要在界面点「模型 / opencode / Qoder / 关闭」，
//! 而 `bin`、`timeout_secs`、`extra_args` 这类细节仍可由高级用户在
//! `config.toml` 里调整。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::ProcessorConfig;

/// 界面可选来源（顺序即按钮顺序）；`off` = 关闭梳理
pub const SOURCES: [&str; 4] = ["off", "llm", "opencode", "qoder"];

/// 叠加在 `config.toml` 之上的用户选择
///
/// 空字符串一律表示「跟随 config.toml」，只有显式选择才覆盖。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessorChoice {
    /// `off` | `llm` | `opencode` | `qoder`；空 = 跟随 config.toml
    pub source: String,
    /// LLM 模型覆盖（空 = 跟随 config.toml）
    pub model: String,
    /// agent 模型覆盖（空 = 跟随 config.toml）
    pub agent_model: String,
    /// agent 可执行文件覆盖（空 = 自动探测）
    pub agent_bin: String,
}

impl ProcessorChoice {
    /// 去首尾空白并小写化来源；未知来源原样保留，交给 [`validate_source`] 报错
    pub fn normalized(mut self) -> Self {
        self.source = self.source.trim().to_lowercase();
        self.model = self.model.trim().to_string();
        self.agent_model = self.agent_model.trim().to_string();
        self.agent_bin = self.agent_bin.trim().to_string();
        self
    }

    /// 是否完全跟随 `config.toml`
    pub fn is_default(&self) -> bool {
        self.source.is_empty()
            && self.model.is_empty()
            && self.agent_model.is_empty()
            && self.agent_bin.is_empty()
    }
}

/// 校验来源标识（空串合法：表示跟随配置）
pub fn validate_source(source: &str) -> Result<&'static str, String> {
    let s = source.trim().to_lowercase();
    if s.is_empty() {
        return Ok("");
    }
    for id in SOURCES {
        if id == s {
            return Ok(id);
        }
    }
    Err(format!(
        "未知梳理来源：{}（可选：{}）",
        source.trim(),
        SOURCES.join(" / ")
    ))
}

/// 把用户选择叠加到 `config.toml` 的处理器配置上（纯函数，便于单测）
pub fn apply(base: &ProcessorConfig, choice: &ProcessorChoice) -> ProcessorConfig {
    let mut cfg = base.clone();
    let source = choice.source.trim().to_lowercase();
    match source.as_str() {
        "off" => {
            cfg.enabled = false;
            cfg.agent.enabled = false;
            cfg.agent_endpoint.clear();
        }
        "llm" => {
            // 显式选了模型：即使 config.toml 里开着 agent 也要让位
            cfg.enabled = true;
            cfg.agent.enabled = false;
            cfg.agent_endpoint.clear();
        }
        "opencode" | "qoder" => {
            cfg.enabled = true;
            cfg.agent.enabled = true;
            cfg.agent.kind = source.clone();
            // agent_endpoint 指「远程 agent 地址」，选了本地 CLI 就不再走它
            cfg.agent_endpoint.clear();
        }
        _ => {}
    }
    if !choice.model.trim().is_empty() {
        cfg.model = choice.model.trim().to_string();
    }
    if !choice.agent_model.trim().is_empty() {
        cfg.agent.model = choice.agent_model.trim().to_string();
    }
    if !choice.agent_bin.trim().is_empty() {
        cfg.agent.bin = choice.agent_bin.trim().to_string();
    }
    cfg
}

/// 选择持久化 + 线程安全读写
pub struct ProcessorSettings {
    path: PathBuf,
    inner: Mutex<ProcessorChoice>,
}

impl ProcessorSettings {
    /// 从 `processor.json` 载入（缺失 / 损坏时退回「跟随 config.toml」）
    pub fn load(path: PathBuf) -> Self {
        let choice = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<ProcessorChoice>(&s).ok())
            .unwrap_or_default()
            .normalized();
        // 文件里存了已废弃的来源时不要卡死：回退到跟随配置
        let choice = if validate_source(&choice.source).is_err() {
            ProcessorChoice::default()
        } else {
            choice
        };
        Self {
            path,
            inner: Mutex::new(choice),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> ProcessorChoice {
        self.inner
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    /// 保存并立即生效；非法来源直接报错、不落盘
    pub fn set(&self, choice: ProcessorChoice) -> Result<ProcessorChoice, String> {
        let choice = choice.normalized();
        validate_source(&choice.source)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("创建设置目录失败：{e}（{}）", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(&choice)
            .map_err(|e| format!("序列化梳理设置失败：{e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("写入设置失败：{e}（{}）", self.path.display()))?;
        match self.inner.lock() {
            Ok(mut g) => *g = choice.clone(),
            Err(e) => *e.into_inner() = choice.clone(),
        }
        Ok(choice)
    }

    /// 清空用户选择（回到 config.toml）
    pub fn reset(&self) -> Result<ProcessorChoice, String> {
        self.set(ProcessorChoice::default())
    }

    /// 生效配置 = config.toml + 用户选择
    pub fn effective(&self, base: &ProcessorConfig) -> ProcessorConfig {
        apply(base, &self.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ProcessorConfig {
        let mut cfg = ProcessorConfig::default();
        cfg.enabled = true;
        cfg.endpoint = "http://127.0.0.1:8125/v1/chat/completions".into();
        cfg.model = "qwen3:8b".into();
        cfg.agent.enabled = false;
        cfg.agent.kind = "opencode".into();
        cfg
    }

    #[test]
    fn default_choice_keeps_config() {
        let b = base();
        let got = apply(&b, &ProcessorChoice::default());
        assert!(got.enabled);
        assert_eq!(got.source(), "llm");
        assert_eq!(got.endpoint, b.endpoint);
    }

    #[test]
    fn choosing_agent_overrides_config_and_clears_endpoint() {
        let mut b = base();
        b.agent_endpoint = "http://127.0.0.1:4096".into();
        let choice = ProcessorChoice {
            source: "Qoder".into(),
            ..Default::default()
        }
        .normalized();
        let got = apply(&b, &choice);
        assert!(got.agent_enabled());
        assert_eq!(got.source(), "qoder");
        assert!(got.agent_endpoint.is_empty(), "选本地 CLI 后不应再走远程端点");
        assert!(got.enabled);
    }

    #[test]
    fn choosing_llm_wins_over_config_agent() {
        let mut b = base();
        b.agent.enabled = true;
        let got = apply(
            &b,
            &ProcessorChoice {
                source: "llm".into(),
                ..Default::default()
            },
        );
        assert!(!got.agent_enabled());
        assert_eq!(got.source(), "llm");
    }

    #[test]
    fn off_disables_processing() {
        let got = apply(
            &base(),
            &ProcessorChoice {
                source: "off".into(),
                ..Default::default()
            },
        );
        assert!(!got.enabled);
        assert!(!got.agent_enabled());
    }

    #[test]
    fn model_overrides_only_when_present() {
        let got = apply(
            &base(),
            &ProcessorChoice {
                source: "llm".into(),
                model: "  ".into(),
                ..Default::default()
            },
        );
        assert_eq!(got.model, "qwen3:8b");
        let got = apply(
            &base(),
            &ProcessorChoice {
                source: "llm".into(),
                model: "qwen3:14b".into(),
                ..Default::default()
            },
        );
        assert_eq!(got.model, "qwen3:14b");
    }

    #[test]
    fn validate_rejects_unknown_source() {
        assert!(validate_source("gemini").is_err());
        assert_eq!(validate_source("").unwrap(), "");
        assert_eq!(validate_source(" OpenCode ").unwrap(), "opencode");
    }

    #[test]
    fn settings_roundtrip_and_reset() {
        let dir = std::env::temp_dir().join(format!("memtools-proc-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("processor.json");
        let s = ProcessorSettings::load(path.clone());
        assert!(s.get().is_default());

        s.set(ProcessorChoice {
            source: "opencode".into(),
            agent_model: "opencode/big-pickle".into(),
            ..Default::default()
        })
        .expect("保存应成功");
        assert!(path.exists(), "应写入 processor.json");

        let again = ProcessorSettings::load(path.clone());
        assert_eq!(again.get().source, "opencode");
        assert_eq!(again.get().agent_model, "opencode/big-pickle");

        again.reset().expect("重置应成功");
        assert!(ProcessorSettings::load(path).get().is_default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_reject_unknown_source_without_touching_disk() {
        let dir = std::env::temp_dir().join(format!("memtools-proc-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("processor.json");
        let s = ProcessorSettings::load(path.clone());
        assert!(s
            .set(ProcessorChoice {
                source: "claude".into(),
                ..Default::default()
            })
            .is_err());
        assert!(!path.exists(), "非法来源不应落盘");
    }
}
