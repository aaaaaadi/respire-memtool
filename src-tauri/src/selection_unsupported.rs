//! 非 Windows 平台的划词悬浮占位实现：接口与 Windows 版一致，全部为空操作。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::AppHandle;

use crate::config::SelectionConfig;

#[derive(Clone, Serialize)]
pub struct SelectionPayload {
    pub text: String,
    pub source: String,
}

pub struct SelectionState {
    enabled: AtomicBool,
    payload: Mutex<Option<SelectionPayload>>,
}

impl SelectionState {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            payload: Mutex::new(None),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn is_visible(&self) -> bool {
        false
    }

    pub fn payload(&self) -> Option<SelectionPayload> {
        self.payload.lock().ok().and_then(|p| p.clone())
    }
}

pub fn start(_app: AppHandle, _cfg: SelectionConfig, _state: Arc<SelectionState>) {
    eprintln!("[memTools] 当前平台不支持全局划词悬浮，已跳过");
}

pub fn resize_selector(
    _app: &AppHandle,
    _state: &SelectionState,
    _expanded: bool,
) -> Result<(), String> {
    Ok(())
}

pub fn hide_selector(_app: &AppHandle, _state: &SelectionState) {}

pub fn set_enabled(_app: &AppHandle, state: &SelectionState, on: bool) {
    state.set_enabled(on);
}
