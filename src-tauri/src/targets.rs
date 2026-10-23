//! 快捷目标存储（P9）—— 最小存根实现，等待完整同步。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// 快捷目标存储（线程安全）
pub struct TargetStore {
    inner: Mutex<TargetStoreInner>,
}

struct TargetStoreInner {
    targets: HashMap<String, QuickTarget>,
    path: PathBuf,
}

impl TargetStore {
    pub fn load(path: PathBuf) -> Self {
        let targets = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<HashMap<String, QuickTarget>>(&s).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self {
            inner: Mutex::new(TargetStoreInner { targets, path }),
        }
    }

    pub fn list(&self) -> Vec<QuickTarget> {
        self.inner.lock().unwrap().targets.values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<QuickTarget> {
        self.inner.lock().unwrap().targets.get(id).cloned()
    }

    pub fn replace(&self, incoming: Vec<QuickTarget>) -> Result<Vec<QuickTarget>, String> {
        let mut inner = self.inner.lock().unwrap();
        inner.targets.clear();
        for t in incoming {
            inner.targets.insert(t.id.clone(), t);
        }
        inner.save()
    }

    pub fn reset(&self) -> Result<Vec<QuickTarget>, String> {
        let mut inner = self.inner.lock().unwrap();
        inner.targets.clear();
        // 默认目标：日记 + 自动判档
        inner.targets.insert(
            "journal".into(),
            QuickTarget {
                id: "journal".into(),
                label: "日记".into(),
                kind: "journal".into(),
                parent: None,
                r#type: None,
                importance: Some("trivial".into()),
            },
        );
        inner.targets.insert(
            "auto".into(),
            QuickTarget {
                id: "auto".into(),
                label: "自动判档".into(),
                kind: "auto".into(),
                parent: None,
                r#type: None,
                importance: Some("important".into()),
            },
        );
        inner.save()
    }
}

impl TargetStoreInner {
    fn save(&mut self) -> Result<Vec<QuickTarget>, String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        let json = serde_json::to_string_pretty(&self.targets).map_err(|e| format!("序列化失败: {e}"))?;
        fs::write(&self.path, json).map_err(|e| format!("写入失败: {e}"))?;
        Ok(self.targets.values().cloned().collect())
    }
}