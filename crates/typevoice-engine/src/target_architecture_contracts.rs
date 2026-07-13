//! Executable contracts for the controller/executor target architecture.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use typevoice_core::workflow::{
    CancelDisposition, CleanupDiagnostic, CommandDisposition, CompletedRunResult, EffectCounts,
    FinalizationAudit, InsertPrepareProgress, InsertPrepareResult, InsertResult, Progress,
    ProgressPayload, RecoveredRunResult, RewriteProgress, RewriteResult, RunAudit, RunId,
    RunOutcomeView, RunPlanSeed, RunTimings, StageKind, StageProgress, StageStatus, Stopped,
    StoppedTerminal, TranscribeProgress, TranscriptionMetrics, TranscriptionResult, WorkflowError,
    WorkflowIntent, WorkflowMode, WorkflowView,
};

use crate::{
    run_executor::{
        BeginAccepted, ExecutorSpawner, ExecutorTaskHandle, PortFuture, ResourceCounts, RunArbiter,
        RunExecutorHandle, RunFactory, RunHandle, RunHandleInspection, RunPorts, RunPortsFactory,
        RunSignalSink, CANCEL_DEADLINE_MS, START_DEADLINE_MS,
    },
    workflow_controller::{RunIdSource, WorkflowClock, WorkflowController, WorkflowSnapshotSink},
};

#[derive(Default)]
struct TargetSubcases {
    executed: usize,
    failures: Vec<String>,
    fixture_errors: Vec<String>,
}

impl TargetSubcases {
    fn check(&mut self, label: &str, passed: bool, evidence: impl Into<String>) {
        self.executed += 1;
        let evidence = evidence.into();
        eprintln!(
            "[EXECUTED] {label}: {} -- {evidence}",
            if passed { "PASS" } else { "TARGET_GAP" }
        );
        if !passed {
            self.failures.push(format!("[{label}] {evidence}"));
        }
    }

    fn finish(self, contract: &str, expected: usize) {
        assert_eq!(
            self.executed, expected,
            "{contract}: expected {expected} named subcases, executed {}",
            self.executed
        );
        assert!(
            self.failures.is_empty() && self.fixture_errors.is_empty(),
            "{contract}: executed {} subcases; target gaps={} fixture errors={}\ntarget gaps:\n{}\nfixture errors:\n{}",
            self.executed,
            self.failures.len(),
            self.fixture_errors.len(),
            self.failures.join("\n"),
            self.fixture_errors.join("\n")
        );
    }
}

#[derive(Default)]
struct CaptureSink {
    snapshots: Mutex<Vec<WorkflowView>>,
}

impl CaptureSink {
    fn len(&self) -> usize {
        self.snapshots.lock().unwrap().len()
    }

    fn values(&self) -> Vec<WorkflowView> {
        self.snapshots.lock().unwrap().clone()
    }
}

impl WorkflowSnapshotSink for CaptureSink {
    fn publish(&self, view: &WorkflowView) -> Result<(), WorkflowError> {
        self.snapshots.lock().unwrap().push(view.clone());
        Ok(())
    }
}

#[derive(Default)]
struct CaptureRunSignalSink {
    stopped: Mutex<Vec<Stopped>>,
    fatal: Mutex<Vec<WorkflowError>>,
}

impl RunSignalSink for CaptureRunSignalSink {
    fn progress(&self, _progress: Progress) {}

    fn stopped(&self, stopped: Stopped) {
        self.stopped.lock().unwrap().push(stopped);
    }

    fn fatal(&self, _run_id: &str, error: WorkflowError, _context: Value, _deadline: Instant) {
        self.fatal.lock().unwrap().push(error);
    }
}

struct DeterministicIds {
    ids: Mutex<VecDeque<RunId>>,
}

impl DeterministicIds {
    fn new(ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            ids: Mutex::new(ids.into_iter().map(Into::into).collect()),
        }
    }
}

impl RunIdSource for DeterministicIds {
    fn next_run_id(&self) -> RunId {
        self.ids
            .lock()
            .unwrap()
            .pop_front()
            .expect("target-contract deterministic run-id fixture exhausted")
    }
}

struct DeterministicClock {
    now: AtomicU64,
}

impl DeterministicClock {
    fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }
}

impl WorkflowClock for DeterministicClock {
    fn now_ms(&self) -> u64 {
        self.now.fetch_add(1, Ordering::AcqRel)
    }
}

#[derive(Clone)]
struct ControlledConfig {
    begin: Result<BeginAccepted, WorkflowError>,
    stop: Result<(), WorkflowError>,
}

impl Default for ControlledConfig {
    fn default() -> Self {
        Self {
            begin: Ok(BeginAccepted {
                capture_started_at_ms: 1_050,
            }),
            stop: Ok(()),
        }
    }
}

struct ControlledHandle {
    run_id: RunId,
    seed: RunPlanSeed,
    sink: Arc<dyn RunSignalSink>,
    config: ControlledConfig,
    arbiter: Arc<RunArbiter>,
    trace: Mutex<Vec<String>>,
    resources: Mutex<ResourceCounts>,
    begin_count: AtomicU32,
    stop_count: AtomicU32,
    stopped_count: AtomicU32,
}

impl ControlledHandle {
    fn stopped(&self, terminal: StoppedTerminal, audit: RunAudit) {
        self.stopped_count.fetch_add(1, Ordering::AcqRel);
        *self.resources.lock().unwrap() = ResourceCounts::default();
        self.sink.stopped(Stopped {
            run_id: self.run_id.clone(),
            terminal,
            audit,
        });
    }
}

impl RunHandle for ControlledHandle {
    fn run_id(&self) -> &str {
        &self.run_id
    }

    fn begin(&self) -> Result<BeginAccepted, WorkflowError> {
        self.begin_count.fetch_add(1, Ordering::AcqRel);
        self.trace.lock().unwrap().push("begin".to_string());
        if self.config.begin.is_ok() {
            *self.resources.lock().unwrap() = ResourceCounts {
                live: 1,
                run_handle: 1,
                ffmpeg: 1,
                provider_request: u32::from(self.seed.asr.provider == "doubao"),
                cancellation_token: 1,
                temporary_asset: 0,
            };
        }
        self.config.begin.clone()
    }

    fn stop(&self) -> Result<(), WorkflowError> {
        self.stop_count.fetch_add(1, Ordering::AcqRel);
        self.trace.lock().unwrap().push("stop".to_string());
        self.config.stop.clone()
    }

    fn settle_control_failure(&self, error: WorkflowError) {
        let sink = self.sink.clone();
        let run_id = self.run_id.clone();
        self.stopped_count.store(1, Ordering::Release);
        *self.resources.lock().unwrap() = ResourceCounts::default();
        std::thread::spawn(move || {
            sink.stopped(Stopped {
                run_id,
                terminal: StoppedTerminal::Failed {
                    error,
                    recovered_result: None,
                    recovery_errors: Vec::new(),
                    record_saved: false,
                    protocol_context: None,
                },
                audit: RunAudit::default(),
            });
        });
    }

    fn request_cancel(&self) -> CancelDisposition {
        let disposition = self.arbiter.request_cancel();
        self.trace.lock().unwrap().push(match disposition {
            CancelDisposition::Accepted => "cancelAccepted".to_string(),
            CancelDisposition::TooLate => "cancelTooLate".to_string(),
        });
        disposition
    }

    fn inspect(&self) -> RunHandleInspection {
        RunHandleInspection {
            trace: self.trace.lock().unwrap().clone(),
            arbiter: self.arbiter.snapshot(),
            resource_counts: *self.resources.lock().unwrap(),
            begin_count: self.begin_count.load(Ordering::Acquire),
            stop_count: self.stop_count.load(Ordering::Acquire),
            stopped_count: self.stopped_count.load(Ordering::Acquire),
        }
    }
}

struct ControlledFactory {
    configs: Mutex<VecDeque<ControlledConfig>>,
    handles: Mutex<Vec<Arc<ControlledHandle>>>,
}

impl ControlledFactory {
    fn new(configs: impl IntoIterator<Item = ControlledConfig>) -> Self {
        Self {
            configs: Mutex::new(configs.into_iter().collect()),
            handles: Mutex::new(Vec::new()),
        }
    }

    fn handle(&self, index: usize) -> Arc<ControlledHandle> {
        self.handles.lock().unwrap()[index].clone()
    }

    fn last_handle(&self) -> Arc<ControlledHandle> {
        self.handles
            .lock()
            .unwrap()
            .last()
            .expect("target-contract controller must have created a run handle")
            .clone()
    }
}

impl RunFactory for ControlledFactory {
    fn create(
        &self,
        run_id: RunId,
        seed: RunPlanSeed,
        signal_sink: Arc<dyn RunSignalSink>,
    ) -> Arc<dyn RunHandle> {
        let config = self.configs.lock().unwrap().pop_front().unwrap_or_default();
        let handle = Arc::new(ControlledHandle {
            run_id,
            seed,
            sink: signal_sink,
            config,
            arbiter: Arc::new(RunArbiter::default()),
            trace: Mutex::new(Vec::new()),
            resources: Mutex::new(ResourceCounts::default()),
            begin_count: AtomicU32::new(0),
            stop_count: AtomicU32::new(0),
            stopped_count: AtomicU32::new(0),
        });
        self.handles.lock().unwrap().push(handle.clone());
        handle
    }
}

#[derive(Default)]
struct PortGate {
    released: std::sync::atomic::AtomicBool,
    notify: tokio::sync::Notify,
}

impl PortGate {
    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        while !self.released.load(Ordering::Acquire) {
            self.notify.notified().await;
        }
    }
}

#[derive(Clone, Default)]
enum PortBehavior {
    #[default]
    Ok,
    Fail(WorkflowError),
    Pending,
    Panic,
    Gate(Arc<PortGate>),
    LateSuccess(Duration),
}

#[derive(Clone, Copy)]
enum ScriptedEffectKind {
    History,
    Copy,
}

#[derive(Clone, Default)]
struct ScriptedEffects {
    active: Arc<AtomicU32>,
    idle: Arc<tokio::sync::Notify>,
    history_commit_count: Arc<AtomicU32>,
    copy_count: Arc<AtomicU32>,
}

impl ScriptedEffects {
    fn start_late_success(
        &self,
        kind: ScriptedEffectKind,
        delay: Duration,
    ) -> tokio::sync::oneshot::Receiver<()> {
        self.active.fetch_add(1, Ordering::AcqRel);
        let effects = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            match kind {
                ScriptedEffectKind::History => {
                    effects.history_commit_count.store(1, Ordering::Release);
                }
                ScriptedEffectKind::Copy => {
                    effects.copy_count.store(1, Ordering::Release);
                }
            }
            if effects.active.fetch_sub(1, Ordering::AcqRel) == 1 {
                effects.idle.notify_waiters();
            }
            let _ = tx.send(());
        });
        rx
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }

    fn snapshot(&self) -> EffectCounts {
        EffectCounts {
            history_commit_count: self.history_commit_count.load(Ordering::Acquire),
            copy_count: self.copy_count.load(Ordering::Acquire),
            paste_count: 0,
        }
    }
}

#[derive(Clone)]
struct PortsScript {
    begin: Result<BeginAccepted, WorkflowError>,
    context: PortBehavior,
    finish_recording: PortBehavior,
    preprocess: PortBehavior,
    transcribe: PortBehavior,
    rewrite: PortBehavior,
    insert_prepare: PortBehavior,
    history: PortBehavior,
    copy: PortBehavior,
    paste: PortBehavior,
    shutdown: PortBehavior,
    shutdown_releases: bool,
    force_shutdown: PortBehavior,
    force_releases: bool,
    transcript: String,
    rewritten: String,
}

impl Default for PortsScript {
    fn default() -> Self {
        Self {
            begin: Ok(BeginAccepted {
                capture_started_at_ms: 1_050,
            }),
            context: PortBehavior::Ok,
            finish_recording: PortBehavior::Ok,
            preprocess: PortBehavior::Ok,
            transcribe: PortBehavior::Ok,
            rewrite: PortBehavior::Ok,
            insert_prepare: PortBehavior::Ok,
            history: PortBehavior::Ok,
            copy: PortBehavior::Ok,
            paste: PortBehavior::Ok,
            shutdown: PortBehavior::Ok,
            shutdown_releases: true,
            force_shutdown: PortBehavior::Ok,
            force_releases: true,
            transcript: "asr text".to_string(),
            rewritten: "rewritten text".to_string(),
        }
    }
}

#[derive(Default)]
struct PortCalls {
    context: AtomicU32,
    finish_recording: AtomicU32,
    preprocess: AtomicU32,
    transcribe: AtomicU32,
    rewrite: AtomicU32,
    insert_prepare: AtomicU32,
    history: AtomicU32,
    copy: AtomicU32,
    paste: AtomicU32,
    shutdown: AtomicU32,
    force_shutdown: AtomicU32,
}

struct ScriptedPorts {
    run_id: String,
    script: PortsScript,
    calls: PortCalls,
    effects: ScriptedEffects,
}

fn port_future<T: Send + 'static>(behavior: PortBehavior, value: T) -> PortFuture<T> {
    Box::pin(async move {
        match behavior {
            PortBehavior::Ok => Ok(value),
            PortBehavior::Fail(error) => Err(error),
            PortBehavior::Pending => std::future::pending().await,
            PortBehavior::Panic => panic!("scripted target-contract port panic"),
            PortBehavior::Gate(gate) => {
                gate.wait().await;
                Ok(value)
            }
            PortBehavior::LateSuccess(delay) => {
                tokio::time::sleep(delay).await;
                Ok(value)
            }
        }
    })
}

impl RunPorts for ScriptedPorts {
    fn begin_recording(&self, _token: CancellationToken) -> Result<BeginAccepted, WorkflowError> {
        self.script.begin.clone()
    }

