use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdout, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::pcm::pcm_peak_abs;
use crate::record_input_cache::CachedRecordInput;
use crate::subprocess::CommandNoConsoleExt;
use crate::transcription_actor::{StreamingSessionConfig, TranscriptionActor};
use crate::ui_events::{UiEvent, UiEventMailbox};
use crate::{data_dir, obs, pipeline};

const STREAMING_FIRST_AUDIO_SEQUENCE: u64 = 2;

fn ffmpeg_record_args(input_spec: &str, output_path: &Path) -> Vec<std::ffi::OsString> {
    let input_backend = if cfg!(target_os = "macos") {
        "avfoundation"
    } else {
        "dshow"
    };
    [
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        input_backend,
        "-i",
        input_spec,
        "-ac",
        "1",
        "-ar",
        "16000",
        "-c:a",
        "pcm_s16le",
    ]
    .into_iter()
    .map(std::ffi::OsString::from)
    .chain(std::iter::once(output_path.as_os_str().to_os_string()))
    .chain(
        [
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            "-f",
            "s16le",
            "pipe:1",
        ]
        .into_iter()
        .map(std::ffi::OsString::from),
    )
    .collect()
}

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
    finish_on_eof: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct RecordedAsset {
    pub asset_id: String,
    pub task_id: Option<String>,
    pub output_path: PathBuf,
    pub record_elapsed_ms: u128,
    created_at: Instant,
}

#[derive(Debug, Clone)]
pub enum RecordingStopOutcome {
    Completed(RecordedAsset),
    Stale,
}

struct RegistryInner {
    active: Option<ActiveRecording>,
    assets: HashMap<String, RecordedAsset>,
}

#[derive(Clone)]
pub struct RecordingRegistry {
    inner: Arc<Mutex<RegistryInner>>,
}

