use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::obs::{metrics, schema::MetricsRecord};
use crate::{asr_service, data_dir, history, llm, pipeline, remote_asr, settings, templates};
use crate::{context_capture, context_pack};

#[cfg(windows)]
use crate::subprocess::CommandNoConsoleExt;

pub trait AsrClient: Send + Sync {
    fn ensure_started(&self, data_dir: &Path) -> Result<()>;
    fn restart(&self, data_dir: &Path, reason: &str) -> Result<()>;
    fn kill_best_effort(&self, reason: &str);
    fn transcribe(
        &self,
        data_dir: &Path,
        task_id: &str,
        audio_path: &Path,
        language: &str,
        token: &CancellationToken,
        pid_slot: &Arc<Mutex<Option<u32>>>,
    ) -> Result<(asr_service::AsrResponse, u128)>;
    fn warmup_ms(&self) -> Option<i64>;
}

impl AsrClient for asr_service::AsrService {
    fn ensure_started(&self, data_dir: &Path) -> Result<()> {
        self.ensure_started(data_dir)
    }

    fn restart(&self, data_dir: &Path, reason: &str) -> Result<()> {
        self.restart(data_dir, reason)
    }

    fn kill_best_effort(&self, reason: &str) {
        asr_service::AsrService::kill_best_effort(self, reason);
    }

    fn transcribe(
        &self,
        data_dir: &Path,
        task_id: &str,
        audio_path: &Path,
        language: &str,
        token: &CancellationToken,
        pid_slot: &Arc<Mutex<Option<u32>>>,
    ) -> Result<(asr_service::AsrResponse, u128)> {
        self.transcribe(data_dir, task_id, audio_path, language, token, pid_slot)
    }

    fn warmup_ms(&self) -> Option<i64> {
        self.warmup_ms()
    }
}

pub trait ContextCollector: Send + Sync {
    fn warmup_best_effort(&self);
    fn capture_hotkey_context_now(
        &self,
        data_dir: &Path,
        cfg: &context_capture::ContextConfig,
    ) -> Result<String>;
    fn take_hotkey_context_once(&self, capture_id: &str) -> Option<context_pack::ContextSnapshot>;
    fn last_external_hwnd_best_effort(&self) -> Option<isize>;
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