    fn capture_context(&self) -> PortFuture<()> {
        self.calls.context.fetch_add(1, Ordering::AcqRel);
        port_future(self.script.context.clone(), ())
    }

    fn finish_recording(&self) -> PortFuture<u128> {
        self.calls.finish_recording.fetch_add(1, Ordering::AcqRel);
        port_future(self.script.finish_recording.clone(), 4)
    }

    fn preprocess(&self) -> PortFuture<u128> {
        self.calls.preprocess.fetch_add(1, Ordering::AcqRel);
        port_future(self.script.preprocess.clone(), 2)
    }

    fn transcribe(&self) -> PortFuture<TranscriptionResult> {
        self.calls.transcribe.fetch_add(1, Ordering::AcqRel);
        port_future(
            self.script.transcribe.clone(),
            transcription_result(&self.run_id, &self.script.transcript),
        )
    }

    fn rewrite(&self, _result: RecoveredRunResult) -> PortFuture<RewriteResult> {
        self.calls.rewrite.fetch_add(1, Ordering::AcqRel);
        port_future(
            self.script.rewrite.clone(),
            RewriteResult {
                transcript_id: self.run_id.clone(),
                final_text: self.script.rewritten.clone(),
                rewrite_ms: 3,
            },
        )
    }

    fn prepare_insertion(&self, _text: String) -> PortFuture<InsertPrepareResult> {
        self.calls.insert_prepare.fetch_add(1, Ordering::AcqRel);
        port_future(
            self.script.insert_prepare.clone(),
            InsertPrepareResult {
                target: "focused-window".to_string(),
                text_digest: "sha256:contract".to_string(),
            },
        )
    }

    fn commit_history(&self, _result: RecoveredRunResult) -> PortFuture<()> {
        self.calls.history.fetch_add(1, Ordering::AcqRel);
        if let PortBehavior::LateSuccess(delay) = &self.script.history {
            let completion = self
                .effects
                .start_late_success(ScriptedEffectKind::History, *delay);
            return Box::pin(async move {
                completion.await.map_err(|_| {
                    WorkflowError::new(
                        "E_SCRIPTED_EFFECT_JOIN",
                        "late history effect worker exited without a result",
                    )
                })?;
                Ok(())
            });
        }
        port_future(self.script.history.clone(), ())
    }

    fn copy_text(&self, _text: String) -> PortFuture<()> {
        self.calls.copy.fetch_add(1, Ordering::AcqRel);
        if let PortBehavior::LateSuccess(delay) = &self.script.copy {
            let completion = self
                .effects
                .start_late_success(ScriptedEffectKind::Copy, *delay);
            return Box::pin(async move {
                completion.await.map_err(|_| {
                    WorkflowError::new(
                        "E_SCRIPTED_EFFECT_JOIN",
                        "late copy effect worker exited without a result",
                    )
                })?;
                Ok(())
            });
        }
        port_future(self.script.copy.clone(), ())
    }

    fn paste_text(&self, _text: String) -> PortFuture<()> {
        self.calls.paste.fetch_add(1, Ordering::AcqRel);
        port_future(self.script.paste.clone(), ())
    }

    fn shutdown(&self) -> PortFuture<bool> {
        self.calls.shutdown.fetch_add(1, Ordering::AcqRel);
        if matches!(&self.script.shutdown, PortBehavior::Ok) {
            let effects = self.effects.clone();
            let releases = self.script.shutdown_releases;
            return Box::pin(async move {
                effects.wait_idle().await;
                Ok(releases)
            });
        }
        port_future(self.script.shutdown.clone(), self.script.shutdown_releases)
    }

    fn force_shutdown(&self) -> PortFuture<bool> {
        self.calls.force_shutdown.fetch_add(1, Ordering::AcqRel);
        if matches!(&self.script.force_shutdown, PortBehavior::Ok) {
            let effects = self.effects.clone();
            let releases = self.script.force_releases;
            return Box::pin(async move {
                effects.wait_idle().await;
                Ok(releases)
            });
        }
        port_future(
            self.script.force_shutdown.clone(),
            self.script.force_releases,
        )
    }

    fn observed_effects(&self) -> EffectCounts {
        self.effects.snapshot()
    }
}

struct ScriptedPortsFactory {
    scripts: Mutex<VecDeque<PortsScript>>,
    ports: Mutex<Vec<Arc<ScriptedPorts>>>,
}

impl ScriptedPortsFactory {
    fn new(scripts: impl IntoIterator<Item = PortsScript>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into_iter().collect()),
            ports: Mutex::new(Vec::new()),
        }
    }

    fn last_ports(&self) -> Arc<ScriptedPorts> {
        self.ports
            .lock()
            .unwrap()
            .last()
            .expect("target-contract executor must create ports")
            .clone()
    }
}

impl RunPortsFactory for ScriptedPortsFactory {
    fn create(&self, run_id: &str, _seed: &RunPlanSeed) -> Arc<dyn RunPorts> {
        let ports = Arc::new(ScriptedPorts {
            run_id: run_id.to_string(),
            script: self.scripts.lock().unwrap().pop_front().unwrap_or_default(),
            calls: PortCalls::default(),
            effects: ScriptedEffects::default(),
        });
        self.ports.lock().unwrap().push(ports.clone());
        ports
    }
}

struct ThreadTaskHandle;

impl ExecutorTaskHandle for ThreadTaskHandle {
    fn abort(&self) {}
}

struct ThreadSpawner;

impl ExecutorSpawner for ThreadSpawner {
    fn spawn(
        &self,
        future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
    ) -> Arc<dyn ExecutorTaskHandle> {
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("target-contract Tokio runtime must build")
                .block_on(future);
        });
        Arc::new(ThreadTaskHandle)
    }
}

struct PanicAfterStoppedSink {
    inner: Arc<dyn RunSignalSink>,
}

impl RunSignalSink for PanicAfterStoppedSink {
    fn progress(&self, progress: Progress) {
        self.inner.progress(progress);
    }

    fn stopped(&self, stopped: Stopped) {
        self.inner.stopped(stopped);
        panic!("scripted supervisor exit after ordinary terminal");
    }

    fn fatal(&self, run_id: &str, error: WorkflowError, context: Value, deadline: Instant) {
        self.inner.fatal(run_id, error, context, deadline);
    }
}

struct CapturingExecutorFactory {
    ports: Arc<dyn RunPortsFactory>,
    spawner: Arc<dyn ExecutorSpawner>,
    handles: Mutex<Vec<Arc<RunExecutorHandle>>>,
    panic_after_stopped: bool,
}

impl CapturingExecutorFactory {
    fn new(ports: Arc<dyn RunPortsFactory>, panic_after_stopped: bool) -> Self {
        Self {
            ports,
            spawner: Arc::new(ThreadSpawner),
            handles: Mutex::new(Vec::new()),
            panic_after_stopped,
        }
    }

    fn last_handle(&self) -> Arc<RunExecutorHandle> {
        self.handles
            .lock()
            .unwrap()
            .last()
            .expect("target-contract executor handle must exist")
            .clone()
    }
}

impl RunFactory for CapturingExecutorFactory {
    fn create(
        &self,
        run_id: RunId,
        seed: RunPlanSeed,
        signal_sink: Arc<dyn RunSignalSink>,
    ) -> Arc<dyn RunHandle> {
        let signal_sink: Arc<dyn RunSignalSink> = if self.panic_after_stopped {
            Arc::new(PanicAfterStoppedSink { inner: signal_sink })
        } else {
            signal_sink
        };
        let ports = self.ports.create(&run_id, &seed);
        let handle =
            RunExecutorHandle::dormant(run_id, seed, ports, signal_sink, self.spawner.clone());
        self.handles.lock().unwrap().push(handle.clone());
        handle
    }
}

struct ControlledHarness {
    controller: Arc<WorkflowController>,
    factory: Arc<ControlledFactory>,
    sink: Arc<CaptureSink>,
}

fn controlled_harness(
    seed: RunPlanSeed,
    ids: impl IntoIterator<Item = impl Into<String>>,
    configs: impl IntoIterator<Item = ControlledConfig>,
) -> ControlledHarness {
    let factory = Arc::new(ControlledFactory::new(configs));
    let sink = Arc::new(CaptureSink::default());
    let controller = WorkflowController::new(
        seed,
        factory.clone(),
        sink.clone(),
        Arc::new(DeterministicIds::new(ids)),
        Arc::new(DeterministicClock::new(1_000)),
    );
    ControlledHarness {
        controller,
        factory,
        sink,
    }
}

struct ExecutorHarness {
    controller: Arc<WorkflowController>,
    factory: Arc<CapturingExecutorFactory>,
    ports: Arc<ScriptedPortsFactory>,
}

fn executor_harness(
    seed: RunPlanSeed,
    run_id: &str,
    script: PortsScript,
    panic_after_stopped: bool,
) -> ExecutorHarness {
    let ports = Arc::new(ScriptedPortsFactory::new([script]));
    let factory = Arc::new(CapturingExecutorFactory::new(
        ports.clone(),
        panic_after_stopped,
    ));
    let sink = Arc::new(CaptureSink::default());
    let controller = WorkflowController::new(
        seed,
        factory.clone(),
        sink.clone(),
        Arc::new(DeterministicIds::new([run_id])),
        Arc::new(DeterministicClock::new(1_000)),
    );
    ExecutorHarness {
        controller,
        factory,
        ports,
    }
}

fn seed(
    settings_revision: &str,
    rewrite_enabled: bool,
    auto_paste: bool,
    context_enabled: bool,
) -> RunPlanSeed {
    let mut seed = RunPlanSeed {
        settings_revision: settings_revision.to_string(),
        ..Default::default()
    };
    seed.rewrite.enabled = rewrite_enabled;
    seed.insertion.auto_paste = auto_paste;
    seed.context.enabled = context_enabled;
    seed
}

fn transcription_result(run_id: &str, text: &str) -> TranscriptionResult {
    TranscriptionResult::new(
        run_id,
        text,
        TranscriptionMetrics {
            rtf: 0.25,
            device_used: "contract-fake".to_string(),
            preprocess_ms: 2,
            asr_ms: 8,
        },
    )
}

fn recovered(asr: &str, final_text: &str) -> RecoveredRunResult {
    RecoveredRunResult {
        asr_text: asr.to_string(),
        final_text: final_text.to_string(),
        timings: RunTimings {
            total_ms: 20,
            preprocess_ms: Some(2),
            asr_ms: Some(8),
            ..Default::default()
        },
        metrics: None,
    }
}

fn completed(final_text: &str, insert_result: InsertResult) -> CompletedRunResult {
    CompletedRunResult {
        asr_text: "asr text".to_string(),
        final_text: final_text.to_string(),
        timings: RunTimings {
            total_ms: 20,
            ..Default::default()
        },
        insert_result,
        metrics: None,
    }
}

fn start(controller: &Arc<WorkflowController>) -> typevoice_core::workflow::WorkflowCommandReply {
    let action_key = controller.snapshot().action_key;
    controller
        .command(WorkflowIntent::Primary { action_key })
        .expect("target-contract start command must be accepted")
}

fn stop(controller: &Arc<WorkflowController>) -> typevoice_core::workflow::WorkflowCommandReply {
    let action_key = controller.snapshot().action_key;
    controller
        .command(WorkflowIntent::Primary { action_key })
        .expect("target-contract stop command must be accepted")
}

fn cancel(
    controller: &Arc<WorkflowController>,
    target_run_id: &str,
) -> typevoice_core::workflow::WorkflowCommandReply {
    controller
        .command(WorkflowIntent::Cancel {
            target_run_id: Some(target_run_id.to_string()),
        })
        .expect("target-contract cancel command must return a protocol reply")
}

fn wire(view: &WorkflowView) -> Value {
    serde_json::to_value(view).expect("target-contract workflow view must serialize")
}

fn inspection(controller: &WorkflowController) -> Value {
    controller.inspection_value()
}

fn wait_until(label: &str, timeout: Duration, predicate: impl Fn() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    eprintln!("[WAIT_TIMEOUT] {label} after {}ms", timeout.as_millis());
    false
}

fn wait_ready(controller: &WorkflowController) -> bool {
    wait_until("controller Ready", Duration::from_secs(3), || {
        controller.snapshot().mode == WorkflowMode::Ready
    })
}

fn wait_stage(controller: &WorkflowController, stage: StageKind) -> bool {
    wait_until("controller stage", Duration::from_secs(3), || {
        controller
            .snapshot()
            .active_run
            .and_then(|run| run.stage)
            .is_some_and(|current| current.kind == stage)
    })
}

fn run_executor_to_terminal(harness: &ExecutorHarness) -> bool {
    let _ = start(&harness.controller);
    if harness.controller.snapshot().mode == WorkflowMode::Recording {
        let _ = stop(&harness.controller);
    }
    wait_ready(&harness.controller)
}

fn stage_payloads(run_id: &str) -> Vec<ProgressPayload> {
    vec![
        ProgressPayload::ContextCapture(StageProgress {
            status: StageStatus::Started,
            elapsed_ms: Some(1),
        }),
        ProgressPayload::ContextCapture(StageProgress {
            status: StageStatus::Completed,
            elapsed_ms: Some(2),
        }),
        ProgressPayload::RecordFinalize(StageProgress {
            status: StageStatus::Started,
            elapsed_ms: Some(3),
        }),
        ProgressPayload::RecordFinalize(StageProgress {
            status: StageStatus::Completed,
            elapsed_ms: Some(4),
        }),
        ProgressPayload::Preprocess(StageProgress {
            status: StageStatus::Started,
            elapsed_ms: Some(5),
        }),
        ProgressPayload::Preprocess(StageProgress {
            status: StageStatus::Completed,
            elapsed_ms: Some(6),
        }),
        ProgressPayload::Transcribe(TranscribeProgress::Started {
            elapsed_ms: Some(7),
        }),
        ProgressPayload::Transcribe(TranscribeProgress::Completed {
            result: transcription_result(run_id, "asr text"),
            elapsed_ms: Some(8),
        }),
        ProgressPayload::Rewrite(RewriteProgress::Started {
            elapsed_ms: Some(9),
        }),
        ProgressPayload::Rewrite(RewriteProgress::Completed {
            result: RewriteResult {
                transcript_id: run_id.to_string(),
                final_text: "rewritten".to_string(),
                rewrite_ms: 3,
            },
            elapsed_ms: Some(10),
        }),
        ProgressPayload::InsertPrepare(InsertPrepareProgress::Started {
            elapsed_ms: Some(11),
        }),
        ProgressPayload::InsertPrepare(InsertPrepareProgress::Completed {
            result: InsertPrepareResult {
                target: "focused-window".to_string(),
                text_digest: "sha256:contract".to_string(),
            },
            elapsed_ms: Some(12),
        }),
        ProgressPayload::Finalize {
            elapsed_ms: Some(13),
        },
    ]
}

