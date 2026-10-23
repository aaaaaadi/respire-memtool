//! 全量快照上的只读分类过滤；词库分类不保证存在同名记忆节点。
use crate::rsrs_client::ListedEntry;
use std::collections::{HashMap, HashSet};

/// 按真实父链中的分类标题或 id 过滤，保持 Runtime 返回的最近顺序；None 表示不截断。
pub fn filter_category(
    entries: Vec<ListedEntry>,
    category: Option<&str>,
    limit: Option<usize>,
) -> Vec<ListedEntry> {
    let index: HashMap<&str, &ListedEntry> = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let mut matched = HashSet::new();
    for entry in &entries {
        let mut current = Some(entry);
        let mut seen = HashSet::new();
        while let Some(node) = current {
            if !seen.insert(node.id.as_str()) {
                break;
            }
            // 分类为空时保留全部；没有对应真实祖先的词库分类自然得到空集。
            if category
                .map(|name| node.id == name || node.title == name)
                .unwrap_or(true)
            {
                matched.insert(entry.id.clone());
                break;
            }
            current = node
                .parent_id
                .as_deref()
                .and_then(|parent| index.get(parent).copied());
        }
    }
    entries
        .into_iter()
        .filter(|entry| matched.contains(&entry.id))
        .take(limit.unwrap_or(usize::MAX))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str, title: &str, parent: Option<&str>) -> ListedEntry {
        ListedEntry {
            id: id.into(),
            title: title.into(),
            parent_id: parent.map(str::to_string),
            ..Default::default()
        }
    }
    fn entries() -> Vec<ListedEntry> {
        vec![
            entry("leaf", "事项", Some("branch")),
            entry("other", "其他", None),
            entry("branch", "专题", Some("root")),
            entry("root", "编程开发", None),
        ]
    }
    #[test]
    fn missing_category_is_empty() {
        assert!(filter_category(entries(), Some("任务计划"), None).is_empty());
    }
    #[test]
    fn category_includes_descendants_and_keeps_recent_order() {
        let ids: Vec<_> = filter_category(entries(), Some("编程开发"), None)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec!["leaf", "branch", "root"]);
    }
    #[test]
    fn all_scope_includes_more_than_two_hundred() {
        let many = (0..478)
            .map(|i| entry(&i.to_string(), "记忆", None))
            .collect();
        assert_eq!(filter_category(many, None, None).len(), 478);
        assert_eq!(filter_category(entries(), None, Some(2)).len(), 2);
    }
    #[test]
    fn orphan_and_cycle_are_safe() {
        let items = vec![
            entry("a", "a", Some("b")),
            entry("b", "b", Some("a")),
            entry("c", "c", Some("missing")),
        ];
        assert!(filter_category(items, Some("任务计划"), None).is_empty());
    }
}