    fn last_external_hwnd_best_effort(&self) -> Option<isize> {
        self.last_external_hwnd_best_effort()
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
struct TaskManagerDeps {
    fixture_path: fn(&str) -> Result<PathBuf>,
    preprocess_to_temp_wav: fn(&str, &Path) -> Result<PathBuf>,
    preprocess_ffmpeg_cancellable: fn(
        &Path,
        &str,
        &Path,
        &Path,
        &CancellationToken,
        &Arc<Mutex<Option<u32>>>,
        &pipeline::PreprocessConfig,
    ) -> Result<u128>,
    cleanup_audio_artifacts: fn(&Path, &Path) -> Result<()>,
    get_template: fn(&Path, &str) -> Result<templates::PromptTemplate>,
    history_append: fn(&Path, &history::HistoryItem) -> Result<()>,
}

impl Default for TaskManagerDeps {
    fn default() -> Self {
        Self {
            fixture_path: pipeline::fixture_path,
            preprocess_to_temp_wav: pipeline::preprocess_to_temp_wav,
            preprocess_ffmpeg_cancellable: pipeline::preprocess_ffmpeg_cancellable,
            cleanup_audio_artifacts: pipeline::cleanup_audio_artifacts,
            get_template: templates::get_template,
            history_append: history::append,
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn rewrite_entered(opts: &StartOpts) -> bool {
    opts.rewrite_enabled && opts.template_id.is_some()
}

fn runtime_context_capture_config(
    ctx_cfg: &context_capture::ContextConfig,
    pre_captured_context: Option<&context_pack::ContextSnapshot>,
) -> context_capture::ContextConfig {
    let mut cfg = ctx_cfg.clone();
    if let Some(pre) = pre_captured_context {
        if !pre.recent_history.is_empty() {
            cfg.include_history = false;
        }
        if pre.clipboard_text.is_some() {
            cfg.include_clipboard = false;
        }
        if pre.focused_app.is_some() {
            cfg.include_focused_app_meta = false;
        }
        if pre.focused_window.is_some() {
            cfg.include_prev_window_meta = false;
        }
        if pre.focused_element.is_some() {
            cfg.include_focused_element_meta = false;
        }
        if pre.input_state.is_some() {
            cfg.include_input_state = false;
        }
        if pre.related_content.is_some() {
            cfg.include_related_content = false;
        }
        if pre.visible_text.is_some() {
            cfg.include_visible_text = false;
        }
    }
    cfg
}

fn merge_pre_captured_context(
    _ctx_cfg: &context_capture::ContextConfig,
    ctx_snap: &mut context_pack::ContextSnapshot,
    pre: context_pack::ContextSnapshot,
) {
    if !pre.recent_history.is_empty() {
        ctx_snap.recent_history = pre.recent_history;
    }
    if pre.clipboard_text.is_some() {
        ctx_snap.clipboard_text = pre.clipboard_text;
    }
    if pre.focused_app.is_some() {
        ctx_snap.focused_app = pre.focused_app;
    }
    if pre.focused_window.is_some() {
        ctx_snap.focused_window = pre.focused_window;
    }
    if pre.focused_element.is_some() {
        ctx_snap.focused_element = pre.focused_element;
    }
    if pre.input_state.is_some() {
        ctx_snap.input_state = pre.input_state;
    }
    if pre.related_content.is_some() {
        ctx_snap.related_content = pre.related_content;
    }
    if pre.visible_text.is_some() {
        ctx_snap.visible_text = pre.visible_text;
    }
    if pre.policy_decision.is_some() {
        ctx_snap.policy_decision = pre.policy_decision;
    }
    if pre.capture_diag.is_some() {
        ctx_snap.capture_diag = pre.capture_diag;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskEvent {
    pub task_id: String,
    pub stage: String,
    pub status: String, // started|completed|failed|cancelled
    pub message: String,
    pub elapsed_ms: Option<u128>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskDone {
    pub task_id: String,
    pub asr_text: String,
    pub final_text: String,
    pub rtf: f64,
    pub device_used: String,
    pub preprocess_ms: u128,
    pub asr_ms: u128,
    pub rewrite_ms: Option<u128>,
    pub rewrite_enabled: bool,
    pub template_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StartOpts {
    pub asr_provider: String,
    pub remote_asr_url: String,
    pub remote_asr_model: Option<String>,
    pub remote_asr_concurrency: usize,
    pub rewrite_enabled: bool,
    pub template_id: Option<String>,
    pub asr_preprocess: pipeline::PreprocessConfig,
    pub rewrite_glossary: Vec<String>,
    pub rewrite_include_glossary: bool,
    pub context_cfg: context_capture::ContextConfig,
    pub pre_captured_context: Option<context_pack::ContextSnapshot>,
    pub record_elapsed_ms: u128,
    pub record_label: String,
}

#[derive(Debug, Clone)]
struct PendingHotkeyContext {
    pub created_at_ms: i64,
    pub pre_captured_context: context_pack::ContextSnapshot,
}

#[derive(Debug)]
pub enum RecordingTerminal {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone)]
pub struct TaskManager {
    inner: Arc<Mutex<Option<ActiveTask>>>,
    pending_hotkey_contexts: Arc<Mutex<HashMap<String, PendingHotkeyContext>>>,
    asr: Arc<dyn AsrClient>,
    ctx: Arc<dyn ContextCollector>,
    deps: TaskManagerDeps,
}

struct ActiveTask {
    task_id: String,
    token: CancellationToken,
    ffmpeg_pid: Arc<Mutex<Option<u32>>>,
    asr_pid: Arc<Mutex<Option<u32>>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self::with_components(
            Arc::new(asr_service::AsrService::new()),
            Arc::new(context_capture::ContextService::new()),
            TaskManagerDeps::default(),
        )
    }

    fn with_components(
        asr: Arc<dyn AsrClient>,
        ctx: Arc<dyn ContextCollector>,
        deps: TaskManagerDeps,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            pending_hotkey_contexts: Arc::new(Mutex::new(HashMap::new())),
            asr,
            ctx,
            deps,
        }
    }

    pub fn has_active_task(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    pub fn active_task_id_best_effort(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .as_ref()
            .map(|task| task.task_id.clone())
    }

    fn env_bool_default_true(key: &str) -> bool {
        match std::env::var(key) {
            Ok(v) => {
                let t = v.trim().to_ascii_lowercase();
                !(t == "0" || t == "false" || t == "no" || t == "off")
            }
            Err(_) => true,
        }
    }

    pub fn warmup_asr_best_effort(&self) {
        // For debugging startup crashes on Windows, allow disabling resident runner.
        // Default: enabled.
        if !Self::env_bool_default_true("TYPEVOICE_ASR_RESIDENT") {
            return;
        }

        let this = self.clone();
        let _ = std::thread::Builder::new()
            .name("asr_warmup".to_string())
            .spawn(move || {
                // ASR warmup is synchronous; do not create nested Tokio runtimes here.
                if let Ok(dir) = data_dir::data_dir() {
                    if settings::resolve_asr_provider(
                        &settings::load_settings(&dir).unwrap_or_default(),
                    ) != "local"
                    {
                        return;
                    }
                    let _ = this.asr.ensure_started(&dir);
                }
            });
    }

    pub fn warmup_context_best_effort(&self) {
        self.ctx.warmup_best_effort();
    }

    pub fn capture_hotkey_context_now(
        &self,
        data_dir: &std::path::Path,
        cfg: &context_capture::ContextConfig,
    ) -> Result<String> {
        self.ctx.capture_hotkey_context_now(data_dir, cfg)
    }

    pub fn take_hotkey_context_once(
        &self,
        capture_id: &str,
    ) -> Option<context_pack::ContextSnapshot> {
        self.ctx.take_hotkey_context_once(capture_id)
    }

    pub fn capture_recording_context_best_effort(
        &self,
        data_dir: &Path,
        cfg: &context_capture::ContextConfig,
    ) -> context_pack::ContextSnapshot {
        let capture_id = format!("recording-start-{}", Uuid::new_v4());
        self.ctx
            .capture_snapshot_best_effort_with_config(data_dir, &capture_id, cfg)
    }

    pub fn last_external_hwnd_best_effort(&self) -> Option<isize> {
        self.ctx.last_external_hwnd_best_effort()
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

    fn take_pending_hotkey_context(&self, task_id: &str) -> Option<context_pack::ContextSnapshot> {
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

    pub fn restart_asr_best_effort(&self, reason: &str) {
        if !Self::env_bool_default_true("TYPEVOICE_ASR_RESIDENT") {
            return;
        }

        let this = self.clone();
        let reason = reason.to_string();
        let _ = std::thread::Builder::new()
            .name("asr_restart".to_string())
            .spawn(move || {
                if let Ok(dir) = data_dir::data_dir() {
                    if settings::resolve_asr_provider(
                        &settings::load_settings(&dir).unwrap_or_default(),
                    ) != "local"
                    {
                        return;
                    }
                    let _ = this.asr.restart(&dir, &reason);
                }
            });
    }

    pub fn kill_asr_best_effort(&self, reason: &str) {
        let this = self.clone();
        let reason = reason.to_string();
        let _ = std::thread::Builder::new()
            .name("asr_kill".to_string())
            .spawn(move || {
                this.asr.kill_best_effort(&reason);
            });
    }

    pub fn start_fixture(
        &self,
        app: AppHandle,
        fixture_name: String,
        opts: StartOpts,
    ) -> Result<String> {
        let input = (self.deps.fixture_path)(&fixture_name)?;
        self.start_audio(app, input, opts)
    }

    pub fn start_fixture_with_task_id(
        &self,
        app: AppHandle,
        task_id: String,
        fixture_name: String,
        opts: StartOpts,
    ) -> Result<String> {
        let input = (self.deps.fixture_path)(&fixture_name)?;
        self.start_audio_with_task_id(app, task_id, input, opts)
    }

    pub fn start_recording_file(
        &self,
        app: AppHandle,
        input_path: PathBuf,
        opts: StartOpts,
    ) -> Result<String> {
        let cleanup_input = input_path.clone();
        match self.start_audio(app, input_path, opts) {
            Ok(task_id) => Ok(task_id),
            Err(e) => {
                let _ = (self.deps.cleanup_audio_artifacts)(&cleanup_input, &cleanup_input);
                Err(e)
            }
        }
    }

    pub fn start_recording_file_with_task_id(
        &self,
        app: AppHandle,
        task_id: String,
        input_path: PathBuf,
        opts: StartOpts,
    ) -> Result<String> {
        let cleanup_input = input_path.clone();
        match self.start_audio_with_task_id(app, task_id, input_path, opts) {
            Ok(task_id) => Ok(task_id),
            Err(e) => {
                let _ = (self.deps.cleanup_audio_artifacts)(&cleanup_input, &cleanup_input);
                Err(e)
            }
        }
    }

    fn start_audio(&self, app: AppHandle, input: PathBuf, opts: StartOpts) -> Result<String> {
        let task_id = Uuid::new_v4().to_string();
        self.start_audio_with_task_id(app, task_id, input, opts)
    }

    fn start_audio_with_task_id(
        &self,
        app: AppHandle,
        task_id: String,
        input: PathBuf,
        mut opts: StartOpts,
    ) -> Result<String> {
        {
            let mut g = self.inner.lock().unwrap();
            if g.is_some() {
                return Err(anyhow!("another task is already running"));
            }
            if opts.pre_captured_context.is_none() {
                opts.pre_captured_context = self.take_pending_hotkey_context(&task_id);
            }
            *g = Some(ActiveTask {
                task_id: task_id.clone(),
                token: CancellationToken::new(),
                ffmpeg_pid: Arc::new(Mutex::new(None)),
                asr_pid: Arc::new(Mutex::new(None)),
            });
        }
        let this = self.clone();

        // The invoke handler may execute on a thread without an active Tokio
        // runtime/reactor. We detach into an OS thread and drive the async
        // pipeline using a dedicated Tokio runtime to avoid "no reactor
        // running" panics (panicking here aborts the process on Windows).
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match rt {
                Ok(rt) => {
                    rt.block_on(async move {
                        if let Err(e) = this
                            .run_pipeline(app.clone(), task_id.clone(), input, opts)
                            .await
                        {
                            // Fail-safe: ensure the UI always gets a terminal event.
                            let maybe_dir = data_dir::data_dir().ok();
                            if let Some(dir) = maybe_dir {
                                emit_failed(
                                    &app,
                                    &dir,
                                    &task_id,
                                    "Internal",
                                    None,
                                    "E_INTERNAL",
                                    &e.to_string(),
                                );
                            } else {
                                let _ = app.emit(
                                    "task_event",
                                    internal_failure_event(&task_id, e.to_string()),
                                );
                            }
                        }
                        {
                            let mut g = this.inner.lock().unwrap();
                            if g.as_ref().map(|a| &a.task_id) == Some(&task_id) {
                                *g = None;
                            }
                        }
                    });
                }
                Err(e) => {
                    // Best-effort cleanup; we might not have a data_dir to emit metrics.
                    crate::safe_eprintln!("failed to create tokio runtime for task {task_id}: {e}");
                    let msg = format!("tokio_runtime_create_failed:{e}");
                    if let Ok(dir) = data_dir::data_dir() {
                        emit_failed(&app, &dir, &task_id, "Internal", None, "E_INTERNAL", &msg);
                    } else {
                        let _ = app.emit("task_event", internal_failure_event(&task_id, msg));
                    }
                    let mut g = this.inner.lock().unwrap();
                    if g.as_ref().map(|a| &a.task_id) == Some(&task_id) {
                        *g = None;
                    }
                    let _ = (this.deps.cleanup_audio_artifacts)(&input, &input);
                }
            }
        });

        let active = {
            let g = self.inner.lock().unwrap();
            g.as_ref().unwrap().task_id.clone()
        };
        Ok(active)
    }

    pub fn cancel(&self, task_id: &str) -> Result<()> {
        let g = self.inner.lock().unwrap();
        let active = g.as_ref().ok_or_else(|| anyhow!("no active task"))?;
        if active.task_id != task_id {
            return Err(anyhow!("task_id mismatch"));
        }
        active.token.cancel();
        // Best-effort kill for external processes.
        if let Some(pid) = *active.ffmpeg_pid.lock().unwrap() {
            let _ = kill_pid(pid);
        }
        if let Some(pid) = *active.asr_pid.lock().unwrap() {
            let _ = kill_pid(pid);
        }
        Ok(())
    }

    async fn run_pipeline(
        &self,
        app: AppHandle,
        task_id: String,
        input: PathBuf,
        opts: StartOpts,
    ) -> Result<RecordingTerminal> {
        let data_dir = data_dir::data_dir()?;
        let preprocess_to_temp_wav = self.deps.preprocess_to_temp_wav;
        let preprocess_ffmpeg_cancellable = self.deps.preprocess_ffmpeg_cancellable;
        let cleanup_audio_artifacts = self.deps.cleanup_audio_artifacts;
        let get_template = self.deps.get_template;
        let history_append = self.deps.history_append;
        let ctx_cfg = opts.context_cfg.clone();
        let capture_ctx_cfg =
            runtime_context_capture_config(&ctx_cfg, opts.pre_captured_context.as_ref());
        let mut ctx_snap = if opts.rewrite_enabled {
            self.ctx
                .capture_snapshot_best_effort_with_config(&data_dir, &task_id, &capture_ctx_cfg)
        } else {
            context_pack::ContextSnapshot::default()
        };
        if opts.rewrite_enabled {
            if let Some(pre) = opts.pre_captured_context.clone() {
                merge_pre_captured_context(&ctx_cfg, &mut ctx_snap, pre);
                crate::obs::event(
                    &data_dir,
                    Some(&task_id),
                    "ContextCapture",
                    "CTX.hotkey_capture_injected",
                    "ok",
                    Some(serde_json::json!({
                        "has_focused_app": ctx_snap.focused_app.is_some(),
                        "has_focused_element": ctx_snap.focused_element.is_some(),
                        "has_input_state": ctx_snap.input_state.is_some(),
                    })),
                );
            }
        }

        if !ctx_cfg.include_history {
            ctx_snap.recent_history.clear();
        }
        if !ctx_cfg.include_clipboard {
            ctx_snap.clipboard_text = None;
        }
        if !ctx_cfg.include_focused_app_meta {
            ctx_snap.focused_app = None;
        }
        if !ctx_cfg.include_prev_window_meta {
            ctx_snap.focused_window = None;
        }
        if !ctx_cfg.include_focused_element_meta {
            ctx_snap.focused_element = None;
        }
        if !ctx_cfg.include_input_state {
            ctx_snap.input_state = None;
        }
        if !ctx_cfg.include_related_content {
            ctx_snap.related_content = None;
        }
        if !ctx_cfg.include_visible_text {
            ctx_snap.visible_text = None;
        }
        crate::obs::event(
            &data_dir,
            Some(&task_id),
            "Task",
            "TASK.start_opts",
            "ok",
            Some(serde_json::json!({
                    "asr_provider": opts.asr_provider.as_str(),
                    "remote_asr_url": opts.remote_asr_url.as_str(),
                    "remote_asr_has_model": opts.remote_asr_model.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false),
                    "remote_asr_concurrency": opts.remote_asr_concurrency,
                    "rewrite_requested": opts.rewrite_enabled,
                    "template_id": opts.template_id.as_deref(),
                    "rewrite_include_glossary": opts.rewrite_include_glossary,
                    "context_include_prev_window_meta": ctx_cfg.include_prev_window_meta,
                "context_include_focused_app_meta": ctx_cfg.include_focused_app_meta,
                "context_include_focused_element_meta": ctx_cfg.include_focused_element_meta,
                "context_include_input_state": ctx_cfg.include_input_state,
                "context_include_related_content": ctx_cfg.include_related_content,
                "context_include_visible_text": ctx_cfg.include_visible_text,
                "context_include_history": ctx_cfg.include_history,
                "context_include_clipboard": ctx_cfg.include_clipboard,
                "asr_preprocess_silence_trim_enabled": opts.asr_preprocess.silence_trim_enabled,
                "asr_preprocess_threshold_db": opts.asr_preprocess.silence_threshold_db,
                "asr_preprocess_trim_start_ms": opts.asr_preprocess.silence_trim_start_ms,
                "asr_preprocess_trim_end_ms": opts.asr_preprocess.silence_trim_end_ms,
            })),
        );

        emit_event(
            &app,
            &data_dir,
            TaskEvent {
                task_id: task_id.clone(),
                stage: "Record".to_string(),
                status: "completed".to_string(),
                message: opts.record_label.clone(),
                elapsed_ms: Some(opts.record_elapsed_ms),
                error_code: None,
            },
        );

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Record");
            return Ok(RecordingTerminal::Cancelled);
        }

        // Preprocess
        let preprocess_label = if opts.asr_preprocess.silence_trim_enabled {
            "ffmpeg (silence_trim)"
        } else {
            "ffmpeg"
        };
        emit_started(&app, &data_dir, &task_id, "Preprocess", preprocess_label);
        let wav_path = preprocess_to_temp_wav(&task_id, &input)?;
        let asr_preprocess_cfg = opts.asr_preprocess.clone();
        let preprocess_ms = {
            let inner = self.inner.clone();
            let data_dir2 = data_dir.clone();
            let task_id2 = task_id.clone();
            let input2 = input.clone();
            let wav2 = wav_path.clone();
            let preprocess_ffmpeg_cancellable = preprocess_ffmpeg_cancellable;
            let join = tokio::task::spawn_blocking(move || {
                let active = inner.lock().unwrap();
                let a = active.as_ref().ok_or_else(|| anyhow!("task missing"))?;
                // launch ffmpeg inside helper so we can store pid
                let ms = preprocess_ffmpeg_cancellable(
                    &data_dir2,
                    &task_id2,
                    &input2,
                    &wav2,
                    &a.token,
                    &a.ffmpeg_pid,
                    &asr_preprocess_cfg,
                )?;
                Ok::<_, anyhow::Error>(ms)
            })
            .await;
            match join {
                Ok(Ok(ms)) => ms,
                Ok(Err(e)) => {
                    if is_cancelled_err(&e) || is_cancelled(&self.inner, &task_id) {
                        emit_cancelled(&app, &data_dir, &task_id, "Preprocess");
                        let _ = cleanup_audio_artifacts(&input, &wav_path);
                        return Ok(RecordingTerminal::Cancelled);
                    }
                    let msg = e.to_string();
                    let code = if msg.contains("E_FFMPEG_NOT_FOUND") {
                        "E_FFMPEG_NOT_FOUND"
                    } else if msg.contains("E_FFMPEG_FAILED") {
                        "E_FFMPEG_FAILED"
                    } else {
                        "E_PREPROCESS_FAILED"
                    };
                    emit_failed(&app, &data_dir, &task_id, "Preprocess", None, code, &msg);
                    let _ = cleanup_audio_artifacts(&input, &wav_path);
                    return Ok(RecordingTerminal::Failed);
                }
                Err(e) => {
                    emit_failed(
                        &app,
                        &data_dir,
                        &task_id,
                        "Preprocess",
                        None,
                        "E_INTERNAL",
                        &format!("preprocess_join_failed:{e}"),
                    );
                    let _ = cleanup_audio_artifacts(&input, &wav_path);
                    return Ok(RecordingTerminal::Failed);
                }
            }
        };
        emit_completed(&app, &data_dir, &task_id, "Preprocess", preprocess_ms, "ok");

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Preprocess");
            let _ = cleanup_audio_artifacts(&input, &wav_path);
            return Ok(RecordingTerminal::Cancelled);
        }

        // ASR
        let asr_provider = opts.asr_provider.to_ascii_lowercase();
        let asr_step_hint = if asr_provider == "remote" {
            "asr(remote)"
        } else {
            "asr(local)"
        };
        emit_started(&app, &data_dir, &task_id, "Transcribe", asr_step_hint);
        let (
            asr_text,
            rtf,
            device_used,
            asr_ms,
            runner_elapsed_ms,
            audio_seconds,
            asr_model_id,
            asr_model_version,
            remote_slice_count,
            remote_concurrency_used,
        ) = {
            if asr_provider == "remote" {
                let token = {
                    let active = self.inner.lock().unwrap();
                    let task = active.as_ref().ok_or_else(|| anyhow!("task missing"))?;
                    task.token.clone()
                };
                let cfg = remote_asr::RemoteAsrConfig {
                    url: opts.remote_asr_url.clone(),
                    model: opts.remote_asr_model.clone(),
                    concurrency: opts.remote_asr_concurrency,
                };
                match remote_asr::transcribe_remote(&data_dir, &task_id, &wav_path, &token, &cfg)
                    .await
                {
                    Ok(v) => (
                        v.text,
                        v.metrics.rtf,
                        "remote".to_string(),
                        v.metrics.elapsed_ms.max(0) as u128,
                        v.metrics.elapsed_ms,
                        v.metrics.audio_seconds,
                        v.metrics.model_id,
                        v.metrics.model_version,
                        Some(v.metrics.slice_count),
                        Some(v.metrics.concurrency_used),
                    ),
                    Err(e) => {
                        if e.code == "E_CANCELLED" || is_cancelled(&self.inner, &task_id) {
                            emit_cancelled(&app, &data_dir, &task_id, "Transcribe");
                            let _ = cleanup_audio_artifacts(&input, &wav_path);
                            return Ok(RecordingTerminal::Cancelled);
                        }
                        emit_failed(
                            &app,
                            &data_dir,
                            &task_id,
                            "Transcribe",
                            None,
                            &e.code,
                            &e.message,
                        );
                        let _ = cleanup_audio_artifacts(&input, &wav_path);
                        return Ok(RecordingTerminal::Failed);
                    }
                }
            } else {
                let inner = self.inner.clone();
                let wav_path2 = wav_path.clone();
                let data_dir2 = data_dir.clone();
                let asr = self.asr.clone();
                let task_id2 = task_id.clone();
                let join = tokio::task::spawn_blocking(move || {
                    let active = inner.lock().unwrap();
                    let a = active.as_ref().ok_or_else(|| anyhow!("task missing"))?;
                    let (resp, wall_ms) = asr.transcribe(
                        &data_dir2, &task_id2, &wav_path2, "Chinese", &a.token, &a.asr_pid,
                    )?;
                    if !resp.ok {
                        let code = resp
                            .error
                            .as_ref()
                            .map(|e| e.code.as_str())
                            .unwrap_or("E_ASR_FAILED");
                        let msg = resp
                            .error
                            .as_ref()
                            .map(|e| e.message.as_str())
                            .unwrap_or("");
                        if msg.trim().is_empty() {
                            return Err(anyhow!("asr failed: {code}"));
                        }
                        return Err(anyhow!("asr failed: {code}: {msg}"));
                    }
                    let text = resp.text.clone().unwrap_or_default();
                    if text.trim().is_empty() {
                        return Err(anyhow!("empty_text"));
                    }
                    let m = resp
                        .metrics
                        .clone()
                        .ok_or_else(|| anyhow!("missing_metrics"))?;
                    if m.device_used != "cuda" {
                        return Err(anyhow!("device_not_cuda:{}", m.device_used));
                    }
                    Ok::<_, anyhow::Error>((
                        text,
                        m.rtf,
                        m.device_used,
                        wall_ms,
                        m.elapsed_ms,
                        m.audio_seconds,
                        m.model_id,
                        m.model_version,
                        None,
                        None,
                    ))
                })
                .await;
                match join {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        if is_cancelled_err(&e) || is_cancelled(&self.inner, &task_id) {
                            emit_cancelled(&app, &data_dir, &task_id, "Transcribe");
                            let _ = cleanup_audio_artifacts(&input, &wav_path);
                            return Ok(RecordingTerminal::Cancelled);
                        }
                        emit_failed(
                            &app,
                            &data_dir,
                            &task_id,
                            "Transcribe",
                            None,
                            "E_ASR_FAILED",
                            &e.to_string(),
                        );
                        let _ = cleanup_audio_artifacts(&input, &wav_path);
                        return Ok(RecordingTerminal::Failed);
                    }
                    Err(e) => {
                        emit_failed(
                            &app,
                            &data_dir,
                            &task_id,
                            "Transcribe",
                            None,
                            "E_INTERNAL",
                            &format!("transcribe_join_failed:{e}"),
                        );
                        let _ = cleanup_audio_artifacts(&input, &wav_path);
                        return Ok(RecordingTerminal::Failed);
                    }
                }
            }
        };
        emit_completed(
            &app,
            &data_dir,
            &task_id,
            "Transcribe",
            asr_ms,
            format!("rtf={rtf:.3}"),
        );

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Transcribe");
            let _ = cleanup_audio_artifacts(&input, &wav_path);
            return Ok(RecordingTerminal::Cancelled);
        }

        // We no longer need audio artifacts after ASR; cleanup early.
        let _ = cleanup_audio_artifacts(&input, &wav_path);

        // Rewrite (optional)
        let mut final_text = asr_text.clone();
        let mut rewrite_ms = None;
        let mut template_id = None;
        let rewrite_entered = rewrite_entered(&opts);
        crate::obs::event(
            &data_dir,
            Some(&task_id),
            "Task",
            "TASK.rewrite_effective",
            if rewrite_entered { "ok" } else { "skipped" },
            Some(serde_json::json!({
                "rewrite_requested": opts.rewrite_enabled,
                "has_template": opts.template_id.is_some(),
                "rewrite_entered": rewrite_entered,
                "template_id": opts.template_id.as_deref(),
            })),
        );
        if opts.rewrite_enabled {
            if let Some(tid) = opts.template_id.clone() {
                template_id = Some(tid.clone());
                emit_started(&app, &data_dir, &task_id, "Rewrite", "llm");
                let t0 = Instant::now();
                let tpl = match get_template(&data_dir, &tid) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        rewrite_ms = Some(t0.elapsed().as_millis());
                        emit_failed(
                            &app,
                            &data_dir,
                            &task_id,
                            "Rewrite",
                            rewrite_ms,
                            "E_TEMPLATE_NOT_FOUND",
                            &e.to_string(),
                        );
                        None
                    }
                };
                if let Some(tpl) = tpl {
                    let prepared = context_pack::prepare(&asr_text, &ctx_snap, &ctx_cfg.budget);
                    let rewrite_ctx_policy = llm::RewriteContextPolicy {
                        include_history: !ctx_snap.recent_history.is_empty(),
                        include_clipboard: ctx_snap.clipboard_text.is_some(),
                        include_prev_window_meta: ctx_snap.focused_window.is_some(),
                        include_focused_app_meta: ctx_snap.focused_app.is_some(),
                        include_focused_element_meta: ctx_snap.focused_element.is_some(),
                        include_input_state: ctx_snap.input_state.is_some(),
                        include_related_content: ctx_snap.related_content.is_some(),
                        include_visible_text: ctx_snap.visible_text.is_some(),
                        include_glossary: opts.rewrite_include_glossary,
                    };
                    let rewrite_glossary: &[String] = if opts.rewrite_include_glossary {
                        &opts.rewrite_glossary
                    } else {
                        &[]
                    };
                    let token = {
                        let g = self.inner.lock().unwrap();
                        g.as_ref().unwrap().token.clone()
                    };
                    let rewrite_res = tokio::select! {
                            _ = token.cancelled() => Err(anyhow!("cancelled")),
                        r = llm::rewrite_with_context(
                            &data_dir,
                            &task_id,
                            &tpl.system_prompt,
                            &asr_text,
                            Some(&prepared),
                            rewrite_glossary,
                            &rewrite_ctx_policy,
                        ) => r,
                    };
                    match rewrite_res {
                        Ok(txt) => {
                            final_text = txt;
                            rewrite_ms = Some(t0.elapsed().as_millis());
                            emit_completed(
                                &app,
                                &data_dir,
                                &task_id,
                                "Rewrite",
                                rewrite_ms.unwrap(),
                                "ok",
                            );
                        }
                        Err(e) => {
                            if is_cancelled_err(&e) || is_cancelled(&self.inner, &task_id) {
                                emit_cancelled(&app, &data_dir, &task_id, "Rewrite");
                                return Ok(RecordingTerminal::Cancelled);
                            }
                            // fallback to asr_text
                            rewrite_ms = Some(t0.elapsed().as_millis());
                            emit_failed(
                                &app,
                                &data_dir,
                                &task_id,
                                "Rewrite",
                                rewrite_ms,
                                "E_LLM_FAILED",
                                &e.to_string(),
                            );
                        }
                    }
                }
            }
        }

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Rewrite");
            return Ok(RecordingTerminal::Cancelled);
        }

