use std::{
    sync::{
        atomic::{AtomicU32, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;
use typevoice_core::{
    context_pack::{ContextBudget, ContextSnapshot},
    workflow::{
        ContextPlan, EffectCounts, InsertPrepareResult, RecoveredRunResult, RunPlanSeed,
        TranscriptionResult, WorkflowError,
    },
};
use typevoice_platform::{
    context_capture, export, pipeline,
    record_input_cache::{CachedRecordInput, RecordInputCacheState},
};
use typevoice_storage::{data_dir, history};

use crate::{
    audio_capture::{RecordedAsset, RecordingRegistry, RecordingStopOutcome},
    rewrite::{self, RewriteTextRequest},
    run_executor::{BeginAccepted, PortFuture, RunPorts, RunPortsFactory, START_DEADLINE_MS},
    task_manager::TaskManager,
    transcription::{PreparedTranscription, TranscriptionInput, TranscriptionService},
    transcription_actor::{StreamingProviderKind, StreamingSessionConfig, TranscriptionActor},
    ui_events::UiEventMailbox,
};

#[derive(Clone)]
pub struct RuntimeRunPortsFactory {
    mailbox: UiEventMailbox,
    task_manager: TaskManager,
    record_input_cache: RecordInputCacheState,
}

impl RuntimeRunPortsFactory {
    pub fn new(
        mailbox: UiEventMailbox,
        task_manager: TaskManager,
        record_input_cache: RecordInputCacheState,
    ) -> Self {
        Self {
            mailbox,
            task_manager,
            record_input_cache,
        }
    }
}

impl RunPortsFactory for RuntimeRunPortsFactory {
    fn create(&self, run_id: &str, seed: &RunPlanSeed) -> Arc<dyn RunPorts> {
        let record_input = match self.record_input_cache.get_for_plan(&seed.recording) {
            Some(cached) => Ok(cached),
            None => Err(WorkflowError::new(
                "E_RECORD_INPUT_CACHE_PLAN_MISMATCH",
                "no validated recording input matches the frozen run plan",
            )),
        };
        Arc::new(RuntimeRunPorts {
            run_id: run_id.to_string(),
            seed: seed.clone(),
            mailbox: self.mailbox.clone(),
            task_manager: self.task_manager.clone(),
            record_input,
            recording: RecordingRegistry::new(),
            transcriber: TranscriptionService::from_plan(&seed.asr),
            actor: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(RuntimePortState::default())),
            operations: OperationTracker::default(),
            effects: EffectTracker::default(),
        })
    }
}

#[derive(Default)]
struct RuntimePortState {
    recording_session_id: Option<String>,
    asset: Option<RecordedAsset>,
    context: Option<ContextSnapshot>,
    prepared: Option<PreparedTranscription>,
    insertion_target: Option<export::InsertionTarget>,
    cancellation: Option<CancellationToken>,
    cleanup_in_progress: bool,
    shutdown_complete: bool,
}

struct RuntimeRunPorts {
    run_id: String,
    seed: RunPlanSeed,
    mailbox: UiEventMailbox,
    task_manager: TaskManager,
    record_input: Result<CachedRecordInput, WorkflowError>,
    recording: RecordingRegistry,
    transcriber: TranscriptionService,
    actor: Arc<Mutex<Option<TranscriptionActor>>>,
    state: Arc<Mutex<RuntimePortState>>,
    operations: OperationTracker,
    effects: EffectTracker,
}

#[derive(Clone, Default)]
struct EffectTracker {
    history_commit_count: Arc<AtomicU32>,
    copy_count: Arc<AtomicU32>,
    paste_count: Arc<AtomicU32>,
}

impl EffectTracker {
    fn snapshot(&self) -> EffectCounts {
        EffectCounts {
            history_commit_count: self.history_commit_count.load(Ordering::Acquire),
            copy_count: self.copy_count.load(Ordering::Acquire),
            paste_count: self.paste_count.load(Ordering::Acquire),
        }
    }
}

#[derive(Clone, Default)]
struct OperationTracker {
    active: Arc<AtomicUsize>,
    idle: Arc<tokio::sync::Notify>,
}

impl OperationTracker {
    fn begin(&self) -> OperationLease {
        self.active.fetch_add(1, Ordering::AcqRel);
        OperationLease {
            active: self.active.clone(),
            idle: self.idle.clone(),
        }
    }

    fn is_idle(&self) -> bool {
        self.active.load(Ordering::Acquire) == 0
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self.is_idle() {
                return;
            }
            notified.await;
        }
    }
}

struct OperationLease {
    active: Arc<AtomicUsize>,
    idle: Arc<tokio::sync::Notify>,
}

struct TrackedOutput<T> {
    value: T,
    lease: OperationLease,
}

impl Drop for OperationLease {
    fn drop(&mut self) {
        if self.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle.notify_one();
        }
    }
}

