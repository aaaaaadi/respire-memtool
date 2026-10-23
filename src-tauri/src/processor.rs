//! processor — 内容梳理（P10：层次一直调 LLM，层次二调 agent）
//!
//! 在 memTools 内「不打开任何 agent」即可让内容被梳理后再落库：
//! 总结 / 结构化 / 起标题 / 翻译 / 提取要点 / 润色。
//!
//! 两条路径，按配置择一：
//! - **层次一**（默认）：复用 [`crate::llm`] 的 OpenAI 兼容 `/v1/chat/completions`
//!   （memory-gateway / Ollama / vLLM），由 `[processor] endpoint` 指定。
//! - **层次二**：`[processor] agent_endpoint` 非空或 `[processor.agent] enabled = true` 时，
//!   交给 [`crate::agent`] 调用本机 agent（opencode / Qoder）的 headless 入口，
//!   适合需要 agent 工具 / skill 的场景。
//!
//! prompt 模板可在 `config.toml [processor]` 覆盖。失败返回明确错误，
//! 由调用方（HTTP 路由 / Tauri 命令）回显，不静默、不回退原文。

use crate::agent;
use crate::config::ProcessorConfig;

/// 可执行的梳理任务
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    /// 分析内容并总结成一条记忆条目（【前因】【行为】【后果】）
    Summarize,
    /// 结构化：补 / 整理三标分段
    Structure,
    /// 起标题（12-24 字事件事实）
    Title,
    /// 翻译（目标语言由 `lang` 指定）
    Translate,
    /// 提取要点列表
    Extract,
    /// 润色
    Polish,
}

impl Task {
    pub const ALL: [Task; 6] = [
        Task::Summarize,
        Task::Structure,
        Task::Title,
        Task::Translate,
        Task::Extract,
        Task::Polish,
    ];

    /// 解析任务名（大小写不敏感）；未知返回 None
    pub fn parse(raw: &str) -> Option<Task> {
        match raw.trim().to_lowercase().as_str() {
            "summarize" | "summary" | "总结" => Some(Task::Summarize),
            "structure" | "结构化" => Some(Task::Structure),
            "title" | "标题" => Some(Task::Title),
            "translate" | "翻译" => Some(Task::Translate),
            "extract" | "提取" => Some(Task::Extract),
            "polish" | "润色" => Some(Task::Polish),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Task::Summarize => "summarize",
            Task::Structure => "structure",
            Task::Title => "title",
            Task::Translate => "translate",
            Task::Extract => "extract",
            Task::Polish => "polish",
        }
    }

    /// 前端展示名
    pub fn label(self) -> &'static str {
        match self {
            Task::Summarize => "总结成记忆",
            Task::Structure => "结构化",
            Task::Title => "起标题",
            Task::Translate => "翻译",
            Task::Extract => "提取要点",
            Task::Polish => "润色",
        }
    }

    /// 模型最大输出 token（标题短、正文长）
    fn max_tokens(self) -> u32 {
        match self {
            Task::Title => 100,
            _ => 1200,
        }
    }
}

const SYSTEM: &str = "你是一个严谨的本地记忆整理工具。只输出处理后的正文，不要 Markdown 标题、不要额外解释。";

/// 组装一次任务的用户提示词（`{content}` / `{lang}` 替换）
pub fn compose_user(cfg: &ProcessorConfig, content: &str, task: &Task, lang: Option<&str>) -> String {
    let lang = match task {
        Task::Translate => lang
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("中文"),
        _ => "",
    };
    cfg.prompts
        .get(task.as_str())
        .replace("{content}", content)
        .replace("{lang}", lang)
}

/// 组装送给 agent 的完整提示词（CLI 没有 system 角色，把约束写进正文）
pub fn compose_agent_prompt(cfg: &ProcessorConfig, content: &str, task: &Task, lang: Option<&str>) -> String {
    let user = compose_user(cfg, content, task, lang);
    format!("{SYSTEM}\n\n{user}\n\n只输出处理后的正文，不要任何解释或前后缀。")
}

