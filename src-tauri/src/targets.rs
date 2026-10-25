//! 快捷目标存储（P9）——磁盘格式 `{"version":1,"targets":[...]}`，数组顺序即优先级（第 1 个 = ★）。
//!
//! 顺序敏感：小窗列表、右键菜单、`GET /targets` 都按数组序展示，故内部用 `Vec` 保存。
//! 兼容两种存量格式：数组文件（现行）与旧版按 id 键控的对象（`{id: {...}}`）。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// 快捷目标定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickTarget {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub parent: Option<String>,
    pub r#type: Option<String>,
    pub importance: Option<String>,
}

/// 磁盘文件格式（数组序＝菜单序，勿改成对象）
#[derive(Debug, Serialize, Deserialize)]
struct TargetsFile {
    #[serde(default = "current_version")]
    version: u32,
    #[serde(default)]
    targets: Vec<QuickTarget>,
}

fn current_version() -> u32 {
    1
}

/// 快捷目标存储（线程安全）
pub struct TargetStore {
    inner: Mutex<TargetStoreInner>,
}

struct TargetStoreInner {
    /// 顺序即优先级，勿改回 HashMap（旧版曾用 HashMap，导致菜单序丢失）
    targets: Vec<QuickTarget>,
    path: PathBuf,
}

impl TargetStore {
    pub fn load(path: PathBuf) -> Self {
        let targets = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .map(|s| parse_targets(&s))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            inner: Mutex::new(TargetStoreInner { targets, path }),
        }
    }

    pub fn list(&self) -> Vec<QuickTarget> {
        self.inner.lock().unwrap().targets.clone()
    }

    pub fn get(&self, id: &str) -> Option<QuickTarget> {
        self.inner
            .lock()
            .unwrap()
            .targets
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    pub fn replace(&self, incoming: Vec<QuickTarget>) -> Result<Vec<QuickTarget>, String> {
        let mut inner = self.inner.lock().unwrap();
        // 验证 kind 字段
        const VALID_KINDS: &[&str] = &["journal", "tree", "auto"];
        let mut seen: HashSet<&str> = HashSet::new();
        for t in &incoming {
            if !VALID_KINDS.contains(&t.kind.as_str()) {
                return Err(format!(
                    "非法目标 kind: {}，合法值: journal/tree/auto",
                    t.kind
                ));
            }
            if t.id.trim().is_empty() {
                return Err("目标 id 不能为空".into());
            }
            if !seen.insert(t.id.as_str()) {
                return Err(format!("目标 id 重复：{}", t.id));
            }
        }
        // 顺序照传入顺序原样保存（列表/菜单的排序就是用户排的顺序）
        inner.targets = incoming;
        inner.save()
    }

    pub fn reset(&self) -> Result<Vec<QuickTarget>, String> {
        let mut inner = self.inner.lock().unwrap();
        // 默认目标：日记 + 自动判档（顺序即默认优先级）
        inner.targets = vec![
            QuickTarget {
                id: "journal".into(),
                label: "日记".into(),
                kind: "journal".into(),
                parent: None,
                r#type: None,
                importance: Some("trivial".into()),
            },
            QuickTarget {
                id: "auto".into(),
                label: "自动判档".into(),
                kind: "auto".into(),
                parent: None,
                r#type: None,
                importance: Some("important".into()),
            },
        ];
        inner.save()
    }
}

impl TargetStoreInner {
    fn save(&mut self) -> Result<Vec<QuickTarget>, String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        let file = TargetsFile {
            version: current_version(),
            targets: self.targets.clone(),
        };
        let json =
            serde_json::to_string_pretty(&file).map_err(|e| format!("序列化失败: {e}"))?;
        fs::write(&self.path, json).map_err(|e| format!("写入失败: {e}"))?;
        Ok(self.targets.clone())
    }
}