impl RunPorts for RuntimeRunPorts {
    fn begin_recording(&self, token: CancellationToken) -> Result<BeginAccepted, WorkflowError> {
        let begin_started = Instant::now();
        let begin_deadline = Duration::from_millis(START_DEADLINE_MS);
        if token.is_cancelled() {
            return Err(WorkflowError::new(
                "E_CANCELLED",
                "run cancelled before recording",
            ));
        }
        self.state.lock().unwrap().cancellation = Some(token.clone());
        let record_input = self.record_input.clone()?;
        let target_rx = if self.seed.insertion.auto_paste {
            let lease = self.operations.begin();
            let (target_tx, target_rx) = std::sync::mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name(format!("insertion_target_{}", self.run_id))
                .spawn(move || {
                    let _lease = lease;
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            WorkflowError::new("E_EXPORT_TARGET_RUNTIME", error.to_string())
                        })
                        .and_then(|runtime| {
                            runtime
                                .block_on(export::capture_insertion_target())
                                .map_err(|error| WorkflowError::new(&error.code, error.message))
                        });
                    let _ = target_tx.send(result);
                })
                .map_err(|error| WorkflowError::new("E_EXPORT_TARGET_WORKER", error.to_string()))?;
            Some(target_rx)
        } else {
            None
        };

        let actor = if self.seed.asr.provider == "doubao" {
            Some(
                TranscriptionActor::new(self.mailbox.clone())
                    .map_err(|error| workflow_error("E_STREAMING_ACTOR_START", error))?,
            )
        } else {
            None
        };
        *self.actor.lock().unwrap() = actor.clone();
        let streaming_config = actor.as_ref().map(|_| StreamingSessionConfig {
            provider: StreamingProviderKind::Doubao,
            chunk_ms: 200,
            chunk_bytes: crate::pcm::pcm_bytes_for_ms(200),
        });
        let pending_session = match (actor.as_ref(), streaming_config.clone()) {
            (Some(actor), Some(config)) => Some(
                actor
                    .start_session_pending(&self.run_id, config)
                    .map_err(|error| workflow_error("E_STREAMING_START", error))?,
            ),
            _ => None,
        };

        let start_token = token.child_token();
        let recording = self.recording.clone();
        let mailbox = self.mailbox.clone();
        let actor_for_start = actor.clone();
        let input = record_input;
        let task_id = self.run_id.clone();
        let worker_token = start_token.clone();
        let lease = self.operations.begin();
        let (start_tx, start_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name(format!("recording_begin_{}", self.run_id))
            .spawn(move || {
                let _lease = lease;
                let result = recording.start_recording(
                    &mailbox,
                    actor_for_start.as_ref(),
                    streaming_config,
                    &input,
                    Some(task_id),
                    &worker_token,
                );
                let _ = start_tx.send(result);
            })
            .map_err(|error| WorkflowError::new("E_RECORD_START_WORKER", error.to_string()))?;
        let remaining = begin_deadline.saturating_sub(begin_started.elapsed());
        let session_id = match start_rx.recv_timeout(remaining) {
            Ok(result) => result.map_err(|error| WorkflowError::new(&error.code, error.message))?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                start_token.cancel();
                if let Some(actor) = actor.as_ref() {
                    let _ = actor.cancel_session(&self.run_id);
                }
                return Err(WorkflowError::new(
                    "E_EXECUTOR_BEGIN_TIMEOUT",
                    "recording capture did not start within the Begin deadline",
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(WorkflowError::new(
                    "E_RECORD_START_WORKER",
                    "recording start worker exited without a result",
                ));
            }
        };
        let insertion_target = match target_rx {
            Some(target_rx) => {
                let remaining = begin_deadline.saturating_sub(begin_started.elapsed());
                match target_rx.recv_timeout(remaining) {
                    Ok(result) => Some(result?),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        start_token.cancel();
                        return Err(WorkflowError::new(
                            "E_EXECUTOR_BEGIN_TIMEOUT",
                            "insertion target was not captured within the Begin deadline",
                        ));
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(WorkflowError::new(
                            "E_EXPORT_TARGET_WORKER",
                            "insertion target worker exited without a result",
                        ));
                    }
                }
            }
            None => None,
        };
        {
            let mut state = self.state.lock().unwrap();
            state.recording_session_id = Some(session_id);
            state.insertion_target = insertion_target;
        }

        let capture_started_at_ms = now_ms();
        if token.is_cancelled() || begin_started.elapsed() >= begin_deadline {
            start_token.cancel();
            if let Some(actor) = actor.as_ref() {
                let _ = actor.cancel_session(&self.run_id);
            }
            return Err(WorkflowError::new(
                "E_EXECUTOR_BEGIN_TIMEOUT",
                "recording capture did not start within the Begin deadline",
            ));
        }
        if let Some(pending_session) = pending_session {
            let remaining = begin_deadline.saturating_sub(begin_started.elapsed());
            if let Err(error) = pending_session.wait_timeout(remaining) {
                if let Some(actor) = actor.as_ref() {
                    let _ = actor.cancel_session(&self.run_id);
                }
                return Err(workflow_error("E_STREAMING_START", error));
            }
        }

        Ok(BeginAccepted {
            capture_started_at_ms,
        })
    }

    fn capture_context(&self) -> PortFuture<()> {
        let task_manager = self.task_manager.clone();
        let state = self.state.clone();
        let operations = self.operations.clone();
        let config = context_config(&self.seed.context, self.seed.rewrite.supports_vision);
        let cancellation = self.state.lock().unwrap().cancellation.clone();
        Box::pin(async move {
            let cancellation = cancellation.ok_or_else(|| {
                WorkflowError::new(
                    "E_RUN_CANCELLATION_MISSING",
                    "run cancellation token is missing",
                )
            })?;
            let dir = data_dir::data_dir().map_err(|error| workflow_error("E_DATA_DIR", error))?;
            let lease = operations.begin();
            let snapshot = tauri::async_runtime::spawn_blocking(move || {
                let _lease = lease;
                task_manager.capture_hotkey_context(&dir, &config, &cancellation)
            })
            .await
            .map_err(|error| WorkflowError::new("E_CONTEXT_JOIN", error.to_string()))?
            .map_err(|error| workflow_error("E_CONTEXT_CAPTURE", error))?;
            let mut guard = state.lock().unwrap();
            guard.context = Some(snapshot);
            Ok(())
        })
    }

    fn finish_recording(&self) -> PortFuture<u128> {
        let recording = self.recording.clone();
        let state = self.state.clone();
        let operations = self.operations.clone();
        Box::pin(async move {
            let session_id = state
                .lock()
                .unwrap()
                .recording_session_id
                .clone()
                .ok_or_else(|| {
                    WorkflowError::new("E_RECORD_SESSION_MISSING", "recording session is missing")
                })?;
            let cancellation = state.lock().unwrap().cancellation.clone().ok_or_else(|| {
                WorkflowError::new(
                    "E_RUN_CANCELLATION_MISSING",
                    "run cancellation token is missing",
                )
            })?;
            let stop_recording = recording.clone();
            let lease = operations.begin();
            let outcome = tauri::async_runtime::spawn_blocking(move || {
                let _lease = lease;
                stop_recording.stop_recording(&session_id, &cancellation)
            })
            .await
            .map_err(|error| WorkflowError::new("E_RECORD_STOP_JOIN", error.to_string()))?
            .map_err(|error| WorkflowError::new(&error.code, error.message))?;
            match outcome {
                RecordingStopOutcome::Completed(asset) => {
                    let asset = recording.take_asset(&asset.asset_id).unwrap_or(asset);
                    let elapsed = asset.record_elapsed_ms;
                    let mut guard = state.lock().unwrap();
                    guard.recording_session_id = None;
                    guard.asset = Some(asset);
                    Ok(elapsed)
                }
                RecordingStopOutcome::Stale => Err(WorkflowError::new(
                    "E_RECORD_SESSION_STALE",
                    "recording session was no longer active",
                )),
            }
        })
    }

    fn preprocess(&self) -> PortFuture<u128> {
        if self.actor.lock().unwrap().is_some() {
            return Box::pin(async { Ok(0) });
        }
        let run_id = self.run_id.clone();
        let transcriber = self.transcriber.clone();
        let state = self.state.clone();
        let operations = self.operations.clone();
        Box::pin(async move {
            let asset = state.lock().unwrap().asset.clone().ok_or_else(|| {
                WorkflowError::new("E_RECORD_ASSET_MISSING", "recorded asset is missing")
            })?;
            let lease = operations.begin();
            let prepare = tauri::async_runtime::spawn(async move {
                transcriber
                    .prepare_audio(TranscriptionInput {
                        task_id: Some(run_id),
                        input_path: asset.output_path,
                        record_elapsed_ms: asset.record_elapsed_ms,
                        record_label: "ffmpeg".to_string(),
                    })
                    .await
                    .map(|value| TrackedOutput { value, lease })
            });
            let tracked = prepare
                .await
                .map_err(|error| WorkflowError::new("E_PREPROCESS_JOIN", error.to_string()))?
                .map_err(|error| WorkflowError::new(&error.code, error.message))?;
            let elapsed = tracked.value.preprocess_ms();
            state.lock().unwrap().prepared = Some(tracked.value);
            drop(tracked.lease);
            Ok(elapsed)
        })
    }

    fn transcribe(&self) -> PortFuture<TranscriptionResult> {
        let run_id = self.run_id.clone();
        let actor = self.actor.lock().unwrap().clone();
        let transcriber = self.transcriber.clone();
        let state = self.state.clone();
        let operations = self.operations.clone();
        Box::pin(async move {
            if let Some(actor) = actor {
                let task_id = run_id.clone();
                let lease = operations.begin();
                return tauri::async_runtime::spawn_blocking(move || {
                    let _lease = lease;
                    actor.finish_session(&task_id)
                })
                .await
                .map_err(|error| WorkflowError::new("E_STREAMING_FINISH_JOIN", error.to_string()))?
                .map_err(|error| workflow_error("E_STREAMING_TRANSCRIBE_FINISH", error));
            }

            let prepared = state.lock().unwrap().prepared.take().ok_or_else(|| {
                WorkflowError::new(
                    "E_PREPROCESSED_AUDIO_MISSING",
                    "preprocessed transcription input is missing",
                )
            })?;
            let lease = operations.begin();
            tauri::async_runtime::spawn(async move {
                let _lease = lease;
                transcriber.transcribe_prepared(prepared).await
            })
            .await
            .map_err(|error| WorkflowError::new("E_TRANSCRIBE_JOIN", error.to_string()))?
            .map_err(|error| WorkflowError::new(&error.code, error.message))
        })
    }

    fn rewrite(
        &self,
        result: RecoveredRunResult,
    ) -> PortFuture<typevoice_core::workflow::RewriteResult> {
        let run_id = self.run_id.clone();
        let rewrite_plan = self.seed.rewrite.clone();
        let context_plan = self.seed.context.clone();
        let context = self.state.lock().unwrap().context.clone();
        let operations = self.operations.clone();
        Box::pin(async move {
            let _lease = operations.begin();
            rewrite::rewrite_text_with_plan(
                context,
                RewriteTextRequest {
                    transcript_id: run_id,
                    text: result.asr_text,
                },
                &rewrite_plan,
                &context_plan,
            )
            .await
            .map_err(|error| WorkflowError::new(&error.code, error.message))
        })
    }

    fn prepare_insertion(&self, text: String) -> PortFuture<InsertPrepareResult> {
        let target = if self.state.lock().unwrap().insertion_target.is_some() {
            "externalWindow"
        } else {
            "currentFocus"
        }
        .to_string();
        Box::pin(async move {
            if text.trim().is_empty() {
                return Err(WorkflowError::new(
                    "E_EXPORT_EMPTY_TEXT",
                    "empty text cannot be exported",
                ));
            }
            Ok(InsertPrepareResult {
                target,
                text_digest: fnv1a_digest(text.as_bytes()),
            })
        })
    }

    fn commit_history(&self, result: RecoveredRunResult) -> PortFuture<()> {
        let run_id = self.run_id.clone();
        let operations = self.operations.clone();
        let effects = self.effects.clone();
        Box::pin(async move {
            let dir = data_dir::data_dir().map_err(|error| workflow_error("E_DATA_DIR", error))?;
            let metrics = result.metrics.clone().unwrap_or_default();
            let rewritten_text = if result.final_text != result.asr_text {
                result.final_text.clone()
            } else {
                String::new()
            };
            let item = history::HistoryItem {
                task_id: run_id,
                created_at_ms: now_ms() as i64,
                asr_text: result.asr_text,
                rewritten_text,
                inserted_text: result.final_text.clone(),
                final_text: result.final_text,
                template_id: None,
                rtf: metrics.rtf,
                device_used: metrics.device_used,
                preprocess_ms: metrics.preprocess_ms as i64,
                asr_ms: metrics.asr_ms as i64,
            };
            let lease = operations.begin();
            tauri::async_runtime::spawn_blocking(move || {
                let _lease = lease;
                let result = history::append(&dir.join("history.sqlite3"), &item);
                if result.is_ok() {
                    effects.history_commit_count.store(1, Ordering::Release);
                }
                result
            })
            .await
            .map_err(|error| WorkflowError::new("E_HISTORY_JOIN", error.to_string()))?
            .map_err(|error| workflow_error("E_HISTORY_APPEND", error))
        })
    }

    fn copy_text(&self, text: String) -> PortFuture<()> {
        let operations = self.operations.clone();
        let effects = self.effects.clone();
        Box::pin(async move {
            let lease = operations.begin();
            tauri::async_runtime::spawn_blocking(move || {
                let _lease = lease;
                let result = export::copy_text_to_clipboard(&text);
                if result.is_ok() {
                    effects.copy_count.store(1, Ordering::Release);
                }
                result
            })
            .await
            .map_err(|error| WorkflowError::new("E_EXPORT_COPY_JOIN", error.to_string()))?
            .map_err(|error| WorkflowError::new(&error.code, error.message))
        })
    }

    fn paste_text(&self, text: String) -> PortFuture<()> {
        #[allow(clippy::clone_on_copy)] // Linux AT-SPI targets are clone-only.
        let target = self.state.lock().unwrap().insertion_target.clone();
        let operations = self.operations.clone();
        let effects = self.effects.clone();
        Box::pin(async move {
            let _lease = operations.begin();
            let target = target.ok_or_else(|| {
                WorkflowError::new(
                    "E_EXPORT_TARGET_MISSING",
                    "no frozen insertion target is available for this run",
                )
            })?;
            let result = export::auto_paste_text(&target, &text)
                .await
                .map_err(|error| WorkflowError::new(&error.code, error.message));
            if result.is_ok() {
                effects.paste_count.store(1, Ordering::Release);
            }
            result
        })
    }

    fn observed_effects(&self) -> EffectCounts {
        self.effects.snapshot()
    }

    fn shutdown(&self) -> PortFuture<bool> {
        self.cleanup(false)
    }

    fn force_shutdown(&self) -> PortFuture<bool> {
        self.cleanup(true)
    }
}

