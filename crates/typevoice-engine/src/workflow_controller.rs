use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use typevoice_core::workflow::{
    ActiveRunView, CancelDisposition, CommandDisposition, LastRunView, Progress, ProgressPayload,
    ProtocolContext, RecoveredRunResult, RunAudit, RunId, RunOutcomeView, RunPlanSeed, StageKind,
    StageStatus, StageView, Stopped, StoppedTerminal, TranscribeProgress, WorkflowCommandReply,
    WorkflowError, WorkflowIntent, WorkflowMode, WorkflowView,
};

use crate::run_executor::{BeginAccepted, RunFactory, RunHandle, RunSignalSink};

pub type WorkflowResult<T> = Result<T, WorkflowError>;

pub trait WorkflowSnapshotSink: Send + Sync {
    fn publish(&self, view: &WorkflowView) -> WorkflowResult<()>;

    fn diagnostic(&self, _run_id: &str, _code: &str, _message: &str, _context: Value) {}

    fn fatal(&self, _run_id: &str, _error: &WorkflowError, _context: Value, _deadline: Instant) {}
}

pub trait RunIdSource: Send + Sync {
    fn next_run_id(&self) -> RunId;
}

pub trait WorkflowClock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct UuidRunIdSource;

impl RunIdSource for UuidRunIdSource {
    fn next_run_id(&self) -> RunId {
        uuid::Uuid::new_v4().to_string()
    }
}

pub struct SystemWorkflowClock;

impl WorkflowClock for SystemWorkflowClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

pub struct NullSnapshotSink;

