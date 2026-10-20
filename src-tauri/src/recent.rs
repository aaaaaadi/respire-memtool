//! recent — 最近记忆浏览的纯逻辑（分类过滤 / 采样窗口）
//!
//! memory_list 不支持按分类过滤，故分类浏览走：
//! ① memory_tree 收集子树 id；② 按窗口拉最近条目；③ 用本模块的 filter 保留命中的条目。

use std::collections::HashSet;

use crate::rsrs_client::ListedEntry;

/// 分类过滤的起始采样窗口：至少 200，按请求条数放大，封顶 2000
pub fn initial_window(limit: u32) -> u32 {
    (limit.saturating_mul(10)).clamp(200, 2000)
}

/// 从最近列表（server 已按 created_at 倒序）中筛出属于 ids 的条目，保持顺序并截断
pub fn filter_entries(
    entries: Vec<ListedEntry>,
    ids: &HashSet<String>,
    limit: usize,
) -> Vec<ListedEntry> {
    entries
        .into_iter()
        .filter(|e| ids.contains(&e.id))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> ListedEntry {
        ListedEntry {
            id: id.into(),
            title: id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn window_scales_and_caps() {
        assert_eq!(initial_window(20), 200);
        assert_eq!(initial_window(200), 2000);
        assert_eq!(initial_window(1), 200);
    }

    #[test]
    fn filter_keeps_order_and_limit() {
        let entries = vec![entry("a"), entry("b"), entry("c"), entry("d")];
        let ids: HashSet<String> = ["a", "c", "d"].iter().map(|s| s.to_string()).collect();
        let out = filter_entries(entries, &ids, 2);
        assert_eq!(
            out.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn filter_accepts_nothing() {
        let entries = vec![entry("a")];
        let ids: HashSet<String> = HashSet::new();
        assert!(filter_entries(entries, &ids, 20).is_empty());
    }
}