impl RuntimeRunPorts {
    fn cleanup(&self, wait_for_operations: bool) -> PortFuture<bool> {
        let run_id = self.run_id.clone();
        let recording = self.recording.clone();
        let transcriber = self.transcriber.clone();
        let actor = self.actor.lock().unwrap().clone();
        let state = self.state.clone();
        let operations = self.operations.clone();
        Box::pin(async move {
            {
                let mut guard = state.lock().unwrap();
                if guard.shutdown_complete && operations.is_idle() {
                    return Ok(true);
                }
                if guard.cleanup_in_progress {
                    return Ok(false);
                }
                guard.cleanup_in_progress = true;
            }
            let _attempt = CleanupAttempt::new(state.clone());
            let released = run_cleanup_sweep(
                &run_id,
                &recording,
                &transcriber,
                actor.as_ref(),
                &state,
                &operations,
            )
            .await?;
            if !wait_for_operations {
                if !released || !operations.is_idle() {
                    return Ok(false);
                }
                state.lock().unwrap().shutdown_complete = true;
                return Ok(true);
            }
            if !operations.is_idle() {
                operations.wait_idle().await;
            }
            let released = run_cleanup_sweep(
                &run_id,
                &recording,
                &transcriber,
                actor.as_ref(),
                &state,
                &operations,
            )
            .await?;
            if released && operations.is_idle() {
                state.lock().unwrap().shutdown_complete = true;
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }
}

struct CleanupAttempt {
    state: Arc<Mutex<RuntimePortState>>,
}

impl CleanupAttempt {
    fn new(state: Arc<Mutex<RuntimePortState>>) -> Self {
        Self { state }
    }
}

impl Drop for CleanupAttempt {
    fn drop(&mut self) {
        self.state.lock().unwrap().cleanup_in_progress = false;
    }
}

async fn run_cleanup_sweep(
    run_id: &str,
    recording: &RecordingRegistry,
    transcriber: &TranscriptionService,
    actor: Option<&TranscriptionActor>,
    state: &Arc<Mutex<RuntimePortState>>,
    operations: &OperationTracker,
) -> Result<bool, WorkflowError> {
    let run_id = run_id.to_string();
    let recording = recording.clone();
    let transcriber = transcriber.clone();
    let actor = actor.cloned();
    let state = state.clone();
    let lease = operations.begin();
    tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let (session_id, asset, mut prepared) = {
            let mut guard = state.lock().unwrap();
            (
                guard.recording_session_id.take(),
                guard.asset.take(),
                guard.prepared.take(),
            )
        };
        let mut released = recording.abort_recording(session_id).is_ok();
        if transcriber.cancel(Some(&run_id)).is_err() {
            released = false;
        }
        if let Some(actor) = actor {
            let _ = actor.cancel_session(&run_id);
            if actor.shutdown().is_err() {
                released = false;
            }
        }
        let mut registry_assets = recording.take_assets_for_task(&run_id);
        let data_dir = data_dir::data_dir().ok();
        if let Some(dir) = data_dir.as_ref() {
            for asset in registry_assets.drain(..) {
                if pipeline::cleanup_input_audio_artifact(&asset.output_path, dir).is_err() {
                    recording.restore_asset(asset);
                    released = false;
                }
            }
            if let Some(asset) = asset {
                if pipeline::cleanup_input_audio_artifact(&asset.output_path, dir).is_err() {
                    state.lock().unwrap().asset = Some(asset);
                    released = false;
                }
            }
            if pipeline::cleanup_preprocess_audio_artifact(dir, &run_id).is_err() {
                released = false;
            }
        } else {
            for asset in registry_assets {
                recording.restore_asset(asset);
            }
            if let Some(asset) = asset {
                state.lock().unwrap().asset = Some(asset);
            }
            released = false;
        }
        if let Some(prepared) = prepared.as_mut() {
            if prepared.cleanup().is_err() {
                released = false;
            }
        }
        if !released {
            if let Some(prepared) = prepared {
                state.lock().unwrap().prepared = Some(prepared);
            }
        }
        released
    })
    .await
    .map_err(|error| WorkflowError::new("E_CLEANUP_JOIN", error.to_string()))
}

fn context_config(plan: &ContextPlan, supports_vision: bool) -> context_capture::ContextConfig {
    let budget = ContextBudget {
        max_history_items: plan.history_n,
        history_window_ms: plan.history_window_ms,
        ..ContextBudget::default()
    };
    context_capture::ContextConfig {
        include_history: plan.include_history,
        include_clipboard: plan.include_clipboard,
        include_prev_window_meta: plan.include_prev_window_meta,
        include_prev_window_screenshot: plan.include_prev_window_screenshot,
        budget,
        llm_supports_vision: supports_vision,
    }
}

fn workflow_error(code: &str, error: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::new(code, error.to_string())
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn fnv1a_digest(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_tracker_waits_until_the_last_lease_is_released() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let tracker = OperationTracker::default();
            let lease = tracker.begin();
            let release = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(10));
                drop(lease);
            });

            tokio::time::timeout(Duration::from_secs(1), tracker.wait_idle())
                .await
                .expect("tracker reaches idle");
            release.join().expect("lease thread");
            assert!(tracker.is_idle());
        });
    }
}