fn controlled_at_progress_index(run_id: &str, target_index: usize) -> ControlledHarness {
    let harness = controlled_harness(
        seed("progress", true, true, true),
        [run_id],
        [ControlledConfig::default()],
    );
    let _ = start(&harness.controller);
    let payloads = stage_payloads(run_id);
    for (index, payload) in payloads.iter().take(target_index).cloned().enumerate() {
        harness
            .controller
            .submit_progress(Progress {
                run_id: run_id.to_string(),
                payload,
            })
            .expect("target-contract progress prefix must be legal");
        if index == 1 {
            let _ = stop(&harness.controller);
        }
    }
    if target_index >= 2 && harness.controller.snapshot().mode == WorkflowMode::Recording {
        let _ = stop(&harness.controller);
    }
    harness
}

fn controlled_at_stage(run_id: &str, stage: StageKind, started: bool) -> ControlledHarness {
    let target = match stage {
        StageKind::ContextCapture => 0,
        StageKind::RecordFinalize => 2,
        StageKind::Preprocess => 4,
        StageKind::Transcribe => 6,
        StageKind::Rewrite => 8,
        StageKind::InsertPrepare => 10,
        StageKind::Finalize => 12,
    };
    controlled_at_progress_index(run_id, target + usize::from(started))
}

fn failed_terminal(code: &str, recovered_result: Option<RecoveredRunResult>) -> StoppedTerminal {
    StoppedTerminal::Failed {
        error: WorkflowError::new(code, format!("scripted {code}")),
        recovered_result,
        recovery_errors: Vec::new(),
        record_saved: false,
        protocol_context: None,
    }
}

fn outcome_name(view: &WorkflowView) -> Option<&'static str> {
    match &view.last_run.as_ref()?.outcome {
        RunOutcomeView::Completed { .. } => Some("completed"),
        RunOutcomeView::Empty { .. } => Some("empty"),
        RunOutcomeView::Failed { .. } => Some("failed"),
        RunOutcomeView::Cancelled { .. } => Some("cancelled"),
    }
}

#[derive(Clone, Copy)]
enum OutcomeFixture {
    Completed,
    Empty,
    Failed,
    Cancelled,
}

impl OutcomeFixture {
    const ALL: [(Self, &'static str); 4] = [
        (Self::Completed, "completed"),
        (Self::Empty, "empty"),
        (Self::Failed, "failed"),
        (Self::Cancelled, "cancelled"),
    ];
}

fn controller_after_outcome(
    outcome: OutcomeFixture,
    old_id: &str,
    next_id: &str,
) -> ControlledHarness {
    let harness = controlled_harness(
        seed("outcome", true, false, true),
        [old_id, next_id],
        [ControlledConfig::default(), ControlledConfig::default()],
    );
    let _ = start(&harness.controller);
    let target_progress = match outcome {
        OutcomeFixture::Completed => 13,
        OutcomeFixture::Empty => 7,
        OutcomeFixture::Failed | OutcomeFixture::Cancelled => 0,
    };
    for (index, payload) in stage_payloads(old_id)
        .into_iter()
        .take(target_progress)
        .enumerate()
    {
        harness
            .controller
            .submit_progress(Progress {
                run_id: old_id.to_string(),
                payload,
            })
            .expect("target-contract outcome progress fixture must be legal");
        if index == 1 {
            let _ = stop(&harness.controller);
        }
    }
    let handle = harness.factory.last_handle();
    match outcome {
        OutcomeFixture::Completed => handle.stopped(
            StoppedTerminal::Completed {
                result: completed("completed text", InsertResult::copy_only()),
                warning: None,
            },
            RunAudit {
                effects: EffectCounts {
                    history_commit_count: 1,
                    copy_count: 1,
                    paste_count: 0,
                },
                finalization: FinalizationAudit { commit_count: 1 },
                cleanup: None,
            },
        ),
        OutcomeFixture::Empty => handle.stopped(
            StoppedTerminal::Empty {
                timings: RunTimings {
                    total_ms: 12,
                    ..Default::default()
                },
            },
            RunAudit::default(),
        ),
        OutcomeFixture::Failed => handle.stopped(
            failed_terminal("E_SCRIPTED_FAILURE", None),
            RunAudit::default(),
        ),
        OutcomeFixture::Cancelled => {
            let reply = cancel(&harness.controller, old_id);
            assert_eq!(reply.disposition, CommandDisposition::Applied);
            handle.stopped(
                StoppedTerminal::Cancelled {
                    recovered_result: None,
                    cleanup_diagnostic: None,
                },
                RunAudit::default(),
            );
        }
    }
    harness
}

#[test]
fn target_contract_t01_start_freezes_seed_and_orders_begin_projection() {
    let mut failures = TargetSubcases::default();
    let current_seed = seed("settings-before-r1", false, false, false);
    let next_seed = seed("settings-after-r1", false, true, false);
    let harness = controlled_harness(
        current_seed.clone(),
        ["run-01-current", "run-01-next"],
        [ControlledConfig::default(), ControlledConfig::default()],
    );

    let current_reply = start(&harness.controller);
    let current_handle = harness.factory.handle(0);
    let current_inspection = inspection(&harness.controller);
    failures.check(
        "T01.seed.current_run_frozen",
        current_handle.seed == current_seed
            && current_reply
                .view
                .active_run
                .as_ref()
                .map(|run| run.run_id.as_str())
                == Some("run-01-current"),
        format!(
            "captured_seed={:?}, expected={current_seed:?}, view={}",
            current_handle.seed,
            wire(&current_reply.view)
        ),
    );

    harness.controller.update_cached_seed(next_seed.clone());
    current_handle.stopped(failed_terminal("E_SCRIPTED_END", None), RunAudit::default());
    let next_reply = start(&harness.controller);
    let next_handle = harness.factory.handle(1);
    failures.check(
        "T01.seed.settings_changed_after_r1_only_affect_next_run",
        current_handle.seed == current_seed
            && next_handle.seed == next_seed
            && next_reply
                .view
                .active_run
                .as_ref()
                .map(|run| run.run_id.as_str())
                == Some("run-01-next"),
        format!(
            "current_seed={:?}, next_seed={:?}, next_view={}",
            current_handle.seed,
            next_handle.seed,
            wire(&next_reply.view)
        ),
    );

    failures.check(
        "T01.order.commit_then_begin_accepted_then_snapshot_then_reply",
        current_inspection.pointer("/projectionOrder")
            == Some(&json!(["commit", "beginAccepted", "snapshot", "reply"]))
            && current_handle.inspect().begin_count == 1
            && harness.sink.len() >= 1,
        format!(
            "inspection={current_inspection}, handle={:?}, sink={:?}",
            current_handle.inspect(),
            harness.sink.values()
        ),
    );

    let committed = current_inspection
        .pointer("/activeRun/committedAtMs")
        .and_then(Value::as_u64);
    let capture_started = current_inspection
        .pointer("/activeRun/captureStartedAtMs")
        .and_then(Value::as_u64);
    failures.check(
        "T01.capture_io.after_commit_and_within_200ms",
        matches!(
            (committed, capture_started),
            (Some(commit), Some(capture))
                if capture >= commit && capture.saturating_sub(commit) <= START_DEADLINE_MS
        ),
        format!(
            "commit={committed:?}, capture_started={capture_started:?}, deadline={START_DEADLINE_MS}, inspection={current_inspection}"
        ),
    );
    failures.finish("T01 start_freezes_seed_and_orders_begin_projection", 4);
}

#[test]
fn target_contract_t02_begin_or_recording_start_failure_uses_typed_failed() {
    let cases = [
        ("T02.begin.delivery_failure", "E_BEGIN_DELIVERY", false),
        ("T02.begin.ack_failure", "E_BEGIN_ACK", false),
        ("T02.recording.start_failure", "E_RECORD_START", false),
        ("T02.doubao_session.start_failure", "E_DOUBAO_START", false),
        (
            "T02.context_failure.before_primary_stop",
            "E_CONTEXT_BEFORE_STOP",
            false,
        ),
        (
            "T02.context_failure.after_primary_stop",
            "E_CONTEXT_AFTER_STOP",
            true,
        ),
    ];
    let mut failures = TargetSubcases::default();

    for (index, (label, code, after_stop)) in cases.into_iter().enumerate() {
        let run_id = format!("run-02-{index}");
        let config = if after_stop {
            ControlledConfig {
                stop: Err(WorkflowError::new(code, label)),
                ..Default::default()
            }
        } else {
            ControlledConfig {
                begin: Err(WorkflowError::new(code, label)),
                ..Default::default()
            }
        };
        let harness = controlled_harness(seed("t02", false, false, false), [&run_id], [config]);
        let start_reply = start(&harness.controller);
        if after_stop {
            let _ = stop(&harness.controller);
        }
        wait_until("control failure terminal", Duration::from_secs(1), || {
            harness.controller.snapshot().mode == WorkflowMode::Ready
        });
        let view = harness.controller.snapshot();
        let observed_code = match view.last_run.as_ref().map(|run| &run.outcome) {
            Some(RunOutcomeView::Failed { primary_error, .. }) => Some(primary_error.code.as_str()),
            _ => None,
        };
        failures.check(
            label,
            start_reply.disposition == CommandDisposition::Applied
                && view.mode == WorkflowMode::Ready
                && view.active_run.is_none()
                && view.last_run.as_ref().map(|run| run.run_id.as_str()) == Some(run_id.as_str())
                && observed_code == Some(code)
                && view.last_run.as_ref().map(|run| run.stopped_count) == Some(1),
            format!("after_stop={after_stop}, view={}", wire(&view)),
        );
    }

    failures.finish("T02 begin_or_recording_start_failure_uses_typed_failed", 6);
}

#[test]
fn target_contract_t03_intent_admission_and_matrix_are_total() {
    #[derive(Clone, Copy)]
    enum ModeCase {
        Ready,
        Recording,
        Processing,
        Cancelling,
    }
    impl ModeCase {
        fn label(self) -> &'static str {
            match self {
                Self::Ready => "ready",
                Self::Recording => "recording",
                Self::Processing => "processing",
                Self::Cancelling => "cancelling",
            }
        }
    }
    #[derive(Clone, Copy)]
    enum IntentCase {
        FreshPrimary,
        Cancel,
        Invalid,
        StaleActionKey,
        MismatchedTargetRunId,
    }
    impl IntentCase {
        fn label(self) -> &'static str {
            match self {
                Self::FreshPrimary => "fresh_primary",
                Self::Cancel => "cancel",
                Self::Invalid => "invalid",
                Self::StaleActionKey => "stale_action_key",
                Self::MismatchedTargetRunId => "mismatched_target_run_id",
            }
        }
    }

    let modes = [
        ModeCase::Ready,
        ModeCase::Recording,
        ModeCase::Processing,
        ModeCase::Cancelling,
    ];
    let intents = [
        IntentCase::FreshPrimary,
        IntentCase::Cancel,
        IntentCase::Invalid,
        IntentCase::StaleActionKey,
        IntentCase::MismatchedTargetRunId,
    ];
    let mut failures = TargetSubcases::default();

    for (mode_index, mode) in modes.into_iter().enumerate() {
        for intent in intents {
            let label = format!("T03.matrix.{}.{}", mode.label(), intent.label());
            let run_id = format!("run-03-{mode_index}-{}", intent.label());
            let harness = controlled_harness(
                seed("t03", false, false, false),
                [&run_id],
                [ControlledConfig::default()],
            );
            match mode {
                ModeCase::Ready => {}
                ModeCase::Recording => {
                    let _ = start(&harness.controller);
                }
                ModeCase::Processing => {
                    let _ = start(&harness.controller);
                    let _ = stop(&harness.controller);
                }
                ModeCase::Cancelling => {
                    let _ = start(&harness.controller);
                    let _ = cancel(&harness.controller, &run_id);
                }
            }
            let before = harness.controller.snapshot();
            let before_sink = harness.sink.len();
            let result = match intent {
                IntentCase::FreshPrimary => harness.controller.command(WorkflowIntent::Primary {
                    action_key: before.action_key.clone(),
                }),
                IntentCase::Cancel => harness.controller.command(WorkflowIntent::Cancel {
                    target_run_id: before.active_run.as_ref().map(|run| run.run_id.clone()),
                }),
                IntentCase::Invalid => {
                    let parsed =
                        serde_json::from_value::<WorkflowIntent>(json!({"kind": "unknownIntent"}));
                    failures.check(
                        &label,
                        parsed.is_err()
                            && harness.controller.snapshot() == before
                            && harness.sink.len() == before_sink,
                        format!("parsed={parsed:?}, before={}", wire(&before)),
                    );
                    continue;
                }
                IntentCase::StaleActionKey => harness.controller.command(WorkflowIntent::Primary {
                    action_key: "stale-action-key".to_string(),
                }),
                IntentCase::MismatchedTargetRunId => {
                    harness.controller.command(WorkflowIntent::Cancel {
                        target_run_id: Some("another-run".to_string()),
                    })
                }
            };
            let after = harness.controller.snapshot();
            let reply = result.expect("typed protocol intent must return a reply");
            let stale = matches!(
                intent,
                IntentCase::StaleActionKey | IntentCase::MismatchedTargetRunId
            );
            failures.check(
                &label,
                before.mode
                    == match mode {
                        ModeCase::Ready => WorkflowMode::Ready,
                        ModeCase::Recording => WorkflowMode::Recording,
                        ModeCase::Processing => WorkflowMode::Processing,
                        ModeCase::Cancelling => WorkflowMode::Cancelling,
                    }
                    && (!stale
                        || (reply.disposition == CommandDisposition::NoOp
                            && after == before
                            && harness.sink.len() == before_sink)),
                format!(
                    "reply={:?}, before={}, after={}, sink_before={before_sink}, sink_after={}",
                    reply.disposition,
                    wire(&before),
                    wire(&after),
                    harness.sink.len()
                ),
            );
        }
    }

    let initial = controlled_harness(
        seed("initial", false, false, false),
        ["run-03-initial"],
        [ControlledConfig::default()],
    );
    let initial_view = initial.controller.snapshot();
    failures.check(
        "T03.ready_key.initial_generation",
        initial_view.mode == WorkflowMode::Ready
            && initial_view.revision == 0
            && initial_view.action_key == "Start(Initial)",
        format!("view={}", wire(&initial_view)),
    );

    let after = controlled_harness(
        seed("after", false, false, false),
        ["run-03-finished"],
        [ControlledConfig::default()],
    );
    let _ = start(&after.controller);
    after
        .factory
        .last_handle()
        .stopped(failed_terminal("E_FINISHED", None), RunAudit::default());
    let after_view = after.controller.snapshot();
    failures.check(
        "T03.ready_key.after_last_run_generation",
        after_view.mode == WorkflowMode::Ready
            && after_view.last_run.as_ref().map(|run| run.run_id.as_str())
                == Some("run-03-finished")
            && after_view.action_key != initial_view.action_key,
        format!(
            "initial={}, after={}",
            wire(&initial_view),
            wire(&after_view)
        ),
    );

    for (label, too_late) in [
        ("T03.active_cancel.accepted", false),
        ("T03.active_cancel.too_late", true),
    ] {
        let run_id = if too_late {
            "run-03-cancel-too-late"
        } else {
            "run-03-cancel-accepted"
        };
        let harness = controlled_harness(
            seed("cancel", false, false, false),
            [run_id],
            [ControlledConfig::default()],
        );
        let _ = start(&harness.controller);
        if too_late {
            let _ = stop(&harness.controller);
            assert!(harness.factory.last_handle().arbiter.begin_finalization());
        }
        let reply = cancel(&harness.controller, run_id);
        let view = harness.controller.snapshot();
        failures.check(
            label,
            if too_late {
                reply.disposition == CommandDisposition::CancelTooLate
                    && view.mode == WorkflowMode::Processing
            } else {
                reply.disposition == CommandDisposition::Applied
                    && view.mode == WorkflowMode::Cancelling
            },
            format!("reply={:?}, view={}", reply.disposition, wire(&view)),
        );
    }
    failures.finish("T03 intent_admission_and_matrix_are_total", 24);
}