/// 执行梳理：成功返回处理后的正文（标题任务返回压平后的标题）
pub fn process(
    cfg: &ProcessorConfig,
    content: &str,
    task: &Task,
    lang: Option<&str>,
) -> Result<String, String> {
    if !cfg.enabled {
        return Err("梳理功能未启用（config.toml [processor] enabled = false）".into());
    }
    let use_agent = cfg.agent_enabled();
    if !use_agent && cfg.endpoint.trim().is_empty() {
        return Err("未配置模型端点（config.toml [processor] endpoint）".into());
    }
    let content = content.trim();
    if content.is_empty() {
        return Err("内容为空".into());
    }
    let len = content.chars().count();
    if len > cfg.max_chars {
        return Err(format!(
            "内容过长（{len} 字，上限 {} 字）：请缩短后重试或分段处理。",
            cfg.max_chars
        ));
    }

    let raw = if use_agent {
        let prompt = compose_agent_prompt(cfg, content, task, lang);
        agent::ask_with(&cfg.agent, &prompt, &cfg.agent_endpoint)?.text
    } else {
        let user = compose_user(cfg, content, task, lang);
        let llm = cfg.as_llm();
        crate::llm::chat(&llm, SYSTEM, &user, task.max_tokens())?
    };
    let out = raw.trim().to_string();
    if out.is_empty() {
        return Err("模型返回空内容".into());
    }

    if *task == Task::Title {
        let title = out
            .replace(['\n', '\r', '\t'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let title = title
            .trim()
            .trim_matches(['"', '\'', '「', '」', '《', '》'])
            .trim()
            .to_string();
        let n = title.chars().count();
        if !(2..=60).contains(&n) {
            return Err(format!("标题长度不合适（{n} 字）：{title}"));
        }
        return Ok(title);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProcessorPrompts;

    #[test]
    fn task_parse_handles_aliases_and_case() {
        assert_eq!(Task::parse("summarize"), Some(Task::Summarize));
        assert_eq!(Task::parse("Summary"), Some(Task::Summarize));
        assert_eq!(Task::parse(" 总结 "), Some(Task::Summarize));
        assert_eq!(Task::parse("结构化"), Some(Task::Structure));
        assert_eq!(Task::parse("TITLE"), Some(Task::Title));
        assert_eq!(Task::parse("翻译"), Some(Task::Translate));
        assert_eq!(Task::parse("extract"), Some(Task::Extract));
        assert_eq!(Task::parse("润色"), Some(Task::Polish));
        assert_eq!(Task::parse("bogus"), None);
        assert_eq!(Task::parse(""), None);
    }

    #[test]
    fn task_as_str_and_label_are_stable() {
        for t in Task::ALL {
            assert!(!t.as_str().is_empty());
            assert!(!t.label().is_empty());
        }
    }

    #[test]
    fn disabled_and_empty_endpoint_short_circuit() {
        let mut cfg = ProcessorConfig::default();
        cfg.enabled = false;
        let err = process(&cfg, "内容", &Task::Summarize, None).unwrap_err();
        assert!(err.contains("未启用"));

        let mut cfg = ProcessorConfig::default();
        cfg.endpoint = "  ".into();
        let err = process(&cfg, "内容", &Task::Summarize, None).unwrap_err();
        assert!(err.contains("端点"));
    }

    #[test]
    fn rejects_empty_and_overlong_content() {
        let cfg = ProcessorConfig::default();
        assert!(process(&cfg, "   ", &Task::Summarize, None)
            .unwrap_err()
            .contains("内容为空"));

        let mut cfg = ProcessorConfig::default();
        cfg.max_chars = 10;
        let long = "一".repeat(11);
        let err = process(&cfg, &long, &Task::Summarize, None).unwrap_err();
        assert!(err.contains("内容过长"));
    }

    #[test]
    fn prompt_template_substitutes_content_and_lang() {
        let cfg = ProcessorConfig::default();
        let t = cfg.prompts.get("summarize");
        assert!(t.contains("{content}"));
        let tr = cfg.prompts.get("translate").replace("{content}", "正文").replace("{lang}", "英文");
        assert!(tr.contains("英文"));
        assert!(tr.contains("正文"));
        // 空模板回退内置默认；未知任务返回空串（不递归）
        let mut empty = ProcessorPrompts::default();
        empty.summarize = String::new();
        assert!(empty.get("summarize").contains("{content}"));
        assert_eq!(empty.get("bogus"), "");
    }

    #[test]
    fn compose_user_fills_lang_only_for_translate() {
        let cfg = ProcessorConfig::default();
        let tr = compose_user(&cfg, "正文", &Task::Translate, Some("英文"));
        assert!(tr.contains("英文") && tr.contains("正文"));
        // 翻译未指定语言 → 默认中文
        let tr_default = compose_user(&cfg, "body", &Task::Translate, None);
        assert!(tr_default.contains("中文"));
        // 非翻译任务不替换 lang（保持字面量）
        let sum = compose_user(&cfg, "正文", &Task::Summarize, Some("英文"));
        assert!(sum.contains("正文") && !sum.contains("英文"));
    }

    #[test]
    fn compose_agent_prompt_wraps_system_and_output_rule() {
        let cfg = ProcessorConfig::default();
        let p = compose_agent_prompt(&cfg, "原始内容", &Task::Summarize, None);
        assert!(p.contains(SYSTEM));
        assert!(p.contains("原始内容"));
        assert!(p.ends_with("不要任何解释或前后缀。"));
    }

    #[test]
    fn agent_route_does_not_need_llm_endpoint() {
        let mut cfg = ProcessorConfig::default();
        cfg.endpoint = String::new();
        cfg.agent.enabled = true;
        cfg.agent.kind = "claude".into(); // 未知类型：在启动进程前就报错
        let err = process(&cfg, "内容", &Task::Summarize, None).unwrap_err();
        assert!(err.contains("未知 agent 类型"), "实际：{err}");
    }

    #[test]
    fn source_reflects_agent_switch() {
        let mut cfg = ProcessorConfig::default();
        assert_eq!(cfg.source(), "llm");
        assert!(!cfg.agent_enabled());

        cfg.agent_endpoint = "http://127.0.0.1:4096".into();
        assert!(cfg.agent_enabled());
        assert_eq!(cfg.source(), "opencode");

        cfg.agent.kind = "qoder".into();
        assert_eq!(cfg.source(), "qoder");

        // 两个开关都关 → 回到 LLM
        cfg.agent_endpoint.clear();
        cfg.agent.enabled = false;
        assert_eq!(cfg.source(), "llm");
    }
}