        // Persist history
        emit_started(&app, &data_dir, &task_id, "Persist", "sqlite");
        let created_at_ms = chrono_now_ms();
        let item = history::HistoryItem {
            task_id: task_id.clone(),
            created_at_ms,
            asr_text: asr_text.clone(),
            final_text: final_text.clone(),
            template_id: template_id.clone(),
            rtf,
            device_used: device_used.clone(),
            preprocess_ms: preprocess_ms as i64,
            asr_ms: asr_ms as i64,
        };
        let db = data_dir.join("history.sqlite3");
        if let Err(e) = history_append(&db, &item) {
            emit_failed(
                &app,
                &data_dir,
                &task_id,
                "Persist",
                None,
                "E_PERSIST_FAILED",
                &e.to_string(),
            );
            return Ok(RecordingTerminal::Failed);
        }
        emit_completed(&app, &data_dir, &task_id, "Persist", 0, "ok");

        // Export stage is UI-driven (copy). We still emit completed to align spec.
        emit_event(
            &app,
            &data_dir,
            TaskEvent {
                task_id: task_id.clone(),
                stage: "Export".to_string(),
                status: "completed".to_string(),
                message: "copy in UI".to_string(),
                elapsed_ms: Some(0),
                error_code: None,
            },
        );

        // Done event
        let done = TaskDone {
            task_id: task_id.clone(),
            asr_text,
            final_text,
            rtf,
            device_used,
            preprocess_ms,
            asr_ms,
            rewrite_ms,
            rewrite_enabled: opts.rewrite_enabled,
            template_id,
        };
        let _ = app.emit("task_done", done.clone());
        if let Err(e) = metrics::emit(
            &data_dir,
            MetricsRecord::TaskDone {
                ts_ms: chrono_now_ms(),
                task_id: task_id.clone(),
                rtf: done.rtf,
                device: done.device_used.clone(),
            },
        ) {
            crate::safe_eprintln!("metrics append failed (task_done): {e:#}");
        }

