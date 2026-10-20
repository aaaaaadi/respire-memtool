//! llm — 模型判档 / 判树（P2 模型路径）
//!
//! 任意 OpenAI 兼容 `/v1/chat/completions`（memory-gateway / Ollama / vLLM）。
//! 只负责「判档 + 判树 + 起标题」这三个可复用字段；判重仍走 rsrs 语义检索，
//! 改/并等破坏性分支仍由用户确认。调用失败由调用方回退本地并告警。

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::LlmConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Verdict {
    pub importance: String,
    pub tree: String,
    pub title: String,
    pub reason: String,
}

/// 过滤模型输出：只接受合法档位、候选内的树名、合理长度的标题
pub fn sanitize(mut v: Verdict, trees: &[String]) -> Verdict {
    let imp = v.importance.trim().to_lowercase();
    v.importance = if imp == "important" || imp == "trivial" {
        imp
    } else {
        String::new()
    };

    let tree = v.tree.trim().to_string();
    v.tree = if trees.iter().any(|t| t == &tree) {
        tree
    } else {
        String::new()
    };

    let title: String = v
        .title
        .replace(['\n', '\r', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title = title
        .trim()
        .trim_matches(['"', '\'', '「', '」', '《', '》'])
        .trim()
        .to_string();
    v.title = if (2..=40).contains(&title.chars().count()) {
        title
    } else {
        String::new()
    };

    v.reason = v.reason.trim().chars().take(120).collect();
    v
}

/// 从模型文本里取 JSON 对象（容忍 ```json 围栏与前后解释）
pub fn extract_json(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(text[start..=end].to_string())
}

/// 调用模型；成功返回已过滤的判档结果（字段为空表示不可用，由调用方回退）
pub fn judge(
    cfg: &LlmConfig,
    content: &str,
    hint: Option<&str>,
    trees: &[String],
) -> Result<Verdict, String> {
    if cfg.endpoint.trim().is_empty() {
        return Err("未配置模型端点（config.toml [automation.llm] endpoint）".into());
    }
    let head = content.chars().take(3000).collect::<String>();
    let hint_line = hint
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(|h| format!("项目/关键词提示：{h}\n"))
        .unwrap_or_default();
    let tree_list = trees.join("、");

    let system = "你是 rsrs 记忆库的分拣器。只输出一个 JSON 对象，不要 Markdown、不要解释。";
    let user = format!(
        "{hint_line}待存内容：\n{head}\n\n\
候选分类（只能从里面选一个）：\n{tree_list}\n\n\
判定规则：\n\
1. importance：可复用结论 / 决策 / 教训 / 配置 / 步骤 → \"important\"；过程流水 / 日常轨迹 / 随手记 → \"trivial\"。\n\
2. tree：从候选分类中选最贴切的一个名称；无法判断填 \"\"。\n\
3. title：12-24 字的事件事实标题，以【行为】做了什么为主；禁止只写分类名，禁止日期前缀。\n\
4. reason：一句话说明理由（≤60 字）。\n\n\
输出格式：{{\"importance\":\"important|trivial\",\"tree\":\"分类名或空\",\"title\":\"标题\",\"reason\":\"理由\"}}"
    );

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(cfg.timeout_secs.max(5)))
        .build();
    let mut req = agent
        .post(cfg.endpoint.trim())
        .set("Content-Type", "application/json");
    let key = cfg.api_key.trim();
    if !key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    let body = serde_json::json!({
        "model": cfg.model,
        "temperature": 0,
        "max_tokens": 500,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });

    let resp: serde_json::Value = match req.send_json(body) {
        Ok(r) => r
            .into_json()
            .map_err(|e| format!("模型响应不是 JSON: {e}"))?,
        Err(ureq::Error::Transport(t)) => return Err(format!("模型端点不可达: {t}")),
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            return Err(format!("模型端点 HTTP {code}: {}", body.chars().take(200).collect::<String>()));
        }
    };

    let text = resp
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| "模型响应缺少 choices[0].message.content".to_string())?;

    let json = extract_json(text).ok_or_else(|| {
        format!(
            "模型未返回 JSON：{}",
            text.chars().take(200).collect::<String>()
        )
    })?;
    let parsed: Verdict =
        serde_json::from_str(&json).map_err(|e| format!("模型 JSON 解析失败: {e}"))?;
    Ok(sanitize(parsed, trees))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trees() -> Vec<String> {
        vec!["编程开发".into(), "生活日常".into(), "AI 协作".into()]
    }

    #[test]
    fn extract_json_handles_fences() {
        let text = "```json\n{\"importance\":\"trivial\",\"tree\":\"生活日常\"}\n```";
        assert_eq!(
            extract_json(text).unwrap(),
            "{\"importance\":\"trivial\",\"tree\":\"生活日常\"}"
        );
        assert!(extract_json("没有 JSON").is_none());
    }

    #[test]
    fn sanitize_drops_unknown_values() {
        let v = sanitize(
            Verdict {
                importance: "IMPORTANT".into(),
                tree: "不存在的分类".into(),
                title: "  修复 托盘 常驻  ".into(),
                reason: "r".into(),
            },
            &trees(),
        );
        assert_eq!(v.importance, "important");
        assert_eq!(v.tree, "");
        assert_eq!(v.title, "修复 托盘 常驻");
    }

    #[test]
    fn sanitize_rejects_bad_title() {
        let v = sanitize(
            Verdict {
                title: "编程开发".repeat(14),
                ..Default::default()
            },
            &trees(),
        );
        assert_eq!(v.title, "");
        let v = sanitize(Verdict { title: "x".into(), ..Default::default() }, &trees());
        assert_eq!(v.title, "");
    }
}
