use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
    process::{Child, ChildStderr, ChildStdout, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::record_input_cache::RecordInputCacheState;
use crate::subprocess::CommandNoConsoleExt;
use crate::ui_events::{UiEvent, UiEventMailbox};
use crate::{data_dir, obs, pipeline};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureError {
    pub code: String,
    pub message: String,
}

impl CaptureError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn render(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

struct ActiveRecording {
    session_id: String,
    task_id: Option<String>,
    output_path: PathBuf,
    child: Option<Child>,
    started_at: Instant,
    meter_join: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct RecordedAsset {
    pub asset_id: String,
    pub task_id: Option<String>,
    pub output_path: PathBuf,
    pub record_elapsed_ms: u128,
    created_at: Instant,
}

struct RegistryInner {
    active: Option<ActiveRecording>,
    assets: HashMap<String, RecordedAsset>,
}

pub struct RecordingRegistry {
    inner: Mutex<RegistryInner>,
}

impl RecordingRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RegistryInner {
                active: None,
                assets: HashMap::new(),
            }),
        }
    }

    pub fn cleanup_expired_assets(&self, max_age: Duration) {
        let mut g = self.inner.lock().unwrap();
        let expired_ids: Vec<String> = g
            .assets
            .iter()
            .filter_map(|(id, asset)| {
                if asset.created_at.elapsed() > max_age {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        for id in expired_ids {
            if let Some(asset) = g.assets.remove(&id) {
                let _ = std::fs::remove_file(&asset.output_path);
            }
        }
    }

    pub fn take_asset(&self, asset_id: &str) -> Option<RecordedAsset> {
        let mut g = self.inner.lock().unwrap();
        g.assets.remove(asset_id)
    }

    pub fn start_recording(
        &self,
        mailbox: &UiEventMailbox,
        record_input_cache: &RecordInputCacheState,
        task_id: Option<String>,
    ) -> Result<String, CaptureError> {
        let dir =
            data_dir::data_dir().map_err(|e| CaptureError::new("E_DATA_DIR", e.to_string()))?;
        let span = obs::Span::start(
            &dir,
            task_id.as_deref(),
            "Cmd",
            "CMD.record_transcribe_start",
            None,
        );
        if !cfg!(windows) {
            let err = CaptureError::new(
                "E_RECORD_UNSUPPORTED",
                "backend recording is only supported on Windows",
            );
            span.err("config", &err.code, &err.render(), None);
            return Err(err);
        }
        self.cleanup_expired_assets(Duration::from_secs(120));
        {
            let g = self.inner.lock().unwrap();
            if g.active.is_some() {
                let err =
                    CaptureError::new("E_RECORD_ALREADY_ACTIVE", "recording is already active");
                span.err("task", &err.code, &err.render(), None);
                return Err(err);
            }
        }

        let root = repo_root()?;
        let tmp = root.join("tmp").join("desktop");
        std::fs::create_dir_all(&tmp)
            .map_err(|e| CaptureError::new("E_RECORD_TMP_CREATE", e.to_string()))?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let output_path = tmp.join(format!("recording-{session_id}.wav"));
        let cached_input = match record_input_cache.get_last_ok() {
            Some(v) => v,
            None => {
                let snapshot = record_input_cache.snapshot();
                let message = "record input cache is not ready; wait for cache refresh and retry";
                span.err(
                    "config",
                    "E_RECORD_INPUT_CACHE_NOT_READY",
                    message,
                    Some(serde_json::json!({
                        "refresh_in_progress": snapshot.refresh_in_progress,
                        "pending_reason": snapshot.pending_reason,
                        "last_error": snapshot.last_error.as_ref().map(|v| serde_json::json!({
                            "code": v.code,
                            "message": v.message,
                            "ts_ms": v.ts_ms,
                            "reason": v.reason,
                        })),
                    })),
                );
                return Err(CaptureError::new("E_RECORD_INPUT_CACHE_NOT_READY", message));
            }
        };
        let resolved_input = cached_input.resolved.clone();
        let input_spec = resolved_input.spec.clone();
        let ffmpeg = pipeline::ffmpeg_cmd()
            .map_err(|e| CaptureError::new("E_FFMPEG_NOT_FOUND", e.to_string()))?;

        let mut child = match std::process::Command::new(&ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "dshow",
                "-i",
                input_spec.as_str(),
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(output_path.as_os_str())
            .args(["-f", "s16le", "pipe:1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .no_console()
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                let err = CaptureError::new(
                    "E_RECORD_START_FAILED",
                    format!("failed to start ffmpeg recorder: {e}"),
                );
                span.err("process", &err.code, &err.render(), None);
                return Err(err);
            }
        };

        let stdout = match child.stdout.take() {
            Some(v) => v,
            None => {
                let err =
                    CaptureError::new("E_RECORD_START_FAILED", "recorder stdout not available");
                span.err("process", &err.code, &err.render(), None);
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&output_path);
                return Err(err);
            }
        };
        let meter_join = spawn_meter_thread(mailbox.clone(), session_id.clone(), stdout);

        std::thread::sleep(Duration::from_millis(120));
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr_tail = child.stderr.as_mut().and_then(read_last_stderr_line);
                let mut message = if status.success() {
                    "recorder exited unexpectedly right after start".to_string()
                } else {
                    format!("recorder exited right after start with {status}")
                };
                if let Some(line) = stderr_tail.as_deref() {
                    message.push_str("; stderr=");
                    message.push_str(line);
                }
                let err = CaptureError::new("E_RECORD_START_FAILED", message);
                span.err("process", &err.code, &err.render(), None);
                let _ = std::fs::remove_file(&output_path);
                let _ = meter_join.join();
                return Err(err);
            }
            Ok(None) => {}
            Err(e) => {
                let err = CaptureError::new(
                    "E_RECORD_START_FAILED",
                    format!("failed to probe recorder process: {e}"),
                );
                span.err("process", &err.code, &err.render(), None);
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&output_path);
                let _ = meter_join.join();
                return Err(err);
            }
        }

        {
            let mut g = self.inner.lock().unwrap();
            g.active = Some(ActiveRecording {
                session_id: session_id.clone(),
                task_id,
                output_path: output_path.clone(),
                child: Some(child),
                started_at: Instant::now(),
                meter_join: Some(meter_join),
            });
        }
        span.ok(Some(serde_json::json!({
            "session_id": session_id,
            "output_path": output_path,
            "record_input_spec": input_spec,
            "record_input_strategy": resolved_input.strategy_used,
            "record_input_resolved_by": resolved_input.resolved_by,
            "record_input_endpoint_id": resolved_input.endpoint_id,
            "record_input_friendly_name": resolved_input.friendly_name,
            "record_input_resolution_log": resolved_input.resolution_log,
            "record_input_cache_reason": cached_input.reason,
            "record_input_cache_refreshed_ts_ms": cached_input.refreshed_at_ms,
        })));
        Ok(session_id)
    }

    pub fn stop_recording(&self, session_id: &str) -> Result<RecordedAsset, CaptureError> {
        let dir =
            data_dir::data_dir().map_err(|e| CaptureError::new("E_DATA_DIR", e.to_string()))?;
        let span = obs::Span::start(
            &dir,
            None,
            "Cmd",
            "CMD.record_transcribe_stop.capture",
            Some(serde_json::json!({"has_session_id": !session_id.trim().is_empty()})),
        );
        self.cleanup_expired_assets(Duration::from_secs(120));
        let mut active = {
            let mut g = self.inner.lock().unwrap();
            match g.active.take() {
                Some(active) => active,
                None => {
                    let err = CaptureError::new("E_RECORD_NOT_ACTIVE", "no active recording");
                    span.err("task", &err.code, &err.render(), None);
                    return Err(err);
                }
            }
        };

        if !session_id.trim().is_empty() && active.session_id != session_id {
            let mut g = self.inner.lock().unwrap();
            g.active = Some(active);
            let err = CaptureError::new("E_RECORD_ID_MISMATCH", "recording id mismatch");
            span.err("task", &err.code, &err.render(), None);
            return Err(err);
        }

        let child = active
            .child
            .as_mut()
            .ok_or_else(|| CaptureError::new("E_RECORD_STOP_FAILED", "recorder process missing"))?;
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = std::io::Write::write_all(stdin, b"q\n");
            let _ = std::io::Write::flush(stdin);
        }

        let mut status = None;
        for _ in 0..100 {
            match child.try_wait() {
                Ok(Some(s)) => {
                    status = Some(s);
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        if status.is_none() {
            let _ = child.kill();
            status = child.wait().ok();
        }
        let status = match status {
            Some(s) => s,
            None => {
                let stderr_tail = child.stderr.as_mut().and_then(read_last_stderr_line);
                let mut message = "recorder process wait failed".to_string();
                if let Some(line) = stderr_tail.as_deref() {
                    message.push_str("; stderr=");
                    message.push_str(line);
                }
                join_meter_thread(&mut active);
                let err = CaptureError::new("E_RECORD_STOP_FAILED", message);
                span.err("process", &err.code, &err.render(), None);
                return Err(err);
            }
        };
        let stderr_tail = child.stderr.as_mut().and_then(read_last_stderr_line);
        if !status.success() {
            let mut message = format!("recorder exited with {status}");
            if let Some(line) = stderr_tail.as_deref() {
                message.push_str("; stderr=");
                message.push_str(line);
            }
            join_meter_thread(&mut active);
            let _ = std::fs::remove_file(&active.output_path);
            let err = CaptureError::new("E_RECORD_STOP_FAILED", message);
            span.err("process", &err.code, &err.render(), None);
            return Err(err);
        }

        if !active.output_path.exists() {
            join_meter_thread(&mut active);
            let err = CaptureError::new("E_RECORD_OUTPUT_MISSING", "recorded file missing");
            span.err("io", &err.code, &err.render(), None);
            return Err(err);
        }
        join_meter_thread(&mut active);

        let elapsed_ms = active.started_at.elapsed().as_millis();
        let asset = self.complete_session(
            active.session_id.clone(),
            active.task_id.clone(),
            active.output_path.clone(),
            elapsed_ms,
        );
        span.ok(Some(serde_json::json!({
            "session_id": active.session_id,
            "recording_asset_id": asset.asset_id,
            "record_elapsed_ms": elapsed_ms,
        })));
        Ok(asset)
    }

    pub fn abort_recording(&self, session_id: Option<String>) -> Result<(), CaptureError> {
        let dir =
            data_dir::data_dir().map_err(|e| CaptureError::new("E_DATA_DIR", e.to_string()))?;
        let span = obs::Span::start(
            &dir,
            None,
            "Cmd",
            "CMD.record_transcribe_cancel.capture",
            Some(serde_json::json!({
                "has_session_id": session_id.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false),
            })),
        );
        let mut active = {
            let mut g = self.inner.lock().unwrap();
            match g.active.take() {
                Some(v) => v,
                None => {
                    span.ok(Some(serde_json::json!({"aborted": false})));
                    return Ok(());
                }
            }
        };
        if let Some(expected) = session_id {
            if !expected.trim().is_empty() && active.session_id != expected {
                let mut g = self.inner.lock().unwrap();
                g.active = Some(active);
                let err = CaptureError::new("E_RECORD_ID_MISMATCH", "recording id mismatch");
                span.err("task", &err.code, &err.render(), None);
                return Err(err);
            }
        }
        if let Some(child) = active.child.as_mut() {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = std::io::Write::write_all(stdin, b"q\n");
                let _ = std::io::Write::flush(stdin);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        join_meter_thread(&mut active);
        let _ = std::fs::remove_file(&active.output_path);
        span.ok(Some(serde_json::json!({"aborted": true})));
        Ok(())
    }

    fn complete_session(
        &self,
        _session_id: String,
        task_id: Option<String>,
        output_path: PathBuf,
        record_elapsed_ms: u128,
    ) -> RecordedAsset {
        let asset_id = uuid::Uuid::new_v4().to_string();
        let asset = RecordedAsset {
            asset_id: asset_id.clone(),
            task_id,
            output_path,
            record_elapsed_ms,
            created_at: Instant::now(),
        };
        let mut g = self.inner.lock().unwrap();
        g.assets.insert(asset_id, asset.clone());
        asset
    }

    #[cfg(test)]
    fn open_test_session(&self, session_id: &str) -> Result<(), CaptureError> {
        let mut g = self.inner.lock().unwrap();
        if g.active.is_some() {
            return Err(CaptureError::new(
                "E_RECORD_ALREADY_ACTIVE",
                "recording is already active",
            ));
        }
        g.active = Some(ActiveRecording {
            session_id: session_id.to_string(),
            task_id: None,
            output_path: PathBuf::new(),
            child: None,
            started_at: Instant::now(),
            meter_join: None,
        });
        Ok(())
    }

    #[cfg(test)]
    fn complete_test_session(
        &self,
        session_id: &str,
        output_path: PathBuf,
        record_elapsed_ms: u128,
    ) -> Result<RecordedAsset, CaptureError> {
        let active = {
            let mut g = self.inner.lock().unwrap();
            g.active.take()
        }
        .ok_or_else(|| CaptureError::new("E_RECORD_NOT_ACTIVE", "no active recording"))?;
        if active.session_id != session_id {
            return Err(CaptureError::new(
                "E_RECORD_ID_MISMATCH",
                "recording id mismatch",
            ));
        }
        Ok(self.complete_session(session_id.to_string(), None, output_path, record_elapsed_ms))
    }
}

fn spawn_meter_thread(
    mailbox: UiEventMailbox,
    recording_id: String,
    mut stdout: ChildStdout,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        const WINDOW_SAMPLES: usize = 800;
        let mut read_buf = [0_u8; 4096];
        let mut carry_low_byte: Option<u8> = None;
        let mut sum_sq = 0.0_f64;
        let mut max_abs = 0_i32;
        let mut sample_count = 0_usize;

        loop {
            let n = match stdout.read(&mut read_buf) {
                Ok(0) => break,
                Ok(v) => v,
                Err(_) => break,
            };

            let mut idx = 0_usize;
            if let Some(low) = carry_low_byte.take() {
                if n > 0 {
                    let sample = i16::from_le_bytes([low, read_buf[0]]);
                    accumulate_sample(
                        sample,
                        &mut sum_sq,
                        &mut max_abs,
                        &mut sample_count,
                        WINDOW_SAMPLES,
                        &mailbox,
                        &recording_id,
                    );
                    idx = 1;
                }
            }

            while idx + 1 < n {
                let sample = i16::from_le_bytes([read_buf[idx], read_buf[idx + 1]]);
                accumulate_sample(
                    sample,
                    &mut sum_sq,
                    &mut max_abs,
                    &mut sample_count,
                    WINDOW_SAMPLES,
                    &mailbox,
                    &recording_id,
                );
                idx += 2;
            }

            if idx < n {
                carry_low_byte = Some(read_buf[idx]);
            }
        }

        mailbox.send(UiEvent::audio_level(recording_id, 0.0, 0.0));
    })
}

