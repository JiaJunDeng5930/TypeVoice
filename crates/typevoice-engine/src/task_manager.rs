use std::path::Path;

use crate::{context_capture, context_pack};
use anyhow::{anyhow, Result};

#[derive(Clone)]
pub struct TaskManager {
    ctx: context_capture::ContextService,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            ctx: context_capture::ContextService::new(),
        }
    }

    pub fn warmup_context_best_effort(&self) {
        self.ctx.warmup_best_effort();
    }

    pub fn last_external_hwnd_best_effort(&self) -> Option<isize> {
        self.ctx.last_external_hwnd_best_effort()
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
