use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock, Weak,
};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use typevoice_core::workflow::{
    CancelDisposition, CleanupDiagnostic, CompletedRunResult, EffectCounts, FinalizationAudit,
    InsertPrepareProgress, InsertPrepareResult, InsertResult, Progress, ProgressPayload,
    RecoveredRunResult, RewriteProgress, RewriteResult, RunAudit, RunId, RunPlanSeed, RunTimings,
    StageKind, StageProgress, StageStatus, Stopped, StoppedTerminal, TranscribeProgress,
    TranscriptionResult, WorkflowError,
};

pub const START_DEADLINE_MS: u64 = 200;
pub const CANCEL_DEADLINE_MS: u64 = 300;
const FORCE_CLEANUP_RESERVE_MS: u64 = 100;
const FATAL_TRACE_RESERVE_MS: u64 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArbiterWinner {
    Cancel,
    Terminal,
    Finalization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArbiterSnapshot {
    pub winner: Option<ArbiterWinner>,
    pub cancel_winner: bool,
    pub launched_resources: Vec<String>,
    pub launched_stages: Vec<StageKind>,
    pub terminal_claims: u32,
    pub finalization_claims: u32,
    pub cancel_requests: u32,
}

#[derive(Default)]
struct ArbiterState {
    winner: Option<ArbiterWinner>,
    launched_resources: BTreeSet<String>,
    launched_stages: BTreeSet<String>,
    terminal_claims: u32,
    finalization_claims: u32,
    cancel_requests: u32,
}

pub struct RunArbiter {
    state: Mutex<ArbiterState>,
    token: CancellationToken,
    cleanup_deadline: OnceLock<Instant>,
}

impl RunArbiter {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            state: Mutex::new(ArbiterState::default()),
            token,
            cleanup_deadline: OnceLock::new(),
        }
    }

    pub fn begin_resource(&self, resource: impl Into<String>) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.winner == Some(ArbiterWinner::Cancel) {
            return false;
        }
        state.launched_resources.insert(resource.into());
        true
    }

    pub fn begin_stage(&self, stage: StageKind) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.winner == Some(ArbiterWinner::Cancel) {
            return false;
        }
        state.launched_stages.insert(stage_name(stage).to_string());
        true
    }

    pub fn begin_terminal(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.terminal_claims = state.terminal_claims.saturating_add(1);
        match state.winner {
            None => {
                state.winner = Some(ArbiterWinner::Terminal);
                true
            }
            Some(ArbiterWinner::Terminal) => true,
            Some(ArbiterWinner::Cancel | ArbiterWinner::Finalization) => false,
        }
    }

    pub fn begin_finalization(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.finalization_claims = state.finalization_claims.saturating_add(1);
        match state.winner {
            None => {
                state.winner = Some(ArbiterWinner::Finalization);
                true
            }
            Some(ArbiterWinner::Finalization) => true,
            Some(ArbiterWinner::Cancel | ArbiterWinner::Terminal) => false,
        }
    }

    pub fn request_cancel(&self) -> CancelDisposition {
        let disposition = {
            let mut state = self.state.lock().unwrap();
            state.cancel_requests = state.cancel_requests.saturating_add(1);
            match state.winner {
                Some(ArbiterWinner::Terminal | ArbiterWinner::Finalization) => {
                    CancelDisposition::TooLate
                }
                Some(ArbiterWinner::Cancel) => {
                    self.ensure_cleanup_deadline();
                    CancelDisposition::Accepted
                }
                None => {
                    self.ensure_cleanup_deadline();
                    state.winner = Some(ArbiterWinner::Cancel);
                    CancelDisposition::Accepted
                }
            }
        };
        if disposition == CancelDisposition::Accepted {
            self.token.cancel();
        }
        disposition
    }

    fn ensure_cleanup_deadline(&self) {
        let _ = self
            .cleanup_deadline
            .set(Instant::now() + Duration::from_millis(CANCEL_DEADLINE_MS));
    }

    fn cleanup_deadline(&self) -> Instant {
        *self
            .cleanup_deadline
            .get_or_init(|| Instant::now() + Duration::from_millis(CANCEL_DEADLINE_MS))
    }

    pub fn winner(&self) -> Option<ArbiterWinner> {
        self.state.lock().unwrap().winner
    }

    pub fn snapshot(&self) -> ArbiterSnapshot {
        let state = self.state.lock().unwrap();
        ArbiterSnapshot {
            winner: state.winner,
            cancel_winner: state.winner == Some(ArbiterWinner::Cancel),
            launched_resources: state.launched_resources.iter().cloned().collect(),
            launched_stages: state
                .launched_stages
                .iter()
                .filter_map(|stage| stage_from_name(stage))
                .collect(),
            terminal_claims: state.terminal_claims,
            finalization_claims: state.finalization_claims,
            cancel_requests: state.cancel_requests,
        }
    }
}

impl Default for RunArbiter {
    fn default() -> Self {
        Self::new(CancellationToken::new())
    }
}

fn stage_name(stage: StageKind) -> &'static str {
    match stage {
        StageKind::ContextCapture => "contextCapture",
        StageKind::RecordFinalize => "recordFinalize",
        StageKind::Preprocess => "preprocess",
        StageKind::Transcribe => "transcribe",
        StageKind::Rewrite => "rewrite",
        StageKind::InsertPrepare => "insertPrepare",
        StageKind::Finalize => "finalize",
    }
}

