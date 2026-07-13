use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;
use typevoice_core::workflow::RecordingPlan;
use typevoice_storage::settings::Settings;

use crate::record_input::ResolvedRecordInput;

#[derive(Debug, Clone)]
pub struct CachedRecordInput {
    pub resolved: ResolvedRecordInput,
    pub recording_plan: RecordingPlan,
    pub refreshed_at_ms: i64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CachedRecordInputError {
    pub code: String,
    pub message: String,
    pub ts_ms: i64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct RecordInputCacheSnapshot {
    pub last_error: Option<CachedRecordInputError>,
    pub refresh_in_progress: bool,
    pub pending_reason: Option<String>,
}

#[derive(Debug, Default)]
struct RecordInputCacheInner {
    last_ok: Option<CachedRecordInput>,
    validated: Vec<CachedRecordInput>,
    last_error: Option<CachedRecordInputError>,
    refresh_in_progress: bool,
    pending_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RecordInputCacheState {
    inner: Arc<Mutex<RecordInputCacheInner>>,
    refresh_serial: Arc<Mutex<()>>,
}

impl RecordInputCacheState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordInputCacheInner::default())),
            refresh_serial: Arc::new(Mutex::new(())),
        }
    }

    pub fn snapshot(&self) -> RecordInputCacheSnapshot {
        let g = self.inner.lock().unwrap();
        RecordInputCacheSnapshot {
            last_error: g.last_error.clone(),
            refresh_in_progress: g.refresh_in_progress,
            pending_reason: g.pending_reason.clone(),
        }
    }

    pub fn get_for_plan(&self, plan: &RecordingPlan) -> Option<CachedRecordInput> {
        let guard = self.inner.lock().unwrap();
        if guard.refresh_in_progress {
            return None;
        }
        guard
            .validated
            .iter()
            .find(|cached| &cached.recording_plan == plan)
            .cloned()
    }

    pub fn refresh_blocking(
        &self,
        data_dir: &Path,
        reason: &str,
    ) -> Result<CachedRecordInput, String> {
        let _refresh_guard = self.refresh_serial.lock().unwrap();
        let span = crate::obs::Span::start(
            data_dir,
            None,
            "App",
            "APP.record_input_cache_refresh",
            Some(json!({ "reason": reason })),
        );
        let settings = match typevoice_storage::settings::load_settings_strict(data_dir) {
            Ok(settings) => settings,
            Err(error) => {
                let msg = format!("E_RECORD_INPUT_CACHE_REFRESH_FAILED: {error}");
                let code = extract_error_code(&msg);
                self.write_error(reason, &code, &msg);
                span.err("config", &code, &msg, Some(json!({ "reason": reason })));
                return Err(msg);
            }
        };
        self.resolve_publish_and_trace(data_dir, reason, &settings, span)
    }

    pub fn save_validated_settings_blocking(
        &self,
        data_dir: &Path,
        reason: &str,
        settings: &Settings,
    ) -> Result<CachedRecordInput, String> {
        let _refresh_guard = self.refresh_serial.lock().unwrap();
        let span = crate::obs::Span::start(
            data_dir,
            None,
            "App",
            "APP.record_input_settings_validate",
            Some(json!({ "reason": reason })),
        );
        let cached = match self.resolve_cached(data_dir, reason, settings) {
            Ok(cached) => cached,
            Err(message) => {
                let code = extract_error_code(&message);
                self.write_error(reason, &code, &message);
                span.err("config", &code, &message, Some(json!({ "reason": reason })));
                return Err(message);
            }
        };
        if let Err(error) = typevoice_storage::settings::save_settings(data_dir, settings) {
            let message = format!("E_SETTINGS_WRITE: {error}");
            self.write_error(reason, "E_SETTINGS_WRITE", &message);
            span.err(
                "io",
                "E_SETTINGS_WRITE",
                &message,
                Some(json!({ "reason": reason })),
            );
            return Err(message);
        }
        self.publish_and_trace(reason, cached, span)
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn request_refresh(&self, data_dir: PathBuf, reason: impl Into<String>) {
        let first_reason = reason.into();
        let mut should_spawn = false;
        {
            let mut g = self.inner.lock().unwrap();
            g.last_ok = None;
            g.validated.clear();
            if g.refresh_in_progress {
                g.pending_reason = Some(first_reason.clone());
            } else {
                g.refresh_in_progress = true;
                should_spawn = true;
            }
        }
        if !should_spawn {
            return;
        }

        let this = self.clone();
        std::thread::spawn(move || {
            let mut current_reason = first_reason;
            loop {
                let _ = this.refresh_blocking(&data_dir, current_reason.as_str());
                let next_reason = {
                    let mut g = this.inner.lock().unwrap();
                    match g.pending_reason.take() {
                        Some(next) => Some(next),
                        None => {
                            g.refresh_in_progress = false;
                            None
                        }
                    }
                };
                match next_reason {
                    Some(next) => current_reason = next,
                    None => break,
                }
            }
        });
    }

    fn write_error(&self, reason: &str, code: &str, message: &str) {
        let mut g = self.inner.lock().unwrap();
        g.last_error = Some(CachedRecordInputError {
            code: code.to_string(),
            message: message.to_string(),
            ts_ms: now_epoch_ms(),
            reason: reason.to_string(),
        });
    }

    fn resolve_publish_and_trace(
        &self,
        data_dir: &Path,
        reason: &str,
        settings: &Settings,
        span: crate::obs::Span,
    ) -> Result<CachedRecordInput, String> {
        let cached = match self.resolve_cached(data_dir, reason, settings) {
            Ok(cached) => cached,
            Err(message) => {
                let code = extract_error_code(&message);
                self.write_error(reason, &code, &message);
                span.err("config", &code, &message, Some(json!({ "reason": reason })));
                return Err(message);
            }
        };
        self.publish_and_trace(reason, cached, span)
    }

    fn resolve_cached(
        &self,
        data_dir: &Path,
        reason: &str,
        settings: &Settings,
    ) -> Result<CachedRecordInput, String> {
        let ffmpeg = crate::pipeline::ffmpeg_cmd().map_err(|error| {
            format!("E_RECORD_INPUT_CACHE_REFRESH_FAILED: resolve ffmpeg failed: {error}")
        })?;
        let recording_plan = recording_plan_from_settings(settings);
        let resolved = crate::record_input::resolve_record_input_for_settings(
            data_dir,
            ffmpeg.as_str(),
            settings,
        )?;
        Ok(CachedRecordInput {
            resolved,
            recording_plan,
            refreshed_at_ms: now_epoch_ms(),
            reason: reason.to_string(),
        })
    }

    fn publish_and_trace(
        &self,
        reason: &str,
        cached: CachedRecordInput,
        span: crate::obs::Span,
    ) -> Result<CachedRecordInput, String> {
        {
            let mut guard = self.inner.lock().unwrap();
            guard.last_ok = Some(cached.clone());
            guard
                .validated
                .retain(|entry| entry.recording_plan != cached.recording_plan);
            guard.validated.insert(0, cached.clone());
            guard.validated.truncate(2);
            guard.last_error = None;
        }
        span.ok(Some(json!({
            "reason": reason,
            "refreshed_at_ms": cached.refreshed_at_ms,
            "record_input_spec": cached.resolved.spec,
            "record_input_strategy": cached.resolved.strategy_used,
            "record_input_resolved_by": cached.resolved.resolved_by,
            "record_input_endpoint_id": cached.resolved.endpoint_id,
            "record_input_friendly_name": cached.resolved.friendly_name,
            "record_input_resolution_log": cached.resolved.resolution_log,
            "recording_plan": cached.recording_plan,
        })));
        Ok(cached)
    }
}