impl WorkflowSnapshotSink for NullSnapshotSink {
    fn publish(&self, _view: &WorkflowView) -> WorkflowResult<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextState {
    Pending,
    Running,
    Completed,
    Skipped,
}

struct ActiveRun {
    run_id: RunId,
    plan_seed: RunPlanSeed,
    handle: Arc<dyn RunHandle>,
    stage: Option<StageView>,
    result: Option<RecoveredRunResult>,
    context: ContextState,
    #[cfg_attr(not(test), allow(dead_code))]
    committed_at_ms: u64,
    begin_accepted_at_ms: Option<u64>,
    capture_started_at_ms: Option<u64>,
    projection_order: Vec<String>,
}

enum ModeState {
    Ready,
    Recording(ActiveRun),
    Processing(ActiveRun),
    Cancelling(ActiveRun),
}

struct ControllerState {
    mode: ModeState,
    revision: u64,
    last_run: Option<LastRunView>,
    cached_seed: RunPlanSeed,
    last_disposition: Option<CommandDisposition>,
    fatal: Option<WorkflowError>,
}

pub struct WorkflowController {
    turn: Mutex<()>,
    state: Mutex<ControllerState>,
    factory: Arc<dyn RunFactory>,
    sink: Arc<dyn WorkflowSnapshotSink>,
    ids: Arc<dyn RunIdSource>,
    clock: Arc<dyn WorkflowClock>,
}

impl WorkflowController {
    pub fn new(
        seed: RunPlanSeed,
        factory: Arc<dyn RunFactory>,
        sink: Arc<dyn WorkflowSnapshotSink>,
        ids: Arc<dyn RunIdSource>,
        clock: Arc<dyn WorkflowClock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            turn: Mutex::new(()),
            state: Mutex::new(ControllerState {
                mode: ModeState::Ready,
                revision: 0,
                last_run: None,
                cached_seed: seed,
                last_disposition: None,
                fatal: None,
            }),
            factory,
            sink,
            ids,
            clock,
        })
    }

    pub fn snapshot(&self) -> WorkflowView {
        view_from_state(&self.state.lock().unwrap())
    }

    pub fn update_cached_seed(&self, seed: RunPlanSeed) {
        self.state.lock().unwrap().cached_seed = seed;
    }

    pub fn command(
        self: &Arc<Self>,
        intent: WorkflowIntent,
    ) -> WorkflowResult<WorkflowCommandReply> {
        self.command_with_source(intent, None)
    }

    fn command_with_source(
        self: &Arc<Self>,
        intent: WorkflowIntent,
        intent_source: Option<&str>,
    ) -> WorkflowResult<WorkflowCommandReply> {
        let _turn = self.turn.lock().unwrap();
        if let Some(error) = self.state.lock().unwrap().fatal.clone() {
            return Err(error);
        }

        if !self.admitted(&intent) {
            return Ok(self.reply(CommandDisposition::NoOp));
        }

        match intent {
            WorkflowIntent::Primary { .. } => self.primary_locked(intent_source),
            WorkflowIntent::Cancel { .. } => self.cancel_locked(),
        }
    }

    pub fn command_from_hotkey(self: &Arc<Self>) -> WorkflowResult<WorkflowCommandReply> {
        let _turn = self.turn.lock().unwrap();
        if let Some(error) = self.state.lock().unwrap().fatal.clone() {
            return Err(error);
        }
        self.primary_locked(Some("hotkey"))
    }

    pub fn submit_progress(&self, progress: Progress) -> WorkflowResult<()> {
        let _turn = self.turn.lock().unwrap();
        let signal_kind = stage_kind_name(progress.payload.kind());
        let mut diagnostic = None;
        let result = {
            let mut state = self.state.lock().unwrap();
            let current_run_id = active_run(&state.mode).map(|active| active.run_id.clone());
            if current_run_id.as_deref() != Some(progress.run_id.as_str()) {
                diagnostic = Some((
                    "E_EXECUTOR_LATE_SIGNAL",
                    "progress signal does not belong to the active run",
                    json!({
                        "signal": "Progress",
                        "signalStage": signal_kind,
                        "currentRunId": current_run_id,
                        "revision": state.revision,
                    }),
                ));
                Ok(None)
            } else if matches!(state.mode, ModeState::Cancelling(_)) {
                diagnostic = Some((
                    "E_EXECUTOR_SIGNAL_AFTER_CANCEL",
                    "progress signal arrived after cancellation was accepted",
                    json!({
                        "signal": "Progress",
                        "signalStage": signal_kind,
                        "revision": state.revision,
                    }),
                ));
                Ok(None)
            } else {
                match apply_progress(&mut state.mode, progress.payload) {
                    Ok(()) => {
                        state.revision = state.revision.saturating_add(1);
                        Ok(Some(view_from_state(&state)))
                    }
                    Err(error) => {
                        diagnostic = Some((
                            "E_EXECUTOR_PROGRESS_ORDER",
                            "progress signal violates the current stage order",
                            json!({
                                "signal": "Progress",
                                "signalStage": signal_kind,
                                "revision": state.revision,
                            }),
                        ));
                        Err(error)
                    }
                }
            }
        };
        if let Some((code, message, context)) = diagnostic {
            self.sink
                .diagnostic(&progress.run_id, code, message, context);
        }
        if let Some(view) = result? {
            self.publish(&view);
        }
        Ok(())
    }

    pub fn submit_stopped(&self, stopped: Stopped) {
        let _turn = self.turn.lock().unwrap();
        let stopped_run_id = stopped.run_id.clone();
        let mut diagnostic = None;
        let view = {
            let mut state = self.state.lock().unwrap();
            let terminal = match &state.mode {
                ModeState::Ready => {
                    diagnostic = Some(json!({
                        "signal": "Stopped",
                        "reason": "controllerReady",
                        "revision": state.revision,
                    }));
                    None
                }
                ModeState::Recording(active)
                | ModeState::Processing(active)
                | ModeState::Cancelling(active)
                    if active.run_id != stopped.run_id =>
                {
                    diagnostic = Some(json!({
                        "signal": "Stopped",
                        "reason": "runIdMismatch",
                        "currentRunId": active.run_id,
                        "revision": state.revision,
                    }));
                    None
                }
                ModeState::Recording(_) => Some(normalize_recording_terminal(stopped.terminal)),
                ModeState::Processing(active) => Some(normalize_processing_terminal(
                    active.stage.as_ref(),
                    stopped.terminal,
                )),
                ModeState::Cancelling(_) => Some(normalize_cancelling_terminal(stopped.terminal)),
            };
            terminal.map(|terminal| {
                let last_run = last_run_from_terminal(stopped.run_id, terminal, stopped.audit);
                state.mode = ModeState::Ready;
                state.last_run = Some(last_run);
                state.revision = state.revision.saturating_add(1);
                view_from_state(&state)
            })
        };
        if let Some(context) = diagnostic {
            self.sink.diagnostic(
                &stopped_run_id,
                "E_EXECUTOR_LATE_SIGNAL",
                "Stopped signal does not belong to the active run",
                context,
            );
        }
        if let Some(view) = view {
            self.publish(&view);
        }
    }

    pub fn submit_fatal(
        &self,
        run_id: &str,
        error: WorkflowError,
        context: Value,
        deadline: Instant,
    ) {
        let _turn = self.turn.lock().unwrap();
        let accepted = {
            let mut state = self.state.lock().unwrap();
            let is_current = match &state.mode {
                ModeState::Ready => false,
                ModeState::Recording(active)
                | ModeState::Processing(active)
                | ModeState::Cancelling(active) => active.run_id == run_id,
            };
            if is_current {
                state.fatal = Some(error.clone());
            }
            is_current
        };
        if accepted {
            self.sink.fatal(run_id, &error, context, deadline);
        }
    }

    fn admitted(&self, intent: &WorkflowIntent) -> bool {
        let state = self.state.lock().unwrap();
        match intent {
            WorkflowIntent::Primary { action_key } => {
                action_key == &derive_action_key(&state.mode, state.last_run.as_ref())
            }
            WorkflowIntent::Cancel { target_run_id } => {
                Some(target_run_id.as_str())
                    == active_run(&state.mode).map(|active| active.run_id.as_str())
            }
        }
    }

    fn primary_locked(
        self: &Arc<Self>,
        intent_source: Option<&str>,
    ) -> WorkflowResult<WorkflowCommandReply> {
        enum PrimaryAction {
            Start,
            Stop(Arc<dyn RunHandle>),
            Cancel(Arc<dyn RunHandle>),
            NoOp,
        }

        let action = {
            let state = self.state.lock().unwrap();
            match &state.mode {
                ModeState::Ready => PrimaryAction::Start,
                ModeState::Recording(active) => PrimaryAction::Stop(active.handle.clone()),
                ModeState::Processing(active) => PrimaryAction::Cancel(active.handle.clone()),
                ModeState::Cancelling(_) => PrimaryAction::NoOp,
            }
        };

        match action {
            PrimaryAction::Start => self.start_locked(intent_source),
            PrimaryAction::Stop(handle) => self.stop_locked(handle),
            PrimaryAction::Cancel(handle) => self.request_cancel_locked(handle),
            PrimaryAction::NoOp => Ok(self.reply(CommandDisposition::NoOp)),
        }
    }

    fn cancel_locked(&self) -> WorkflowResult<WorkflowCommandReply> {
        let handle = {
            let state = self.state.lock().unwrap();
            match &state.mode {
                ModeState::Ready | ModeState::Cancelling(_) => None,
                ModeState::Recording(active) | ModeState::Processing(active) => {
                    Some(active.handle.clone())
                }
            }
        };
        match handle {
            Some(handle) => self.request_cancel_locked(handle),
            None => Ok(self.reply(CommandDisposition::NoOp)),
        }
    }

    fn start_locked(
        self: &Arc<Self>,
        intent_source: Option<&str>,
    ) -> WorkflowResult<WorkflowCommandReply> {
        let run_id = self.ids.next_run_id();
        let committed_at_ms = self.clock.now_ms();
        let mut seed = self.state.lock().unwrap().cached_seed.clone();
        if let Some(intent_source) = intent_source {
            seed.intent_source = intent_source.to_string();
        }
        let signal_sink: Arc<dyn RunSignalSink> = Arc::new(ControllerSignalTarget {
            controller: Arc::downgrade(self),
        });
        let handle = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.factory
                .create(run_id.clone(), seed.clone(), signal_sink.clone())
        }))
        .map_err(|_| {
            WorkflowError::new(
                "E_EXECUTOR_ABNORMAL_EXIT",
                "run executor factory panicked during construction",
            )
        })?;
        let initial_context = if seed.context.enabled {
            ContextState::Pending
        } else {
            ContextState::Skipped
        };
        let stage = if seed.context.enabled {
            Some(StageView::pending(StageKind::ContextCapture))
        } else {
            None
        };

        {
            let mut state = self.state.lock().unwrap();
            state.mode = ModeState::Recording(ActiveRun {
                run_id: run_id.clone(),
                plan_seed: seed,
                handle: handle.clone(),
                stage,
                result: None,
                context: initial_context,
                committed_at_ms,
                begin_accepted_at_ms: None,
                capture_started_at_ms: None,
                projection_order: vec!["commit".to_string()],
            });
            state.revision = state.revision.saturating_add(1);
        }

        let begin = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle.begin()))
            .unwrap_or_else(|_| {
                Err(WorkflowError::new(
                    "E_EXECUTOR_ABNORMAL_EXIT",
                    "run executor panicked during Begin",
                ))
            });
        let recording_view = {
            let mut state = self.state.lock().unwrap();
            if let ModeState::Recording(active) = &mut state.mode {
                if active.run_id == run_id {
                    match begin {
                        Ok(BeginAccepted {
                            capture_started_at_ms,
                        }) => {
                            active.begin_accepted_at_ms = Some(self.clock.now_ms());
                            active.capture_started_at_ms = Some(capture_started_at_ms);
                            active.projection_order.push("beginAccepted".to_string());
                        }
                        Err(_) => active.projection_order.push("beginFailed".to_string()),
                    }
                    active.projection_order.push("snapshot".to_string());
                }
            }
            view_from_state(&state)
        };
        self.publish(&recording_view);

        if let Err(error) = begin {
            self.settle_control_failure_safely(&handle, error);
        }

        {
            let mut state = self.state.lock().unwrap();
            if let Some(active) = active_run_mut(&mut state.mode) {
                if active.run_id == handle.run_id() {
                    active.projection_order.push("reply".to_string());
                }
            }
        }
        Ok(self.reply(CommandDisposition::Applied))
    }

    fn stop_locked(&self, handle: Arc<dyn RunHandle>) -> WorkflowResult<WorkflowCommandReply> {
        let view = {
            let mut state = self.state.lock().unwrap();
            let active = match std::mem::replace(&mut state.mode, ModeState::Ready) {
                ModeState::Recording(active) => active,
                other => {
                    state.mode = other;
                    return Err(WorkflowError::new(
                        "E_CONTROLLER_INVARIANT",
                        "Stop was selected without an active Recording run",
                    ));
                }
            };
            let stage = match active.context {
                ContextState::Pending => StageView::pending(StageKind::ContextCapture),
                ContextState::Running => StageView {
                    kind: StageKind::ContextCapture,
                    status: StageStatus::Started,
                    elapsed_ms: active.stage.as_ref().and_then(|stage| stage.elapsed_ms),
                },
                ContextState::Completed | ContextState::Skipped => {
                    StageView::pending(StageKind::RecordFinalize)
                }
            };
            state.mode = ModeState::Processing(ActiveRun {
                stage: Some(stage),
                ..active
            });
            state.revision = state.revision.saturating_add(1);
            view_from_state(&state)
        };

        let stop_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle.stop()))
            .unwrap_or_else(|_| {
                Err(WorkflowError::new(
                    "E_EXECUTOR_ABNORMAL_EXIT",
                    "run executor panicked while delivering Stop",
                ))
            });
        self.publish(&view);
        if let Err(error) = stop_result {
            self.settle_control_failure_safely(&handle, error);
        }
        Ok(self.reply(CommandDisposition::Applied))
    }

    fn request_cancel_locked(
        &self,
        handle: Arc<dyn RunHandle>,
    ) -> WorkflowResult<WorkflowCommandReply> {
        let disposition =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle.request_cancel()))
                .map_err(|_| {
                    WorkflowError::new(
                        "E_EXECUTOR_ABNORMAL_EXIT",
                        "run executor panicked while arbitrating Cancel",
                    )
                });
        let disposition = match disposition {
            Ok(disposition) => disposition,
            Err(error) => {
                self.settle_control_failure_safely(&handle, error.clone());
                return Err(error);
            }
        };
        match disposition {
            CancelDisposition::TooLate => Ok(self.reply(CommandDisposition::CancelTooLate)),
            CancelDisposition::Accepted => {
                let view = {
                    let mut state = self.state.lock().unwrap();
                    let previous = std::mem::replace(&mut state.mode, ModeState::Ready);
                    state.mode = match previous {
                        ModeState::Recording(active) | ModeState::Processing(active) => {
                            ModeState::Cancelling(active)
                        }
                        other => other,
                    };
                    state.revision = state.revision.saturating_add(1);
                    view_from_state(&state)
                };
                self.publish(&view);
                Ok(self.reply(CommandDisposition::Applied))
            }
        }
    }

    fn reply(&self, disposition: CommandDisposition) -> WorkflowCommandReply {
        let mut state = self.state.lock().unwrap();
        state.last_disposition = Some(disposition);
        WorkflowCommandReply {
            disposition,
            view: view_from_state(&state),
        }
    }

    fn publish(&self, view: &WorkflowView) {
        let _ = self.sink.publish(view);
    }

    fn settle_control_failure_safely(&self, handle: &Arc<dyn RunHandle>, error: WorkflowError) {
        let run_id = handle.run_id().to_string();
        let control_error = error.clone();
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle.settle_control_failure(error)
        }))
        .is_err()
        {
            let fatal_deadline = Instant::now() + Duration::from_millis(100);
            let fatal = WorkflowError::new(
                "E_RESOURCE_RELEASE_UNPROVEN",
                "run control failure terminalizer panicked before resource release was proven",
            );
            let view = {
                let mut state = self.state.lock().unwrap();
                state.fatal = Some(fatal.clone());
                view_from_state(&state)
            };
            self.publish(&view);
            self.sink.diagnostic(
                &run_id,
                "workflow.control_terminalization_fatal",
                "run control failure terminalizer panicked",
                json!({
                    "controlError": control_error,
                    "fatalError": fatal,
                }),
            );
            self.sink.fatal(
                &run_id,
                &fatal,
                json!({
                    "source": "controlTerminalizer",
                    "controlErrorCode": control_error.code.as_str(),
                    "fatalErrorCode": fatal.code.as_str(),
                }),
                fatal_deadline,
            );
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn inspection_value(&self) -> Value {
        let state = self.state.lock().unwrap();
        let mut value = serde_json::to_value(view_from_state(&state)).unwrap_or_else(|_| json!({}));
        let Value::Object(root) = &mut value else {
            return value;
        };
        if let Some(disposition) = state.last_disposition {
            root.insert(
                "lastCommand".to_string(),
                json!({"disposition": disposition}),
            );
            root.insert("commandDisposition".to_string(), json!(disposition));
        }
        if let Some(error) = &state.fatal {
            root.insert("fatal".to_string(), json!({"error": error}));
            root.insert("mode".to_string(), json!("fatal"));
        }
        if let Some(active) = active_run(&state.mode) {
            if let Some(Value::Object(active_value)) = root.get_mut("activeRun") {
                active_value.insert("planSeed".to_string(), json!(active.plan_seed));
                active_value.insert("committedAtMs".to_string(), json!(active.committed_at_ms));
                active_value.insert(
                    "beginAcceptedAtMs".to_string(),
                    json!(active.begin_accepted_at_ms),
                );
                active_value.insert(
                    "captureStartedAtMs".to_string(),
                    json!(active.capture_started_at_ms),
                );
                let inspection = active.handle.inspect();
                active_value.insert("arbiter".to_string(), json!(inspection.arbiter));
                active_value.insert(
                    "resourceCounts".to_string(),
                    json!(inspection.resource_counts),
                );
            }
            root.insert(
                "projectionOrder".to_string(),
                json!(active.projection_order),
            );
            if let Some(stage) = &active.stage {
                root.insert(
                    "stage".to_string(),
                    json!({"name": stage_kind_name(stage.kind), "status": stage.status}),
                );
            }
        }
        value
    }
}

