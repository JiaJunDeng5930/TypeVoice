use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::obs::{metrics, schema::MetricsRecord};
use crate::ports::{PortError, PortResult};
use crate::{data_dir, pipeline, remote_asr, settings};

#[cfg(windows)]
use crate::subprocess::CommandNoConsoleExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Remote,
    Doubao,
}

#[derive(Debug, Clone, Copy)]
enum MetricStageStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

impl MetricStageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl ProviderKind {
    pub fn from_settings_value(raw: &str) -> Self {
        if raw.trim().eq_ignore_ascii_case("remote") {
            Self::Remote
        } else {
            Self::Doubao
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Doubao => "doubao",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionMetrics {
    pub rtf: f64,
    pub device_used: String,
    pub preprocess_ms: u128,
    pub asr_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResult {
    pub transcript_id: String,
    pub asr_text: String,
    pub final_text: String,
    pub metrics: TranscriptionMetrics,
    pub history_id: String,
}

impl TranscriptionResult {
    pub fn new(
        transcript_id: impl Into<String>,
        asr_text: impl Into<String>,
        metrics: TranscriptionMetrics,
    ) -> Self {
        let transcript_id = transcript_id.into();
        let asr_text = asr_text.into();
        Self {
            transcript_id: transcript_id.clone(),
            asr_text: asr_text.clone(),
            final_text: asr_text,
            metrics,
            history_id: transcript_id,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeFixtureRequest {
    pub fixture_name: String,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TranscriptionInput {
    pub task_id: Option<String>,
    pub input_path: PathBuf,
    pub record_elapsed_ms: u128,
    pub record_label: String,
}

#[derive(Debug, Clone)]
struct TranscriptionOptions {
    provider: ProviderKind,
    remote_url: String,
    remote_model: Option<String>,
    remote_concurrency: usize,
    preprocess: pipeline::PreprocessConfig,
}

#[derive(Clone)]
struct ActiveTranscription {
    task_id: String,
    token: CancellationToken,
    ffmpeg_pid: Arc<Mutex<Option<u32>>>,
    stale: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct TranscriptionService {
    inner: Arc<Mutex<Option<ActiveTranscription>>>,
}

impl TranscriptionService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn cancel(&self, task_id: Option<&str>) -> PortResult<()> {
        let active = {
            let g = self.inner.lock().unwrap();
            let Some(active) = g.as_ref().cloned() else {
                return Ok(());
            };
            if let Some(expected) = task_id {
                if !expected.trim().is_empty() && active.task_id != expected {
                    return Ok(());
                }
            }
            active
        };
        cancel_active_transcription(&active, false);
        Ok(())
    }

    pub async fn transcribe_fixture(
        &self,
        req: TranscribeFixtureRequest,
    ) -> PortResult<TranscriptionResult> {
        let input_path = pipeline::fixture_path(&req.fixture_name)
            .map_err(|e| PortError::from_message("E_FIXTURE_NOT_FOUND", e.to_string()))?;
        self.transcribe_audio(TranscriptionInput {
            task_id: req.task_id,
            input_path,
            record_elapsed_ms: 0,
            record_label: "Record (fixture)".to_string(),
        })
        .await
    }

    pub async fn transcribe_audio(
        &self,
        input: TranscriptionInput,
    ) -> PortResult<TranscriptionResult> {
        let data_dir = data_dir::data_dir()
            .map_err(|e| PortError::from_message("E_DATA_DIR", e.to_string()))?;
        let opts = TranscriptionOptions::from_settings(&data_dir)?;
        let task_id = input
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.replace_active_task(task_id.clone());

        let result = self
            .transcribe_audio_inner(&data_dir, task_id.clone(), input, opts)
            .await;
        if !self.clear_active(&task_id) {
            return Err(PortError::new("E_TASK_STALE", "stale transcription task"));
        }
        result
    }

    async fn transcribe_audio_inner(
        &self,
        data_dir: &Path,
        task_id: String,
        input: TranscriptionInput,
        opts: TranscriptionOptions,
    ) -> PortResult<TranscriptionResult> {
        emit_stage_metric(
            data_dir,
            &task_id,
            "Record",
            MetricStageStatus::Completed,
            input.record_label.clone(),
            Some(input.record_elapsed_ms),
            None,
        );
        if self.is_cancelled(&task_id) {
            emit_stage_metric(
                data_dir,
                &task_id,
                "Record",
                MetricStageStatus::Cancelled,
                "cancelled",
                None,
                Some("E_CANCELLED"),
            );
            return Err(PortError::new("E_CANCELLED", "cancelled"));
        }

        emit_stage_metric(
            data_dir,
            &task_id,
            "Preprocess",
            MetricStageStatus::Started,
            if opts.preprocess.silence_trim_enabled {
                "ffmpeg (silence_trim)"
            } else {
                "ffmpeg"
            },
            None,
            None,
        );
        let wav_path = pipeline::preprocess_to_temp_wav(&task_id, &input.input_path)
            .map_err(|e| PortError::from_message("E_PREPROCESS_FAILED", e.to_string()))?;
        let preprocess_ms = match self
            .run_preprocess(
                data_dir,
                &task_id,
                &input.input_path,
                &wav_path,
                &opts.preprocess,
            )
            .await
        {
            Ok(ms) => ms,
            Err(e) => {
                let _ = pipeline::cleanup_audio_artifacts(&input.input_path, &wav_path);
                emit_stage_metric(
                    data_dir,
                    &task_id,
                    "Preprocess",
                    if e.code == "E_CANCELLED" {
                        MetricStageStatus::Cancelled
                    } else {
                        MetricStageStatus::Failed
                    },
                    e.message.clone(),
                    None,
                    Some(&e.code),
                );
                return Err(e);
            }
        };
        emit_stage_metric(
            data_dir,
            &task_id,
            "Preprocess",
            MetricStageStatus::Completed,
            "ok",
            Some(preprocess_ms),
            None,
        );

        emit_stage_metric(
            data_dir,
            &task_id,
            "Transcribe",
            MetricStageStatus::Started,
            format!("asr({})", opts.provider.as_str()),
            None,
            None,
        );
        let transcript = match self
            .run_transcriber(data_dir, &task_id, &wav_path, &opts)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                let _ = pipeline::cleanup_audio_artifacts(&input.input_path, &wav_path);
                emit_stage_metric(
                    data_dir,
                    &task_id,
                    "Transcribe",
                    if e.code == "E_CANCELLED" {
                        MetricStageStatus::Cancelled
                    } else {
                        MetricStageStatus::Failed
                    },
                    e.message.clone(),
                    None,
                    Some(&e.code),
                );
                return Err(e);
            }
        };
        let _ = pipeline::cleanup_audio_artifacts(&input.input_path, &wav_path);
        emit_stage_metric(
            data_dir,
            &task_id,
            "Transcribe",
            MetricStageStatus::Completed,
            format!("rtf={:.3}", transcript.rtf),
            Some(transcript.asr_ms),
            None,
        );

        let metrics = TranscriptionMetrics {
            rtf: transcript.rtf,
            device_used: transcript.device_used.clone(),
            preprocess_ms,
            asr_ms: transcript.asr_ms,
        };
        let result = TranscriptionResult::new(&task_id, transcript.text.clone(), metrics);
        emit_perf_metrics(
            data_dir,
            &task_id,
            opts.provider,
            &opts.preprocess,
            preprocess_ms,
            &transcript,
        );
        Ok(result)
    }

    async fn run_preprocess(
        &self,
        data_dir: &Path,
        task_id: &str,
        input_path: &Path,
        wav_path: &Path,
        cfg: &pipeline::PreprocessConfig,
    ) -> PortResult<u128> {
        let active = self.active_for_task(task_id)?;
        let data_dir = data_dir.to_path_buf();
        let task_id = task_id.to_string();
        let input_path = input_path.to_path_buf();
        let wav_path = wav_path.to_path_buf();
        let cfg = cfg.clone();
        let join = tokio::task::spawn_blocking(move || {
            pipeline::preprocess_ffmpeg_cancellable(
                &data_dir,
                &task_id,
                &input_path,
                &wav_path,
                &active.token,
                &active.ffmpeg_pid,
                &cfg,
            )
        })
        .await;
        match join {
            Ok(Ok(ms)) => Ok(ms),
            Ok(Err(e)) => {
                let message = e.to_string();
                if message.contains("cancelled") {
                    if active.stale.load(Ordering::SeqCst) {
                        Err(PortError::new("E_TASK_STALE", "stale transcription task"))
                    } else {
                        Err(PortError::new("E_CANCELLED", "cancelled"))
                    }
                } else {
                    Err(PortError::from_message("E_PREPROCESS_FAILED", message))
                }
            }
            Err(e) => Err(PortError::new(
                "E_INTERNAL",
                format!("preprocess_join_failed:{e}"),
            )),
        }
    }

    async fn run_transcriber(
        &self,
        data_dir: &Path,
        task_id: &str,
        wav_path: &Path,
        opts: &TranscriptionOptions,
    ) -> PortResult<ProviderTranscript> {
        if opts.provider == ProviderKind::Remote {
            self.run_remote_transcriber(data_dir, task_id, wav_path, opts)
                .await
        } else {
            Err(PortError::new(
                "E_DOUBAO_FIXTURE_UNSUPPORTED",
                "doubao transcription is available through streaming recording",
            ))
        }
    }

    async fn run_remote_transcriber(
        &self,
        data_dir: &Path,
        task_id: &str,
        wav_path: &Path,
        opts: &TranscriptionOptions,
    ) -> PortResult<ProviderTranscript> {
        let active = self.active_for_task(task_id)?;
        let cfg = remote_asr::RemoteAsrConfig {
            url: opts.remote_url.clone(),
            model: opts.remote_model.clone(),
            concurrency: opts.remote_concurrency,
        };
        match remote_asr::transcribe_remote(data_dir, task_id, wav_path, &active.token, &cfg).await
        {
            Ok(v) => Ok(ProviderTranscript {
                text: v.text,
                rtf: v.metrics.rtf,
                device_used: "remote".to_string(),
                asr_ms: v.metrics.elapsed_ms.max(0) as u128,
                provider_elapsed_ms: v.metrics.elapsed_ms,
                audio_seconds: v.metrics.audio_seconds,
                model_id: v.metrics.model_id,
                model_version: v.metrics.model_version,
                remote_slice_count: Some(v.metrics.slice_count),
                remote_concurrency_used: Some(v.metrics.concurrency_used),
            }),
            Err(e) if e.code == "E_CANCELLED" => {
                if active.stale.load(Ordering::SeqCst) {
                    Err(PortError::new("E_TASK_STALE", "stale transcription task"))
                } else {
                    Err(PortError::new("E_CANCELLED", e.message))
                }
            }
            Err(e) => Err(PortError::new(&e.code, e.message)),
        }
    }

    fn active_for_task(&self, task_id: &str) -> PortResult<ActiveTranscription> {
        let g = self.inner.lock().unwrap();
        let active = g
            .as_ref()
            .cloned()
            .ok_or_else(|| PortError::new("E_TASK_STALE", "stale transcription task"))?;
        if active.task_id != task_id {
            return Err(PortError::new("E_TASK_STALE", "stale transcription task"));
        }
        Ok(active)
    }

    fn is_cancelled(&self, task_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .as_ref()
            .filter(|active| active.task_id == task_id)
            .map(|active| active.token.is_cancelled())
            .unwrap_or(false)
    }

    fn clear_active(&self, task_id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.as_ref().map(|active| active.task_id.as_str()) == Some(task_id) {
            *g = None;
            return true;
        }
        false
    }

    fn replace_active_task(&self, task_id: String) -> ActiveTranscription {
        let active = ActiveTranscription::new(task_id);
        let stale = {
            let mut g = self.inner.lock().unwrap();
            std::mem::replace(&mut *g, Some(active.clone()))
        };
        if let Some(stale) = stale {
            cancel_active_transcription(&stale, true);
        }
        active
    }
}

impl ActiveTranscription {
    fn new(task_id: String) -> Self {
        Self {
            task_id,
            token: CancellationToken::new(),
            ffmpeg_pid: Arc::new(Mutex::new(None)),
            stale: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn cancel_active_transcription(active: &ActiveTranscription, stale: bool) {
    if stale {
        active.stale.store(true, Ordering::SeqCst);
    }
    active.token.cancel();
    if let Some(pid) = *active.ffmpeg_pid.lock().unwrap() {
        let _ = kill_pid(pid);
    }
}

impl Default for TranscriptionService {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptionOptions {
    fn from_settings(data_dir: &Path) -> PortResult<Self> {
        let s = settings::load_settings_strict(data_dir)
            .map_err(|e| PortError::from_message("E_SETTINGS_INVALID", e.to_string()))?;
        Ok(Self {
            provider: ProviderKind::from_settings_value(&settings::resolve_asr_provider(&s)),
            remote_url: settings::resolve_remote_asr_url(&s),
            remote_model: settings::resolve_remote_asr_model(&s),
            remote_concurrency: settings::resolve_remote_asr_concurrency(&s),
            preprocess: resolve_asr_preprocess_config(&s),
        })
    }
}

#[derive(Debug, Clone)]
struct ProviderTranscript {
    text: String,
    rtf: f64,
    device_used: String,
    asr_ms: u128,
    provider_elapsed_ms: i64,
    audio_seconds: f64,
    model_id: String,
    model_version: Option<String>,
    remote_slice_count: Option<usize>,
    remote_concurrency_used: Option<usize>,
}

fn resolve_asr_preprocess_config(s: &settings::Settings) -> pipeline::PreprocessConfig {
    let mut cfg = pipeline::PreprocessConfig::default();
    if let Some(v) = s.asr_preprocess_silence_trim_enabled {
        cfg.silence_trim_enabled = v;
    }
    if let Some(v) = s.asr_preprocess_silence_threshold_db {
        cfg.silence_threshold_db = v;
    }
    if let Some(v) = s.asr_preprocess_silence_start_ms {
        cfg.silence_trim_start_ms = v;
    }
    if let Some(v) = s.asr_preprocess_silence_end_ms {
        cfg.silence_trim_end_ms = v;
    }
    cfg
}

fn emit_stage_metric(
    data_dir: &Path,
    task_id: &str,
    stage: &str,
    status: MetricStageStatus,
    message: impl Into<String>,
    elapsed_ms: Option<u128>,
    error_code: Option<&str>,
) {
    let message = message.into();
    let _ = metrics::emit(
        data_dir,
        MetricsRecord::TaskEvent {
            ts_ms: now_ms(),
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: status.as_str().to_string(),
            elapsed_ms,
            error_code: error_code.map(ToOwned::to_owned),
            message,
        },
    );
}

fn emit_perf_metrics(
    data_dir: &Path,
    task_id: &str,
    provider: ProviderKind,
    preprocess_cfg: &pipeline::PreprocessConfig,
    preprocess_ms: u128,
    transcript: &ProviderTranscript,
) {
    let overhead_ms_u128 = transcript
        .asr_ms
        .saturating_sub(transcript.provider_elapsed_ms.max(0) as u128);
    let _ = metrics::emit(
        data_dir,
        MetricsRecord::TaskDone {
            ts_ms: now_ms(),
            task_id: task_id.to_string(),
            rtf: transcript.rtf,
            device: transcript.device_used.clone(),
        },
    );
    let _ = metrics::emit(
        data_dir,
        MetricsRecord::TaskPerf {
            ts_ms: now_ms(),
            task_id: task_id.to_string(),
            asr_provider: provider.as_str().to_string(),
            audio_seconds: transcript.audio_seconds,
            preprocess_ms,
            asr_roundtrip_ms: transcript.asr_ms,
            asr_provider_elapsed_ms: transcript.provider_elapsed_ms,
            asr_transport_overhead_ms: overhead_ms_u128.min(u64::MAX as u128) as u64,
            rtf: transcript.rtf,
            rewrite_ms: None,
            device_used: transcript.device_used.clone(),
            asr_model_id: transcript.model_id.clone(),
            asr_model_version: transcript.model_version.clone(),
            remote_asr_slice_count: transcript.remote_slice_count,
            remote_asr_concurrency_used: transcript.remote_concurrency_used,
            asr_preprocess_silence_trim_enabled: preprocess_cfg.silence_trim_enabled,
            asr_preprocess_threshold_db: preprocess_cfg.silence_threshold_db,
            asr_preprocess_trim_start_ms: preprocess_cfg.silence_trim_start_ms,
            asr_preprocess_trim_end_ms: preprocess_cfg.silence_trim_end_ms,
        },
    );
}

fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => dur.as_millis() as i64,
        Err(_) => 0,
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) -> anyhow::Result<()> {
    let status = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()?;
    if !status.success() {
        return Err(anyhow::anyhow!("kill exit={status}"));
    }
    Ok(())
}

#[cfg(windows)]
fn kill_pid(pid: u32) -> anyhow::Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .no_console()
        .status()?;
    if !status.success() {
        return Err(anyhow::anyhow!("taskkill exit={status}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_uses_remote_only_when_requested() {
        assert_eq!(
            ProviderKind::from_settings_value("remote"),
            ProviderKind::Remote
        );
        assert_eq!(
            ProviderKind::from_settings_value("REMOTE"),
            ProviderKind::Remote
        );
        assert_eq!(
            ProviderKind::from_settings_value("doubao"),
            ProviderKind::Doubao
        );
        assert_eq!(ProviderKind::from_settings_value(""), ProviderKind::Doubao);
    }

    #[test]
    fn transcription_result_uses_asr_text_as_initial_final_text() {
        let result = TranscriptionResult::new(
            "task-1",
            "hello",
            TranscriptionMetrics {
                rtf: 0.5,
                device_used: "cuda".to_string(),
                preprocess_ms: 10,
                asr_ms: 20,
            },
        );

        assert_eq!(result.transcript_id, "task-1");
        assert_eq!(result.asr_text, "hello");
        assert_eq!(result.final_text, "hello");
    }

    #[test]
    fn new_transcription_replaces_existing_active_task() {
        let service = TranscriptionService::new();

        let first = service.replace_active_task("task-1".to_string());
        let second = service.replace_active_task("task-2".to_string());

        assert!(first.token.is_cancelled());
        assert!(first.stale.load(Ordering::SeqCst));
        assert!(!second.token.is_cancelled());
        assert_eq!(
            service.active_for_task("task-2").expect("current").task_id,
            "task-2"
        );
        let stale = match service.active_for_task("task-1") {
            Ok(_) => panic!("old task should be stale"),
            Err(err) => err,
        };
        assert_eq!(stale.code, "E_TASK_STALE");
    }

    #[test]
    fn stale_or_missing_cancel_succeeds_without_affecting_current_task() {
        let service = TranscriptionService::new();

        service.cancel(Some("missing")).expect("missing cancel");
        let active = service.replace_active_task("task-2".to_string());

        service.cancel(Some("task-1")).expect("stale cancel");
        assert!(!active.token.is_cancelled());
        assert!(!active.stale.load(Ordering::SeqCst));
    }

    #[test]
    fn current_cancel_cancels_current_task_without_marking_stale() {
        let service = TranscriptionService::new();
        let active = service.replace_active_task("task-1".to_string());

        service.cancel(Some("task-1")).expect("current cancel");

        assert!(active.token.is_cancelled());
        assert!(!active.stale.load(Ordering::SeqCst));
    }
}