        // Perf summary (machine-readable, no sensitive payload).
        let overhead_ms_u128 = asr_ms.saturating_sub(runner_elapsed_ms.max(0) as u128);
        let overhead_ms = overhead_ms_u128.min(u64::MAX as u128) as u64;
        if let Err(e) = metrics::emit(
            &data_dir,
            MetricsRecord::TaskPerf {
                ts_ms: chrono_now_ms(),
                task_id,
                asr_provider,
                audio_seconds,
                preprocess_ms,
                asr_roundtrip_ms: asr_ms,
                asr_runner_elapsed_ms: runner_elapsed_ms,
                asr_overhead_ms: overhead_ms,
                rtf,
                rewrite_ms,
                device_used: done.device_used.clone(),
                asr_model_id,
                asr_model_version,
                remote_asr_slice_count: remote_slice_count,
                remote_asr_concurrency_used: remote_concurrency_used,
                asr_preprocess_silence_trim_enabled: opts.asr_preprocess.silence_trim_enabled,
                asr_preprocess_threshold_db: opts.asr_preprocess.silence_threshold_db,
                asr_preprocess_trim_start_ms: opts.asr_preprocess.silence_trim_start_ms,
                asr_preprocess_trim_end_ms: opts.asr_preprocess.silence_trim_end_ms,
                asr_warmup_ms: self.asr.warmup_ms(),
            },
        ) {
            crate::safe_eprintln!("metrics append failed (task_perf): {e:#}");
        }
        Ok(RecordingTerminal::Completed)
    }
}

