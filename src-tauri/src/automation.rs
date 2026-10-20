//! automation — 自动化路径（P2）
//!
//! 依据 PLAN.md §4.2：判重两轮 → 读候选全文 → 择一有序执行（改 > 并 > 挂 > 存）。
//! 本模块在「本地降级」模式下运行，不需模型：用语义分阈值 + 关键词判档/判树。
//!
//! 红线：
//!  - 破坏性分支（改/并）默认需用户确认，不静默执行；
//!  - 改/并执行前必须备份（写 backup_dir），改后立即回读校验；
//!  - 候选悬空（判重命中）不得丢弃，必须回传 UI 择一；
//!  - 不裸 --force。

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::config::AutomationConfig;
use crate::rsrs_client::{RememberRequest, RsrsClient, RsrsError, TaxonomyItem};

// ---------------------------------------------------------------------------
// 输出模型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CandidateOut {
    /// 完整 id 或 8 位前缀
    pub id: String,
    pub title: String,
    /// 归一化相似度 0..1
    pub score: f64,
    /// 来源：recall（语义检索）/ remember（判重候选）
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    /// important | trivial
    pub importance: String,
    pub title: String,
    /// journal | attach | create | confirm
    pub branch: String,
    /// attach 时为候选 id；create 时为树根
    pub suggested_parent: Option<String>,
    /// create 时建议的树根
    pub suggested_tree: Option<String>,
    pub candidates: Vec<CandidateOut>,
    /// 是否需用户确认（破坏性分支）
    pub needs_confirm: bool,
    pub message: String,
    pub warnings: Vec<String>,
    /// model | local（判档来源）
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub ok: bool,
    /// 实际执行的分支：journal | attach | create | update | merge
    pub branch: String,
    /// rsrs summary.action
    pub action: Option<String>,
    pub id: Option<String>,
    pub parent: Option<String>,
    pub title: Option<String>,
    pub importance: String,
    pub message: String,
    pub warnings: Vec<String>,
    /// 需确认时回传候选
    pub candidates: Vec<CandidateOut>,
    pub needs_confirm: bool,
}

// ---------------------------------------------------------------------------
// 判档 / 标题
// ---------------------------------------------------------------------------

const IMPORTANT_CUES: &[&str] = &[
    "【前因】", "【行为】", "【后果】", "结论", "决策", "教训", "方案", "配置", "步骤", "命令",
    "修复", "踩坑", "坑", "方法", "流程", "报错", "部署", "架构", "原理", "原因", "要点",
    "注册", "安装", "优化", "注意",
];
const TRIVIAL_CUES: &[&str] = &[
    "日记", "今天", "刚刚", "记录一下", "随手", "活动轨迹", "吃", "买", "看", "去了",
];

/// 本地判档：可复用结论 → important；过程流水 → trivial
pub fn judge_importance(content: &str) -> &'static str {
    let flat = content.trim();
    // 三标分段 = 主区条目
    if flat.contains("【前因】") || flat.contains("【后果】") || flat.contains("【行为】") {
        return "important";
    }
    let important_hits = IMPORTANT_CUES.iter().filter(|c| flat.contains(*c)).count();
    let trivial_hits = TRIVIAL_CUES.iter().filter(|c| flat.contains(*c)).count();
    let len = flat.chars().count();

    // 时间前缀（【HH:MM】/ 日期开头）视为轨迹流水
    let looks_time = flat.starts_with('【') && flat.chars().nth(1).map(|c| c.is_ascii_digit()).unwrap_or(false);

    if important_hits >= 1 || len >= 120 {
        "important"
    } else if looks_time || trivial_hits >= 1 {
        "trivial"
    } else if len >= 60 {
        "important"
    } else {
        "trivial"
    }
}

