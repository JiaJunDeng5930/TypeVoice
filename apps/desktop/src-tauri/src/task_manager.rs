use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Result};
use uuid::Uuid;

use crate::{context_capture, context_pack};

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

#[derive(Debug, Clone)]
struct PendingHotkeyContext {
    created_at_ms: i64,
    pre_captured_context: context_pack::ContextSnapshot,
}

#[derive(Clone)]
pub struct TaskManager {
    pending_hotkey_contexts: Arc<Mutex<HashMap<String, PendingHotkeyContext>>>,
    ctx: Arc<dyn ContextCollector>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            pending_hotkey_contexts: Arc::new(Mutex::new(HashMap::new())),
            ctx: Arc::new(context_capture::ContextService::new()),
        }
    }

    pub fn warmup_context_best_effort(&self) {
        self.ctx.warmup_best_effort();
    }

    pub fn open_hotkey_task(
        &self,
        data_dir: &Path,
        context_cfg: &context_capture::ContextConfig,
        capture_required: bool,
    ) -> Result<String> {
        self.cleanup_orphan_pending_hotkey_contexts(60_000);

        let task_id = Uuid::new_v4().to_string();
        if capture_required {
            let capture_id = self.ctx.capture_hotkey_context_now(data_dir, context_cfg)?;
            let pre_captured_context = self
                .ctx
                .take_hotkey_context_once(&capture_id)
                .ok_or_else(|| anyhow!("failed to retrieve hotkey context payload"))?;
            let mut g = self.pending_hotkey_contexts.lock().unwrap();
            g.insert(
                task_id.clone(),
                PendingHotkeyContext {
                    created_at_ms: now_ms(),
                    pre_captured_context,
                },
            );
        }
        Ok(task_id)
    }

    pub fn take_pending_hotkey_context_for_rewrite(
        &self,
        task_id: &str,
    ) -> Option<context_pack::ContextSnapshot> {
        let mut g = self.pending_hotkey_contexts.lock().unwrap();
        g.remove(task_id).map(|ctx| ctx.pre_captured_context)
    }

    pub fn abort_pending_task(&self, task_id: &str) -> bool {
        let mut g = self.pending_hotkey_contexts.lock().unwrap();
        g.remove(task_id).is_some()
    }

    pub fn cleanup_orphan_pending_hotkey_contexts(&self, max_age_ms: i64) {
        let now = now_ms();
        let mut g = self.pending_hotkey_contexts.lock().unwrap();
        g.retain(|_, v| now.saturating_sub(v.created_at_ms) <= max_age_ms);
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            pending_hotkey_contexts: Arc::new(Mutex::new(HashMap::new())),
            ctx: Arc::new(FakeContext::default()),
        }
    }

    #[test]
    fn pending_hotkey_context_is_consumed_once() {
        let manager = manager_with_fake_context();
        let dir = tempfile::tempdir().expect("tempdir");
        let task_id = manager
            .open_hotkey_task(dir.path(), &context_capture::ContextConfig::default(), true)
            .expect("open");

        assert!(manager
            .take_pending_hotkey_context_for_rewrite(&task_id)
            .is_some());
        assert!(manager
            .take_pending_hotkey_context_for_rewrite(&task_id)
            .is_none());
    }
}