/// 解析磁盘内容：现行数组文件优先，其次兼容旧版 `{id: {...}}` 对象。
/// 解析不出来时返回空表（不 panic，避免整个小窗起不来）。
fn parse_targets(raw: &str) -> Vec<QuickTarget> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(_) => serde_json::from_value::<Vec<QuickTarget>>(value)
            .unwrap_or_default(),
        serde_json::Value::Object(obj) => {
            if obj.contains_key("targets") {
                let file = serde_json::from_value::<TargetsFile>(serde_json::Value::Object(obj))
                    .unwrap_or(TargetsFile {
                    version: current_version(),
                    targets: Vec::new(),
                });
                file.targets
            } else {
                // 旧格式：HashMap<String, QuickTarget>，本身无顺序信息，按 id 稳定排序
                let map = serde_json::from_value::<HashMap<String, QuickTarget>>(
                    serde_json::Value::Object(obj),
                )
                .unwrap_or_default();
                let mut list: Vec<QuickTarget> = map.into_values().collect();
                list.sort_by(|a, b| a.id.cmp(&b.id));
                list
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, kind: &str) -> QuickTarget {
        QuickTarget {
            id: id.into(),
            label: id.into(),
            kind: kind.into(),
            parent: None,
            r#type: None,
            importance: None,
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "memtools-targets-{tag}-{}-{stamp}.json",
            std::process::id()
        ))
    }

    #[test]
    fn parses_array_file_keeping_order() {
        let raw = r#"{
          "version": 1,
          "targets": [
            {"id":"journal","label":"日记","kind":"journal","parent":null,"type":null,"importance":"trivial"},
            {"id":"auto","label":"自动判档","kind":"auto","parent":null,"type":null,"importance":null}
          ]
        }"#;
        let list = parse_targets(raw);
        assert_eq!(list.len(), 2);
        // 顺序必须与文件一致：第 1 个即最优先
        assert_eq!(list[0].id, "journal");
        assert_eq!(list[1].id, "auto");
    }

    #[test]
    fn parses_legacy_object_file() {
        let raw = r#"{
          "auto": {"id":"auto","label":"A","kind":"auto","parent":null,"type":null,"importance":null},
          "journal": {"id":"journal","label":"J","kind":"journal","parent":null,"type":null,"importance":"trivial"}
        }"#;
        let list = parse_targets(raw);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "auto");
        assert_eq!(list[1].id, "journal");
    }

    #[test]
    fn garbage_file_yields_empty_list() {
        assert!(parse_targets("not json").is_empty());
        assert!(parse_targets("{}").is_empty());
        assert!(parse_targets("null").is_empty());
    }

    #[test]
    fn replace_then_load_roundtrip_keeps_order() {
        let path = temp_path("roundtrip");
        let store = TargetStore::load(path.clone());
        let saved = store
            .replace(vec![target("b", "tree"), target("a", "auto")])
            .unwrap();
        assert_eq!(saved.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);

        let reloaded = TargetStore::load(path.clone());
        let list = reloaded.list();
        assert_eq!(list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert!(reloaded.get("a").is_some());
        // 落盘仍是数组格式，version 保留
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"targets\""));
        assert!(raw.contains("\"version\": 1"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn replace_rejects_bad_kind_and_duplicate_id() {
        let path = temp_path("reject");
        let store = TargetStore::load(path.clone());
        let err = store.replace(vec![target("x", "bogus")]).unwrap_err();
        assert!(err.contains("kind"), "{err}");

        let err = store
            .replace(vec![target("dup", "journal"), target("dup", "auto")])
            .unwrap_err();
        assert!(err.contains("重复"), "{err}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn reset_restores_journal_and_auto_order() {
        let path = temp_path("reset");
        let store = TargetStore::load(path.clone());
        store.replace(vec![target("x", "tree")]).unwrap();
        let list = store.reset().unwrap();
        assert_eq!(
            list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["journal", "auto"]
        );
        let _ = fs::remove_file(&path);
    }
}