/// 提取某一段（`【标记】` 到下一个标记之间）的正文；空段返回 None
fn section_text(content: &str, marker: &str) -> Option<String> {
    let idx = content.find(marker)?;
    let rest = &content[idx + marker.len()..];
    let end = ["【前因】", "【行为】", "【后果】", "【触发】", "【结论】"]
        .iter()
        .filter(|m| **m != marker)
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    let s = rest[..end].trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 去掉标题开头的日期/时间戳 token（如 `2026-10-17`、`10:20:`）
fn strip_leading_stamp(s: &str) -> String {
    let mut out = s.trim();
    loop {
        let first = out.split_whitespace().next().unwrap_or("");
        if first.is_empty() {
            break;
        }
        let has_digit = first.chars().any(|c| c.is_ascii_digit());
        let dateish = has_digit
            && first.chars().count() <= 20
            && (first.contains('-') || first.contains(':') || first.contains('/'));
        if !dateish {
            break;
        }
        out = out[first.len()..].trim_start_matches(['：', ':', ' ', '\t']);
        if out.is_empty() {
            break;
        }
    }
    out.to_string()
}

/// 标题生成（12-24 字事件事实）
///
/// - 优先取【行为】段（做了什么），其次【前因】，无标记时用整段；
/// - 跳过【触发】行，去掉开头日期/时间戳；
/// - 截到首个句末标点或 24 字（首句过短也保留，不跨句拼接）；
/// - 禁主题分类名当标题（调用方不得传树根名）。
pub fn build_title(content: &str) -> String {
    let raw = section_text(content, "【行为】")
        .or_else(|| section_text(content, "【前因】"))
        .unwrap_or_else(|| content.to_string());

    let lines: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("【触发】"))
        .map(|l| l.replace(['【', '】', '\n', '\r', '\t'], " "))
        .collect();
    let mut collapsed = String::new();
    for c in lines.join(" ").chars() {
        if c.is_whitespace() {
            if !collapsed.ends_with(' ') {
                collapsed.push(' ');
            }
        } else {
            collapsed.push(c);
        }
    }
    let flat = strip_leading_stamp(collapsed.trim());

    let chars: Vec<char> = flat.chars().collect();
    if chars.is_empty() {
        return "未命名记忆".to_string();
    }
    let mut take = chars.len().min(24);
    for (i, c) in chars.iter().enumerate() {
        if i >= 24 {
            take = 24;
            break;
        }
        if matches!(c, '。' | '！' | '？' | '；' | ';') {
            take = i;
            break;
        }
    }
    let title: String = chars[..take].iter().collect();
    let title = title
        .trim()
        .trim_end_matches(['，', ',', '、', '：', ':', '。', '；', ';'])
        .trim()
        .to_string();
    if title.is_empty() {
        "未命名记忆".to_string()
    } else {
        title
    }
}

/// 三标补全（本地降级，不改写事实）
///
/// 主区条目必须含【前因】【行为】【后果】，否则 recall 时正文会被截断。
/// - 已有任一段标记 → 原样返回；
/// - 单段自由文本 → 整体标为【行为】（前因/后果留白并告警）；
/// - 两段 → 【前因】/【行为】；三段及以上 → 首段前因、中段行为、末段后果；
/// - `【触发】` 行原样保留在末尾。
pub fn normalize_sections(content: &str) -> (String, Vec<String>) {
    let trimmed = content.trim();
    if trimmed.contains("【前因】") || trimmed.contains("【行为】") || trimmed.contains("【后果】") {
        return (trimmed.to_string(), Vec::new());
    }
    let mut warnings = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut trigger: Vec<&str> = Vec::new();
    for line in trimmed.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if l.starts_with("【触发】") {
            trigger.push(l);
        } else {
            body.push(l);
        }
    }

    let mut out = match body.len() {
        0 => String::new(),
        1 => {
            warnings.push(
                "未补【前因】【后果】：单段文本已整体标为【行为】，可用「三段模板」补全。".into(),
            );
            format!("【行为】{}", body[0])
        }
        2 => {
            warnings.push("两段文本：首段记【前因】、次段记【行为】；未补【后果】。".into());
            format!("【前因】{}\n【行为】{}", body[0], body[1])
        }
        n => format!(
            "【前因】{}\n【行为】{}\n【后果】{}",
            body[0],
            body[1..n - 1].join("\n"),
            body[n - 1]
        ),
    };
    if !trigger.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&trigger.join("\n"));
    }
    if out.is_empty() {
        out = trimmed.to_string();
    }
    (out, warnings)
}

fn strip_markers(content: &str) -> String {
    content
        .replace("【前因】", " ")
        .replace("【行为】", " ")
        .replace("【后果】", " ")
        .replace(['\n', '\r'], " ")
        .trim()
        .to_string()
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn tail(s: &str, n: usize) -> String {
    let total = s.chars().count();
    if total <= n {
        return s.to_string();
    }
    s.chars().skip(total - n).collect()
}

// ---------------------------------------------------------------------------
// 判树（关键词 bigram 重叠）
// ---------------------------------------------------------------------------

fn bigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation() && !matches!(c, '：' | '，' | '、' | '。' | '；'))
        .collect();
    let mut set = HashSet::new();
    for w in chars.windows(2) {
        set.insert(w.iter().collect::<String>());
    }
    set
}