fn stage_from_name(value: &str) -> Option<StageKind> {
    match value {
        "contextCapture" => Some(StageKind::ContextCapture),
        "recordFinalize" => Some(StageKind::RecordFinalize),
        "preprocess" => Some(StageKind::Preprocess),
        "transcribe" => Some(StageKind::Transcribe),
        "rewrite" => Some(StageKind::Rewrite),
        "insertPrepare" => Some(StageKind::InsertPrepare),
        "finalize" => Some(StageKind::Finalize),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCounts {
    pub live: u32,
    pub run_handle: u32,
    pub ffmpeg: u32,
    pub provider_request: u32,
    pub cancellation_token: u32,
    pub temporary_asset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunHandleInspection {
    pub trace: Vec<String>,
    pub arbiter: ArbiterSnapshot,
    pub resource_counts: ResourceCounts,
    pub begin_count: u32,
    pub stop_count: u32,
    pub stopped_count: u32,
}

impl Default for RunHandleInspection {
    fn default() -> Self {
        Self {
            trace: Vec::new(),
            arbiter: RunArbiter::default().snapshot(),
            resource_counts: ResourceCounts::default(),
            begin_count: 0,
            stop_count: 0,
            stopped_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeginAccepted {
    pub capture_started_at_ms: u64,
}

pub trait RunSignalSink: Send + Sync {
    fn progress(&self, progress: Progress);
    fn stopped(&self, stopped: Stopped);
    fn fatal(
        &self,
        run_id: &str,
        error: WorkflowError,
        context: serde_json::Value,
        deadline: Instant,
    );
}

pub trait RunHandle: Send + Sync {
    fn run_id(&self) -> &str;
    fn begin(&self) -> Result<BeginAccepted, WorkflowError>;
    fn stop(&self) -> Result<(), WorkflowError>;
    fn settle_control_failure(&self, error: WorkflowError);
    fn request_cancel(&self) -> CancelDisposition;
    fn inspect(&self) -> RunHandleInspection;
}

pub trait RunFactory: Send + Sync {
    fn create(
        &self,
        run_id: RunId,
        seed: RunPlanSeed,
        signal_sink: Arc<dyn RunSignalSink>,
    ) -> Arc<dyn RunHandle>;
}

pub type PortFuture<T> = Pin<Box<dyn Future<Output = Result<T, WorkflowError>> + Send + 'static>>;

pub trait RunPorts: Send + Sync {
    fn begin_recording(&self, token: CancellationToken) -> Result<BeginAccepted, WorkflowError>;
    fn capture_context(&self) -> PortFuture<()>;
    fn finish_recording(&self) -> PortFuture<u128>;
    fn preprocess(&self) -> PortFuture<u128>;
    fn transcribe(&self) -> PortFuture<TranscriptionResult>;
    fn rewrite(&self, result: RecoveredRunResult) -> PortFuture<RewriteResult>;
    fn prepare_insertion(&self, text: String) -> PortFuture<InsertPrepareResult>;
    fn commit_history(&self, result: RecoveredRunResult) -> PortFuture<()>;
    fn copy_text(&self, text: String) -> PortFuture<()>;
    fn paste_text(&self, text: String) -> PortFuture<()>;
    fn observed_effects(&self) -> EffectCounts {
        EffectCounts::default()
    }
    fn shutdown(&self) -> PortFuture<bool>;
    fn force_shutdown(&self) -> PortFuture<bool>;
}

pub trait RunPortsFactory: Send + Sync {
    fn create(&self, run_id: &str, seed: &RunPlanSeed) -> Arc<dyn RunPorts>;
}

pub trait ExecutorTaskHandle: Send + Sync {
    fn abort(&self);
}

pub trait ExecutorSpawner: Send + Sync {
    fn spawn(
        &self,
        future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
    ) -> Arc<dyn ExecutorTaskHandle>;
}

pub struct TauriExecutorSpawner;

struct TauriTaskHandle {
    join: tauri::async_runtime::JoinHandle<()>,
}

impl ExecutorTaskHandle for TauriTaskHandle {
    fn abort(&self) {
        self.join.abort();
    }
}

impl ExecutorSpawner for TauriExecutorSpawner {
    fn spawn(
        &self,
        future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
    ) -> Arc<dyn ExecutorTaskHandle> {
        Arc::new(TauriTaskHandle {
            join: tauri::async_runtime::spawn(future),
        })
    }
}

pub struct RunExecutorFactory {
    ports: Arc<dyn RunPortsFactory>,
    spawner: Arc<dyn ExecutorSpawner>,
}

impl RunExecutorFactory {
    pub fn new(ports: Arc<dyn RunPortsFactory>, spawner: Arc<dyn ExecutorSpawner>) -> Self {
        Self { ports, spawner }
    }
}

impl RunFactory for RunExecutorFactory {
    fn create(
        &self,
        run_id: RunId,
        seed: RunPlanSeed,
        signal_sink: Arc<dyn RunSignalSink>,
    ) -> Arc<dyn RunHandle> {
        let ports = self.ports.create(&run_id, &seed);
        RunExecutorHandle::dormant(run_id, seed, ports, signal_sink, self.spawner.clone())
    }
}

enum RunControl {
    Stop,
    Cancel,
}

struct ExecutorInspection {
    trace: Vec<String>,
    resources: ResourceCounts,
    begin_count: u32,
    stop_count: u32,
    stopped_count: u32,
}

#[derive(Clone, Default)]
struct ExecutorCheckpoint {
    recovered: Option<RecoveredRunResult>,
    primary_error: Option<WorkflowError>,
    recovery_errors: Vec<WorkflowError>,
    record_saved: bool,
    audit: RunAudit,
}

impl Default for ExecutorInspection {
    fn default() -> Self {
        Self {
            trace: Vec::new(),
            resources: ResourceCounts {
                live: 0,
                run_handle: 1,
                ffmpeg: 0,
                provider_request: 0,
                cancellation_token: 1,
                temporary_asset: 0,
            },
            begin_count: 0,
            stop_count: 0,
            stopped_count: 0,
        }
    }
}

pub struct RunExecutorHandle {
    self_weak: Weak<RunExecutorHandle>,
    run_id: RunId,
    seed: RunPlanSeed,
    ports: Arc<dyn RunPorts>,
    sink: Arc<dyn RunSignalSink>,
    spawner: Arc<dyn ExecutorSpawner>,
    arbiter: Arc<RunArbiter>,
    token: CancellationToken,
    terminalization: Arc<CompletionGuard>,
    completion: Arc<CompletionGuard>,
    control_failure_settled: AtomicBool,
    control: Mutex<Option<tokio::sync::mpsc::UnboundedSender<RunControl>>>,
    tasks: Mutex<Vec<Arc<dyn ExecutorTaskHandle>>>,
    inspection: Mutex<ExecutorInspection>,
    checkpoint: Mutex<ExecutorCheckpoint>,
    began: AtomicBool,
    stopped: AtomicBool,
    terminal_emitted: AtomicBool,
}

impl RunExecutorHandle {
    pub fn dormant(
        run_id: RunId,
        seed: RunPlanSeed,
        ports: Arc<dyn RunPorts>,
        sink: Arc<dyn RunSignalSink>,
        spawner: Arc<dyn ExecutorSpawner>,
    ) -> Arc<Self> {
        let token = CancellationToken::new();
        Arc::new_cyclic(|self_weak| Self {
            self_weak: self_weak.clone(),
            run_id,
            seed,
            ports,
            sink,
            spawner,
            arbiter: Arc::new(RunArbiter::new(token.clone())),
            token,
            terminalization: Arc::new(CompletionGuard::new()),
            completion: Arc::new(CompletionGuard::new()),
            control_failure_settled: AtomicBool::new(false),
            control: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
            inspection: Mutex::new(ExecutorInspection::default()),
            checkpoint: Mutex::new(ExecutorCheckpoint::default()),
            began: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            terminal_emitted: AtomicBool::new(false),
        })
    }

    pub fn arbiter(&self) -> Arc<RunArbiter> {
        self.arbiter.clone()
    }

    #[cfg(test)]
    pub(crate) fn close_control_for_test(&self) {
        self.control.lock().unwrap().take();
    }

    #[cfg(test)]
    pub(crate) async fn recover_abnormal_exit_for_test(self: Arc<Self>, message: &str) {
        recover_abnormal_exit(self, message).await;
    }

    fn record_trace(&self, value: impl Into<String>) {
        self.inspection.lock().unwrap().trace.push(value.into());
    }

    fn cleanup_deadline(&self) -> Instant {
        self.arbiter.cleanup_deadline()
    }

    #[cfg(not(test))]
    fn trace_event(&self, step_id: &str, status: &str, context: serde_json::Value) {
        if let Ok(dir) = crate::data_dir::data_dir() {
            crate::obs::event(
                &dir,
                Some(&self.run_id),
                "Run",
                step_id,
                status,
                Some(context),
            );
        }
    }

    #[cfg(test)]
    fn trace_event(&self, _step_id: &str, _status: &str, _context: serde_json::Value) {}

    fn begin_terminal(&self) -> bool {
        let previous = self.arbiter.winner();
        let accepted = self.arbiter.begin_terminal();
        if accepted && previous.is_none() {
            self.trace_event(
                "run.terminal_won",
                "won",
                serde_json::json!({"winner": "terminal"}),
            );
        }
        accepted
    }

    fn begin_finalization(&self) -> bool {
        let previous = self.arbiter.winner();
        let accepted = self.arbiter.begin_finalization();
        if accepted && previous.is_none() {
            self.trace_event(
                "run.finalization_won",
                "won",
                serde_json::json!({"winner": "finalization"}),
            );
        }
        accepted
    }

    fn set_resources_started(&self) {
        let mut inspection = self.inspection.lock().unwrap();
        inspection.resources.live = 1;
        inspection.resources.ffmpeg = 1;
        inspection.resources.provider_request = u32::from(self.seed.asr.provider == "doubao");
    }

    fn set_resources_released(&self) {
        let mut inspection = self.inspection.lock().unwrap();
        inspection.resources = ResourceCounts::default();
    }

    fn checkpoint_recovered(&self, recovered: RecoveredRunResult) {
        self.checkpoint.lock().unwrap().recovered = Some(recovered);
    }

    fn checkpoint_finalization(
        &self,
        recovered: RecoveredRunResult,
        primary_error: Option<WorkflowError>,
        recovery_errors: Vec<WorkflowError>,
        record_saved: bool,
        audit: RunAudit,
    ) {
        *self.checkpoint.lock().unwrap() = ExecutorCheckpoint {
            recovered: Some(recovered),
            primary_error,
            recovery_errors,
            record_saved,
            audit,
        };
    }

    fn checkpoint_failure(
        &self,
        recovered: Option<RecoveredRunResult>,
        primary_error: WorkflowError,
        recovery_errors: Vec<WorkflowError>,
        record_saved: bool,
        audit: RunAudit,
    ) {
        *self.checkpoint.lock().unwrap() = ExecutorCheckpoint {
            recovered,
            primary_error: Some(primary_error),
            recovery_errors,
            record_saved,
            audit,
        };
    }

    fn emit_progress(&self, payload: ProgressPayload) {
        self.sink.progress(Progress {
            run_id: self.run_id.clone(),
            payload,
        });
    }

    fn emit_stopped(&self, terminal: StoppedTerminal, audit: RunAudit) {
        if self.terminal_emitted.swap(true, Ordering::AcqRel) {
            return;
        }
        self.completion.claim();
        self.inspection.lock().unwrap().stopped_count = 1;
        self.sink.stopped(Stopped {
            run_id: self.run_id.clone(),
            terminal,
            audit,
        });
    }
}

impl RunHandle for RunExecutorHandle {
    fn run_id(&self) -> &str {
        &self.run_id
    }

    fn begin(&self) -> Result<BeginAccepted, WorkflowError> {
        if self.began.swap(true, Ordering::AcqRel) {
            return Err(WorkflowError::new(
                "E_EXECUTOR_BEGIN_DUPLICATE",
                "run executor Begin was already delivered",
            ));
        }
        if !self.arbiter.begin_resource("recording") {
            return Err(WorkflowError::new(
                "E_CANCELLED",
                "run was cancelled before recording resource launch",
            ));
        }
        if self.seed.asr.provider == "doubao" && !self.arbiter.begin_resource("providerSession") {
            return Err(WorkflowError::new(
                "E_CANCELLED",
                "run was cancelled before provider resource launch",
            ));
        }
        self.record_trace("begin");
        self.trace_event(
            "run.begin",
            "accepted",
            serde_json::json!({
                "provider": self.seed.asr.provider,
                "settingsRevision": self.seed.settings_revision,
                "intentSource": self.seed.intent_source,
            }),
        );
        self.inspection.lock().unwrap().begin_count = 1;
        let accepted = self.ports.begin_recording(self.token.clone())?;
        self.set_resources_started();
        let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
        *self.control.lock().unwrap() = Some(control_tx);
        let executor = self.self_weak.upgrade().ok_or_else(|| {
            WorkflowError::new(
                "E_EXECUTOR_HANDLE_DROPPED",
                "run executor handle was dropped before Begin",
            )
        })?;
        let task = self.spawner.spawn(Box::pin(async move {
            run_executor_supervised(executor, control_rx).await;
        }));
        self.tasks.lock().unwrap().push(task);
        Ok(accepted)
    }

    fn stop(&self) -> Result<(), WorkflowError> {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.record_trace("stop");
        self.inspection.lock().unwrap().stop_count = 1;
        let control = self.control.lock().unwrap().clone().ok_or_else(|| {
            WorkflowError::new(
                "E_EXECUTOR_STOP_DELIVERY",
                "run control channel is not available",
            )
        })?;
        control.send(RunControl::Stop).map_err(|_| {
            WorkflowError::new(
                "E_EXECUTOR_STOP_DELIVERY",
                "failed to deliver Stop to run executor",
            )
        })
    }

    fn settle_control_failure(&self, error: WorkflowError) {
        if self.completion.is_completed()
            || self.control_failure_settled.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let Some(executor) = self.self_weak.upgrade() else {
            return;
        };
        let task = self.spawner.spawn(Box::pin(async move {
            terminalize_control_failure_supervised(executor, error).await;
        }));
        self.tasks.lock().unwrap().push(task);
    }

    fn request_cancel(&self) -> CancelDisposition {
        let disposition = self.arbiter.request_cancel();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.trace_event(
                "run.cancel_arbitrated",
                "decided",
                serde_json::json!({
                    "disposition": match disposition {
                        CancelDisposition::Accepted => "accepted",
                        CancelDisposition::TooLate => "tooLate",
                    },
                    "winner": self.arbiter.winner(),
                }),
            );
            if disposition == CancelDisposition::Accepted {
                self.record_trace("cancelAccepted");
                if let Some(control) = self.control.lock().unwrap().as_ref() {
                    let _ = control.send(RunControl::Cancel);
                }
            } else {
                self.record_trace("cancelTooLate");
            }
        }));
        disposition
    }

    fn inspect(&self) -> RunHandleInspection {
        let inspection = self.inspection.lock().unwrap();
        RunHandleInspection {
            trace: inspection.trace.clone(),
            arbiter: self.arbiter.snapshot(),
            resource_counts: inspection.resources,
            begin_count: inspection.begin_count,
            stop_count: inspection.stop_count,
            stopped_count: inspection.stopped_count,
        }
    }
}

async fn run_executor_supervised(
    executor: Arc<RunExecutorHandle>,
    control: tokio::sync::mpsc::UnboundedReceiver<RunControl>,
) {
    use futures_util::FutureExt;

    let result = std::panic::AssertUnwindSafe(run_executor(executor.clone(), control))
        .catch_unwind()
        .await;
    if !executor.completion.is_completed() {
        let message = if result.is_err() {
            "run executor panicked"
        } else {
            "run executor exited without a terminal signal"
        };
        recover_abnormal_exit(executor, message).await;
    }
}

async fn terminalize_control_failure_supervised(
    executor: Arc<RunExecutorHandle>,
    error: WorkflowError,
) {
    use futures_util::FutureExt;

    let recovery = executor.clone();
    let result =
        std::panic::AssertUnwindSafe(finish_failed(executor, error, None, Vec::new(), false))
            .catch_unwind()
            .await;
    if result.is_err() && !recovery.completion.is_completed() {
        recover_abnormal_exit(recovery, "control failure terminalization panicked").await;
    }
}

async fn recover_abnormal_exit(executor: Arc<RunExecutorHandle>, message: &str) {
    use futures_util::FutureExt;

    let recovery = executor.clone();
    let abnormal = WorkflowError::new("E_EXECUTOR_ABNORMAL_EXIT", message);
    let checkpoint = executor.checkpoint.lock().unwrap().clone();
    let cancel_won = executor.arbiter.winner() == Some(ArbiterWinner::Cancel);
    let result = std::panic::AssertUnwindSafe(async move {
        if cancel_won {
            let diagnostic = CleanupDiagnostic {
                detail: Some(format!("{}: {}", abnormal.code, abnormal.message)),
                ..Default::default()
            };
            finish_cancelled_with_diagnostic(executor, checkpoint.recovered, diagnostic).await;
        } else {
            let (error, recovery_errors) = match checkpoint.primary_error {
                Some(primary_error) => {
                    let mut recovery_errors = checkpoint.recovery_errors;
                    recovery_errors.push(abnormal);
                    (primary_error, recovery_errors)
                }
                None => (abnormal, checkpoint.recovery_errors),
            };
            finish_failed_with_audit(
                executor,
                error,
                checkpoint.recovered,
                recovery_errors,
                checkpoint.record_saved,
                checkpoint.audit,
            )
            .await;
        }
    })
    .catch_unwind()
    .await;
    if result.is_err() && !recovery.completion.is_completed() && recovery.terminalization.claim() {
        let fatal = WorkflowError::new(
            "E_RESOURCE_RELEASE_UNPROVEN",
            "terminalization panicked while proving resource release",
        );
        let checkpoint = recovery.checkpoint.lock().unwrap().clone();
        let context = serde_json::json!({
            "fatalStep": "run.terminalization_panic",
            "winner": recovery.arbiter.winner(),
            "checkpoint": checkpoint_diagnostic_context(&checkpoint),
        });
        let deadline = recovery.cleanup_deadline();
        recovery.completion.claim();
        recovery
            .sink
            .fatal(&recovery.run_id, fatal, context, deadline);
    }
}

async fn run_executor(
    executor: Arc<RunExecutorHandle>,
    mut control: tokio::sync::mpsc::UnboundedReceiver<RunControl>,
) {
    if executor.seed.context.enabled {
        if !executor.arbiter.begin_stage(StageKind::ContextCapture) {
            finish_cancelled(executor, None).await;
            return;
        }
        executor.emit_progress(ProgressPayload::ContextCapture(StageProgress {
            status: StageStatus::Started,
            elapsed_ms: None,
        }));
        match cancellable_call(&executor, executor.ports.capture_context()).await {
            Ok(()) => executor.emit_progress(ProgressPayload::ContextCapture(StageProgress {
                status: StageStatus::Completed,
                elapsed_ms: None,
            })),
            Err(error) if is_cancel_winner(&executor) => {
                finish_cancelled(executor, None).await;
                return;
            }
            Err(error) => {
                finish_failed(executor, error, None, Vec::new(), false).await;
                return;
            }
        }
    }

    let next = tokio::select! {
        biased;
        _ = executor.token.cancelled() => RunControl::Cancel,
        control = control.recv() => match control {
            Some(control) => control,
            None => {
                finish_failed(
                    executor,
                    WorkflowError::new("E_EXECUTOR_CONTROL_CLOSED", "run control channel closed"),
                    None,
                    Vec::new(),
                    false,
                ).await;
                return;
            }
        },
    };
    if matches!(next, RunControl::Cancel) {
        finish_cancelled(executor, None).await;
        return;
    }

    if !executor.arbiter.begin_stage(StageKind::RecordFinalize) {
        finish_cancelled(executor, None).await;
        return;
    }
    executor.emit_progress(stage_started(StageKind::RecordFinalize));
    let record_ms = match cancellable_call(&executor, executor.ports.finish_recording()).await {
        Ok(value) => value,
        Err(error) if is_cancel_winner(&executor) => {
            finish_cancelled(executor, None).await;
            return;
        }
        Err(error) => {
            finish_failed(executor, error, None, Vec::new(), false).await;
            return;
        }
    };
    executor.emit_progress(stage_completed(StageKind::RecordFinalize, Some(record_ms)));

    if !executor.arbiter.begin_stage(StageKind::Preprocess) {
        finish_cancelled(executor, None).await;
        return;
    }
    executor.emit_progress(stage_started(StageKind::Preprocess));
    let preprocess_ms = match cancellable_call(&executor, executor.ports.preprocess()).await {
        Ok(value) => value,
        Err(error) if is_cancel_winner(&executor) => {
            finish_cancelled(executor, None).await;
            return;
        }
        Err(error) => {
            finish_failed(executor, error, None, Vec::new(), false).await;
            return;
        }
    };
    executor.emit_progress(stage_completed(StageKind::Preprocess, Some(preprocess_ms)));

    if !executor.arbiter.begin_stage(StageKind::Transcribe) {
        finish_cancelled(executor, None).await;
        return;
    }
    executor.emit_progress(ProgressPayload::Transcribe(TranscribeProgress::Started {
        elapsed_ms: None,
    }));
    let transcription = match cancellable_call(&executor, executor.ports.transcribe()).await {
        Ok(value) => value,
        Err(error) if is_cancel_winner(&executor) => {
            finish_cancelled(executor, None).await;
            return;
        }
        Err(error) => {
            finish_failed(executor, error, None, Vec::new(), false).await;
            return;
        }
    };
    if transcription.asr_text.trim().is_empty() {
        if !executor.begin_terminal() {
            finish_cancelled(executor, None).await;
            return;
        }
        let timings = RunTimings {
            total_ms: record_ms + preprocess_ms + transcription.metrics.asr_ms,
            record_ms: Some(record_ms),
            preprocess_ms: Some(preprocess_ms),
            asr_ms: Some(transcription.metrics.asr_ms),
            rewrite_ms: None,
        };
        finish_with_terminal(
            executor,
            StoppedTerminal::Empty { timings },
            RunAudit::default(),
        )
        .await;
        return;
    }
    let mut recovered = RecoveredRunResult {
        asr_text: transcription.asr_text.clone(),
        final_text: transcription.final_text.clone(),
        timings: RunTimings {
            total_ms: record_ms + preprocess_ms + transcription.metrics.asr_ms,
            record_ms: Some(record_ms),
            preprocess_ms: Some(preprocess_ms),
            asr_ms: Some(transcription.metrics.asr_ms),
            rewrite_ms: None,
        },
        metrics: Some(transcription.metrics.clone()),
    };
    executor.checkpoint_recovered(recovered.clone());
    executor.emit_progress(ProgressPayload::Transcribe(TranscribeProgress::Completed {
        elapsed_ms: Some(transcription.metrics.asr_ms),
        result: transcription.clone(),
    }));

    if executor.seed.rewrite.enabled {
        if !executor.arbiter.begin_stage(StageKind::Rewrite) {
            finish_cancelled(executor, Some(recovered)).await;
            return;
        }
        executor.emit_progress(ProgressPayload::Rewrite(RewriteProgress::Started {
            elapsed_ms: None,
        }));
        match cancellable_call(&executor, executor.ports.rewrite(recovered.clone())).await {
            Ok(rewrite) => {
                recovered.final_text = rewrite.final_text.clone();
                recovered.timings.rewrite_ms = Some(rewrite.rewrite_ms);
                recovered.timings.total_ms = recovered
                    .timings
                    .total_ms
                    .saturating_add(rewrite.rewrite_ms);
                executor.checkpoint_recovered(recovered.clone());
                executor.emit_progress(ProgressPayload::Rewrite(RewriteProgress::Completed {
                    elapsed_ms: Some(rewrite.rewrite_ms),
                    result: rewrite,
                }));
            }
            Err(error) if is_cancel_winner(&executor) => {
                finish_cancelled(executor, Some(recovered)).await;
                return;
            }
            Err(primary_error) => {
                finish_rewrite_failure(executor, primary_error, recovered).await;
                return;
            }
        }
    }

    if !executor.arbiter.begin_stage(StageKind::InsertPrepare) {
        finish_cancelled(executor, Some(recovered)).await;
        return;
    }
    executor.emit_progress(ProgressPayload::InsertPrepare(
        InsertPrepareProgress::Started { elapsed_ms: None },
    ));
    let prepared = match cancellable_call(
        &executor,
        executor
            .ports
            .prepare_insertion(recovered.final_text.clone()),
    )
    .await
    {
        Ok(value) => value,
        Err(error) if is_cancel_winner(&executor) => {
            finish_cancelled(executor, Some(recovered)).await;
            return;
        }
        Err(error) => {
            finish_failed(executor, error, Some(recovered), Vec::new(), false).await;
            return;
        }
    };
    executor.emit_progress(ProgressPayload::InsertPrepare(
        InsertPrepareProgress::Completed {
            elapsed_ms: None,
            result: prepared,
        },
    ));

    finish_success(executor, recovered).await;
}

#[cfg(not(test))]
const EFFECT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(test)]
const EFFECT_DEADLINE: std::time::Duration = std::time::Duration::from_millis(20);

async fn cancellable_call<T>(
    executor: &Arc<RunExecutorHandle>,
    future: PortFuture<T>,
) -> Result<T, WorkflowError> {
    tokio::select! {
        biased;
        _ = executor.token.cancelled() => Err(WorkflowError::new("E_CANCELLED", "run cancelled")),
        result = tokio::time::timeout(EFFECT_DEADLINE, future) => match result {
            Ok(result) => result,
            Err(_) => Err(WorkflowError::new("E_EFFECT_TIMEOUT", "run effect deadline exceeded")),
        }
    }
}

fn is_cancel_winner(executor: &RunExecutorHandle) -> bool {
    executor.arbiter.winner() == Some(ArbiterWinner::Cancel)
}

fn stage_started(kind: StageKind) -> ProgressPayload {
    let progress = StageProgress {
        status: StageStatus::Started,
        elapsed_ms: None,
    };
    match kind {
        StageKind::RecordFinalize => ProgressPayload::RecordFinalize(progress),
        StageKind::Preprocess => ProgressPayload::Preprocess(progress),
        _ => unreachable!("stage_started is only used for simple processing stages"),
    }
}

fn stage_completed(kind: StageKind, elapsed_ms: Option<u128>) -> ProgressPayload {
    let progress = StageProgress {
        status: StageStatus::Completed,
        elapsed_ms,
    };
    match kind {
        StageKind::RecordFinalize => ProgressPayload::RecordFinalize(progress),
        StageKind::Preprocess => ProgressPayload::Preprocess(progress),
        _ => unreachable!("stage_completed is only used for simple processing stages"),
    }
}

async fn finish_success(executor: Arc<RunExecutorHandle>, recovered: RecoveredRunResult) {
    if !executor.begin_finalization() {
        finish_cancelled(executor, Some(recovered)).await;
        return;
    }
    let mut audit = RunAudit {
        finalization: FinalizationAudit { commit_count: 1 },
        ..Default::default()
    };
    executor.checkpoint_finalization(recovered.clone(), None, Vec::new(), false, audit.clone());
    executor.emit_progress(ProgressPayload::Finalize { elapsed_ms: None });
    if let Err(error) =
        cancellable_call(&executor, executor.ports.commit_history(recovered.clone())).await
    {
        finish_failed_with_audit(executor, error, Some(recovered), Vec::new(), false, audit).await;
        return;
    }
    audit.effects.history_commit_count = 1;
    executor.checkpoint_finalization(recovered.clone(), None, Vec::new(), true, audit.clone());
    if let Err(error) = cancellable_call(
        &executor,
        executor.ports.copy_text(recovered.final_text.clone()),
    )
    .await
    {
        finish_failed_with_audit(executor, error, Some(recovered), Vec::new(), true, audit).await;
        return;
    }
    audit.effects.copy_count = 1;
    executor.checkpoint_finalization(recovered.clone(), None, Vec::new(), true, audit.clone());

    let mut insert_result = InsertResult::copy_only();
    let mut warning = None;
    if executor.seed.insertion.auto_paste {
        audit.effects.paste_count = 1;
        executor.checkpoint_finalization(recovered.clone(), None, Vec::new(), true, audit.clone());
        match cancellable_call(
            &executor,
            executor.ports.paste_text(recovered.final_text.clone()),
        )
        .await
        {
            Ok(()) => insert_result = InsertResult::pasted(),
            Err(error) => {
                insert_result = InsertResult::paste_failed(&error.code, error.message.clone());
                warning = Some(error);
            }
        }
    }
    let result = CompletedRunResult {
        asr_text: recovered.asr_text,
        final_text: recovered.final_text,
        timings: recovered.timings,
        insert_result,
        metrics: recovered.metrics,
    };
    finish_with_terminal(
        executor,
        StoppedTerminal::Completed { result, warning },
        audit,
    )
    .await;
}

async fn finish_rewrite_failure(
    executor: Arc<RunExecutorHandle>,
    primary_error: WorkflowError,
    recovered: RecoveredRunResult,
) {
    if !executor.begin_finalization() {
        finish_cancelled(executor, Some(recovered)).await;
        return;
    }
    let mut audit = RunAudit {
        finalization: FinalizationAudit { commit_count: 1 },
        ..Default::default()
    };
    executor.checkpoint_finalization(
        recovered.clone(),
        Some(primary_error.clone()),
        Vec::new(),
        false,
        audit.clone(),
    );
    executor.emit_progress(ProgressPayload::Finalize { elapsed_ms: None });
    let history =
        cancellable_call(&executor, executor.ports.commit_history(recovered.clone())).await;
    let (record_saved, recovery_errors) = match history {
        Ok(()) => {
            audit.effects.history_commit_count = 1;
            (true, Vec::new())
        }
        Err(error) => (false, vec![error]),
    };
    executor.checkpoint_finalization(
        recovered.clone(),
        Some(primary_error.clone()),
        recovery_errors.clone(),
        record_saved,
        audit.clone(),
    );
    finish_failed_with_audit(
        executor,
        primary_error,
        Some(recovered),
        recovery_errors,
        record_saved,
        audit,
    )
    .await;
}

async fn finish_failed(
    executor: Arc<RunExecutorHandle>,
    error: WorkflowError,
    recovered: Option<RecoveredRunResult>,
    recovery_errors: Vec<WorkflowError>,
    record_saved: bool,
) {
    finish_failed_with_audit(
        executor,
        error,
        recovered,
        recovery_errors,
        record_saved,
        RunAudit::default(),
    )
    .await;
}

async fn finish_failed_with_audit(
    executor: Arc<RunExecutorHandle>,
    error: WorkflowError,
    recovered: Option<RecoveredRunResult>,
    recovery_errors: Vec<WorkflowError>,
    record_saved: bool,
    audit: RunAudit,
) {
    executor.checkpoint_failure(
        recovered.clone(),
        error.clone(),
        recovery_errors.clone(),
        record_saved,
        audit.clone(),
    );
    match executor.arbiter.winner() {
        Some(ArbiterWinner::Cancel) => {
            finish_cancelled(executor, recovered).await;
            return;
        }
        None => {
            if !executor.begin_terminal() {
                finish_cancelled(executor, recovered).await;
                return;
            }
        }
        Some(ArbiterWinner::Terminal | ArbiterWinner::Finalization) => {}
    }
    finish_with_terminal(
        executor,
        StoppedTerminal::Failed {
            error,
            recovered_result: recovered,
            recovery_errors,
            record_saved,
            protocol_context: None,
        },
        audit,
    )
    .await;
}

async fn finish_cancelled(executor: Arc<RunExecutorHandle>, recovered: Option<RecoveredRunResult>) {
    let terminal = StoppedTerminal::Cancelled {
        recovered_result: recovered,
        cleanup_diagnostic: None,
    };
    finish_with_terminal(executor, terminal, RunAudit::default()).await;
}

async fn finish_cancelled_with_diagnostic(
    executor: Arc<RunExecutorHandle>,
    recovered: Option<RecoveredRunResult>,
    diagnostic: CleanupDiagnostic,
) {
    let terminal = StoppedTerminal::Cancelled {
        recovered_result: recovered,
        cleanup_diagnostic: Some(diagnostic),
    };
    finish_with_terminal(executor, terminal, RunAudit::default()).await;
}

async fn finish_with_terminal(
    executor: Arc<RunExecutorHandle>,
    mut terminal: StoppedTerminal,
    mut audit: RunAudit,
) {
    if !executor.terminalization.claim() {
        return;
    }
    let mut ownership = TerminalizationOwnership::new(executor.terminalization.clone());
    let cleanup_deadline = executor.cleanup_deadline();
    let started = cleanup_deadline
        .checked_sub(Duration::from_millis(CANCEL_DEADLINE_MS))
        .unwrap_or_else(Instant::now);
    let cleanup_cutoff = cleanup_deadline
        .checked_sub(Duration::from_millis(FATAL_TRACE_RESERVE_MS))
        .unwrap_or(cleanup_deadline);
    let cleanup_remaining = cleanup_cutoff.saturating_duration_since(Instant::now());
    let force_reserve = Duration::from_millis(FORCE_CLEANUP_RESERVE_MS).min(cleanup_remaining);
    let graceful_budget = cleanup_remaining.saturating_sub(force_reserve);
    let graceful = tokio::time::timeout(graceful_budget, executor.ports.shutdown()).await;
    let released = matches!(graceful, Ok(Ok(true)));
    if !released {
        executor.trace_event(
            "run.cleanup_escalated",
            "escalated",
            serde_json::json!({
                "elapsedMs": started.elapsed().as_millis(),
                "gracefulTimedOut": graceful.is_err(),
            }),
        );
        let remaining = cleanup_cutoff.saturating_duration_since(Instant::now());
        let forced = tokio::time::timeout(remaining, executor.ports.force_shutdown()).await;
        let force_succeeded = matches!(forced, Ok(Ok(true)));
        let mut diagnostic = CleanupDiagnostic {
            elapsed_ms: started.elapsed().as_millis(),
            graceful_timed_out: graceful.is_err(),
            force_attempted: true,
            force_succeeded,
            detail: Some(if force_succeeded {
                "resource cleanup required force".to_string()
            } else {
                "resource release could not be proven".to_string()
            }),
        };
        if !force_succeeded {
            let error = WorkflowError::new(
                "E_RESOURCE_RELEASE_UNPROVEN",
                "run resources remain live after force shutdown",
            );
            let context = serde_json::json!({
                "fatalStep": "run.cleanup_fatal",
                "elapsedMs": diagnostic.elapsed_ms,
                "forceAttempted": true,
                "forceSucceeded": false,
                "observedEffectsAtFatal": executor.ports.observed_effects(),
                "effectSnapshotFinal": false,
                "pendingTerminal": terminal_diagnostic_context(&terminal),
                "pendingAudit": serde_json::to_value(&audit)
                    .unwrap_or_else(|_| serde_json::json!({"serialization": "failed"})),
            });
            executor.completion.claim();
            executor
                .sink
                .fatal(&executor.run_id, error, context, cleanup_deadline);
            ownership.retain();
            return;
        }
        executor.trace_event(
            "run.cleanup_forced",
            "released",
            serde_json::json!({
                "elapsedMs": diagnostic.elapsed_ms,
                "forceAttempted": true,
                "forceSucceeded": true,
            }),
        );
        audit.cleanup = Some(diagnostic.clone());
        if let StoppedTerminal::Cancelled {
            cleanup_diagnostic, ..
        } = &mut terminal
        {
            if let Some(existing) = cleanup_diagnostic.as_ref() {
                diagnostic.detail = match (&existing.detail, &diagnostic.detail) {
                    (Some(existing), Some(cleanup)) => Some(format!("{existing}; {cleanup}")),
                    (Some(existing), None) => Some(existing.clone()),
                    (None, cleanup) => cleanup.clone(),
                };
            }
            *cleanup_diagnostic = Some(diagnostic);
        }
    }
    let observed_effects = executor.ports.observed_effects();
    audit.effects.history_commit_count = audit
        .effects
        .history_commit_count
        .max(observed_effects.history_commit_count);
    audit.effects.copy_count = audit.effects.copy_count.max(observed_effects.copy_count);
    audit.effects.paste_count = audit.effects.paste_count.max(observed_effects.paste_count);
    if let StoppedTerminal::Failed { record_saved, .. } = &mut terminal {
        *record_saved |= observed_effects.history_commit_count > 0;
    }
    executor.set_resources_released();
    executor.emit_stopped(terminal, audit);
    ownership.retain();
}

fn terminal_diagnostic_context(terminal: &StoppedTerminal) -> serde_json::Value {
    match terminal {
        StoppedTerminal::Completed { warning, .. } => serde_json::json!({
            "variant": "completed",
            "warningCode": warning.as_ref().map(|error| error.code.as_str()),
        }),
        StoppedTerminal::Empty { .. } => serde_json::json!({
            "variant": "empty",
        }),
        StoppedTerminal::Failed {
            error,
            recovered_result,
            recovery_errors,
            record_saved,
            protocol_context,
        } => serde_json::json!({
            "variant": "failed",
            "primaryErrorCode": error.code,
            "recoveryErrorCodes": recovery_errors
                .iter()
                .map(|error| error.code.as_str())
                .collect::<Vec<_>>(),
            "hasRecoveredResult": recovered_result.is_some(),
            "recordSaved": record_saved,
            "protocolVariant": protocol_context
                .as_ref()
                .map(|context| context.original_variant.as_str()),
            "protocolErrorCode": protocol_context
                .as_ref()
                .and_then(|context| context.original_error.as_ref())
                .map(|error| error.code.as_str()),
        }),
        StoppedTerminal::Cancelled {
            recovered_result,
            cleanup_diagnostic,
        } => serde_json::json!({
            "variant": "cancelled",
            "hasRecoveredResult": recovered_result.is_some(),
            "cleanupCode": cleanup_diagnostic
                .as_ref()
                .and_then(|diagnostic| diagnostic.detail.as_deref())
                .and_then(|detail| detail.split(':').next())
                .filter(|code| code.starts_with("E_")),
            "forceAttempted": cleanup_diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.force_attempted),
            "forceSucceeded": cleanup_diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.force_succeeded),
        }),
    }
}