struct ControllerSignalTarget {
    controller: Weak<WorkflowController>,
}

impl RunSignalSink for ControllerSignalTarget {
    fn progress(&self, progress: Progress) {
        if let Some(controller) = self.controller.upgrade() {
            let _ = controller.submit_progress(progress);
        }
    }

    fn stopped(&self, stopped: Stopped) {
        if let Some(controller) = self.controller.upgrade() {
            controller.submit_stopped(stopped);
        }
    }

    fn fatal(&self, run_id: &str, error: WorkflowError, context: Value, deadline: Instant) {
        if let Some(controller) = self.controller.upgrade() {
            controller.submit_fatal(run_id, error, context, deadline);
        }
    }
}

fn active_run(mode: &ModeState) -> Option<&ActiveRun> {
    match mode {
        ModeState::Ready => None,
        ModeState::Recording(active)
        | ModeState::Processing(active)
        | ModeState::Cancelling(active) => Some(active),
    }
}

fn active_run_mut(mode: &mut ModeState) -> Option<&mut ActiveRun> {
    match mode {
        ModeState::Ready => None,
        ModeState::Recording(active)
        | ModeState::Processing(active)
        | ModeState::Cancelling(active) => Some(active),
    }
}

fn derive_action_key(mode: &ModeState, last_run: Option<&LastRunView>) -> String {
    match mode {
        ModeState::Ready => match last_run {
            Some(last_run) => format!("Start(After({}))", last_run.run_id),
            None => "Start(Initial)".to_string(),
        },
        ModeState::Recording(active) => format!("Stop({})", active.run_id),
        ModeState::Processing(active) => format!("Cancel({})", active.run_id),
        ModeState::Cancelling(active) => format!("NoOp({})", active.run_id),
    }
}