/// 从 23 树根中按内容关键词重叠挑最贴切者
pub fn pick_tree(content: &str, taxonomy: &[TaxonomyItem]) -> Option<String> {
    let cb = bigrams(&strip_markers(content));
    if cb.is_empty() {
        return None;
    }
    let mut best: Option<(i64, String)> = None;
    for root in taxonomy {
        let mut text = root.name.clone();
        if let Some(d) = &root.description {
            text.push_str(d);
        }
        let rb = bigrams(&text);
        let overlap = cb.intersection(&rb).count() as i64;
        // 名称命中加权
        let name_hit = if content.contains(&root.name) { 5 } else { 0 };
        let score = overlap + name_hit;
        if score <= 0 {
            continue;
        }
        if best.as_ref().map(|(b, _)| score > *b).unwrap_or(true) {
            best = Some((score, root.name.clone()));
        }
    }
    best.map(|(_, name)| name)
}

// ---------------------------------------------------------------------------
// 判重（recall 两轮）
// ---------------------------------------------------------------------------

fn dedupe_candidates(mut items: Vec<CandidateOut>) -> Vec<CandidateOut> {
    items.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for c in items.drain(..) {
        let key = c.id.clone();
        if seen.insert(key) {
            out.push(c);
        }
    }
    out
}

/// 两轮 recall 收集候选（去重、按分排序、截断 limit）
pub fn recall_rounds(
    client: &RsrsClient,
    content: &str,
    hint: Option<&str>,
    cfg: &AutomationConfig,
) -> Result<Vec<CandidateOut>, RsrsError> {
    let base = strip_markers(content);
    let mut queries: Vec<String> = Vec::new();
    if let Some(h) = hint {
        if !h.trim().is_empty() {
            queries.push(format!("{} {}", h.trim(), head(&base, 30)));
        }
    }
    queries.push(head(&base, 40));
    if base.chars().count() > 40 {
        queries.push(tail(&base, 40));
    }

    let rounds = cfg.recall_rounds.max(1) as usize;
    let limit = cfg.recall_limit.max(1);
    let mut all: Vec<CandidateOut> = Vec::new();
    for q in queries.iter().take(rounds) {
        if q.trim().is_empty() {
            continue;
        }
        let list = client.recall_candidates(q, limit)?;
        for c in list {
            all.push(CandidateOut {
                id: c.entry.id,
                title: c.entry.title,
                score: c.score,
                source: "recall".into(),
            });
        }
    }
    let mut deduped = dedupe_candidates(all);
    deduped.truncate(limit as usize);
    Ok(deduped)
}

// ---------------------------------------------------------------------------
// 规划
// ---------------------------------------------------------------------------

/// 生成计划：判档 → 判重 → 定分支。破坏性分支（改/并）标记 needs_confirm。
pub fn plan(
    client: &RsrsClient,
    content: &str,
    hint: Option<&str>,
    taxonomy: &[TaxonomyItem],
    cfg: &AutomationConfig,
) -> Result<Plan, RsrsError> {
    plan_with(client, content, hint, taxonomy, cfg, None)
}

