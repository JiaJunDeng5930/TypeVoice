use std::{path::Path, sync::Arc};

use crate::{context_capture, context_pack};
use anyhow::{anyhow, Result};

pub trait ContextCollector: Send + Sync {
    fn warmup_best_effort(&self);
    fn capture_hotkey_context_now(
        &self,
        data_dir: &Path,
        cfg: &context_capture::ContextConfig,
    ) -> Result<String>;
    fn take_hotkey_context_once(&self, capture_id: &str) -> Option<context_pack::ContextSnapshot>;
    fn capture_snapshot_best_effort_with_config(
        &self,
        data_dir: &Path,
        task_id: &str,
        cfg: &context_capture::ContextConfig,
    ) -> context_pack::ContextSnapshot;
}

impl ContextCollector for context_capture::ContextService {
    fn warmup_best_effort(&self) {
        self.warmup_best_effort();
    }

    fn capture_hotkey_context_now(
        &self,
        data_dir: &Path,
        cfg: &context_capture::ContextConfig,
    ) -> Result<String> {
        self.capture_hotkey_context_now(data_dir, cfg)
    }

    fn take_hotkey_context_once(&self, capture_id: &str) -> Option<context_pack::ContextSnapshot> {
        self.take_hotkey_context_once(capture_id)
    }

    fn capture_snapshot_best_effort_with_config(
        &self,
        data_dir: &Path,
        task_id: &str,
        cfg: &context_capture::ContextConfig,
    ) -> context_pack::ContextSnapshot {
        self.capture_snapshot_best_effort_with_config(data_dir, task_id, cfg)
    }
}

#[derive(Clone)]
pub struct TaskManager {
    ctx: Arc<dyn ContextCollector>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            ctx: Arc::new(context_capture::ContextService::new()),
        }
    }

    pub fn warmup_context_best_effort(&self) {
        self.ctx.warmup_best_effort();
    }

    pub fn capture_hotkey_context(
        &self,
        data_dir: &Path,
        context_cfg: &context_capture::ContextConfig,
    ) -> Result<context_pack::ContextSnapshot> {
        let capture_id = self.ctx.capture_hotkey_context_now(data_dir, context_cfg)?;
        self.ctx
            .take_hotkey_context_once(&capture_id)
            .ok_or_else(|| anyhow!("failed to retrieve hotkey context payload"))
    }

    pub fn capture_snapshot_best_effort_with_config(
        &self,
        data_dir: &Path,
        task_id: &str,
        cfg: &context_capture::ContextConfig,
    ) -> context_pack::ContextSnapshot {
        self.ctx
            .capture_snapshot_best_effort_with_config(data_dir, task_id, cfg)
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeContext {
        payloads: Mutex<HashMap<String, context_pack::ContextSnapshot>>,
    }

    impl ContextCollector for FakeContext {
        fn warmup_best_effort(&self) {}

        fn capture_hotkey_context_now(
            &self,
            _data_dir: &Path,
            _cfg: &context_capture::ContextConfig,
        ) -> Result<String> {
            let id = "capture-1".to_string();
            self.payloads
                .lock()
                .unwrap()
                .insert(id.clone(), context_pack::ContextSnapshot::default());
            Ok(id)
        }

        fn take_hotkey_context_once(
            &self,
            capture_id: &str,
        ) -> Option<context_pack::ContextSnapshot> {
            self.payloads.lock().unwrap().remove(capture_id)
        }

        fn capture_snapshot_best_effort_with_config(
            &self,
            _data_dir: &Path,
            _task_id: &str,
            _cfg: &context_capture::ContextConfig,
        ) -> context_pack::ContextSnapshot {
            context_pack::ContextSnapshot::default()
        }
    }

    fn manager_with_fake_context() -> TaskManager {
        TaskManager {
            ctx: Arc::new(FakeContext::default()),
        }
    }

    #[test]
    fn hotkey_context_capture_returns_snapshot() {
        let manager = manager_with_fake_context();
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(manager
            .capture_hotkey_context(dir.path(), &context_capture::ContextConfig::default())
            .is_ok());
    }
}