fn view_from_state(state: &ControllerState) -> WorkflowView {
    let (mode, primary_label, primary_disabled, cancel_enabled) = match &state.mode {
        ModeState::Ready => (WorkflowMode::Ready, "START", false, false),
        ModeState::Recording(_) => (WorkflowMode::Recording, "STOP", false, true),
        ModeState::Processing(active) => (
            WorkflowMode::Processing,
            "CANCEL",
            false,
            active
                .stage
                .as_ref()
                .is_none_or(|stage| stage.kind != StageKind::Finalize),
        ),
        ModeState::Cancelling(_) => (WorkflowMode::Cancelling, "CANCELLING", true, false),
    };
    WorkflowView {
        revision: state.revision,
        action_key: derive_action_key(&state.mode, state.last_run.as_ref()),
        mode,
        active_run: active_run(&state.mode).map(|active| ActiveRunView {
            run_id: active.run_id.clone(),
            stage: active.stage.clone(),
            result: active.result.clone(),
        }),
        last_run: state.last_run.clone(),
        primary_label: primary_label.to_string(),
        primary_disabled,
        cancel_enabled,
    }
}

fn apply_progress(mode: &mut ModeState, payload: ProgressPayload) -> WorkflowResult<()> {
    match mode {
        ModeState::Recording(active) => apply_recording_progress(active, payload),
        ModeState::Processing(active) => apply_processing_progress(active, payload),
        ModeState::Ready | ModeState::Cancelling(_) => Ok(()),
    }
}