#[test]
fn target_contract_t04_stop_is_once_and_delivery_failure_is_terminal() {
    let mut failures = TargetSubcases::default();

    let double = controlled_harness(
        seed("double", false, false, false),
        ["run-04-double"],
        [ControlledConfig::default()],
    );
    let _ = start(&double.controller);
    let stop_key = double.controller.snapshot().action_key;
    let started = Instant::now();
    let first = double
        .controller
        .command(WorkflowIntent::Primary {
            action_key: stop_key.clone(),
        })
        .unwrap();
    let elapsed = started.elapsed();
    let duplicate = double
        .controller
        .command(WorkflowIntent::Primary {
            action_key: stop_key,
        })
        .unwrap();
    failures.check(
        "T04.double_primary.same_action_key",
        first.disposition == CommandDisposition::Applied
            && duplicate.disposition == CommandDisposition::NoOp
            && elapsed <= Duration::from_millis(START_DEADLINE_MS)
            && double.factory.last_handle().inspect().stop_count == 1
            && double.controller.snapshot().mode == WorkflowMode::Processing,
        format!(
            "first={:?}, duplicate={:?}, elapsed={elapsed:?}, inspection={:?}, view={}",
            first.disposition,
            duplicate.disposition,
            double.factory.last_handle().inspect(),
            wire(&double.controller.snapshot())
        ),
    );

    let progress = controlled_harness(
        seed("progress", false, false, true),
        ["run-04-progress"],
        [ControlledConfig::default()],
    );
    let _ = start(&progress.controller);
    let stale_key = progress.controller.snapshot().action_key;
    progress
        .controller
        .submit_progress(Progress {
            run_id: "run-04-progress".to_string(),
            payload: ProgressPayload::ContextCapture(StageProgress {
                status: StageStatus::Started,
                elapsed_ms: Some(1),
            }),
        })
        .unwrap();
    let after_progress = progress.controller.snapshot();
    let result = progress
        .controller
        .command(WorkflowIntent::Primary {
            action_key: stale_key.clone(),
        })
        .unwrap();
    failures.check(
        "T04.progress_between_view_and_primary.same_key_still_stops",
        after_progress.action_key == stale_key
            && result.disposition == CommandDisposition::Applied
            && result.view.mode == WorkflowMode::Processing
            && progress.factory.last_handle().inspect().stop_count == 1,
        format!(
            "stale_key={stale_key}, after_progress={}, reply={:?}, inspection={:?}",
            wire(&after_progress),
            result.disposition,
            progress.factory.last_handle().inspect()
        ),
    );

    let delivery = controlled_harness(
        seed("delivery", false, false, false),
        ["run-04-delivery"],
        [ControlledConfig {
            stop: Err(WorkflowError::new(
                "E_STOP_DELIVERY",
                "scripted Stop delivery failure",
            )),
            ..Default::default()
        }],
    );
    let _ = start(&delivery.controller);
    let reply = stop(&delivery.controller);
    wait_until(
        "Stop delivery failure terminal",
        Duration::from_secs(1),
        || delivery.controller.snapshot().mode == WorkflowMode::Ready,
    );
    let view = delivery.controller.snapshot();
    let code = match view.last_run.as_ref().map(|run| &run.outcome) {
        Some(RunOutcomeView::Failed { primary_error, .. }) => Some(primary_error.code.as_str()),
        _ => None,
    };
    failures.check(
        "T04.stop_delivery_failure.supervisor_terminalizes",
        reply.disposition == CommandDisposition::Applied
            && view.mode == WorkflowMode::Ready
            && view.active_run.is_none()
            && code == Some("E_STOP_DELIVERY")
            && view.last_run.as_ref().map(|run| run.stopped_count) == Some(1),
        format!("reply={:?}, view={}", reply.disposition, wire(&view)),
    );
    failures.finish("T04 stop_is_once_and_delivery_failure_is_terminal", 3);
}