impl RecordingRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RegistryInner {
                active: None,
                assets: HashMap::new(),
            })),
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
            let removed = g
                .assets
                .get(&id)
                .is_some_and(|asset| remove_recording_file(&asset.output_path).is_ok());
            if removed {
                g.assets.remove(&id);
            }
        }
    }

    pub fn take_asset(&self, asset_id: &str) -> Option<RecordedAsset> {
        let mut g = self.inner.lock().unwrap();
        g.assets.remove(asset_id)
    }

    pub fn take_assets_for_task(&self, task_id: &str) -> Vec<RecordedAsset> {
        let mut g = self.inner.lock().unwrap();
        let asset_ids = g
            .assets
            .iter()
            .filter(|(_, asset)| asset.task_id.as_deref() == Some(task_id))
            .map(|(asset_id, _)| asset_id.clone())
            .collect::<Vec<_>>();
        asset_ids
            .into_iter()
            .filter_map(|asset_id| g.assets.remove(&asset_id))
            .collect()
    }

    pub fn restore_asset(&self, asset: RecordedAsset) {
        self.inner
            .lock()
            .unwrap()
            .assets
            .insert(asset.asset_id.clone(), asset);
    }

    pub fn start_recording(
        &self,
        mailbox: &UiEventMailbox,
        transcriber: Option<&TranscriptionActor>,
        streaming_config: Option<StreamingSessionConfig>,
        cached_input: &CachedRecordInput,
        task_id: Option<String>,
        cancellation: &CancellationToken,
    ) -> Result<String, CaptureError> {
        if cancellation.is_cancelled() {
            return Err(CaptureError::new(
                "E_CANCELLED",
                "recording start was cancelled",
            ));
        }
        let dir =
            data_dir::data_dir().map_err(|e| CaptureError::new("E_DATA_DIR", e.to_string()))?;
        let span = obs::Span::start(
            &dir,
            task_id.as_deref(),
            "Recording",
            "run.recording_begin",
            None,
        );
        if !cfg!(any(windows, target_os = "macos")) {
            let err = CaptureError::new(
                "E_RECORD_UNSUPPORTED",
                "backend recording is unsupported on this platform",
            );
            span.err("config", &err.code, &err.render(), None);
            return Err(err);
        }
        self.cleanup_expired_assets(Duration::from_secs(120));
        if self.inner.lock().unwrap().active.is_some() {
            let err = CaptureError::new(
                "E_RECORD_ALREADY_ACTIVE",
                "a recording process is still owned by the previous run",
            );
            span.err("state", &err.code, &err.render(), None);
            return Err(err);
        }

        let tmp = recording_tmp_dir(&dir);
        std::fs::create_dir_all(&tmp)
            .map_err(|e| CaptureError::new("E_RECORD_TMP_CREATE", e.to_string()))?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let output_path = tmp.join(format!("recording-{session_id}.wav"));
        let resolved_input = cached_input.resolved.clone();
        let input_spec = resolved_input.spec.clone();
        let ffmpeg = pipeline::ffmpeg_cmd()
            .map_err(|e| CaptureError::new("E_FFMPEG_NOT_FOUND", e.to_string()))?;
        if cancellation.is_cancelled() {
            return Err(CaptureError::new(
                "E_CANCELLED",
                "recording start was cancelled",
            ));
        }

        let mut child = match std::process::Command::new(&ffmpeg)
            .args(ffmpeg_record_args(
                input_spec.as_str(),
                output_path.as_path(),
            ))
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

        let finish_on_eof = Arc::new(AtomicBool::new(false));
        let stdout = match child.stdout.take() {
            Some(v) => v,
            None => {
                let err =
                    CaptureError::new("E_RECORD_START_FAILED", "recorder stdout not available");
                span.err("process", &err.code, &err.render(), None);
                self.restore_active(ActiveRecording {
                    session_id,
                    task_id,
                    output_path,
                    child: Some(child),
                    started_at: Instant::now(),
                    meter_join: None,
                    finish_on_eof,
                });
                return Err(err);
            }
        };
        let meter_join = match spawn_meter_thread(
            mailbox.clone(),
            transcriber.cloned(),
            task_id.clone(),
            session_id.clone(),
            stdout,
            streaming_config.map(|config| config.chunk_bytes),
            finish_on_eof.clone(),
        ) {
            Ok(join) => join,
            Err(err) => {
                span.err("process", &err.code, &err.render(), None);
                self.restore_active(ActiveRecording {
                    session_id,
                    task_id,
                    output_path,
                    child: Some(child),
                    started_at: Instant::now(),
                    meter_join: None,
                    finish_on_eof,
                });
                return Err(err);
            }
        };
        let mut active = ActiveRecording {
            session_id: session_id.clone(),
            task_id,
            output_path: output_path.clone(),
            child: Some(child),
            started_at: Instant::now(),
            meter_join: Some(meter_join),
            finish_on_eof,
        };

        for _ in 0..12 {
            if cancellation.is_cancelled() {
                let err = CaptureError::new("E_CANCELLED", "recording start was cancelled");
                span.err("process", &err.code, &err.render(), None);
                self.restore_active(active);
                return Err(err);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let child = active
            .child
            .as_mut()
            .expect("new recording owns its recorder process");
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
                self.restore_active(active);
                return Err(err);
            }
            Ok(None) => {}
            Err(e) => {
                let err = CaptureError::new(
                    "E_RECORD_START_FAILED",
                    format!("failed to probe recorder process: {e}"),
                );
                span.err("process", &err.code, &err.render(), None);
                self.restore_active(active);
                return Err(err);
            }
        }

        self.restore_active(active);
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

    pub fn stop_recording(
        &self,
        session_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<RecordingStopOutcome, CaptureError> {
        let dir =
            data_dir::data_dir().map_err(|e| CaptureError::new("E_DATA_DIR", e.to_string()))?;
        self.cleanup_expired_assets(Duration::from_secs(120));
        let mut active = {
            let mut g = self.inner.lock().unwrap();
            match g.active.take() {
                Some(active) => active,
                None => {
                    let span = obs::Span::start(
                        &dir,
                        None,
                        "Recording",
                        "run.recording_finish",
                        Some(serde_json::json!({
                            "has_session_id": !session_id.trim().is_empty()
                        })),
                    );
                    span.ok(Some(serde_json::json!({"stale": true})));
                    return Ok(RecordingStopOutcome::Stale);
                }
            }
        };
        let span = obs::Span::start(
            &dir,
            active.task_id.as_deref(),
            "Recording",
            "run.recording_finish",
            Some(serde_json::json!({"has_session_id": !session_id.trim().is_empty()})),
        );

        if !session_id.trim().is_empty() && active.session_id != session_id {
            let mut g = self.inner.lock().unwrap();
            g.active = Some(active);
            span.ok(Some(serde_json::json!({"stale": true})));
            return Ok(RecordingStopOutcome::Stale);
        }

        if active.child.is_none() {
            let err = CaptureError::new("E_RECORD_STOP_FAILED", "recorder process missing");
            span.err("process", &err.code, &err.render(), None);
            self.restore_active(active);
            return Err(err);
        }
        active.finish_on_eof.store(true, Ordering::SeqCst);
        let status_result = {
            let child = active
                .child
                .as_mut()
                .expect("recording process checked above");
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = std::io::Write::write_all(stdin, b"q\n");
                let _ = std::io::Write::flush(stdin);
            }

            let mut status = None;
            let mut wait_error = None;
            for _ in 0..100 {
                if cancellation.is_cancelled() {
                    active.finish_on_eof.store(false, Ordering::SeqCst);
                    wait_error = Some(
                        match terminate_child_bounded(child, Duration::from_millis(100)) {
                            Ok(_) => {
                                CaptureError::new("E_CANCELLED", "recording stop was cancelled")
                            }
                            Err(error) => error,
                        },
                    );
                    break;
                }
                match child.try_wait() {
                    Ok(Some(value)) => {
                        status = Some(value);
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(error) => {
                        wait_error = Some(CaptureError::new(
                            "E_RECORD_STOP_FAILED",
                            format!("failed to query recorder process: {error}"),
                        ));
                        break;
                    }
                }
            }
            match (status, wait_error) {
                (Some(status), _) => Ok(status),
                (_, Some(error)) => Err(error),
                (None, None) => terminate_child_bounded(child, Duration::from_millis(100)),
            }
        };
        let status = match status_result {
            Ok(status) => status,
            Err(err) => {
                span.err("process", &err.code, &err.render(), None);
                self.restore_active(active);
                return Err(err);
            }
        };
        let stderr_tail = active
            .child
            .as_mut()
            .and_then(|child| child.stderr.as_mut())
            .and_then(read_last_stderr_line);
        if !status.success() {
            let mut message = format!("recorder exited with {status}");
            if let Some(line) = stderr_tail.as_deref() {
                message.push_str("; stderr=");
                message.push_str(line);
            }
            join_meter_thread(&mut active);
            if let Err(cleanup_error) = remove_recording_file(&active.output_path) {
                span.err("io", &cleanup_error.code, &cleanup_error.render(), None);
                self.restore_active(active);
                return Err(cleanup_error);
            }
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
        Ok(RecordingStopOutcome::Completed(asset))
    }

    pub fn abort_recording(&self, session_id: Option<String>) -> Result<(), CaptureError> {
        let dir =
            data_dir::data_dir().map_err(|e| CaptureError::new("E_DATA_DIR", e.to_string()))?;
        let mut active = {
            let mut g = self.inner.lock().unwrap();
            match g.active.take() {
                Some(v) => v,
                None => {
                    let span = obs::Span::start(
                        &dir,
                        None,
                        "Recording",
                        "run.recording_abort",
                        Some(serde_json::json!({
                            "has_session_id": session_id
                                .as_ref()
                                .map(|s| !s.trim().is_empty())
                                .unwrap_or(false),
                        })),
                    );
                    span.ok(Some(serde_json::json!({"aborted": false})));
                    return Ok(());
                }
            }
        };
        let span = obs::Span::start(
            &dir,
            active.task_id.as_deref(),
            "Recording",
            "run.recording_abort",
            Some(serde_json::json!({
                "has_session_id": session_id.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false),
            })),
        );
        if let Some(expected) = session_id {
            if !expected.trim().is_empty() && active.session_id != expected {
                let mut g = self.inner.lock().unwrap();
                g.active = Some(active);
                span.ok(Some(serde_json::json!({
                    "aborted": false,
                    "stale": true,
                })));
                return Ok(());
            }
        }
        if let Some(child) = active.child.as_mut() {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = std::io::Write::write_all(stdin, b"q\n");
                let _ = std::io::Write::flush(stdin);
            }
            if let Err(err) = terminate_child_bounded(child, Duration::from_millis(100)) {
                span.err("process", &err.code, &err.render(), None);
                self.restore_active(active);
                return Err(err);
            }
        }
        join_meter_thread(&mut active);
        if let Err(err) = remove_recording_file(&active.output_path) {
            span.err("io", &err.code, &err.render(), None);
            self.restore_active(active);
            return Err(err);
        }
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

    fn restore_active(&self, active: ActiveRecording) {
        let mut guard = self.inner.lock().unwrap();
        debug_assert!(guard.active.is_none());
        guard.active = Some(active);
    }

    #[cfg(test)]
    fn open_test_session(&self, session_id: &str) -> Result<(), CaptureError> {
        let mut g = self.inner.lock().unwrap();
        g.active = Some(ActiveRecording {
            session_id: session_id.to_string(),
            task_id: None,
            output_path: PathBuf::new(),
            child: None,
            started_at: Instant::now(),
            meter_join: None,
            finish_on_eof: Arc::new(AtomicBool::new(false)),
        });
        Ok(())
    }

    #[cfg(test)]
    fn active_session_id_for_test(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|active| active.session_id.clone())
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
    transcriber: Option<TranscriptionActor>,
    task_id: Option<String>,
    recording_id: String,
    mut stdout: ChildStdout,
    chunk_bytes: Option<usize>,
    finish_on_eof: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>, CaptureError> {
    std::thread::Builder::new()
        .name(format!("recording_meter_{recording_id}"))
        .spawn(move || {
            const WINDOW_SAMPLES: usize = 800;
            let mut read_buf = [0_u8; 4096];
            let mut chunk = Vec::with_capacity(chunk_bytes.unwrap_or(0).max(1));
            let mut sequence = STREAMING_FIRST_AUDIO_SEQUENCE;
            let mut carry_low_byte: Option<u8> = None;
            let mut levels = LevelAccumulator::default();
            let task_id = task_id.unwrap_or_else(|| recording_id.clone());
            let mut stdout_read_bytes = 0_usize;
            let mut stdout_read_iterations = 0_usize;
            let mut sent_frames = 0_usize;
            let mut sent_bytes = 0_usize;
            let mut non_silent_frames = 0_usize;
            let mut first_sequence: Option<u64> = None;
            let mut last_sequence: Option<u64> = None;
            let mut send_errors = 0_usize;

            loop {
                let n = match stdout.read(&mut read_buf) {
                    Ok(0) => break,
                    Ok(v) => v,
                    Err(_) => break,
                };
                stdout_read_bytes += n;
                stdout_read_iterations += 1;
                if let (Some(transcriber), Some(chunk_bytes)) = (transcriber.as_ref(), chunk_bytes)
                {
                    chunk.extend_from_slice(&read_buf[..n]);
                    while chunk.len() >= chunk_bytes && chunk_bytes > 0 {
                        let rest = chunk.split_off(chunk_bytes);
                        let pcm = std::mem::replace(&mut chunk, rest);
                        sent_frames += 1;
                        sent_bytes += pcm.len();
                        non_silent_frames += usize::from(pcm_peak_abs(&pcm) > 0);
                        first_sequence.get_or_insert(sequence);
                        last_sequence = Some(sequence);
                        if transcriber
                            .send_audio_chunk(&task_id, sequence, pcm, false)
                            .is_err()
                        {
                            send_errors += 1;
                        }
                        sequence += 1;
                    }
                }

                let mut idx = 0_usize;
                if let Some(low) = carry_low_byte.take() {
                    if n > 0 {
                        let sample = i16::from_le_bytes([low, read_buf[0]]);
                        levels.push(sample, WINDOW_SAMPLES, &mailbox, &task_id, &recording_id);
                        idx = 1;
                    }
                }

                while idx + 1 < n {
                    let sample = i16::from_le_bytes([read_buf[idx], read_buf[idx + 1]]);
                    levels.push(sample, WINDOW_SAMPLES, &mailbox, &task_id, &recording_id);
                    idx += 2;
                }

                if idx < n {
                    carry_low_byte = Some(read_buf[idx]);
                }
            }

            if finish_on_eof.load(Ordering::SeqCst) {
                let Some(transcriber) = transcriber.as_ref() else {
                    mailbox.send(UiEvent::audio_level(task_id, recording_id, 0.0, 0.0));
                    return;
                };
                if !chunk.is_empty() {
                    let pcm = std::mem::take(&mut chunk);
                    sent_frames += 1;
                    sent_bytes += pcm.len();
                    non_silent_frames += usize::from(pcm_peak_abs(&pcm) > 0);
                    first_sequence.get_or_insert(sequence);
                    last_sequence = Some(sequence);
                    if transcriber
                        .send_audio_chunk(&task_id, sequence, pcm, true)
                        .is_err()
                    {
                        send_errors += 1;
                    }
                } else {
                    sent_frames += 1;
                    first_sequence.get_or_insert(sequence);
                    last_sequence = Some(sequence);
                    if transcriber
                        .send_audio_chunk(&task_id, sequence, Vec::new(), true)
                        .is_err()
                    {
                        send_errors += 1;
                    }
                }
            }
            if let Ok(dir) = data_dir::data_dir() {
                obs::event(
                    &dir,
                    Some(&task_id),
                    "Transcribe",
                    "ASR.streaming_pcm_source_summary",
                    "ok",
                    Some(serde_json::json!({
                        "recording_id": recording_id,
                        "stdout_read_bytes": stdout_read_bytes,
                        "stdout_read_iterations": stdout_read_iterations,
                        "sent_frames": sent_frames,
                        "sent_bytes": sent_bytes,
                        "non_silent_frames": non_silent_frames,
                        "first_sequence": first_sequence,
                        "last_sequence": last_sequence,
                        "pending_tail_bytes": chunk.len(),
                        "send_errors": send_errors,
                        "finish_on_eof": finish_on_eof.load(Ordering::SeqCst),
                    })),
                );
            }
            mailbox.send(UiEvent::audio_level(task_id, recording_id, 0.0, 0.0));
        })
        .map_err(|error| {
            CaptureError::new(
                "E_RECORD_METER_SPAWN",
                format!("failed to start recording meter: {error}"),
            )
        })
}

#[derive(Default)]
struct LevelAccumulator {
    sum_sq: f64,
    max_abs: i32,
    sample_count: usize,
}

impl LevelAccumulator {
    fn push(
        &mut self,
        sample: i16,
        window_samples: usize,
        mailbox: &UiEventMailbox,
        task_id: &str,
        recording_id: &str,
    ) {
        let sample_i32 = i32::from(sample);
        let normalized = f64::from(sample_i32) / 32768.0;
        self.sum_sq += normalized * normalized;
        self.max_abs = self.max_abs.max(sample_i32.abs());
        self.sample_count += 1;
        if self.sample_count >= window_samples {
            let rms = (self.sum_sq / self.sample_count as f64).sqrt();
            let peak = self.max_abs as f64 / 32768.0;
            mailbox.send(UiEvent::audio_level(
                task_id.to_string(),
                recording_id.to_string(),
                rms,
                peak,
            ));
            *self = Self::default();
        }
    }
}

fn join_meter_thread(active: &mut ActiveRecording) {
    if let Some(join_handle) = active.meter_join.take() {
        let _ = join_handle.join();
    }
}

fn terminate_child_bounded(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, CaptureError> {
    let _ = child.kill();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                return Err(CaptureError::new(
                    "E_RECORD_ABORT_TIMEOUT",
                    "recorder process did not exit before the cleanup deadline",
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                return Err(CaptureError::new(
                    "E_RECORD_ABORT_FAILED",
                    format!("failed to query recorder process: {error}"),
                ));
            }
        }
    }
}

fn remove_recording_file(path: &Path) -> Result<(), CaptureError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CaptureError::new(
            "E_RECORD_CLEANUP_FAILED",
            format!(
                "failed to remove recording artifact {}: {error}",
                path.display()
            ),
        )),
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

fn recording_tmp_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("recordings")
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
    fn registry_replaces_active_recording_resource() {
        let registry = RecordingRegistry::new();

        let first = registry.open_test_session("session-1");
        assert!(first.is_ok());

        registry.open_test_session("session-2").expect("replace");

        assert_eq!(
            registry.active_session_id_for_test().as_deref(),
            Some("session-2")
        );
    }

    #[test]
    fn recording_tmp_dir_uses_runtime_data_dir() {
        let data_dir = PathBuf::from("runtime-data");
        let tmp = recording_tmp_dir(&data_dir);

        assert_eq!(tmp, data_dir.join("recordings"));
    }

    #[test]
    fn stale_stop_preserves_current_recording() {
        let registry = RecordingRegistry::new();
        registry.open_test_session("session-2").expect("open");

        let outcome = registry
            .stop_recording("session-1", &CancellationToken::new())
            .expect("stale stop");

        assert!(matches!(outcome, RecordingStopOutcome::Stale));
        assert_eq!(
            registry.active_session_id_for_test().as_deref(),
            Some("session-2")
        );
    }

    #[test]
    fn stale_abort_preserves_current_recording() {
        let registry = RecordingRegistry::new();
        registry.open_test_session("session-2").expect("open");

        registry
            .abort_recording(Some("session-1".to_string()))
            .expect("stale abort succeeds");

        assert_eq!(
            registry.active_session_id_for_test().as_deref(),
            Some("session-2")
        );
    }

    #[test]
    fn matching_abort_clears_current_recording() {
        let registry = RecordingRegistry::new();
        registry.open_test_session("session-1").expect("open");

        registry
            .abort_recording(Some("session-1".to_string()))
            .expect("matching abort succeeds");

        assert_eq!(registry.active_session_id_for_test(), None);
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

    #[test]
    fn cleanup_can_drain_late_assets_by_run() {
        let registry = RecordingRegistry::new();
        registry.complete_session(
            "session-a".to_string(),
            Some("run-a".to_string()),
            PathBuf::from("a.wav"),
            20,
        );
        registry.complete_session(
            "session-b".to_string(),
            Some("run-b".to_string()),
            PathBuf::from("b.wav"),
            30,
        );

        let drained = registry.take_assets_for_task("run-a");

        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].task_id.as_deref(), Some("run-a"));
        assert!(registry.take_assets_for_task("run-a").is_empty());
        assert_eq!(registry.take_assets_for_task("run-b").len(), 1);
    }

    #[test]
    fn streaming_audio_sequence_starts_after_full_client_request() {
        assert_eq!(STREAMING_FIRST_AUDIO_SEQUENCE, 2);
    }

    #[test]
    fn ffmpeg_record_args_transcodes_file_and_stream_outputs() {
        let args = ffmpeg_record_args(
            "audio=@device_cm_{33D9A762-90C8-11D0-BD43-00A0C911CE86}\\wave_{52B28A7E-31C7-4BB2-AFB4-1529B7F2C7CD}",
            Path::new("sample.wav"),
        )
        .into_iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

        let output_idx = args
            .iter()
            .position(|v| v == "sample.wav")
            .expect("wav output path exists");
        assert_eq!(
            &args[4..8],
            [
                "-f",
                if cfg!(target_os = "macos") {
                    "avfoundation"
                } else {
                    "dshow"
                },
                "-i",
                "audio=@device_cm_{33D9A762-90C8-11D0-BD43-00A0C911CE86}\\wave_{52B28A7E-31C7-4BB2-AFB4-1529B7F2C7CD}",
            ]
        );
        assert_eq!(
            &args[output_idx - 6..output_idx],
            ["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"]
        );
        assert_eq!(
            &args[output_idx + 1..],
            [
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "s16le",
                "pipe:1"
            ]
        );
    }
}