fn apply_recording_progress(
    active: &mut ActiveRun,
    payload: ProgressPayload,
) -> WorkflowResult<()> {
    let ProgressPayload::ContextCapture(progress) = payload else {
        return Err(progress_order_error());
    };
    let current = active.stage.as_ref().ok_or_else(progress_order_error)?;
    if current.kind != StageKind::ContextCapture
        || !valid_status_transition(current.status, progress.status)
    {
        return Err(progress_order_error());
    }
    active.context = match progress.status {
        StageStatus::Pending => return Err(progress_order_error()),
        StageStatus::Started => ContextState::Running,
        StageStatus::Completed => ContextState::Completed,
    };
    active.stage = Some(StageView {
        kind: StageKind::ContextCapture,
        status: progress.status,
        elapsed_ms: progress.elapsed_ms,
    });
    Ok(())
}

fn apply_processing_progress(
    active: &mut ActiveRun,
    payload: ProgressPayload,
) -> WorkflowResult<()> {
    let next_kind = payload.kind();
    let next_status = payload.status();
    let current = active.stage.as_ref().ok_or_else(progress_order_error)?;
    let same_stage = current.kind == next_kind;
    let rewrite_recovery_finalization = current.kind == StageKind::Rewrite
        && current.status == StageStatus::Started
        && next_kind == StageKind::Finalize
        && next_status == StageStatus::Started;
    if !(same_stage && valid_status_transition(current.status, next_status)
        || rewrite_recovery_finalization)
    {
        return Err(progress_order_error());
    }

    match &payload {
        ProgressPayload::ContextCapture(_) => {
            active.context = match next_status {
                StageStatus::Started => ContextState::Running,
                StageStatus::Completed => ContextState::Completed,
                StageStatus::Pending => return Err(progress_order_error()),
            };
        }
        ProgressPayload::Transcribe(TranscribeProgress::Completed { result, .. }) => {
            active.result = Some(RecoveredRunResult {
                asr_text: result.asr_text.clone(),
                final_text: result.final_text.clone(),
                timings: typevoice_core::workflow::RunTimings {
                    total_ms: result.metrics.preprocess_ms + result.metrics.asr_ms,
                    preprocess_ms: Some(result.metrics.preprocess_ms),
                    asr_ms: Some(result.metrics.asr_ms),
                    ..Default::default()
                },
                metrics: Some(result.metrics.clone()),
            });
        }
        ProgressPayload::Rewrite(typevoice_core::workflow::RewriteProgress::Completed {
            result,
            ..
        }) => {
            let Some(recovered) = active.result.as_mut() else {
                return Err(progress_order_error());
            };
            recovered.final_text = result.final_text.clone();
            recovered.timings.rewrite_ms = Some(result.rewrite_ms);
            recovered.timings.total_ms =
                recovered.timings.total_ms.saturating_add(result.rewrite_ms);
        }
        _ => {}
    }
    active.stage = Some(StageView {
        kind: next_kind,
        status: next_status,
        elapsed_ms: payload.elapsed_ms(),
    });
    if next_status == StageStatus::Completed {
        if let Some(next_kind) = next_processing_stage(next_kind, &active.plan_seed) {
            active.stage = Some(StageView::pending(next_kind));
        }
    }
    Ok(())
}