#[test]
fn target_contract_t05_typed_progress_domain_is_strict_and_monotonic() {
    let mut failures = TargetSubcases::default();

    for (index, (label, context_progress_count, expected_stage)) in [
        (
            "T05.stop.before_context_started",
            0_usize,
            StageKind::ContextCapture,
        ),
        (
            "T05.stop.between_context_started_and_completed",
            1,
            StageKind::ContextCapture,
        ),
        (
            "T05.stop.after_context_completed",
            2,
            StageKind::RecordFinalize,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let run_id = format!("run-05-stop-{index}");
        let harness = controlled_harness(
            seed("stop", false, false, true),
            [&run_id],
            [ControlledConfig::default()],
        );
        let _ = start(&harness.controller);
        for payload in stage_payloads(&run_id)
            .into_iter()
            .take(context_progress_count)
        {
            harness
                .controller
                .submit_progress(Progress {
                    run_id: run_id.clone(),
                    payload,
                })
                .unwrap();
        }
        let reply = stop(&harness.controller);
        let stage = reply
            .view
            .active_run
            .as_ref()
            .and_then(|run| run.stage.as_ref())
            .map(|stage| stage.kind);
        failures.check(
            label,
            reply.disposition == CommandDisposition::Applied
                && reply.view.mode == WorkflowMode::Processing
                && stage == Some(expected_stage),
            format!(
                "context_progress_count={context_progress_count}, stage={stage:?}, view={}",
                wire(&reply.view)
            ),
        );
    }

    let labels = [
        "T05.progress.legal.contextCapture.started",
        "T05.progress.legal.contextCapture.completed",
        "T05.progress.legal.recordFinalize.started",
        "T05.progress.legal.recordFinalize.completed",
        "T05.progress.legal.preprocess.started",
        "T05.progress.legal.preprocess.completed",
        "T05.progress.legal.transcribe.started",
        "T05.progress.legal.transcribe.completed",
        "T05.progress.legal.rewrite.started",
        "T05.progress.legal.rewrite.completed",
        "T05.progress.legal.insertPrepare.started",
        "T05.progress.legal.insertPrepare.completed",
        "T05.progress.legal.finalize.started",
    ];
    for (index, label) in labels.into_iter().enumerate() {
        let run_id = format!("run-05-plan-{index}");
        let harness = controlled_at_progress_index(&run_id, index);
        let before = harness.controller.snapshot();
        let payload = stage_payloads(&run_id)[index].clone();
        let (expected_kind, expected_status) = match index {
            3 => (StageKind::Preprocess, StageStatus::Pending),
            5 => (StageKind::Transcribe, StageStatus::Pending),
            7 => (StageKind::Rewrite, StageStatus::Pending),
            9 => (StageKind::InsertPrepare, StageStatus::Pending),
            11 => (StageKind::Finalize, StageStatus::Pending),
            _ => (payload.kind(), payload.status()),
        };
        let result = harness
            .controller
            .submit_progress(Progress { run_id, payload });
        let after = harness.controller.snapshot();
        let stage = after.active_run.as_ref().and_then(|run| run.stage.as_ref());
        failures.check(
            label,
            result.is_ok()
                && after.revision > before.revision
                && stage.map(|stage| stage.kind) == Some(expected_kind)
                && stage.map(|stage| stage.status) == Some(expected_status),
            format!(
                "result={result:?}, before={}, after={}",
                wire(&before),
                wire(&after)
            ),
        );
    }

    let gate = Arc::new(PortGate::default());
    let script = PortsScript {
        rewrite: PortBehavior::Fail(WorkflowError::new(
            "E_REWRITE_FAILED",
            "scripted rewrite failure",
        )),
        history: PortBehavior::Gate(gate.clone()),
        ..Default::default()
    };
    let recovery = executor_harness(
        seed("rewrite-failure", true, false, false),
        "run-05-rewrite-failed",
        script,
        false,
    );
    let _ = start(&recovery.controller);
    let _ = stop(&recovery.controller);
    let observed = wait_stage(&recovery.controller, StageKind::Finalize);
    let recovery_view = recovery.controller.snapshot();
    failures.check(
        "T05.progress.rewrite_failure_recovery_to_finalize",
        observed
            && recovery_view.mode == WorkflowMode::Processing
            && recovery_view
                .active_run
                .as_ref()
                .and_then(|run| run.result.as_ref())
                .map(|result| result.asr_text.as_str())
                == Some("asr text"),
        format!("observed={observed}, view={}", wire(&recovery_view)),
    );
    gate.release();
    let _ = wait_ready(&recovery.controller);

    let duplicate = controlled_at_progress_index("run-05-invalid-0", 6);
    let first = stage_payloads("run-05-invalid-0")[6].clone();
    duplicate
        .controller
        .submit_progress(Progress {
            run_id: "run-05-invalid-0".to_string(),
            payload: first.clone(),
        })
        .unwrap();
    let before = duplicate.controller.snapshot();
    let second = duplicate.controller.submit_progress(Progress {
        run_id: "run-05-invalid-0".to_string(),
        payload: first,
    });
    failures.check(
        "T05.progress.invalid.duplicate",
        second.is_err() && duplicate.controller.snapshot() == before,
        format!("result={second:?}, view={}", wire(&before)),
    );

    let regression = controlled_at_progress_index("run-05-invalid-1", 9);
    let before = regression.controller.snapshot();
    let result = regression.controller.submit_progress(Progress {
        run_id: "run-05-invalid-1".to_string(),
        payload: ProgressPayload::Transcribe(TranscribeProgress::Completed {
            result: transcription_result("run-05-invalid-1", "regressed"),
            elapsed_ms: Some(2),
        }),
    });
    failures.check(
        "T05.progress.invalid.regression",
        result.is_err() && regression.controller.snapshot() == before,
        format!("result={result:?}, view={}", wire(&before)),
    );

    let mismatch = controlled_at_progress_index("run-05-invalid-2", 7);
    let before = mismatch.controller.snapshot();
    let parsed = serde_json::from_value::<Progress>(json!({
        "runId": "run-05-invalid-2",
        "payload": {
            "transcribe": {
                "completed": {
                    "rewriteResult": {"finalText": "wrong union member"}
                }
            }
        }
    }));
    failures.check(
        "T05.progress.invalid.payload_mismatch",
        parsed.is_err() && mismatch.controller.snapshot() == before,
        format!("parsed={parsed:?}, view={}", wire(&before)),
    );
    failures.finish("T05 typed_progress_domain_is_strict_and_monotonic", 20);
}

#[test]
fn target_contract_t06_completed_terminal_is_typed_and_ordered() {
    let cases = [
        ("T06.completed.from_finalize", StageKind::Finalize, true),
        (
            "T06.completed.from_recording",
            StageKind::ContextCapture,
            false,
        ),
        (
            "T06.completed.from_processing_before_finalize",
            StageKind::Transcribe,
            false,
        ),
    ];
    let mut failures = TargetSubcases::default();

    for (index, (label, origin, legal)) in cases.into_iter().enumerate() {
        let run_id = format!("run-06-{index}");
        let harness = if origin == StageKind::ContextCapture {
            controlled_harness(
                seed("t06", false, false, false),
                [&run_id],
                [ControlledConfig::default()],
            )
        } else {
            controlled_at_stage(&run_id, origin, true)
        };
        if origin == StageKind::ContextCapture {
            let _ = start(&harness.controller);
        }
        harness.factory.last_handle().stopped(
            StoppedTerminal::Completed {
                result: completed("final text", InsertResult::pasted()),
                warning: None,
            },
            RunAudit::default(),
        );
        let view = harness.controller.snapshot();
        let observed = match view.last_run.as_ref().map(|run| &run.outcome) {
            Some(RunOutcomeView::Completed { result, .. }) if legal => {
                result.final_text == "final text" && result.insert_result.copied
            }
            Some(RunOutcomeView::Failed { primary_error, .. }) if !legal => {
                primary_error.code == "E_EXECUTOR_TERMINAL_ORDER"
            }
            _ => false,
        };
        failures.check(
            label,
            view.mode == WorkflowMode::Ready && view.active_run.is_none() && observed,
            format!("legal={legal}, view={}", wire(&view)),
        );
    }

    let harness = controlled_at_stage("run-06-missing", StageKind::Finalize, true);
    let before = harness.controller.snapshot();
    let parsed = serde_json::from_value::<Stopped>(json!({
        "runId": "run-06-missing",
        "terminal": {"completed": {"warning": null}},
        "audit": {
            "effects": {"historyCommitCount": 0, "copyCount": 0, "pasteCount": 0},
            "finalization": {"commitCount": 0}
        }
    }));
    failures.check(
        "T06.completed.missing_result_rejected_at_typed_boundary",
        parsed.is_err() && harness.controller.snapshot() == before,
        format!("parsed={parsed:?}, view={}", wire(&before)),
    );
    failures.finish("T06 completed_terminal_is_typed_and_ordered", 4);
}

#[test]
fn target_contract_t07_empty_terminal_is_typed_and_ordered() {
    let cases = [
        ("T07.empty.from_transcribe", StageKind::Transcribe, true),
        ("T07.empty.from_recording", StageKind::ContextCapture, false),
        (
            "T07.empty.from_processing_context_capture",
            StageKind::ContextCapture,
            false,
        ),
        (
            "T07.empty.from_record_finalize",
            StageKind::RecordFinalize,
            false,
        ),
        ("T07.empty.from_preprocess", StageKind::Preprocess, false),
        ("T07.empty.from_rewrite", StageKind::Rewrite, false),
        (
            "T07.empty.from_insert_prepare",
            StageKind::InsertPrepare,
            false,
        ),
        ("T07.empty.from_finalize", StageKind::Finalize, false),
    ];
    let mut failures = TargetSubcases::default();

    for (index, (label, stage, legal)) in cases.into_iter().enumerate() {
        let run_id = format!("run-07-{index}");
        let harness = if label.ends_with("from_recording") {
            let harness = controlled_harness(
                seed("t07", false, false, false),
                [&run_id],
                [ControlledConfig::default()],
            );
            let _ = start(&harness.controller);
            harness
        } else {
            controlled_at_stage(&run_id, stage, true)
        };
        harness.factory.last_handle().stopped(
            StoppedTerminal::Empty {
                timings: RunTimings {
                    total_ms: 12,
                    ..Default::default()
                },
            },
            RunAudit::default(),
        );
        let view = harness.controller.snapshot();
        let observed = match view.last_run.as_ref().map(|run| &run.outcome) {
            Some(RunOutcomeView::Empty { .. }) if legal => true,
            Some(RunOutcomeView::Failed { primary_error, .. }) if !legal => {
                primary_error.code == "E_EXECUTOR_TERMINAL_ORDER"
            }
            _ => false,
        };
        failures.check(
            label,
            view.mode == WorkflowMode::Ready && view.active_run.is_none() && observed,
            format!("legal={legal}, view={}", wire(&view)),
        );
    }

    let harness = controlled_at_stage("run-07-illegal-error", StageKind::Transcribe, true);
    let before = harness.controller.snapshot();
    let parsed = serde_json::from_value::<Stopped>(json!({
        "runId": "run-07-illegal-error",
        "terminal": {
            "empty": {
                "timings": {"totalMs": 12},
                "error": {"code": "E_ILLEGAL", "message": "illegal"}
            }
        },
        "audit": {
            "effects": {"historyCommitCount": 0, "copyCount": 0, "pasteCount": 0},
            "finalization": {"commitCount": 0}
        }
    }));
    failures.check(
        "T07.empty.error_field_rejected_at_typed_boundary",
        parsed.is_err() && harness.controller.snapshot() == before,
        format!("parsed={parsed:?}, view={}", wire(&before)),
    );
    failures.finish("T07 empty_terminal_is_typed_and_ordered", 9);
}

fn effect_script(effect: &str, error: WorkflowError) -> (RunPlanSeed, PortsScript) {
    let mut script = PortsScript::default();
    let mut plan = seed("effect", false, true, false);
    match effect {
        "context_capture" => {
            plan.context.enabled = true;
            script.context = PortBehavior::Fail(error);
        }
        "recording" | "provider_session" => script.begin = Err(error),
        "record_finalize" => script.finish_recording = PortBehavior::Fail(error),
        "preprocess" => script.preprocess = PortBehavior::Fail(error),
        "transcribe" => script.transcribe = PortBehavior::Fail(error),
        "rewrite" => {
            plan.rewrite.enabled = true;
            script.rewrite = PortBehavior::Fail(error);
        }
        "insert_prepare" => script.insert_prepare = PortBehavior::Fail(error),
        "history" => script.history = PortBehavior::Fail(error),
        "copy" => script.copy = PortBehavior::Fail(error),
        "auto_paste" => script.paste = PortBehavior::Fail(error),
        "finalize" => script.history = PortBehavior::Fail(error),
        _ => panic!("unknown target-contract effect {effect}"),
    }
    (plan, script)
}

#[test]
fn target_contract_t08_failure_timeout_or_abnormal_exit_is_terminal_once() {
    let effects = [
        "context_capture",
        "recording",
        "provider_session",
        "record_finalize",
        "preprocess",
        "transcribe",
        "rewrite",
        "insert_prepare",
        "history",
        "copy",
        "auto_paste",
        "finalize",
    ];
    let mut failures = TargetSubcases::default();

    for (effect_index, effect) in effects.into_iter().enumerate() {
        for (kind, code_prefix) in [("error", "E_EFFECT_ERROR"), ("timeout", "E_EFFECT_TIMEOUT")] {
            let label = format!("T08.effect.{effect}.{kind}");
            let late_side_effect = kind == "timeout" && matches!(effect, "history" | "copy");
            let real_timeout =
                kind == "timeout" && !matches!(effect, "recording" | "provider_session");
            let code = if real_timeout {
                "E_EFFECT_TIMEOUT".to_string()
            } else {
                format!("{code_prefix}_{}", effect.to_ascii_uppercase())
            };
            let (plan, mut script) = effect_script(
                effect,
                WorkflowError::new(&code, format!("scripted {effect} {kind}")),
            );
            if real_timeout {
                let behavior = if late_side_effect {
                    PortBehavior::LateSuccess(Duration::from_millis(100))
                } else {
                    PortBehavior::Pending
                };
                match effect {
                    "context_capture" => script.context = behavior,
                    "record_finalize" => script.finish_recording = behavior,
                    "preprocess" => script.preprocess = behavior,
                    "transcribe" => script.transcribe = behavior,
                    "rewrite" => script.rewrite = behavior,
                    "insert_prepare" => script.insert_prepare = behavior,
                    "history" | "finalize" => script.history = behavior,
                    "copy" => script.copy = behavior,
                    "auto_paste" => script.paste = behavior,
                    _ => unreachable!("real timeout effect must use an asynchronous port"),
                }
            }
            let run_id = format!("run-08-{effect_index}-{kind}");
            let harness = executor_harness(plan, &run_id, script, false);
            let reached_terminal = run_executor_to_terminal(&harness);
            let view = harness.controller.snapshot();
            let terminal_matches = match view.last_run.as_ref().map(|run| &run.outcome) {
                Some(RunOutcomeView::Completed { warning, .. }) if effect == "auto_paste" => {
                    warning.as_ref().map(|warning| warning.code.as_str()) == Some(code.as_str())
                }
                Some(RunOutcomeView::Failed {
                    primary_error,
                    record_saved,
                    ..
                }) => primary_error.code == code && (!late_side_effect || *record_saved),
                _ => false,
            };
            let observed_effect_matches = view.last_run.as_ref().is_some_and(|run| {
                !late_side_effect
                    || (effect == "history"
                        && run.effects.history_commit_count == 1
                        && run.effects.copy_count == 0)
                    || (effect == "copy"
                        && run.effects.history_commit_count == 1
                        && run.effects.copy_count == 1)
            });
            failures.check(
                &label,
                reached_terminal
                    && view.mode == WorkflowMode::Ready
                    && view.active_run.is_none()
                    && terminal_matches
                    && observed_effect_matches
                    && view.last_run.as_ref().map(|run| run.stopped_count) == Some(1),
                format!(
                    "effect={effect}, kind={kind}, reached_terminal={reached_terminal}, handle={:?}, view={}",
                    harness.factory.last_handle().inspect(),
                    wire(&view)
                ),
            );
        }
    }

    for (index, (exit, supervisor_wins)) in [
        ("inner_panic", false),
        ("inner_panic", true),
        ("control_channel_close", false),
        ("control_channel_close", true),
    ]
    .into_iter()
    .enumerate()
    {
        let winner = if supervisor_wins {
            "supervisor_wins"
        } else {
            "ordinary_terminal_wins"
        };
        let label = format!("T08.abnormal_exit.{exit}.{winner}");
        let run_id = format!("run-08-abnormal-{index}");
        let mut script = PortsScript::default();
        if exit == "inner_panic" && supervisor_wins {
            script.copy = PortBehavior::Panic;
        }
        let harness = executor_harness(
            seed("abnormal", false, false, false),
            &run_id,
            script,
            !supervisor_wins,
        );
        let _ = start(&harness.controller);
        let handle = harness.factory.last_handle();
        if exit == "control_channel_close" && supervisor_wins {
            handle.close_control_for_test();
        } else {
            let _ = stop(&harness.controller);
        }
        let reached_terminal = wait_ready(&harness.controller);
        if exit == "control_channel_close" && !supervisor_wins {
            handle.close_control_for_test();
            std::thread::sleep(Duration::from_millis(5));
        }
        let view = harness.controller.snapshot();
        let checkpoint_matches = if exit == "inner_panic" && supervisor_wins {
            matches!(
                view.last_run.as_ref().map(|run| &run.outcome),
                Some(RunOutcomeView::Failed {
                    recovered_result: Some(_),
                    record_saved: true,
                    ..
                })
            ) && view
                .last_run
                .as_ref()
                .is_some_and(|run| run.effects.history_commit_count == 1)
        } else {
            true
        };
        let outcome_matches = if supervisor_wins {
            matches!(
                view.last_run.as_ref().map(|run| &run.outcome),
                Some(RunOutcomeView::Failed { primary_error, .. })
                    if primary_error.code == "E_EXECUTOR_ABNORMAL_EXIT"
                        || primary_error.code == "E_EXECUTOR_CONTROL_CLOSED"
            )
        } else {
            matches!(
                view.last_run.as_ref().map(|run| &run.outcome),
                Some(RunOutcomeView::Completed { .. })
            )
        };
        failures.check(
            &label,
            reached_terminal
                && view.mode == WorkflowMode::Ready
                && outcome_matches
                && checkpoint_matches
                && view.last_run.as_ref().map(|run| run.stopped_count) == Some(1)
                && handle.inspect().stopped_count == 1,
            format!(
                "exit={exit}, supervisor_wins={supervisor_wins}, reached_terminal={reached_terminal}, handle={:?}, view={}",
                handle.inspect(),
                wire(&view)
            ),
        );
    }
    failures.finish("T08 failure_timeout_or_abnormal_exit_is_terminal_once", 28);
}

#[test]
fn target_contract_t09_plan_without_rewrite_still_copies_once() {
    let mut failures = TargetSubcases::default();
    let harness = executor_harness(
        seed("t09", false, false, false),
        "run-09",
        PortsScript::default(),
        false,
    );
    let reached_terminal = run_executor_to_terminal(&harness);
    let view = harness.controller.snapshot();
    let ports = harness.ports.last_ports();
    let observed = matches!(
        view.last_run.as_ref().map(|run| &run.outcome),
        Some(RunOutcomeView::Completed { result, .. })
            if result.final_text == "asr text"
                && result.insert_result.copied
                && !result.insert_result.auto_paste_attempted
    );
    failures.check(
        "T09.plan.rewrite_disabled_auto_paste_disabled",
        reached_terminal
            && observed
            && ports.calls.rewrite.load(Ordering::Acquire) == 0
            && ports.calls.history.load(Ordering::Acquire) == 1
            && ports.calls.copy.load(Ordering::Acquire) == 1
            && ports.calls.paste.load(Ordering::Acquire) == 0,
        format!(
            "reached_terminal={reached_terminal}, rewrite={}, history={}, copy={}, paste={}, view={}",
            ports.calls.rewrite.load(Ordering::Acquire),
            ports.calls.history.load(Ordering::Acquire),
            ports.calls.copy.load(Ordering::Acquire),
            ports.calls.paste.load(Ordering::Acquire),
            wire(&view)
        ),
    );
    failures.finish("T09 plan_without_rewrite_still_copies_once", 1);
}

#[test]
fn target_contract_t10_plan_with_rewrite_and_autopaste_runs_once() {
    let mut failures = TargetSubcases::default();
    let harness = executor_harness(
        seed("t10", true, true, false),
        "run-10",
        PortsScript::default(),
        false,
    );
    let reached_terminal = run_executor_to_terminal(&harness);
    let view = harness.controller.snapshot();
    let ports = harness.ports.last_ports();
    let observed = matches!(
        view.last_run.as_ref().map(|run| &run.outcome),
        Some(RunOutcomeView::Completed { result, .. })
            if result.final_text == "rewritten text"
                && result.insert_result.auto_paste_attempted
                && result.insert_result.auto_paste_ok
    );
    failures.check(
        "T10.plan.rewrite_enabled_auto_paste_enabled",
        reached_terminal
            && observed
            && ports.calls.rewrite.load(Ordering::Acquire) == 1
            && ports.calls.history.load(Ordering::Acquire) == 1
            && ports.calls.copy.load(Ordering::Acquire) == 1
            && ports.calls.paste.load(Ordering::Acquire) == 1,
        format!(
            "reached_terminal={reached_terminal}, rewrite={}, history={}, copy={}, paste={}, view={}",
            ports.calls.rewrite.load(Ordering::Acquire),
            ports.calls.history.load(Ordering::Acquire),
            ports.calls.copy.load(Ordering::Acquire),
            ports.calls.paste.load(Ordering::Acquire),
            wire(&view)
        ),
    );
    failures.finish("T10 plan_with_rewrite_and_autopaste_runs_once", 1);
}

#[derive(Default)]
struct ClipboardExportFake {
    copied: Mutex<Vec<String>>,
}

impl ClipboardExportFake {
    fn copy(&self, text: &str) {
        self.copied.lock().unwrap().push(text.to_string());
    }
}

#[test]
fn target_contract_t11_rewrite_failure_preserves_asr_and_double_faults() {
    let mut failures = TargetSubcases::default();

    for (index, (label, history_succeeds)) in [
        ("T11.recovery_history.success", true),
        ("T11.recovery_history.failure", false),
    ]
    .into_iter()
    .enumerate()
    {
        let history = if history_succeeds {
            PortBehavior::Ok
        } else {
            PortBehavior::Fail(WorkflowError::new(
                "E_HISTORY_RECOVERY_FAILED",
                "scripted recovery History failure",
            ))
        };
        let script = PortsScript {
            rewrite: PortBehavior::Fail(WorkflowError::new(
                "E_REWRITE_FAILED",
                "scripted rewrite failure",
            )),
            history,
            ..Default::default()
        };
        let run_id = format!("run-11-history-{index}");
        let harness = executor_harness(seed("t11", true, false, false), &run_id, script, false);
        let reached_terminal = run_executor_to_terminal(&harness);
        let view = harness.controller.snapshot();
        let observed = match view.last_run.as_ref().map(|run| &run.outcome) {
            Some(RunOutcomeView::Failed {
                primary_error,
                recovered_result,
                recovery_errors,
                record_saved,
                ..
            }) => {
                primary_error.code == "E_REWRITE_FAILED"
                    && recovered_result
                        .as_ref()
                        .map(|result| result.final_text.as_str())
                        == Some("asr text")
                    && if history_succeeds {
                        recovery_errors.is_empty() && *record_saved
                    } else {
                        !*record_saved
                            && recovery_errors
                                .iter()
                                .any(|error| error.code == "E_HISTORY_RECOVERY_FAILED")
                    }
            }
            _ => false,
        };
        failures.check(
            label,
            reached_terminal && observed,
            format!(
                "history_succeeds={history_succeeds}, reached_terminal={reached_terminal}, view={}",
                wire(&view)
            ),
        );
    }

    let script = PortsScript {
        rewrite: PortBehavior::Fail(WorkflowError::new(
            "E_REWRITE_FAILED",
            "scripted rewrite failure",
        )),
        ..Default::default()
    };
    let harness = executor_harness(
        seed("manual-copy", true, false, false),
        "run-11-manual-copy",
        script,
        false,
    );
    let _ = run_executor_to_terminal(&harness);
    let before = harness.controller.snapshot();
    let recovered_text = match before.last_run.as_ref().map(|run| &run.outcome) {
        Some(RunOutcomeView::Failed {
            recovered_result: Some(result),
            ..
        }) => Some(result.final_text.clone()),
        _ => None,
    };
    let clipboard = ClipboardExportFake::default();
    if let Some(text) = recovered_text.as_deref() {
        clipboard.copy(text);
    }
    let after = harness.controller.snapshot();
    failures.check(
        "T11.manual_copy.recovered_result",
        recovered_text.as_deref() == Some("asr text")
            && clipboard.copied.lock().unwrap().as_slice() == ["asr text"]
            && after.revision == before.revision
            && after == before,
        format!(
            "recovered_text={recovered_text:?}, clipboard={:?}, before={}, after={}",
            clipboard.copied.lock().unwrap(),
            wire(&before),
            wire(&after)
        ),
    );
    failures.finish("T11 rewrite_failure_preserves_asr_and_double_faults", 3);
}

#[test]
fn target_contract_t12_paste_warning_keeps_completed_copy() {
    let mut failures = TargetSubcases::default();
    let script = PortsScript {
        paste: PortBehavior::Fail(WorkflowError::new(
            "E_EXPORT_TARGET_UNAVAILABLE",
            "scripted native input failure",
        )),
        ..Default::default()
    };
    let harness = executor_harness(seed("t12", false, true, false), "run-12", script, false);
    let reached_terminal = run_executor_to_terminal(&harness);
    let view = harness.controller.snapshot();
    let ports = harness.ports.last_ports();
    let observed = matches!(
        view.last_run.as_ref().map(|run| &run.outcome),
        Some(RunOutcomeView::Completed { result, warning })
            if result.final_text == "asr text"
                && result.insert_result.copied
                && result.insert_result.auto_paste_attempted
                && !result.insert_result.auto_paste_ok
                && warning.as_ref().map(|warning| warning.code.as_str())
                    == Some("E_EXPORT_TARGET_UNAVAILABLE")
    );
    failures.check(
        "T12.completed.copy_succeeds_auto_paste_fails",
        reached_terminal
            && observed
            && ports.calls.copy.load(Ordering::Acquire) == 1
            && ports.calls.paste.load(Ordering::Acquire) == 1,
        format!(
            "reached_terminal={reached_terminal}, copy={}, paste={}, view={}",
            ports.calls.copy.load(Ordering::Acquire),
            ports.calls.paste.load(Ordering::Acquire),
            wire(&view)
        ),
    );
    failures.finish("T12 paste_warning_keeps_completed_copy", 1);
}

#[test]
fn target_contract_t13_cancel_recording_resources_and_context_are_serial() {
    let mut failures = TargetSubcases::default();

    for (label, launch_before_cancel) in [
        ("resource_launch_then_cancel", true),
        ("cancel_then_resource_launch", false),
    ] {
        let arbiter = RunArbiter::default();
        let initial_launch = !launch_before_cancel || arbiter.begin_resource("recording+context");
        let disposition = arbiter.request_cancel();
        let post_cancel_launch = arbiter.begin_resource("post-cancel");

        let run_id = format!("run-13-{label}");
        let script = PortsScript {
            context: PortBehavior::Pending,
            ..Default::default()
        };
        let harness = executor_harness(seed("t13", false, false, true), &run_id, script, false);
        let _ = start(&harness.controller);
        if launch_before_cancel {
            let _ = wait_stage(&harness.controller, StageKind::ContextCapture);
        }
        let started = Instant::now();
        let reply = cancel(&harness.controller, &run_id);
        let cancelling = harness.controller.snapshot();
        let at_cancel = harness.factory.last_handle().inspect();
        let reached_terminal = wait_ready(&harness.controller);
        let elapsed = started.elapsed();
        let terminal = harness.controller.snapshot();
        failures.check(
            label,
            initial_launch
                && disposition == CancelDisposition::Accepted
                && !post_cancel_launch
                && reply.disposition == CommandDisposition::Applied
                && cancelling.mode == WorkflowMode::Cancelling
                && at_cancel.arbiter.cancel_winner
                && reached_terminal
                && elapsed <= Duration::from_millis(CANCEL_DEADLINE_MS)
                && outcome_name(&terminal) == Some("cancelled")
                && harness
                    .factory
                    .last_handle()
                    .inspect()
                    .resource_counts
                    == ResourceCounts::default(),
            format!(
                "launch_before_cancel={launch_before_cancel}, disposition={disposition:?}, post_cancel_launch={post_cancel_launch}, reply={:?}, elapsed={elapsed:?}, at_cancel={at_cancel:?}, terminal={}",
                reply.disposition,
                wire(&terminal)
            ),
        );
    }

    for (label, abnormal_message) in [
        ("accepted_then_inner_panic", "run executor panicked"),
        (
            "accepted_then_control_close",
            "run executor control channel closed",
        ),
    ] {
        let run_id = format!("run-13-{label}");
        let ports = Arc::new(ScriptedPorts {
            run_id: run_id.clone(),
            script: PortsScript::default(),
            calls: PortCalls::default(),
            effects: ScriptedEffects::default(),
        });
        let sink = Arc::new(CaptureRunSignalSink::default());
        let handle = RunExecutorHandle::dormant(
            run_id.clone(),
            seed("t13-abnormal", false, false, true),
            ports,
            sink.clone(),
            Arc::new(ThreadSpawner),
        );
        let started = Instant::now();
        let disposition = handle.arbiter().request_cancel();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("target-contract Tokio runtime must build")
            .block_on(
                handle
                    .clone()
                    .recover_abnormal_exit_for_test(abnormal_message),
            );
        let elapsed = started.elapsed();
        let stopped = sink.stopped.lock().unwrap().clone();
        let diagnostic = stopped.first().and_then(|event| match &event.terminal {
            StoppedTerminal::Cancelled {
                cleanup_diagnostic, ..
            } => cleanup_diagnostic.as_ref(),
            _ => None,
        });
        failures.check(
            label,
            disposition == CancelDisposition::Accepted
                && elapsed <= Duration::from_millis(CANCEL_DEADLINE_MS)
                && stopped.len() == 1
                && diagnostic.is_some_and(|diagnostic| {
                    !diagnostic.force_attempted
                        && diagnostic
                            .detail
                            .as_deref()
                            .is_some_and(|detail| detail.contains("E_EXECUTOR_ABNORMAL_EXIT"))
                })
                && sink.fatal.lock().unwrap().is_empty()
                && handle.inspect().resource_counts == ResourceCounts::default(),
            format!(
                "disposition={disposition:?}, elapsed={elapsed:?}, stopped={stopped:?}, handle={:?}",
                handle.inspect()
            ),
        );
    }

    let harness = controlled_harness(
        seed("direct-cancelled", false, false, false),
        ["run-13-direct-cancelled"],
        [ControlledConfig::default()],
    );
    let _ = start(&harness.controller);
    harness.factory.last_handle().stopped(
        StoppedTerminal::Cancelled {
            recovered_result: None,
            cleanup_diagnostic: None,
        },
        RunAudit::default(),
    );
    let view = harness.controller.snapshot();
    failures.check(
        "direct_recording_cancelled_is_protocol_failure",
        matches!(
            view.last_run.as_ref().map(|run| &run.outcome),
            Some(RunOutcomeView::Failed { primary_error, .. })
                if primary_error.code == "E_EXECUTOR_CANCEL_UNACKNOWLEDGED"
        ),
        format!("view={}", wire(&view)),
    );
    failures.finish("T13 cancel_recording_resources_and_context_are_serial", 5);
}

fn processing_at_stage(run_id: &str, stage: StageKind, started: bool) -> ControlledHarness {
    let harness = controlled_at_stage(run_id, stage, started);
    if harness.controller.snapshot().mode == WorkflowMode::Recording {
        let _ = stop(&harness.controller);
    }
    harness
}

#[test]
fn target_contract_t14_cancel_each_processing_stage_and_reject_direct_terminal() {
    let stages = [
        ("contextCapture", StageKind::ContextCapture),
        ("recordFinalize", StageKind::RecordFinalize),
        ("preprocess", StageKind::Preprocess),
        ("transcribe", StageKind::Transcribe),
        ("rewrite", StageKind::Rewrite),
        ("insertPrepare", StageKind::InsertPrepare),
    ];
    let mut failures = TargetSubcases::default();

    for (stage_name, stage) in stages {
        for (order, launch_first) in [("beforeLaunch", false), ("afterLaunch", true)] {
            for intent in ["primary", "cancel"] {
                let label = format!("{stage_name}_{order}_{intent}");
                let run_id = format!("run-14-{label}");
                let harness = processing_at_stage(&run_id, stage, launch_first);
                let handle = harness.factory.last_handle();
                if launch_first {
                    assert!(handle.arbiter.begin_stage(stage));
                }
                let before = harness.controller.snapshot();
                let reply = if intent == "primary" {
                    harness
                        .controller
                        .command(WorkflowIntent::Primary {
                            action_key: before.action_key.clone(),
                        })
                        .unwrap()
                } else {
                    cancel(&harness.controller, &run_id)
                };
                let after_cancel = harness.controller.snapshot();
                let post_cancel_launch = handle.arbiter.begin_stage(stage);
                handle.stopped(
                    StoppedTerminal::Cancelled {
                        recovered_result: None,
                        cleanup_diagnostic: Some(CleanupDiagnostic {
                            elapsed_ms: CANCEL_DEADLINE_MS as u128,
                            graceful_timed_out: false,
                            force_attempted: false,
                            force_succeeded: false,
                            detail: None,
                        }),
                    },
                    RunAudit::default(),
                );
                let terminal = harness.controller.snapshot();
                let stage_before = before
                    .active_run
                    .as_ref()
                    .and_then(|run| run.stage.as_ref());
                failures.check(
                    &label,
                    stage_before.map(|stage| stage.kind) == Some(stage)
                        && stage_before.map(|stage| stage.status)
                            == Some(if launch_first {
                                StageStatus::Started
                            } else {
                                StageStatus::Pending
                            })
                        && before.cancel_enabled
                        && reply.disposition == CommandDisposition::Applied
                        && after_cancel.mode == WorkflowMode::Cancelling
                        && !after_cancel.cancel_enabled
                        && after_cancel.revision == before.revision + 1
                        && !post_cancel_launch
                        && outcome_name(&terminal) == Some("cancelled")
                        && terminal
                            .last_run
                            .as_ref()
                            .is_some_and(|run| run.effects == EffectCounts::default())
                        && terminal
                            .last_run
                            .as_ref()
                            .and_then(|run| run.cleanup_diagnostic.as_ref())
                            .is_some_and(|diagnostic| {
                                diagnostic.elapsed_ms <= CANCEL_DEADLINE_MS as u128
                            }),
                    format!(
                        "intent={intent}, launch_first={launch_first}, post_cancel_launch={post_cancel_launch}, before={}, reply={:?}, after_cancel={}, terminal={}",
                        wire(&before),
                        reply.disposition,
                        wire(&after_cancel),
                        wire(&terminal)
                    ),
                );
            }
        }

        let label = format!("{stage_name}_direct_cancelled_terminal");
        let run_id = format!("run-14-{label}");
        let harness = processing_at_stage(&run_id, stage, true);
        harness.factory.last_handle().stopped(
            StoppedTerminal::Cancelled {
                recovered_result: None,
                cleanup_diagnostic: None,
            },
            RunAudit::default(),
        );
        let view = harness.controller.snapshot();
        failures.check(
            &label,
            matches!(
                view.last_run.as_ref().map(|run| &run.outcome),
                Some(RunOutcomeView::Failed { primary_error, .. })
                    if primary_error.code == "E_EXECUTOR_CANCEL_UNACKNOWLEDGED"
            ),
            format!("view={}", wire(&view)),
        );
    }
    failures.finish(
        "T14 cancel_each_processing_stage_and_reject_direct_terminal",
        30,
    );
}

fn protocol_terminal(kind: &str) -> StoppedTerminal {
    match kind {
        "completed" => StoppedTerminal::Completed {
            result: completed("preserved result", InsertResult::copy_only()),
            warning: Some(WorkflowError::new(
                "E_ORIGINAL",
                "original completed warning",
            )),
        },
        "empty" => StoppedTerminal::Empty {
            timings: RunTimings {
                total_ms: 12,
                ..Default::default()
            },
        },
        "failed" => StoppedTerminal::Failed {
            error: WorkflowError::new("E_ORIGINAL", "original failure"),
            recovered_result: Some(recovered("preserved", "preserved result")),
            recovery_errors: vec![WorkflowError::new("E_RECOVERY", "recovery failure")],
            record_saved: false,
            protocol_context: None,
        },
        _ => panic!("unknown protocol terminal {kind}"),
    }
}

#[test]
fn target_contract_t15_cancel_terminal_and_finalization_are_linearizable() {
    let mut failures = TargetSubcases::default();

    let stopped_first = controlled_harness(
        seed("t15", false, false, false),
        ["run-15-stopped-first"],
        [ControlledConfig::default()],
    );
    let _ = start(&stopped_first.controller);
    stopped_first
        .factory
        .last_handle()
        .stopped(failed_terminal("E_SCRIPTED", None), RunAudit::default());
    let before = stopped_first.controller.snapshot();
    let before_sink = stopped_first.sink.len();
    let reply = cancel(&stopped_first.controller, "run-15-stopped-first");
    let after = stopped_first.controller.snapshot();
    failures.check(
        "stopped_then_old_target_cancel_is_a1_noop",
        reply.disposition == CommandDisposition::NoOp
            && before == after
            && stopped_first.sink.len() == before_sink
            && before.last_run.as_ref().map(|run| run.stopped_count) == Some(1),
        format!(
            "reply={:?}, sink_before={before_sink}, sink_after={}, before={}, after={}",
            reply.disposition,
            stopped_first.sink.len(),
            wire(&before),
            wire(&after)
        ),
    );

    let processing = controlled_harness(
        seed("t15", false, false, false),
        ["run-15-processing-race"],
        [ControlledConfig::default()],
    );
    let _ = start(&processing.controller);
    let _ = stop(&processing.controller);
    let reply = cancel(&processing.controller, "run-15-processing-race");
    let view = processing.controller.snapshot();
    failures.check(
        "recording_to_processing_same_run_cancel_reaches_arbiter",
        matches!(
            reply.disposition,
            CommandDisposition::Applied | CommandDisposition::CancelTooLate
        ) && matches!(
            view.mode,
            WorkflowMode::Cancelling | WorkflowMode::Processing
        ) && processing
            .factory
            .last_handle()
            .inspect()
            .arbiter
            .cancel_requests
            == 1,
        format!(
            "reply={:?}, handle={:?}, view={}",
            reply.disposition,
            processing.factory.last_handle().inspect(),
            wire(&view)
        ),
    );

    let finalize = controlled_at_stage("run-15-finalize-race", StageKind::Finalize, true);
    assert!(finalize.factory.last_handle().arbiter.begin_finalization());
    let reply = cancel(&finalize.controller, "run-15-finalize-race");
    let view = finalize.controller.snapshot();
    failures.check(
        "finalize_started_then_same_run_cancel_is_too_late",
        reply.disposition == CommandDisposition::CancelTooLate
            && view.mode == WorkflowMode::Processing
            && !view.cancel_enabled,
        format!("reply={:?}, view={}", reply.disposition, wire(&view)),
    );

    let cancel_winner = controlled_harness(
        seed("t15", false, false, true),
        ["run-15-cancel-winner"],
        [ControlledConfig::default()],
    );
    let _ = start(&cancel_winner.controller);
    let first_cancel = cancel(&cancel_winner.controller, "run-15-cancel-winner");
    let duplicate_cancel = cancel(&cancel_winner.controller, "run-15-cancel-winner");
    let before_progress = cancel_winner.controller.snapshot();
    cancel_winner
        .controller
        .submit_progress(Progress {
            run_id: "run-15-cancel-winner".to_string(),
            payload: ProgressPayload::Transcribe(TranscribeProgress::Started { elapsed_ms: None }),
        })
        .unwrap();
    let handle = cancel_winner.factory.last_handle();
    handle.stopped(
        StoppedTerminal::Cancelled {
            recovered_result: None,
            cleanup_diagnostic: None,
        },
        RunAudit::default(),
    );
    let terminal = cancel_winner.controller.snapshot();
    handle.stopped(
        StoppedTerminal::Completed {
            result: completed("late", InsertResult::copy_only()),
            warning: None,
        },
        RunAudit::default(),
    );
    failures.check(
        "accepted_cancel_duplicate_and_progress_end_once_cancelled",
        first_cancel.disposition == CommandDisposition::Applied
            && duplicate_cancel.disposition == CommandDisposition::NoOp
            && before_progress == cancel_winner.sink.values()[1]
            && outcome_name(&cancel_winner.controller.snapshot()) == Some("cancelled")
            && cancel_winner
                .controller
                .snapshot()
                .last_run
                .as_ref()
                .map(|run| run.stopped_count)
                == Some(1)
            && terminal == cancel_winner.controller.snapshot(),
        format!(
            "first={:?}, duplicate={:?}, before_progress={}, terminal={}, after_duplicate={}",
            first_cancel.disposition,
            duplicate_cancel.disposition,
            wire(&before_progress),
            wire(&terminal),
            wire(&cancel_winner.controller.snapshot())
        ),
    );

    for (label, finalization, expected) in [
        ("begin_terminal_then_cancel_too_late", false, "failed"),
        ("begin_finalization_then_cancel_too_late", true, "completed"),
    ] {
        let run_id = format!("run-15-{label}");
        let harness = if finalization {
            controlled_at_stage(&run_id, StageKind::Finalize, true)
        } else {
            controlled_at_stage(&run_id, StageKind::Transcribe, true)
        };
        let handle = harness.factory.last_handle();
        if finalization {
            assert!(handle.arbiter.begin_finalization());
        } else {
            assert!(handle.arbiter.begin_terminal());
        }
        let reply = cancel(&harness.controller, &run_id);
        let terminal = if finalization {
            StoppedTerminal::Completed {
                result: completed("final text", InsertResult::copy_only()),
                warning: None,
            }
        } else {
            failed_terminal("E_SCRIPTED", None)
        };
        handle.stopped(terminal, RunAudit::default());
        let view = harness.controller.snapshot();
        failures.check(
            label,
            reply.disposition == CommandDisposition::CancelTooLate
                && outcome_name(&view) == Some(expected),
            format!("reply={:?}, view={}", reply.disposition, wire(&view)),
        );
    }

    for terminal_kind in ["completed", "empty", "failed"] {
        let label = format!("accepted_cancel_then_illegal_{terminal_kind}");
        let run_id = format!("run-15-{label}");
        let harness = controlled_harness(
            seed("t15-illegal", false, false, false),
            [&run_id],
            [ControlledConfig::default()],
        );
        let _ = start(&harness.controller);
        let _ = cancel(&harness.controller, &run_id);
        harness
            .factory
            .last_handle()
            .stopped(protocol_terminal(terminal_kind), RunAudit::default());
        let view = harness.controller.snapshot();
        let observed = match view.last_run.as_ref().map(|run| &run.outcome) {
            Some(RunOutcomeView::Failed {
                primary_error,
                recovered_result,
                recovery_errors,
                protocol_context,
                ..
            }) => {
                primary_error.code == "E_EXECUTOR_TERMINAL_AFTER_CANCEL"
                    && protocol_context
                        .as_ref()
                        .map(|context| context.original_variant.as_str())
                        == Some(terminal_kind)
                    && match terminal_kind {
                        "completed" => {
                            recovered_result
                                .as_ref()
                                .map(|result| result.final_text.as_str())
                                == Some("preserved result")
                        }
                        "empty" => recovered_result.is_none() && recovery_errors.is_empty(),
                        "failed" => recovery_errors
                            .iter()
                            .any(|error| error.code == "E_RECOVERY"),
                        _ => false,
                    }
            }
            _ => false,
        };
        failures.check(&label, observed, format!("view={}", wire(&view)));
    }
    failures.finish("T15 cancel_terminal_and_finalization_are_linearizable", 9);
}

#[test]
fn target_contract_t16_late_or_duplicate_signal_has_no_effect() {
    let mut failures = TargetSubcases::default();

    for mode in ["ready", "active", "cancelling"] {
        for signal in ["progress", "terminal"] {
            let label = format!("{mode}_receives_old_run_{signal}");
            let harness = controlled_harness(
                seed("t16", false, false, false),
                ["run-16-current"],
                [ControlledConfig::default()],
            );
            if mode != "ready" {
                let _ = start(&harness.controller);
            }
            if mode == "cancelling" {
                let _ = cancel(&harness.controller, "run-16-current");
            }
            let before = harness.controller.snapshot();
            let before_sink = harness.sink.len();
            if signal == "progress" {
                harness
                    .controller
                    .submit_progress(Progress {
                        run_id: "run-16-old".to_string(),
                        payload: ProgressPayload::Transcribe(TranscribeProgress::Started {
                            elapsed_ms: None,
                        }),
                    })
                    .unwrap();
            } else {
                harness.controller.submit_stopped(Stopped {
                    run_id: "run-16-old".to_string(),
                    terminal: StoppedTerminal::Completed {
                        result: completed("old", InsertResult::copy_only()),
                        warning: None,
                    },
                    audit: RunAudit::default(),
                });
            }
            let after = harness.controller.snapshot();
            failures.check(
                &label,
                before == after && harness.sink.len() == before_sink,
                format!(
                    "signal={signal}, sink_before={before_sink}, sink_after={}, before={}, after={}",
                    harness.sink.len(),
                    wire(&before),
                    wire(&after)
                ),
            );
        }
    }

    for (outcome, name) in OutcomeFixture::ALL {
        let run_id = format!("run-16-duplicate-{name}");
        let harness = controller_after_outcome(outcome, &run_id, "run-16-next");
        let before = harness.controller.snapshot();
        let before_sink = harness.sink.len();
        harness.controller.submit_stopped(Stopped {
            run_id: run_id.clone(),
            terminal: match outcome {
                OutcomeFixture::Completed => StoppedTerminal::Completed {
                    result: completed("duplicate", InsertResult::copy_only()),
                    warning: None,
                },
                OutcomeFixture::Empty => StoppedTerminal::Empty {
                    timings: RunTimings::default(),
                },
                OutcomeFixture::Failed => failed_terminal("E_DUPLICATE", None),
                OutcomeFixture::Cancelled => StoppedTerminal::Cancelled {
                    recovered_result: None,
                    cleanup_diagnostic: None,
                },
            },
            audit: RunAudit::default(),
        });
        let after = harness.controller.snapshot();
        failures.check(
            &format!("ready_{name}_receives_duplicate_terminal"),
            before == after
                && harness.sink.len() == before_sink
                && after.last_run.as_ref().map(|run| run.stopped_count) == Some(1),
            format!(
                "sink_before={before_sink}, sink_after={}, before={}, after={}",
                harness.sink.len(),
                wire(&before),
                wire(&after)
            ),
        );
    }
    failures.finish("T16 late_or_duplicate_signal_has_no_effect", 10);
}

#[test]
fn target_contract_t17_new_run_after_any_outcome_gets_new_identity() {
    let mut failures = TargetSubcases::default();

    for (outcome, name) in OutcomeFixture::ALL {
        let old_run_id = format!("run-17-{name}");
        let next_run_id = format!("run-17-{name}-next");
        let harness = controller_after_outcome(outcome, &old_run_id, &next_run_id);
        let terminal = harness.controller.snapshot();
        let reply = start(&harness.controller);
        let active = harness.controller.snapshot();
        failures.check(
            &format!("fresh_primary_after_{name}"),
            reply.disposition == CommandDisposition::Applied
                && terminal.last_run.as_ref().map(|run| run.run_id.as_str())
                    == Some(old_run_id.as_str())
                && active.active_run.as_ref().map(|run| run.run_id.as_str())
                    == Some(next_run_id.as_str())
                && next_run_id != old_run_id,
            format!(
                "reply={:?}, terminal={}, active={}",
                reply.disposition,
                wire(&terminal),
                wire(&active)
            ),
        );
    }
    failures.finish("T17 new_run_after_any_outcome_gets_new_identity", 4);
}

#[test]
fn target_contract_t18_finalization_gate_excludes_partial_or_cancelled_commit() {
    let mut failures = TargetSubcases::default();

    let cancel_first = controlled_harness(
        seed("t18", false, false, false),
        ["run-18-cancel-first"],
        [ControlledConfig::default()],
    );
    let _ = start(&cancel_first.controller);
    let handle = cancel_first.factory.last_handle();
    let _ = cancel(&cancel_first.controller, "run-18-cancel-first");
    let finalization_rejected = !handle.arbiter.begin_finalization();
    handle.stopped(
        StoppedTerminal::Cancelled {
            recovered_result: None,
            cleanup_diagnostic: None,
        },
        RunAudit::default(),
    );
    let view = cancel_first.controller.snapshot();
    failures.check(
        "cancel_wins_before_finalization",
        finalization_rejected
            && outcome_name(&view) == Some("cancelled")
            && view
                .last_run
                .as_ref()
                .is_some_and(|run| run.effects == EffectCounts::default()),
        format!(
            "finalization_rejected={finalization_rejected}, view={}",
            wire(&view)
        ),
    );

    let finalization_first =
        controlled_at_stage("run-18-finalization-first", StageKind::Finalize, true);
    let handle = finalization_first.factory.last_handle();
    assert!(handle.arbiter.begin_finalization());
    let cancel_reply = cancel(&finalization_first.controller, "run-18-finalization-first");
    handle.stopped(
        StoppedTerminal::Completed {
            result: completed("final text", InsertResult::copy_only()),
            warning: None,
        },
        RunAudit {
            effects: EffectCounts {
                history_commit_count: 1,
                copy_count: 1,
                paste_count: 0,
            },
            finalization: FinalizationAudit { commit_count: 1 },
            cleanup: None,
        },
    );
    let view = finalization_first.controller.snapshot();
    failures.check(
        "finalization_wins_before_cancel",
        cancel_reply.disposition == CommandDisposition::CancelTooLate
            && outcome_name(&view) == Some("completed")
            && view.last_run.as_ref().is_some_and(|run| {
                run.effects.history_commit_count == 1 && run.effects.copy_count == 1
            }),
        format!("reply={:?}, view={}", cancel_reply.disposition, wire(&view)),
    );

    let duplicate = controller_after_outcome(
        OutcomeFixture::Completed,
        "run-18-duplicate",
        "run-18-unused",
    );
    let before = duplicate.controller.snapshot();
    let before_sink = duplicate.sink.len();
    duplicate.controller.submit_stopped(Stopped {
        run_id: "run-18-duplicate".to_string(),
        terminal: StoppedTerminal::Completed {
            result: completed("duplicate", InsertResult::copy_only()),
            warning: None,
        },
        audit: RunAudit {
            effects: EffectCounts {
                history_commit_count: 1,
                copy_count: 1,
                paste_count: 0,
            },
            finalization: FinalizationAudit { commit_count: 1 },
            cleanup: None,
        },
    });
    let after = duplicate.controller.snapshot();
    failures.check(
        "duplicate_finalization_callback_is_noop",
        before == after
            && duplicate.sink.len() == before_sink
            && after.last_run.as_ref().is_some_and(|run| {
                run.finalization.commit_count == 1
                    && run.effects.history_commit_count == 1
                    && run.effects.copy_count == 1
            }),
        format!(
            "sink_before={before_sink}, sink_after={}, before={}, after={}",
            duplicate.sink.len(),
            wire(&before),
            wire(&after)
        ),
    );

    let script = PortsScript {
        history: PortBehavior::Fail(WorkflowError::new(
            "E_HISTORY_WRITE",
            "scripted History write failure",
        )),
        transcript: "recoverable text".to_string(),
        ..Default::default()
    };
    let history = executor_harness(
        seed("history", false, false, false),
        "run-18-history-failure",
        script,
        false,
    );
    let reached_terminal = run_executor_to_terminal(&history);
    let view = history.controller.snapshot();
    let ports = history.ports.last_ports();
    let observed = matches!(
        view.last_run.as_ref().map(|run| &run.outcome),
        Some(RunOutcomeView::Failed {
            primary_error,
            recovered_result: Some(result),
            ..
        }) if primary_error.code == "E_HISTORY_WRITE"
            && result.final_text == "recoverable text"
    );
    failures.check(
        "history_failure_stops_copy_and_preserves_result",
        reached_terminal
            && observed
            && ports.calls.history.load(Ordering::Acquire) == 1
            && ports.calls.copy.load(Ordering::Acquire) == 0
            && view.last_run.as_ref().is_some_and(|run| {
                run.effects.history_commit_count == 0 && run.effects.copy_count == 0
            }),
        format!(
            "reached_terminal={reached_terminal}, history_calls={}, copy_calls={}, view={}",
            ports.calls.history.load(Ordering::Acquire),
            ports.calls.copy.load(Ordering::Acquire),
            wire(&view)
        ),
    );

    for terminal_kind in ["completed", "empty", "failed"] {
        let label = format!("cancel_winner_rejects_{terminal_kind}");
        let run_id = format!("run-18-{label}");
        let harness = controlled_harness(
            seed("t18-illegal", false, false, false),
            [&run_id],
            [ControlledConfig::default()],
        );
        let _ = start(&harness.controller);
        let _ = cancel(&harness.controller, &run_id);
        harness
            .factory
            .last_handle()
            .stopped(protocol_terminal(terminal_kind), RunAudit::default());
        let view = harness.controller.snapshot();
        let observed = matches!(
            view.last_run.as_ref().map(|run| &run.outcome),
            Some(RunOutcomeView::Failed {
                primary_error,
                protocol_context: Some(context),
                ..
            }) if primary_error.code == "E_EXECUTOR_TERMINAL_AFTER_CANCEL"
                && context.original_variant == terminal_kind
        );
        failures.check(&label, observed, format!("view={}", wire(&view)));
    }
    failures.finish(
        "T18 finalization_gate_excludes_partial_or_cancelled_commit",
        7,
    );
}

#[test]
fn target_contract_t19_projection_bootstrap_and_revision_order_are_total() {
    let mut failures = TargetSubcases::default();
    let ready = controlled_harness(
        seed("initial", false, false, false),
        ["run-19-order"],
        [ControlledConfig::default()],
    );
    let initial = ready.controller.snapshot();
    failures.check(
        "initial_snapshot_revision_zero",
        initial.mode == WorkflowMode::Ready
            && initial.revision == 0
            && initial.action_key == "Start(Initial)",
        format!("view={}", wire(&initial)),
    );

    let reply = start(&ready.controller);
    let diagnostic = inspection(&ready.controller);
    failures.check(
        "r1_commit_begin_sink_reply_projection",
        reply.view.mode == WorkflowMode::Recording
            && reply.view.revision == 1
            && reply
                .view
                .active_run
                .as_ref()
                .map(|run| run.run_id.as_str())
                == Some("run-19-order")
            && diagnostic.pointer("/projectionOrder")
                == Some(&json!(["commit", "beginAccepted", "snapshot", "reply"])),
        format!("reply={}, diagnostic={diagnostic}", wire(&reply.view)),
    );

    let before = ready.controller.snapshot();
    let before_sink = ready.sink.len();
    let parsed = serde_json::from_value::<WorkflowIntent>(json!({
        "kind": "reportCompleted",
        "runId": "run-19-order",
        "result": {"finalText": "frontend-owned"}
    }));
    let after = ready.controller.snapshot();
    failures.check(
        "frontend_report_apply_path_is_absent",
        parsed.is_err() && before == after && ready.sink.len() == before_sink,
        format!(
            "parsed={parsed:?}, sink_before={before_sink}, sink_after={}, before={}, after={}",
            ready.sink.len(),
            wire(&before),
            wire(&after)
        ),
    );
    failures.finish("T19 projection_bootstrap_and_revision_order_are_total", 3);
}

#[test]
fn target_contract_t20_ready_implies_no_live_run_resources() {
    let mut failures = TargetSubcases::default();

    for (_outcome, name) in OutcomeFixture::ALL {
        let run_id = format!("run-20-{name}");
        let mut plan = seed("t20", false, false, false);
        let mut script = PortsScript::default();
        match name {
            "completed" => {}
            "empty" => script.transcript.clear(),
            "failed" => {
                script.transcribe = PortBehavior::Fail(WorkflowError::new(
                    "E_SCRIPTED_FAILURE",
                    "scripted terminal failure",
                ));
            }
            "cancelled" => {
                plan.context.enabled = true;
                script.context = PortBehavior::Pending;
            }
            _ => unreachable!(),
        }
        let harness = executor_harness(plan, &run_id, script, false);
        let _ = start(&harness.controller);
        if name == "cancelled" {
            let _ = cancel(&harness.controller, &run_id);
        } else if harness.controller.snapshot().mode == WorkflowMode::Recording {
            let _ = stop(&harness.controller);
        }
        let reached_terminal = wait_ready(&harness.controller);
        let view = harness.controller.snapshot();
        let resources = harness.factory.last_handle().inspect().resource_counts;
        failures.check(
            &format!("ready_after_{name}_has_no_live_resources"),
            reached_terminal
                && view.mode == WorkflowMode::Ready
                && view.active_run.is_none()
                && outcome_name(&view) == Some(name)
                && resources == ResourceCounts::default(),
            format!(
                "reached_terminal={reached_terminal}, resources={resources:?}, view={}",
                wire(&view)
            ),
        );
    }
    failures.finish("T20 ready_implies_no_live_run_resources", 4);
}

#[test]
fn target_contract_t21_cleanup_timeout_escalates_or_fails_closed() {
    let mut failures = TargetSubcases::default();

    for winner in ["failed", "cancelled"] {
        let run_id = format!("run-21-force-success-{winner}");
        let mut plan = seed("t21", false, false, false);
        let mut script = PortsScript {
            shutdown: PortBehavior::Pending,
            force_shutdown: PortBehavior::Ok,
            force_releases: true,
            ..Default::default()
        };
        if winner == "failed" {
            script.transcribe = PortBehavior::Fail(WorkflowError::new(
                "E_EFFECT_TIMEOUT",
                "scripted cooperative timeout",
            ));
        } else {
            plan.context.enabled = true;
            script.context = PortBehavior::Pending;
        }
        let harness = executor_harness(plan, &run_id, script, false);
        let _ = start(&harness.controller);
        if winner == "failed" {
            let _ = stop(&harness.controller);
        } else {
            let _ = cancel(&harness.controller, &run_id);
        }
        let reached_terminal = wait_ready(&harness.controller);
        let view = harness.controller.snapshot();
        let diagnostic = view
            .last_run
            .as_ref()
            .and_then(|run| run.cleanup_diagnostic.as_ref());
        failures.check(
            &format!("cooperative_timeout_force_success_{winner}"),
            reached_terminal
                && outcome_name(&view) == Some(winner)
                && diagnostic.is_some_and(|diagnostic| {
                    diagnostic.graceful_timed_out
                        && diagnostic.force_attempted
                        && diagnostic.force_succeeded
                })
                && view.last_run.as_ref().map(|run| run.stopped_count) == Some(1)
                && harness
                    .factory
                    .last_handle()
                    .inspect()
                    .resource_counts
                    == ResourceCounts::default(),
            format!(
                "winner={winner}, reached_terminal={reached_terminal}, diagnostic={diagnostic:?}, handle={:?}, view={}",
                harness.factory.last_handle().inspect(),
                wire(&view)
            ),
        );
    }

    let script = PortsScript {
        context: PortBehavior::Pending,
        shutdown: PortBehavior::Pending,
        force_shutdown: PortBehavior::Ok,
        force_releases: false,
        ..Default::default()
    };
    let harness = executor_harness(
        seed("t21-fatal", false, false, true),
        "run-21-force-failure",
        script,
        false,
    );
    let _ = start(&harness.controller);
    let _ = cancel(&harness.controller, "run-21-force-failure");
    let fatal_observed = wait_until("fatal containment", Duration::from_secs(3), || {
        inspection(&harness.controller).pointer("/mode") == Some(&json!("fatal"))
    });
    let fatal = inspection(&harness.controller);
    let next = harness.controller.command(WorkflowIntent::Primary {
        action_key: harness.controller.snapshot().action_key,
    });
    failures.check(
        "cooperative_and_force_timeout_fail_closed",
        fatal_observed
            && fatal.pointer("/mode") == Some(&json!("fatal"))
            && fatal.pointer("/fatal/error/code") == Some(&json!("E_RESOURCE_RELEASE_UNPROVEN"))
            && fatal.get("lastRun") == Some(&Value::Null)
            && next
                .as_ref()
                .is_err_and(|error| error.code == "E_RESOURCE_RELEASE_UNPROVEN"),
        format!(
            "fatal_observed={fatal_observed}, next_error={:?}, fatal={fatal}",
            next.as_ref().err()
        ),
    );
    failures.finish("T21 cleanup_timeout_escalates_or_fails_closed", 3);
}