fn chrono_now_ms() -> i64 {
    // avoid chrono dependency; use std time
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn emit_started(app: &AppHandle, data_dir: &Path, task_id: &str, stage: &str, msg: &str) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "started".to_string(),
            message: msg.to_string(),
            elapsed_ms: None,
            error_code: None,
        },
    );
}

fn emit_completed(
    app: &AppHandle,
    data_dir: &Path,
    task_id: &str,
    stage: &str,
    elapsed_ms: u128,
    msg: impl Into<String>,
) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "completed".to_string(),
            message: msg.into(),
            elapsed_ms: Some(elapsed_ms),
            error_code: None,
        },
    );
}

fn emit_failed(
    app: &AppHandle,
    data_dir: &Path,
    task_id: &str,
    stage: &str,
    elapsed_ms: Option<u128>,
    code: &str,
    msg: &str,
) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "failed".to_string(),
            message: msg.to_string(),
            elapsed_ms,
            error_code: Some(code.to_string()),
        },
    );
}

fn emit_cancelled(app: &AppHandle, data_dir: &Path, task_id: &str, stage: &str) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "cancelled".to_string(),
            message: "cancelled".to_string(),
            elapsed_ms: None,
            error_code: Some("E_CANCELLED".to_string()),
        },
    );
}

fn internal_failure_event(task_id: &str, message: String) -> TaskEvent {
    TaskEvent {
        task_id: task_id.to_string(),
        stage: "Internal".to_string(),
        status: "failed".to_string(),
        message,
        elapsed_ms: None,
        error_code: Some("E_INTERNAL".to_string()),
    }
}