fn valid_status_transition(current: StageStatus, next: StageStatus) -> bool {
    matches!(
        (current, next),
        (StageStatus::Pending, StageStatus::Started)
            | (StageStatus::Started, StageStatus::Completed)
    )
}

fn next_processing_stage(kind: StageKind, seed: &RunPlanSeed) -> Option<StageKind> {
    match kind {
        StageKind::ContextCapture => Some(StageKind::RecordFinalize),
        StageKind::RecordFinalize => Some(StageKind::Preprocess),
        StageKind::Preprocess => Some(StageKind::Transcribe),
        StageKind::Transcribe if seed.rewrite.enabled => Some(StageKind::Rewrite),
        StageKind::Transcribe => Some(StageKind::InsertPrepare),
        StageKind::Rewrite => Some(StageKind::InsertPrepare),
        StageKind::InsertPrepare => Some(StageKind::Finalize),
        StageKind::Finalize => None,
    }
}

fn progress_order_error() -> WorkflowError {
    WorkflowError::new(
        "E_EXECUTOR_PROGRESS_ORDER",
        "executor progress is not valid for the current run stage",
    )
}

fn normalize_recording_terminal(terminal: StoppedTerminal) -> StoppedTerminal {
    match terminal {
        StoppedTerminal::Failed { .. } => terminal,
        StoppedTerminal::Cancelled {
            recovered_result, ..
        } => protocol_failure(
            "E_EXECUTOR_CANCEL_UNACKNOWLEDGED",
            "cancelled",
            None,
            recovered_result,
            Vec::new(),
        ),
        StoppedTerminal::Completed { result, warning } => protocol_failure(
            "E_EXECUTOR_TERMINAL_ORDER",
            "completed",
            warning,
            Some(recovered_from_completed(&result)),
            Vec::new(),
        ),
        StoppedTerminal::Empty { .. } => {
            protocol_failure("E_EXECUTOR_TERMINAL_ORDER", "empty", None, None, Vec::new())
        }
    }
}

