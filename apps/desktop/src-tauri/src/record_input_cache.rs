use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::record_input::ResolvedRecordInput;

#[derive(Debug, Clone)]
pub struct CachedRecordInput {
    pub resolved: ResolvedRecordInput,
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
    last_error: Option<CachedRecordInputError>,
    refresh_in_progress: bool,
    pending_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RecordInputCacheState {
    inner: Arc<Mutex<RecordInputCacheInner>>,
}

impl RecordInputCacheState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordInputCacheInner::default())),
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

    pub fn get_last_ok(&self) -> Option<CachedRecordInput> {
        self.inner.lock().unwrap().last_ok.clone()
    }

    pub fn refresh_blocking(
        &self,
        data_dir: &Path,
        reason: &str,
    ) -> Result<CachedRecordInput, String> {
        let span = crate::trace::Span::start(
            data_dir,
            None,
            "App",
            "APP.record_input_cache_refresh",
            Some(json!({ "reason": reason })),
        );

        let ffmpeg = match crate::pipeline::ffmpeg_cmd() {
            Ok(v) => v,
            Err(e) => {
                let msg =
                    format!("E_RECORD_INPUT_CACHE_REFRESH_FAILED: resolve ffmpeg failed: {e}");
                let code = extract_error_code(msg.as_str());
                self.write_error(reason, &code, &msg);
                span.err("config", &code, &msg, Some(json!({ "reason": reason })));
                return Err(msg);
            }
        };

        let resolved = match crate::record_input::resolve_record_input_for_recording(
            data_dir,
            ffmpeg.as_str(),
        ) {
            Ok(v) => v,
            Err(e) => {
                let code = extract_error_code(e.as_str());
                self.write_error(reason, &code, &e);
                span.err("config", &code, &e, Some(json!({ "reason": reason })));
                return Err(e);
            }
        };

        let cached = CachedRecordInput {
            resolved: resolved.clone(),
            refreshed_at_ms: now_epoch_ms(),
            reason: reason.to_string(),
        };
        {
            let mut g = self.inner.lock().unwrap();
            g.last_ok = Some(cached.clone());
            g.last_error = None;
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
        })));
        Ok(cached)
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn request_refresh(&self, data_dir: PathBuf, reason: impl Into<String>) {
        let first_reason = reason.into();
        let mut should_spawn = false;
        {
            let mut g = self.inner.lock().unwrap();
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