/// 同 `plan`，可传入模型判档结果（P2 模型路径）；空字段自动回退本地规则。
pub fn plan_with(
    client: &RsrsClient,
    content: &str,
    hint: Option<&str>,
    taxonomy: &[TaxonomyItem],
    cfg: &AutomationConfig,
    verdict: Option<&crate::llm::Verdict>,
) -> Result<Plan, RsrsError> {
    let content = content.trim();
    let local_importance = judge_importance(content);
    let use_model = verdict
        .map(|v| v.importance == "important" || v.importance == "trivial")
        .unwrap_or(false);
    let importance = match verdict.map(|v| v.importance.as_str()) {
        Some(v) if v == "important" || v == "trivial" => v.to_string(),
        _ => local_importance.to_string(),
    };
    let title = verdict
        .map(|v| v.title.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .unwrap_or_else(|| build_title(content));
    let source = if use_model {
        "model".to_string()
    } else {
        "local".to_string()
    };

    // 日记：一步落库，不判树不判重
    if importance == "trivial" {
        return Ok(Plan {
            importance,
            title,
            branch: "journal".into(),
            suggested_parent: None,
            suggested_tree: None,
            candidates: Vec::new(),
            needs_confirm: false,
            message: "判档=trivial，走日记一步落库。".into(),
            warnings: Vec::new(),
            source,
        });
    }

    let candidates = recall_rounds(client, content, hint, cfg)?;
    let top = candidates.first().cloned();

    if let Some(t) = &top {
        if t.score >= cfg.dup_threshold {
            return Ok(Plan {
                importance,
                title,
                branch: "confirm".into(),
                suggested_parent: Some(t.id.clone()),
                suggested_tree: None,
                candidates,
                needs_confirm: true,
                message: format!(
                    "判重命中高相似条目（{:.0}%）：{}。请择一：改 / 并 / 挂 / 存新条。",
                    t.score * 100.0,
                    t.title
                ),
                warnings: Vec::new(),
                source,
            });
        }
        if t.score >= cfg.attach_threshold {
            return Ok(Plan {
                importance,
                title,
                branch: "attach".into(),
                suggested_parent: Some(t.id.clone()),
                suggested_tree: None,
                candidates,
                needs_confirm: false,
                message: format!(
                    "判重命中同题条目（{:.0}%）：{}，自动挂其下。",
                    t.score * 100.0,
                    t.title
                ),
                warnings: Vec::new(),
                source,
            });
        }
    }

    // 新存：模型判树优先，其次关键词
    let tree = verdict
        .map(|v| v.tree.trim())
        .filter(|t| !t.is_empty() && taxonomy.iter().any(|r| r.name == *t))
        .map(|t| t.to_string())
        .or_else(|| pick_tree(content, taxonomy));
    Ok(Plan {
        importance,
        title,
        branch: "create".into(),
        suggested_parent: tree.clone(),
        suggested_tree: tree.clone(),
        candidates,
        needs_confirm: tree.is_none(),
        message: match &tree {
            Some(t) => format!("未命中相近条目，判树挂「{t}」新建。"),
            None => "未命中相近条目，且关键词无法判树；请手动选择分类树。".into(),
        },
        warnings: Vec::new(),
        source,
    })
}

// ---------------------------------------------------------------------------
// 执行
// ---------------------------------------------------------------------------

pub(crate) fn backup_json(dir: &Path, tag: &str, value: &serde_json::Value) -> Result<(), RsrsError> {
    let _ = std::fs::create_dir_all(dir);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("memtools-backup-{tag}-{stamp}.json"));
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| RsrsError::Other(format!("备份序列化失败: {e}")))?;
    std::fs::write(&path, text).map_err(|e| RsrsError::Other(format!("备份写入失败: {e}")))?;
    Ok(())
}

fn remember_to_outcome(
    branch: &str,
    importance: &str,
    res: crate::rsrs_client::RememberResponse,
    message: String,
) -> Outcome {
    let summary = res.summary.clone().unwrap_or_default();
    Outcome {
        ok: !res.is_dangling(),
        branch: branch.to_string(),
        action: summary.action.clone(),
        id: summary.id.clone(),
        parent: summary.parent.clone(),
        title: summary.title.clone(),
        importance: importance.to_string(),
        message,
        warnings: res.warnings.clone(),
        candidates: Vec::new(),
        needs_confirm: res.is_dangling(),
    }
}