fn normalize_processing_terminal(
    stage: Option<&StageView>,
    terminal: StoppedTerminal,
) -> StoppedTerminal {
    match terminal {
        StoppedTerminal::Completed { result, warning }
            if stage.is_some_and(|stage| stage.kind == StageKind::Finalize) =>
        {
            StoppedTerminal::Completed { result, warning }
        }
        StoppedTerminal::Completed { result, warning } => protocol_failure(
            "E_EXECUTOR_TERMINAL_ORDER",
            "completed",
            warning,
            Some(recovered_from_completed(&result)),
            Vec::new(),
        ),
        StoppedTerminal::Empty { timings }
            if stage.is_some_and(|stage| stage.kind == StageKind::Transcribe) =>
        {
            StoppedTerminal::Empty { timings }
        }
        StoppedTerminal::Empty { .. } => {
            protocol_failure("E_EXECUTOR_TERMINAL_ORDER", "empty", None, None, Vec::new())
        }
        StoppedTerminal::Failed { .. } => terminal,
        StoppedTerminal::Cancelled {
            recovered_result, ..
        } => protocol_failure(
            "E_EXECUTOR_CANCEL_UNACKNOWLEDGED",
            "cancelled",
            None,
            recovered_result,
            Vec::new(),
        ),
    }
}