fn checkpoint_diagnostic_context(checkpoint: &ExecutorCheckpoint) -> serde_json::Value {
    serde_json::json!({
        "hasRecoveredResult": checkpoint.recovered.is_some(),
        "primaryErrorCode": checkpoint
            .primary_error
            .as_ref()
            .map(|error| error.code.as_str()),
        "recoveryErrorCodes": checkpoint
            .recovery_errors
            .iter()
            .map(|error| error.code.as_str())
            .collect::<Vec<_>>(),
        "recordSaved": checkpoint.record_saved,
        "audit": checkpoint.audit,
    })
}

struct TerminalizationOwnership {
    guard: Arc<CompletionGuard>,
    retained: bool,
}

impl TerminalizationOwnership {
    fn new(guard: Arc<CompletionGuard>) -> Self {
        Self {
            guard,
            retained: false,
        }
    }

    fn retain(&mut self) {
        self.retained = true;
    }
}

impl Drop for TerminalizationOwnership {
    fn drop(&mut self) {
        if !self.retained {
            self.guard.release();
        }
    }
}

pub struct CompletionGuard {
    completed: AtomicBool,
}

impl CompletionGuard {
    pub fn new() -> Self {
        Self {
            completed: AtomicBool::new(false),
        }
    }

    pub fn claim(&self) -> bool {
        !self.completed.swap(true, Ordering::AcqRel)
    }

    pub fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    fn release(&self) {
        self.completed.store(false, Ordering::Release);
    }
}

impl Default for CompletionGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_and_terminal_are_linearized() {
        let token = CancellationToken::new();
        let arbiter = RunArbiter::new(token.clone());
        assert_eq!(arbiter.request_cancel(), CancelDisposition::Accepted);
        assert!(token.is_cancelled());
        assert!(!arbiter.begin_terminal());
        assert!(!arbiter.begin_finalization());
    }

    #[test]
    fn finalization_makes_cancel_too_late() {
        let arbiter = RunArbiter::default();
        assert!(arbiter.begin_finalization());
        assert_eq!(arbiter.request_cancel(), CancelDisposition::TooLate);
    }

    #[test]
    fn completion_guard_has_one_winner() {
        let guard = CompletionGuard::new();
        assert!(guard.claim());
        assert!(!guard.claim());
    }
}