fn accumulate_sample(
    sample: i16,
    sum_sq: &mut f64,
    max_abs: &mut i32,
    sample_count: &mut usize,
    window_samples: usize,
    mailbox: &UiEventMailbox,
    recording_id: &str,
) {
    let sample_i32 = i32::from(sample);
    let normalized = f64::from(sample_i32) / 32768.0;
    *sum_sq += normalized * normalized;
    *max_abs = (*max_abs).max(sample_i32.abs());
    *sample_count += 1;
    if *sample_count >= window_samples {
        let rms = (*sum_sq / *sample_count as f64).sqrt();
        let peak = *max_abs as f64 / 32768.0;
        mailbox.send(UiEvent::audio_level(recording_id.to_string(), rms, peak));
        *sum_sq = 0.0;
        *max_abs = 0;
        *sample_count = 0;
    }
}

fn join_meter_thread(active: &mut ActiveRecording) {
    if let Some(join_handle) = active.meter_join.take() {
        let _ = join_handle.join();
    }
}

fn read_last_stderr_line(stderr: &mut ChildStderr) -> Option<String> {
    let mut buf = String::new();
    if stderr.read_to_string(&mut buf).is_err() {
        return None;
    }
    buf.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn repo_root() -> Result<PathBuf, CaptureError> {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf())
        .ok_or_else(|| CaptureError::new("E_REPO_ROOT", "repo root not found"))
}

impl Default for RecordingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_allows_only_one_active_recording() {
        let registry = RecordingRegistry::new();

        let first = registry.open_test_session("session-1");
        assert!(first.is_ok());

        let second = registry
            .open_test_session("session-2")
            .expect_err("second session fails");
        assert_eq!(second.code, "E_RECORD_ALREADY_ACTIVE");
    }

    #[test]
    fn stopped_session_becomes_consumable_asset_once() {
        let registry = RecordingRegistry::new();
        registry.open_test_session("session-1").expect("open");

        let asset = registry
            .complete_test_session("session-1", std::path::PathBuf::from("sample.wav"), 20)
            .expect("complete");

        assert_eq!(asset.record_elapsed_ms, 20);
        assert!(registry.take_asset(&asset.asset_id).is_some());
        assert!(registry.take_asset(&asset.asset_id).is_none());
    }
}