fn normalize_cancelling_terminal(terminal: StoppedTerminal) -> StoppedTerminal {
    match terminal {
        StoppedTerminal::Cancelled { .. } => terminal,
        StoppedTerminal::Completed { result, warning } => protocol_failure(
            "E_EXECUTOR_TERMINAL_AFTER_CANCEL",
            "completed",
            warning,
            Some(recovered_from_completed(&result)),
            Vec::new(),
        ),
        StoppedTerminal::Empty { .. } => protocol_failure(
            "E_EXECUTOR_TERMINAL_AFTER_CANCEL",
            "empty",
            None,
            None,
            Vec::new(),
        ),
        StoppedTerminal::Failed {
            error,
            recovered_result,
            recovery_errors,
            ..
        } => protocol_failure(
            "E_EXECUTOR_TERMINAL_AFTER_CANCEL",
            "failed",
            Some(error),
            recovered_result,
            recovery_errors,
        ),
    }
}

fn protocol_failure(
    code: &str,
    original_variant: &str,
    original_error: Option<WorkflowError>,
    recovered_result: Option<RecoveredRunResult>,
    recovery_errors: Vec<WorkflowError>,
) -> StoppedTerminal {
    StoppedTerminal::Failed {
        error: WorkflowError::new(code, "executor terminal violates the workflow protocol"),
        recovered_result,
        recovery_errors,
        record_saved: false,
        protocol_context: Some(ProtocolContext {
            original_variant: original_variant.to_string(),
            original_error,
        }),
    }
}

fn recovered_from_completed(
    result: &typevoice_core::workflow::CompletedRunResult,
) -> RecoveredRunResult {
    RecoveredRunResult {
        asr_text: result.asr_text.clone(),
        final_text: result.final_text.clone(),
        timings: result.timings.clone(),
        metrics: result.metrics.clone(),
    }
}

fn last_run_from_terminal(
    run_id: RunId,
    terminal: StoppedTerminal,
    audit: RunAudit,
) -> LastRunView {
    let terminal_cleanup = match &terminal {
        StoppedTerminal::Cancelled {
            cleanup_diagnostic, ..
        } => cleanup_diagnostic.clone(),
        _ => None,
    };
    let outcome = match terminal {
        StoppedTerminal::Completed { result, warning } => {
            RunOutcomeView::Completed { result, warning }
        }
        StoppedTerminal::Empty { timings } => RunOutcomeView::Empty { timings },
        StoppedTerminal::Failed {
            error,
            recovered_result,
            recovery_errors,
            record_saved,
            protocol_context,
        } => RunOutcomeView::Failed {
            primary_error: error,
            recovered_result,
            recovery_errors,
            record_saved,
            protocol_context,
        },
        StoppedTerminal::Cancelled {
            recovered_result, ..
        } => RunOutcomeView::Cancelled { recovered_result },
    };
    LastRunView {
        run_id,
        outcome,
        stopped_count: 1,
        cleanup_diagnostic: terminal_cleanup.or(audit.cleanup),
        effects: audit.effects,
        finalization: audit.finalization,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn stage_kind_name(kind: StageKind) -> &'static str {
    match kind {
        StageKind::ContextCapture => "contextCapture",
        StageKind::RecordFinalize => "recordFinalize",
        StageKind::Preprocess => "preprocess",
        StageKind::Transcribe => "transcribe",
        StageKind::Rewrite => "rewrite",
        StageKind::InsertPrepare => "insertPrepare",
        StageKind::Finalize => "finalize",
    }
}