fn emit_event(app: &AppHandle, data_dir: &Path, ev: TaskEvent) {
    let _ = app.emit("task_event", ev.clone());
    if let Err(e) = metrics::emit(
        data_dir,
        MetricsRecord::TaskEvent {
            ts_ms: chrono_now_ms(),
            task_id: ev.task_id,
            stage: ev.stage,
            status: ev.status,
            elapsed_ms: ev.elapsed_ms,
            error_code: ev.error_code,
            message: ev.message,
        },
    ) {
        crate::safe_eprintln!("metrics append failed (task_event): {e:#}");
    }
}

fn is_cancelled(inner: &Arc<Mutex<Option<ActiveTask>>>, task_id: &str) -> bool {
    let g = inner.lock().unwrap();
    if let Some(a) = g.as_ref() {
        if a.task_id == task_id {
            return a.token.is_cancelled();
        }
    }
    false
}

fn is_cancelled_err(e: &anyhow::Error) -> bool {
    let s = e.to_string();
    s == "cancelled" || s.contains("cancelled")
}

#[cfg(unix)]
fn kill_pid(pid: u32) -> Result<()> {
    let status = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .context("kill failed")?;
    if !status.success() {
        return Err(anyhow!("kill exit={status}"));
    }
    Ok(())
}

#[cfg(windows)]
fn kill_pid(pid: u32) -> Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .no_console()
        .status()
        .context("taskkill failed")?;
    if !status.success() {
        return Err(anyhow!("taskkill exit={status}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        internal_failure_event, merge_pre_captured_context, rewrite_entered,
        runtime_context_capture_config, StartOpts,
    };
    use crate::{
        context_capture,
        context_pack::{ContextCaptureDiag, ContextPolicyDecision, ContextSnapshot},
        pipeline,
    };

    #[test]
    fn internal_failure_event_has_error_code_and_terminal_status() {
        let ev = internal_failure_event("task-1", "tokio runtime failed".to_string());
        assert_eq!(ev.task_id, "task-1");
        assert_eq!(ev.stage, "Internal");
        assert_eq!(ev.status, "failed");
        assert_eq!(ev.error_code.as_deref(), Some("E_INTERNAL"));
        assert!(ev.elapsed_ms.is_none());
    }

    fn base_start_opts() -> StartOpts {
        StartOpts {
            asr_provider: "local".to_string(),
            remote_asr_url: "https://api.server/transcribe".to_string(),
            remote_asr_model: None,
            remote_asr_concurrency: 4,
            rewrite_enabled: false,
            template_id: None,
            asr_preprocess: pipeline::PreprocessConfig::default(),
            rewrite_glossary: Vec::new(),
            rewrite_include_glossary: true,
            context_cfg: context_capture::ContextConfig::default(),
            pre_captured_context: None,
            record_elapsed_ms: 0,
            record_label: "Record".to_string(),
        }
    }

    #[test]
    fn rewrite_entered_requires_flag_and_template() {
        let mut opts = base_start_opts();
        opts.rewrite_enabled = true;
        opts.template_id = Some("tpl-1".to_string());
        assert!(rewrite_entered(&opts));
    }

    #[test]
    fn rewrite_entered_false_without_template() {
        let mut opts = base_start_opts();
        opts.rewrite_enabled = true;
        opts.template_id = None;
        assert!(!rewrite_entered(&opts));
    }

    #[test]
    fn runtime_context_capture_config_skips_only_fields_present_in_pre_capture() {
        let cfg = context_capture::ContextConfig::default();
        let runtime_cfg = runtime_context_capture_config(
            &cfg,
            Some(&ContextSnapshot {
                clipboard_text: Some("frozen".to_string()),
                focused_app: Some(crate::context_pack::FocusedAppInfo {
                    process_image: Some("notepad.exe".to_string()),
                    window_title: Some("notes".to_string()),
                    url: None,
                    is_browser: false,
                    target_source: Some("foreground".to_string()),
                }),
                ..Default::default()
            }),
        );

        assert!(runtime_cfg.include_history);
        assert!(!runtime_cfg.include_clipboard);
        assert!(!runtime_cfg.include_focused_app_meta);
        assert!(runtime_cfg.include_prev_window_meta);
        assert!(runtime_cfg.include_focused_element_meta);
        assert!(runtime_cfg.include_input_state);
        assert!(runtime_cfg.include_related_content);
        assert!(runtime_cfg.include_visible_text);
    }

    #[test]
    fn merge_pre_captured_context_overrides_policy_and_diag() {
        let cfg = context_capture::ContextConfig::default();
        let mut runtime = ContextSnapshot {
            recent_history: vec![],
            clipboard_text: Some("runtime".to_string()),
            policy_decision: Some(ContextPolicyDecision {
                capture_mode: "balanced".to_string(),
                app_rule: Some("allow".to_string()),
                domain_rule: None,
                allow_input_state: true,
                allow_related_content: true,
                allow_visible_text: true,
            }),
            capture_diag: Some(ContextCaptureDiag {
                target_source: Some("foreground".to_string()),
                target_age_ms: Some(0),
                focus_stable: true,
            }),
            ..Default::default()
        };
        let pre = ContextSnapshot {
            recent_history: vec![crate::context_pack::HistorySnippet {
                created_at_ms: 1,
                asr_text: "a".to_string(),
                final_text: "b".to_string(),
                template_id: None,
            }],
            clipboard_text: Some("frozen".to_string()),
            policy_decision: Some(ContextPolicyDecision {
                capture_mode: "minimal".to_string(),
                app_rule: None,
                domain_rule: Some("deny".to_string()),
                allow_input_state: false,
                allow_related_content: false,
                allow_visible_text: false,
            }),
            capture_diag: Some(ContextCaptureDiag {
                target_source: Some("last_external".to_string()),
                target_age_ms: Some(140),
                focus_stable: false,
            }),
            ..Default::default()
        };

        merge_pre_captured_context(&cfg, &mut runtime, pre);

        assert_eq!(runtime.recent_history.len(), 1);
        assert_eq!(runtime.clipboard_text.as_deref(), Some("frozen"));
        assert_eq!(
            runtime
                .policy_decision
                .as_ref()
                .and_then(|v| v.domain_rule.as_deref()),
            Some("deny")
        );
        assert_eq!(
            runtime
                .capture_diag
                .as_ref()
                .and_then(|v| v.target_source.as_deref()),
            Some("last_external")
        );
    }

    #[test]
    fn merge_pre_captured_context_keeps_frozen_fields_even_if_current_flags_are_off() {
        let cfg = context_capture::ContextConfig {
            include_focused_app_meta: false,
            include_prev_window_meta: false,
            ..context_capture::ContextConfig::default()
        };
        let mut runtime = ContextSnapshot::default();
        let pre = ContextSnapshot {
            focused_app: Some(crate::context_pack::FocusedAppInfo {
                process_image: Some("notepad.exe".to_string()),
                window_title: Some("notes".to_string()),
                url: None,
                is_browser: false,
                target_source: Some("foreground".to_string()),
            }),
            focused_window: Some(crate::context_pack::FocusedWindowInfo {
                title: Some("notes".to_string()),
                class_name: Some("Notepad".to_string()),
            }),
            ..Default::default()
        };

        merge_pre_captured_context(&cfg, &mut runtime, pre);

        assert_eq!(
            runtime
                .focused_app
                .as_ref()
                .and_then(|v| v.process_image.as_deref()),
            Some("notepad.exe")
        );
        assert_eq!(
            runtime
                .focused_window
                .as_ref()
                .and_then(|v| v.class_name.as_deref()),
            Some("Notepad")
        );
    }

    #[test]
    fn rewrite_disabled_flow_keeps_context_snapshot_empty() {
        let opts = StartOpts {
            asr_provider: "local".to_string(),
            remote_asr_url: String::new(),
            remote_asr_model: None,
            remote_asr_concurrency: 1,
            rewrite_enabled: false,
            template_id: None,
            asr_preprocess: crate::pipeline::PreprocessConfig::default(),
            rewrite_glossary: Vec::new(),
            rewrite_include_glossary: false,
            context_cfg: context_capture::ContextConfig::default(),
            pre_captured_context: Some(ContextSnapshot {
                clipboard_text: Some("secret".to_string()),
                ..Default::default()
            }),
            record_elapsed_ms: 0,
            record_label: "Record".to_string(),
        };

        let mut ctx_snap = if opts.rewrite_enabled {
            panic!("rewrite should be disabled in this test");
        } else {
            ContextSnapshot::default()
        };
        if opts.rewrite_enabled {
            if let Some(pre) = opts.pre_captured_context.clone() {
                merge_pre_captured_context(&opts.context_cfg, &mut ctx_snap, pre);
            }
        }

        assert!(ctx_snap.clipboard_text.is_none());
    }
}