/// 执行某个分支（改/并会先备份，改后回读校验）
#[allow(clippy::too_many_arguments)]
pub fn apply(
    client: &RsrsClient,
    content: &str,
    title: &str,
    branch: &str,
    candidate: Option<&str>,
    tree: Option<&str>,
    importance: &str,
    backup_dir: &Path,
) -> Result<Outcome, RsrsError> {
    let content = content.trim();
    let mut req = RememberRequest {
        content: content.to_string(),
        title: Some(title.to_string()),
        ..Default::default()
    };

    match branch {
        "journal" => {
            req.importance = Some("trivial".into());
            req.title = None;
            let res = client.remember(req)?;
            Ok(remember_to_outcome(
                "journal",
                "trivial",
                res,
                "日记已并入当日轨迹。".into(),
            ))
        }
        "attach" => {
            let parent = candidate
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| RsrsError::Other("挂：缺少候选 id".into()))?;
            req.parent = Some(parent.to_string());
            req.importance = Some(importance.to_string());
            req.r#type = Some("context".into());
            let res = client.remember(req)?;
            Ok(remember_to_outcome(
                "attach",
                importance,
                res,
                format!("已挂到候选 {parent} 之下。"),
            ))
        }
        "create" => {
            let parent = tree
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| RsrsError::Other("存：缺少分类树".into()))?;
            req.parent = Some(parent.to_string());
            req.importance = Some(importance.to_string());
            req.r#type = Some("context".into());
            let res = client.remember(req)?;
            Ok(remember_to_outcome(
                "create",
                importance,
                res,
                format!("已在「{parent}」下新建。"),
            ))
        }
        "update" => {
            let id = candidate
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| RsrsError::Other("改：缺少候选 id".into()))?;
            // 备份原条全文
            let old = client.show_entry(id)?;
            backup_json(
                backup_dir,
                &id.chars().take(8).collect::<String>(),
                &serde_json::to_value(&old.entry).unwrap_or_default(),
            )?;
            // 更新
            let env = client.update(id, Some(content), Some(title))?;
            if !env.errors.is_empty() {
                return Err(RsrsError::Mcp(format!(
                    "改（update）返回错误: {:?}",
                    env.errors
                )));
            }
            // 回读校验
            let verify = client.show_entry(id)?;
            if verify.entry.content.trim() != content {
                return Err(RsrsError::Other(
                    "改后回读校验不一致（content 未更新成功），请检查。".into(),
                ));
            }
            Ok(Outcome {
                ok: true,
                branch: "update".into(),
                action: Some("updated".into()),
                id: Some(verify.entry.id.clone()),
                parent: verify.entry.parent_id.clone(),
                title: Some(verify.entry.title.clone()),
                importance: verify
                    .entry
                    .importance
                    .clone()
                    .unwrap_or_else(|| importance.to_string()),
                message: "已更新原条（改前已备份，改后回读校验一致）。".into(),
                warnings: Vec::new(),
                candidates: Vec::new(),
                needs_confirm: false,
            })
        }
        "merge" => {
            let ids = candidate
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| RsrsError::Other("并：缺少候选 id".into()))?;
            // 备份被并条目全文 + 其子孙（防孤儿）
            let mut backup = Vec::new();
            for id in ids.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if let Ok(shown) = client.show_entry(id) {
                    backup.push(serde_json::json!({
                        "id": shown.entry.id,
                        "title": shown.entry.title,
                        "children": shown.children,
                        "content": shown.entry.content,
                    }));
                }
            }
            backup_json(backup_dir, "merge", &serde_json::Value::Array(backup))?;

            req.merge_ids = Some(ids.to_string());
            req.importance = Some(importance.to_string());
            req.r#type = Some("context".into());
            let res = client.remember(req)?;
            Ok(remember_to_outcome(
                "merge",
                importance,
                res,
                format!("已合并候选 {ids} 为新条（合并前已备份各条全文）。"),
            ))
        }
        other => Err(RsrsError::Other(format!("未知分支: {other}"))),
    }
}

// ---------------------------------------------------------------------------
// 单测（不依赖 rsrs runtime）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_prefers_action_section() {
        let c = "【前因】用户反馈关闭窗口后热键失效\n【行为】修复托盘常驻与关闭隐藏\n【后果】热键恢复可用";
        assert_eq!(build_title(c), "修复托盘常驻与关闭隐藏");
    }

    #[test]
    fn title_strips_timestamp_and_trigger() {
        let c = "【行为】2026-10-17 10:20 补齐 task 触发行校验\n【触发】2026-10-20";
        assert_eq!(build_title(c), "补齐 task 触发行校验");
    }

    #[test]
    fn title_cuts_at_sentence_end() {
        let c = "【行为】补齐触发行校验。后续再补提醒机制与可视化挂点展示";
        assert_eq!(build_title(c), "补齐触发行校验");
    }

    #[test]
    fn normalize_single_paragraph_marks_behavior() {
        let (out, w) = normalize_sections("随手记一条");
        assert_eq!(out, "【行为】随手记一条");
        assert!(!w.is_empty());
    }

    #[test]
    fn normalize_three_paragraphs() {
        let (out, w) = normalize_sections("背景\n动作\n结果");
        assert_eq!(out, "【前因】背景\n【行为】动作\n【后果】结果");
        assert!(w.is_empty());
    }

    #[test]
    fn normalize_keeps_trigger_line() {
        let (out, _) = normalize_sections("背景\n动作\n结果\n【触发】2026-10-20");
        assert!(out.ends_with("【触发】2026-10-20"));
        assert_eq!(out.lines().count(), 4);
    }

    #[test]
    fn normalize_idempotent_when_marked() {
        let s = "【前因】a\n【行为】b\n【后果】c";
        assert_eq!(normalize_sections(s).0, s);
    }
}