pub fn recording_plan_from_settings(settings: &Settings) -> RecordingPlan {
    RecordingPlan {
        input_strategy: settings
            .record_input_strategy
            .clone()
            .unwrap_or_else(|| "follow_default".to_string()),
        follow_default_role: settings
            .record_follow_default_role
            .clone()
            .unwrap_or_else(|| "communications".to_string()),
        fixed_endpoint_id: settings.record_fixed_endpoint_id.clone(),
    }
}

impl Default for RecordInputCacheState {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_error_code(message: &str) -> String {
    let first = message.split(':').next().unwrap_or("").trim();
    if first.starts_with("E_") {
        return first.to_string();
    }
    let token = message.split_whitespace().next().unwrap_or("").trim();
    if token.starts_with("E_") {
        return token.trim_end_matches(':').to_string();
    }
    "E_RECORD_INPUT_CACHE_REFRESH_FAILED".to_string()
}

fn now_epoch_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(v) => v.as_millis() as i64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record_input::ResolvedRecordInput;

    #[test]
    fn refresh_in_progress_invalidates_previously_validated_input() {
        let state = RecordInputCacheState::new();
        let plan = RecordingPlan {
            input_strategy: "follow_default".to_string(),
            follow_default_role: "communications".to_string(),
            fixed_endpoint_id: None,
        };
        state
            .inner
            .lock()
            .unwrap()
            .validated
            .push(CachedRecordInput {
                resolved: ResolvedRecordInput {
                    spec: "audio=old-default".to_string(),
                    strategy_used: "follow_default".to_string(),
                    endpoint_id: Some("old-default".to_string()),
                    friendly_name: Some("Old default".to_string()),
                    resolved_by: "default_endpoint".to_string(),
                    resolution_log: Vec::new(),
                },
                recording_plan: plan.clone(),
                refreshed_at_ms: 1,
                reason: "startup".to_string(),
            });
        assert!(state.get_for_plan(&plan).is_some());

        state.inner.lock().unwrap().refresh_in_progress = true;

        assert!(state.get_for_plan(&plan).is_none());
    }
}
