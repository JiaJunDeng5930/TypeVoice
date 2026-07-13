//! Executable contracts for the target workflow architecture.
//!
//! These tests deliberately exercise the current `VoiceWorkflow` boundary. They do not model the
//! target reducer in test code. When the current boundary cannot express a target capability, the
//! assertion names that missing capability and includes the observed wire/state value. That keeps
//! a red result attributable while leaving the scenario ready to be wired to `WorkflowController`
//! during the business refactor.

use std::sync::mpsc::TryRecvError;

use serde_json::{json, Value};

use crate::{
    insertion::InsertResult,
    rewrite::RewriteResult,
    transcription::{TranscriptionMetrics, TranscriptionResult},
    ui_events::UiEventMailbox,
    voice_workflow::{
        VoiceWorkflow, WorkflowApplyEventRequest, WorkflowAsrEmptyRequest, WorkflowCommandRequest,
        WorkflowTaskRequest,
    },
};

#[derive(Default)]
struct SubcaseFailures {
    executed: usize,
    failures: Vec<String>,
}

impl SubcaseFailures {
    fn check(&mut self, label: &str, passed: bool, evidence: impl Into<String>) {
        self.executed += 1;
        eprintln!(
            "[EXECUTED] {label}: {}",
            if passed { "PASS" } else { "TARGET_GAP" }
        );
        if !passed {
            self.failures.push(format!("[{label}] {}", evidence.into()));
        }
    }

    fn finish(self, contract: &str) {
        assert!(
            self.failures.is_empty(),
            "{contract}: executed {} subcases; failures:\n{}",
            self.executed,
            self.failures.join("\n")
        );
    }
}

#[derive(Default)]
struct TargetSubcases {
    executed: usize,
    target_gaps: Vec<String>,
    fixture_errors: Vec<String>,
}

impl TargetSubcases {
    fn check(&mut self, label: &str, passed: bool, evidence: impl Into<String>) {
        self.executed += 1;
        let evidence = evidence.into();
        if passed {
            eprintln!("[{label}] EXECUTED PASS: {evidence}");
        } else {
            eprintln!("[{label}] EXECUTED TARGET_GAP: {evidence}");
            self.target_gaps.push(format!("[{label}] {evidence}"));
        }
    }

    fn fixture_error(&mut self, label: &str, evidence: impl Into<String>) {
        self.executed += 1;
        let evidence = evidence.into();
        eprintln!("[{label}] EXECUTED FIXTURE_ERROR: {evidence}");
        self.fixture_errors.push(format!("[{label}] {evidence}"));
    }

    fn finish(self, contract: &str) {
        assert!(
            self.target_gaps.is_empty() && self.fixture_errors.is_empty(),
            "{contract}: executed {} subcases; target gaps={} fixture errors={}\ntarget gaps:\n{}\nfixture errors:\n{}",
            self.executed,
            self.target_gaps.len(),
            self.fixture_errors.len(),
            self.target_gaps.join("\n"),
            self.fixture_errors.join("\n")
        );
    }
}

#[derive(Default)]
struct ScriptedRunHandle {
    trace: Vec<&'static str>,
    arbiter_claims: Vec<&'static str>,
    completion_signals: usize,
    supervisor_exits: usize,
    live_resources: usize,
}

#[derive(Default)]
struct CountingPorts {
    asr: usize,
    rewrite: usize,
    history: usize,
    copy: usize,
    paste: usize,
    context: usize,
}

#[derive(Default)]
struct CaptureSink {
    snapshots: Vec<Value>,
    events: Vec<Value>,
}

struct Determinism {
    ids: Vec<String>,
    now_ms: u64,
    start_deadline_ms: u64,
    cancel_deadline_ms: u64,
}

struct ContractHarness {
    handle: ScriptedRunHandle,
    ports: CountingPorts,
    sink: CaptureSink,
    determinism: Determinism,
}

impl ContractHarness {
    fn observe(workflow: &VoiceWorkflow, ids: &[&str]) -> Self {
        Self {
            handle: ScriptedRunHandle::default(),
            ports: CountingPorts::default(),
            sink: CaptureSink {
                snapshots: vec![wire_view(workflow)],
                events: Vec::new(),
            },
            determinism: Determinism {
                ids: ids.iter().map(|id| (*id).to_string()).collect(),
                now_ms: 1_000,
                start_deadline_ms: 200,
                cancel_deadline_ms: 300,
            },
        }
    }

    fn evidence(&self) -> String {
        format!(
            "handle(trace={:?}, claims={:?}, completions={}, supervisor_exits={}, resources={}); ports(asr={}, rewrite={}, history={}, copy={}, paste={}, context={}); sink(snapshots={}, events={}); deterministic(ids={:?}, now={}, start_deadline={}, cancel_deadline={})",
            self.handle.trace,
            self.handle.arbiter_claims,
            self.handle.completion_signals,
            self.handle.supervisor_exits,
            self.handle.live_resources,
            self.ports.asr,
            self.ports.rewrite,
            self.ports.history,
            self.ports.copy,
            self.ports.paste,
            self.ports.context,
            self.sink.snapshots.len(),
            self.sink.events.len(),
            self.determinism.ids,
            self.determinism.now_ms,
            self.determinism.start_deadline_ms,
            self.determinism.cancel_deadline_ms,
        )
    }
}

fn recording_workflow(run_id: &str) -> VoiceWorkflow {
    let workflow = VoiceWorkflow::new();
    workflow
        .open_recording_for_test(run_id, &format!("recording-{run_id}"))
        .expect("target-contract fixture must open a current recording");
    workflow
}

fn transcribing_workflow(run_id: &str) -> VoiceWorkflow {
    let workflow = recording_workflow(run_id);
    workflow
        .begin_transcribing_for_test(&format!("recording-{run_id}"))
        .expect("target-contract fixture must enter current transcribing phase");
    workflow
}

fn transcribed_workflow(run_id: &str, text: &str) -> VoiceWorkflow {
    let workflow = transcribing_workflow(run_id);
    workflow
        .complete_transcription_for_test(transcription_result(run_id, text))
        .expect("target-contract fixture must complete current transcription");
    workflow
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

fn workflow_after_outcome(outcome: OutcomeFixture, run_id: &str) -> VoiceWorkflow {
    match outcome {
        OutcomeFixture::Completed => {
            let workflow = transcribed_workflow(run_id, "completed text");
            workflow
                .begin_insert_for_task_for_test(run_id)
                .expect("completed fixture must enter current insert phase");
            workflow
                .complete_insert_for_test()
                .expect("completed fixture must leave current insert phase");
            workflow
        }
        OutcomeFixture::Empty => {
            let workflow = transcribing_workflow(run_id);
            let (mailbox, _events) = UiEventMailbox::for_test();
            workflow
                .report_asr_empty(
                    &mailbox,
                    WorkflowAsrEmptyRequest {
                        transcript_id: run_id.to_string(),
                    },
                )
                .expect("empty fixture must complete through the current report boundary");
            workflow
        }
        OutcomeFixture::Failed => {
            let workflow = recording_workflow(run_id);
            workflow.fail_for_test("E_SCRIPTED_FAILURE", "scripted target-contract failure");
            workflow
        }
        OutcomeFixture::Cancelled => {
            let workflow = recording_workflow(run_id);
            workflow
                .cancel_current_recording_for_test()
                .expect("cancelled fixture must accept current cancellation");
            workflow
        }
    }
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

fn wire_view(workflow: &VoiceWorkflow) -> Value {
    serde_json::to_value(
        workflow
            .snapshot_view()
            .expect("current workflow snapshot must serialize"),
    )
    .expect("current workflow view must be valid JSON")
}

fn display_event(
    event_id: &str,
    kind: &str,
    task_id: &str,
    status: &str,
    payload: Value,
) -> WorkflowApplyEventRequest {
    WorkflowApplyEventRequest {
        event_id: event_id.to_string(),
        kind: kind.to_string(),
        task_id: Some(task_id.to_string()),
        status: Some(status.to_string()),
        message: kind.to_string(),
        error_code: None,
        payload: Some(payload),
    }
}

fn observed_phase(workflow: &VoiceWorkflow) -> String {
    workflow
        .snapshot_view()
        .expect("current workflow view must exist")
        .phase
}

fn workflow_at_current_stage(stage: &str, run_id: &str) -> VoiceWorkflow {
    match stage {
        "contextCapture" => recording_workflow(run_id),
        "recordFinalize" | "preprocess" | "transcribe" => transcribing_workflow(run_id),
        "rewrite" => {
            let workflow = transcribed_workflow(run_id, "asr");
            workflow
                .begin_rewrite_for_test(run_id)
                .expect("rewrite stage fixture must enter current rewrite phase");
            workflow
        }
        "insertPrepare" => {
            let workflow = transcribed_workflow(run_id, "asr");
            workflow
                .begin_insert_for_task_for_test(run_id)
                .expect("insert stage fixture must enter current insert phase");
            workflow
        }
        _ => panic!("fixture requested an unknown documented processing stage: {stage}"),
    }
}

fn try_recording_workflow(run_id: &str) -> Result<VoiceWorkflow, String> {
    let workflow = VoiceWorkflow::new();
    workflow
        .open_recording_for_test(run_id, &format!("recording-{run_id}"))
        .map_err(|error| format!("open recording failed: {}", error.render()))?;
    Ok(workflow)
}

fn try_transcribing_workflow(run_id: &str) -> Result<VoiceWorkflow, String> {
    let workflow = try_recording_workflow(run_id)?;
    workflow
        .begin_transcribing_for_test(&format!("recording-{run_id}"))
        .map_err(|error| format!("enter transcribing failed: {}", error.render()))?;
    Ok(workflow)
}

fn try_transcribed_workflow(run_id: &str, text: &str) -> Result<VoiceWorkflow, String> {
    let workflow = try_transcribing_workflow(run_id)?;
    workflow
        .complete_transcription_for_test(transcription_result(run_id, text))
        .map_err(|error| format!("complete transcription failed: {}", error.render()))?;
    Ok(workflow)
}

fn try_rewriting_workflow(run_id: &str) -> Result<VoiceWorkflow, String> {
    let workflow = try_transcribed_workflow(run_id, "recoverable asr")?;
    workflow
        .begin_rewrite_for_test(run_id)
        .map_err(|error| format!("enter rewrite failed: {}", error.render()))?;
    Ok(workflow)
}

fn try_inserting_workflow(run_id: &str) -> Result<VoiceWorkflow, String> {
    let workflow = try_transcribed_workflow(run_id, "completed text")?;
    workflow
        .begin_insert_for_task_for_test(run_id)
        .map_err(|error| format!("enter insert failed: {}", error.render()))?;
    Ok(workflow)
}

fn try_workflow_for_target_stage(stage: &str, run_id: &str) -> Result<VoiceWorkflow, String> {
    match stage {
        "contextCapture" => try_recording_workflow(run_id),
        "recordFinalize" | "preprocess" | "transcribe" => try_transcribing_workflow(run_id),
        "rewrite" => try_rewriting_workflow(run_id),
        "insertPrepare" | "finalize" | "history" | "copy" | "autoPaste" => {
            try_inserting_workflow(run_id)
        }
        _ => Err(format!("unknown documented target stage: {stage}")),
    }
}

fn record_fixture<T>(
    failures: &mut TargetSubcases,
    label: &str,
    fixture: Result<T, String>,
) -> Option<T> {
    match fixture {
        Ok(value) => Some(value),
        Err(error) => {
            failures.fixture_error(label, error);
            None
        }
    }
}

fn completed_terminal_payload(run_id: &str, final_text: &str, insert: InsertResult) -> Value {
    json!({
        "result": {
            "runId": run_id,
            "asrText": "asr text",
            "finalText": final_text,
            "timings": {"totalMs": 20},
            "insertResult": insert,
        }
    })
}

#[test]
fn target_contract_t01_start_freezes_seed_and_orders_begin_projection() {
    let mut failures = TargetSubcases::default();
    let Some(current) = record_fixture(
        &mut failures,
        "T01.seed.current_run_frozen",
        try_recording_workflow("run-01-current"),
    ) else {
        failures.finish("T01 start_freezes_seed_and_orders_begin_projection");
        return;
    };
    let Some(next) = record_fixture(
        &mut failures,
        "T01.seed.next_run_reads_new_settings",
        try_recording_workflow("run-01-next"),
    ) else {
        failures.finish("T01 start_freezes_seed_and_orders_begin_projection");
        return;
    };
    let current_wire = wire_view(&current);
    let next_wire = wire_view(&next);
    let harness = ContractHarness::observe(&current, &["run-01-current", "run-01-next"]);
    let expected_current_seed = json!({
        "settingsRevision": "settings-before-r1",
        "asr": {"provider": "doubao"},
        "insertion": {"autoPaste": false},
    });
    let expected_next_seed = json!({
        "settingsRevision": "settings-after-r1",
        "asr": {"provider": "doubao"},
        "insertion": {"autoPaste": true},
    });

    failures.check(
        "T01.seed.current_run_frozen",
        current_wire.pointer("/activeRun/planSeed/asr/provider")
            == expected_current_seed.pointer("/asr/provider")
            && current_wire.pointer("/activeRun/planSeed/insertion/autoPaste")
                == expected_current_seed.pointer("/insertion/autoPaste")
            && current_wire.pointer("/activeRun/runId") == Some(&json!("run-01-current")),
        format!(
            "expected cached seed={expected_current_seed}, current production projection={current_wire}; {}",
            harness.evidence()
        ),
    );
    failures.check(
        "T01.seed.settings_changed_after_r1_only_affect_next_run",
        current_wire.pointer("/activeRun/planSeed/settingsRevision")
            == expected_current_seed.pointer("/settingsRevision")
            && next_wire.pointer("/activeRun/planSeed/settingsRevision")
                == expected_next_seed.pointer("/settingsRevision")
            && next_wire.pointer("/activeRun/planSeed/insertion/autoPaste")
                == expected_next_seed.pointer("/insertion/autoPaste"),
        format!(
            "expected current seed={expected_current_seed}, expected next seed={expected_next_seed}, current run projection={current_wire}, next run projection={next_wire}"
        ),
    );
    failures.check(
        "T01.order.commit_then_begin_accepted_then_snapshot_then_reply",
        current_wire.get("revision").is_some()
            && current_wire.pointer("/activeRun/beginAcceptedAtMs").is_some()
            && current_wire.pointer("/mode") == Some(&json!("recording")),
        format!(
            "current API exposes no connected ordered handle/sink observation; production projection={current_wire}"
        ),
    );
    let committed_at = current_wire
        .pointer("/activeRun/committedAtMs")
        .and_then(Value::as_u64);
    let capture_started_at = current_wire
        .pointer("/activeRun/captureStartedAtMs")
        .and_then(Value::as_u64);
    failures.check(
        "T01.capture_io.after_commit_and_within_200ms",
        matches!(
            (committed_at, capture_started_at),
            (Some(commit), Some(start)) if start >= commit && start.saturating_sub(commit) <= 200
        ),
        format!(
            "committedAtMs={committed_at:?}, captureStartedAtMs={capture_started_at:?}, projection={current_wire}"
        ),
    );
    failures.finish("T01 start_freezes_seed_and_orders_begin_projection");
}

#[test]
fn target_contract_t02_begin_or_recording_start_failure_uses_typed_failed() {
    #[derive(Clone, Copy)]
    struct FailureCase {
        label: &'static str,
        code: &'static str,
        after_stop: bool,
    }

    let cases = [
        FailureCase {
            label: "T02.begin.delivery_failure",
            code: "E_BEGIN_DELIVERY",
            after_stop: false,
        },
        FailureCase {
            label: "T02.begin.ack_failure",
            code: "E_BEGIN_ACK",
            after_stop: false,
        },
        FailureCase {
            label: "T02.recording.start_failure",
            code: "E_RECORD_START",
            after_stop: false,
        },
        FailureCase {
            label: "T02.doubao_session.start_failure",
            code: "E_DOUBAO_START",
            after_stop: false,
        },
        FailureCase {
            label: "T02.context_failure.before_primary_stop",
            code: "E_CONTEXT_BEFORE_STOP",
            after_stop: false,
        },
        FailureCase {
            label: "T02.context_failure.after_primary_stop",
            code: "E_CONTEXT_AFTER_STOP",
            after_stop: true,
        },
    ];
    let mut failures = TargetSubcases::default();

    for (index, case) in cases.iter().enumerate() {
        let run_id = format!("run-02-{index}");
        let fixture = if case.after_stop {
            try_transcribing_workflow(&run_id)
        } else {
            try_recording_workflow(&run_id)
        };
        let Some(workflow) = record_fixture(&mut failures, case.label, fixture) else {
            continue;
        };
        workflow.fail_for_test(case.code, case.label);
        let snapshot = workflow.snapshot();
        let wire = wire_view(&workflow);
        failures.check(
            case.label,
            observed_phase(&workflow) == "ready"
                && snapshot.session.is_none()
                && wire.pointer("/lastRun/runId") == Some(&json!(run_id))
                && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                    == Some(&json!(case.code))
                && wire.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64)),
            format!(
                "after_stop={}, phase={}, session_live={}, wire={wire}",
                case.after_stop,
                observed_phase(&workflow),
                snapshot.session.is_some()
            ),
        );
    }

    failures.finish("T02 begin_or_recording_start_failure_uses_typed_failed");
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

    fn mode_fixture(mode: ModeCase, run_id: &str) -> Result<VoiceWorkflow, String> {
        match mode {
            ModeCase::Ready => Ok(VoiceWorkflow::new()),
            ModeCase::Recording => try_recording_workflow(run_id),
            ModeCase::Processing => try_transcribing_workflow(run_id),
            ModeCase::Cancelling => {
                let workflow = try_recording_workflow(run_id)?;
                workflow
                    .cancel_current_recording_for_test()
                    .map_err(|error| {
                        format!("enter current cancellation failed: {}", error.render())
                    })?;
                Ok(workflow)
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

    for (mode_index, mode) in modes.iter().copied().enumerate() {
        for intent in intents.iter().copied() {
            let label = format!("T03.matrix.{}.{}", mode.label(), intent.label());
            let run_id = format!("run-03-{mode_index}");
            let Some(workflow) = record_fixture(&mut failures, &label, mode_fixture(mode, &run_id))
            else {
                continue;
            };
            let before = wire_view(&workflow);
            let action_key = before
                .get("actionKey")
                .and_then(Value::as_str)
                .unwrap_or("current-action-key");
            let payload = match intent {
                IntentCase::FreshPrimary => {
                    json!({"command": "primary", "actionKey": action_key})
                }
                IntentCase::Cancel => {
                    let target = (mode.label() != "ready").then_some(run_id.as_str());
                    json!({"command": "cancel", "targetRunId": target})
                }
                IntentCase::Invalid => json!({"command": "unknown-intent"}),
                IntentCase::StaleActionKey => {
                    json!({"command": "primary", "actionKey": "stale-action-key"})
                }
                IntentCase::MismatchedTargetRunId => {
                    json!({"command": "cancel", "targetRunId": "another-run"})
                }
            };
            let parsed = serde_json::from_value::<WorkflowCommandRequest>(payload.clone());
            let after = wire_view(&workflow);
            let parse_matches_envelope = matches!(intent, IntentCase::Invalid) == parsed.is_err();
            let no_op_input = matches!(
                intent,
                IntentCase::StaleActionKey | IntentCase::MismatchedTargetRunId
            );
            failures.check(
                &label,
                parse_matches_envelope
                    && before.get("mode") == Some(&json!(mode.label()))
                    && before.get("revision").is_some()
                    && before.get("actionKey").is_some()
                    && (!no_op_input || after == before),
                format!(
                    "payload={payload}, parsed={}, current_phase={}, before={before}, after={after}",
                    parsed.is_ok(),
                    observed_phase(&workflow)
                ),
            );
        }
    }

    let initial = VoiceWorkflow::new();
    let initial_wire = wire_view(&initial);
    failures.check(
        "T03.ready_key.initial_generation",
        initial_wire.pointer("/mode") == Some(&json!("ready"))
            && initial_wire.get("revision").is_some()
            && initial_wire.get("actionKey").is_some(),
        format!("initial production projection={initial_wire}"),
    );
    let Some(after_run) = record_fixture(
        &mut failures,
        "T03.ready_key.after_last_run_generation",
        try_inserting_workflow("run-03-finished"),
    ) else {
        failures.finish("T03 intent_admission_and_matrix_are_total");
        return;
    };
    if let Err(error) = after_run.complete_insert_for_test() {
        failures.fixture_error(
            "T03.ready_key.after_last_run_generation",
            format!("complete current run fixture failed: {}", error.render()),
        );
    } else {
        let after_wire = wire_view(&after_run);
        failures.check(
            "T03.ready_key.after_last_run_generation",
            after_wire.pointer("/mode") == Some(&json!("ready"))
                && after_wire.pointer("/lastRun/runId") == Some(&json!("run-03-finished"))
                && after_wire.get("actionKey").is_some()
                && after_wire.get("actionKey") != initial_wire.get("actionKey"),
            format!("initial={initial_wire}, after completed run={after_wire}"),
        );
    }

    for (label, too_late) in [
        ("T03.active_cancel.accepted", false),
        ("T03.active_cancel.too_late", true),
    ] {
        let fixture = if too_late {
            try_inserting_workflow("run-03-cancel-too-late")
        } else {
            try_recording_workflow("run-03-cancel-accepted")
        };
        let Some(workflow) = record_fixture(&mut failures, label, fixture) else {
            continue;
        };
        let result = workflow.cancel_current_recording_for_test();
        let wire = wire_view(&workflow);
        failures.check(
            label,
            if too_late {
                result.is_ok() && wire.pointer("/mode") == Some(&json!("processing"))
            } else {
                result.is_ok() && wire.pointer("/mode") == Some(&json!("cancelling"))
            },
            format!(
                "cancel_result={:?}, phase={}, wire={wire}",
                result.as_ref().err().map(|error| error.code.as_str()),
                observed_phase(&workflow)
            ),
        );
    }

    failures.finish("T03 intent_admission_and_matrix_are_total");
}

#[test]
fn target_contract_t04_stop_is_once_and_delivery_failure_is_terminal() {
    let mut failures = TargetSubcases::default();

    let Some(double_click) = record_fixture(
        &mut failures,
        "T04.double_primary.same_action_key",
        try_recording_workflow("run-04-double"),
    ) else {
        failures.finish("T04 stop_is_once_and_delivery_failure_is_terminal");
        return;
    };
    let first_started_at = std::time::Instant::now();
    let first = double_click.prepare_stop_for_test();
    let first_elapsed_ms = first_started_at.elapsed().as_millis();
    let after_first = wire_view(&double_click);
    let duplicate = double_click.prepare_stop_for_test();
    let after_duplicate = wire_view(&double_click);
    failures.check(
        "T04.double_primary.same_action_key",
        matches!(first, Ok(WorkflowTaskRequest::StopRecordTranscribe { .. }))
            && duplicate.is_ok()
            && first_elapsed_ms <= 200
            && after_first.pointer("/mode") == Some(&json!("processing"))
            && after_first.get("revision").is_some()
            && after_duplicate == after_first,
        format!(
            "first={:?}, first_elapsed_ms={first_elapsed_ms}, duplicate_error={:?}, after_first={after_first}, after_duplicate={after_duplicate}",
            first.as_ref().err().map(|error| error.code.as_str()),
            duplicate.as_ref().err().map(|error| error.code.as_str())
        ),
    );

    let Some(progress_race) = record_fixture(
        &mut failures,
        "T04.progress_between_view_and_primary.same_key_still_stops",
        try_recording_workflow("run-04-progress"),
    ) else {
        failures.finish("T04 stop_is_once_and_delivery_failure_is_terminal");
        return;
    };
    let (mailbox, _events) = UiEventMailbox::for_test();
    let stale_view = wire_view(&progress_race);
    let progress_result = progress_race.apply_event(
        &mailbox,
        display_event(
            "progress-04-context",
            "workflow.progress.contextCapture",
            "run-04-progress",
            "started",
            json!({"elapsedMs": 1}),
        ),
    );
    let after_progress = wire_view(&progress_race);
    let started_at = std::time::Instant::now();
    let stop_result = progress_race.prepare_stop_for_test();
    let elapsed_ms = started_at.elapsed().as_millis();
    let after_stop = wire_view(&progress_race);
    failures.check(
        "T04.progress_between_view_and_primary.same_key_still_stops",
        progress_result.is_ok()
            && stop_result.is_ok()
            && stale_view.get("actionKey") == after_progress.get("actionKey")
            && after_stop.pointer("/mode") == Some(&json!("processing"))
            && elapsed_ms <= 200,
        format!(
            "progress_error={:?}, stop_error={:?}, elapsed_ms={elapsed_ms}, stale={stale_view}, after_progress={after_progress}, after_stop={after_stop}",
            progress_result.as_ref().err().map(|error| error.code.as_str()),
            stop_result.as_ref().err().map(|error| error.code.as_str())
        ),
    );

    let Some(delivery_failure) = record_fixture(
        &mut failures,
        "T04.stop_delivery_failure.supervisor_terminalizes",
        try_recording_workflow("run-04-delivery"),
    ) else {
        failures.finish("T04 stop_is_once_and_delivery_failure_is_terminal");
        return;
    };
    let stop_result = delivery_failure.prepare_stop_for_test();
    delivery_failure.fail_for_test("E_STOP_DELIVERY", "scripted Stop delivery failure");
    let snapshot = delivery_failure.snapshot();
    let wire = wire_view(&delivery_failure);
    failures.check(
        "T04.stop_delivery_failure.supervisor_terminalizes",
        stop_result.is_ok()
            && observed_phase(&delivery_failure) == "ready"
            && snapshot.session.is_none()
            && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                == Some(&json!("E_STOP_DELIVERY"))
            && wire.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64)),
        format!(
            "stop_error={:?}, phase={}, session_live={}, wire={wire}",
            stop_result.as_ref().err().map(|error| error.code.as_str()),
            observed_phase(&delivery_failure),
            snapshot.session.is_some()
        ),
    );

    failures.finish("T04 stop_is_once_and_delivery_failure_is_terminal");
}

#[test]
fn target_contract_t05_typed_progress_domain_is_strict_and_monotonic() {
    let mut failures = TargetSubcases::default();

    for (index, (label, context_progress_count, expected_stage)) in [
        ("T05.stop.before_context_started", 0_usize, "contextCapture"),
        (
            "T05.stop.between_context_started_and_completed",
            1,
            "contextCapture",
        ),
        ("T05.stop.after_context_completed", 2, "recordFinalize"),
    ]
    .iter()
    .copied()
    .enumerate()
    {
        let run_id = format!("run-05-stop-{index}");
        let Some(workflow) = record_fixture(&mut failures, label, try_recording_workflow(&run_id))
        else {
            continue;
        };
        let (mailbox, _events) = UiEventMailbox::for_test();
        let mut progress_ok = true;
        for progress_index in 0..context_progress_count {
            let status = if progress_index == 0 {
                "started"
            } else {
                "completed"
            };
            progress_ok &= workflow
                .apply_event(
                    &mailbox,
                    display_event(
                        &format!("progress-05-context-{index}-{progress_index}"),
                        "workflow.progress.contextCapture",
                        &run_id,
                        status,
                        json!({"status": status, "elapsedMs": progress_index + 1}),
                    ),
                )
                .is_ok();
        }
        let stop = workflow.prepare_stop_for_test();
        let wire = wire_view(&workflow);
        failures.check(
            label,
            progress_ok
                && stop.is_ok()
                && wire.pointer("/mode") == Some(&json!("processing"))
                && wire.pointer("/activeRun/stage/kind") == Some(&json!(expected_stage)),
            format!(
                "context_progress_count={context_progress_count}, stop_error={:?}, phase={}, wire={wire}",
                stop.as_ref().err().map(|error| error.code.as_str()),
                observed_phase(&workflow)
            ),
        );
    }

    let legal_progress = vec![
        ("contextCapture", "started", json!({"elapsedMs": 1})),
        ("contextCapture", "completed", json!({"elapsedMs": 2})),
        ("recordFinalize", "started", json!({"elapsedMs": 3})),
        ("recordFinalize", "completed", json!({"elapsedMs": 4})),
        ("preprocess", "started", json!({"elapsedMs": 5})),
        ("preprocess", "completed", json!({"elapsedMs": 6})),
        ("transcribe", "started", json!({"elapsedMs": 7})),
        (
            "transcribe",
            "completed",
            json!({"result": transcription_result("run-05-plan-7", "asr text")}),
        ),
        ("rewrite", "started", json!({"elapsedMs": 9})),
        (
            "rewrite",
            "completed",
            json!({"result": {"runId": "run-05-plan-9", "finalText": "rewritten"}}),
        ),
        ("insertPrepare", "started", json!({"elapsedMs": 11})),
        (
            "insertPrepare",
            "completed",
            json!({"target": "focused-window", "textDigest": "sha256:contract"}),
        ),
        ("finalize", "started", json!({"elapsedMs": 13})),
    ];
    for (index, (stage, status, payload)) in legal_progress.into_iter().enumerate() {
        let label = format!("T05.progress.legal.{stage}.{status}");
        let run_id = format!("run-05-plan-{index}");
        let Some(workflow) = record_fixture(
            &mut failures,
            &label,
            try_workflow_for_target_stage(stage, &run_id),
        ) else {
            continue;
        };
        let (mailbox, _events) = UiEventMailbox::for_test();
        let before = wire_view(&workflow);
        let result = workflow.apply_event(
            &mailbox,
            display_event(
                &format!("progress-05-plan-{index}"),
                &format!("workflow.progress.{stage}"),
                &run_id,
                status,
                payload,
            ),
        );
        let after = wire_view(&workflow);
        let before_revision = before.get("revision").and_then(Value::as_u64);
        let after_revision = after.get("revision").and_then(Value::as_u64);
        failures.check(
            &label,
            result.is_ok()
                && matches!((before_revision, after_revision), (Some(before), Some(after)) if after > before)
                && after.pointer("/activeRun/stage/kind") == Some(&json!(stage))
                && after.pointer("/activeRun/stage/status") == Some(&json!(status)),
            format!(
                "apply_error={:?}, before={before}, after={after}",
                result.as_ref().err().map(|error| error.code.as_str())
            ),
        );
    }

    let rewrite_label = "T05.progress.rewrite_failure_recovery_to_finalize";
    if let Some(workflow) = record_fixture(
        &mut failures,
        rewrite_label,
        try_rewriting_workflow("run-05-rewrite-failed"),
    ) {
        workflow.fail_for_test("E_REWRITE_FAILED", "scripted rewrite failure");
        let wire = wire_view(&workflow);
        failures.check(
            rewrite_label,
            wire.pointer("/mode") == Some(&json!("processing"))
                && wire.pointer("/activeRun/stage/kind") == Some(&json!("finalize"))
                && wire.pointer("/activeRun/result/asrText") == Some(&json!("recoverable asr")),
            format!("phase={}, wire={wire}", observed_phase(&workflow)),
        );
    }

    #[derive(Clone, Copy)]
    struct InvalidProgressCase {
        label: &'static str,
        first_stage: &'static str,
        first_status: &'static str,
        second_stage: &'static str,
        second_status: &'static str,
        second_payload: fn() -> Value,
    }
    let invalid_cases = [
        InvalidProgressCase {
            label: "T05.progress.invalid.duplicate",
            first_stage: "transcribe",
            first_status: "started",
            second_stage: "transcribe",
            second_status: "started",
            second_payload: || json!({"elapsedMs": 2}),
        },
        InvalidProgressCase {
            label: "T05.progress.invalid.regression",
            first_stage: "rewrite",
            first_status: "started",
            second_stage: "transcribe",
            second_status: "completed",
            second_payload: || json!({"result": {"finalText": "wrong type"}}),
        },
        InvalidProgressCase {
            label: "T05.progress.invalid.payload_mismatch",
            first_stage: "transcribe",
            first_status: "started",
            second_stage: "transcribe",
            second_status: "completed",
            second_payload: || json!({"rewriteResult": {"finalText": "wrong union member"}}),
        },
    ];
    for (index, case) in invalid_cases.iter().enumerate() {
        let run_id = format!("run-05-invalid-{index}");
        let Some(workflow) = record_fixture(
            &mut failures,
            case.label,
            try_workflow_for_target_stage(case.first_stage, &run_id),
        ) else {
            continue;
        };
        let (mailbox, _events) = UiEventMailbox::for_test();
        let first = workflow.apply_event(
            &mailbox,
            display_event(
                &format!("progress-05-invalid-{index}-first"),
                &format!("workflow.progress.{}", case.first_stage),
                &run_id,
                case.first_status,
                json!({"elapsedMs": 1}),
            ),
        );
        let before_invalid = wire_view(&workflow);
        let second = workflow.apply_event(
            &mailbox,
            display_event(
                &format!("progress-05-invalid-{index}-second"),
                &format!("workflow.progress.{}", case.second_stage),
                &run_id,
                case.second_status,
                (case.second_payload)(),
            ),
        );
        let after_invalid = wire_view(&workflow);
        failures.check(
            case.label,
            first.is_ok() && second.is_err() && after_invalid == before_invalid,
            format!(
                "first_error={:?}, invalid_error={:?}, before_invalid={before_invalid}, after_invalid={after_invalid}",
                first.as_ref().err().map(|error| error.code.as_str()),
                second.as_ref().err().map(|error| error.code.as_str())
            ),
        );
    }

    failures.finish("T05 typed_progress_domain_is_strict_and_monotonic");
}

#[test]
fn target_contract_t06_completed_terminal_is_typed_and_ordered() {
    #[derive(Clone, Copy)]
    enum CompletedOrigin {
        Finalize,
        Recording,
        BeforeFinalize,
    }

    let cases = [
        (
            "T06.completed.from_finalize",
            CompletedOrigin::Finalize,
            true,
        ),
        (
            "T06.completed.from_recording",
            CompletedOrigin::Recording,
            false,
        ),
        (
            "T06.completed.from_processing_before_finalize",
            CompletedOrigin::BeforeFinalize,
            false,
        ),
    ];
    let mut failures = TargetSubcases::default();

    for (index, (label, origin, legal)) in cases.iter().copied().enumerate() {
        let run_id = format!("run-06-{index}");
        let fixture = match origin {
            CompletedOrigin::Finalize => try_inserting_workflow(&run_id),
            CompletedOrigin::Recording => try_recording_workflow(&run_id),
            CompletedOrigin::BeforeFinalize => try_transcribing_workflow(&run_id),
        };
        let Some(workflow) = record_fixture(&mut failures, label, fixture) else {
            continue;
        };
        let (mailbox, _events) = UiEventMailbox::for_test();
        let result = workflow.apply_event(
            &mailbox,
            display_event(
                &format!("terminal-06-{index}"),
                "workflow.stopped.completed",
                &run_id,
                "completed",
                completed_terminal_payload(&run_id, "final text", InsertResult::pasted()),
            ),
        );
        let wire = wire_view(&workflow);
        let terminal_observed = if legal {
            wire.pointer("/lastRun/outcome/completed/finalText") == Some(&json!("final text"))
                && wire.pointer("/lastRun/outcome/completed/insertResult/copied")
                    == Some(&json!(true))
        } else {
            wire.pointer("/lastRun/outcome/failed/primaryError/code")
                == Some(&json!("E_EXECUTOR_TERMINAL_ORDER"))
        };
        failures.check(
            label,
            result.is_ok()
                && observed_phase(&workflow) == "ready"
                && workflow.snapshot().session.is_none()
                && terminal_observed,
            format!(
                "legal={legal}, apply_error={:?}, phase={}, wire={wire}",
                result.as_ref().err().map(|error| error.code.as_str()),
                observed_phase(&workflow)
            ),
        );
    }

    let label = "T06.completed.missing_result_rejected_at_typed_boundary";
    if let Some(workflow) = record_fixture(
        &mut failures,
        label,
        try_inserting_workflow("run-06-missing"),
    ) {
        let (mailbox, _events) = UiEventMailbox::for_test();
        let before = wire_view(&workflow);
        let result = workflow.apply_event(
            &mailbox,
            display_event(
                "terminal-06-missing",
                "workflow.stopped.completed",
                "run-06-missing",
                "completed",
                json!({}),
            ),
        );
        let after = wire_view(&workflow);
        failures.check(
            label,
            result.is_err() && after == before,
            format!(
                "accepted={}, error={:?}, before={before}, after={after}",
                result.is_ok(),
                result.as_ref().err().map(|error| error.code.as_str())
            ),
        );
    }

    failures.finish("T06 completed_terminal_is_typed_and_ordered");
}

#[test]
fn target_contract_t07_empty_terminal_is_typed_and_ordered() {
    #[derive(Clone, Copy)]
    enum EmptyOrigin {
        Transcribe,
        Recording,
        OtherProcessing(&'static str),
    }

    let cases = [
        ("T07.empty.from_transcribe", EmptyOrigin::Transcribe, true),
        ("T07.empty.from_recording", EmptyOrigin::Recording, false),
        (
            "T07.empty.from_processing_context_capture",
            EmptyOrigin::OtherProcessing("contextCapture"),
            false,
        ),
        (
            "T07.empty.from_record_finalize",
            EmptyOrigin::OtherProcessing("recordFinalize"),
            false,
        ),
        (
            "T07.empty.from_preprocess",
            EmptyOrigin::OtherProcessing("preprocess"),
            false,
        ),
        (
            "T07.empty.from_rewrite",
            EmptyOrigin::OtherProcessing("rewrite"),
            false,
        ),
        (
            "T07.empty.from_insert_prepare",
            EmptyOrigin::OtherProcessing("insertPrepare"),
            false,
        ),
        (
            "T07.empty.from_finalize",
            EmptyOrigin::OtherProcessing("finalize"),
            false,
        ),
    ];
    let mut failures = TargetSubcases::default();

    for (index, (label, origin, legal)) in cases.iter().copied().enumerate() {
        let run_id = format!("run-07-{index}");
        let fixture = match origin {
            EmptyOrigin::Transcribe => try_transcribing_workflow(&run_id),
            EmptyOrigin::Recording => try_recording_workflow(&run_id),
            EmptyOrigin::OtherProcessing(stage) => try_workflow_for_target_stage(stage, &run_id),
        };
        let Some(workflow) = record_fixture(&mut failures, label, fixture) else {
            continue;
        };
        let (mailbox, _events) = UiEventMailbox::for_test();
        let result = workflow.apply_event(
            &mailbox,
            display_event(
                &format!("terminal-07-{index}"),
                "workflow.stopped.empty",
                &run_id,
                "completed",
                json!({"timings": {"totalMs": 12}}),
            ),
        );
        let wire = wire_view(&workflow);
        let outcome_observed = if legal {
            wire.pointer("/lastRun/outcome/empty").is_some()
        } else {
            wire.pointer("/lastRun/outcome/failed/primaryError/code")
                == Some(&json!("E_EXECUTOR_TERMINAL_ORDER"))
        };
        failures.check(
            label,
            result.is_ok()
                && observed_phase(&workflow) == "ready"
                && workflow.snapshot().session.is_none()
                && outcome_observed,
            format!(
                "legal={legal}, apply_error={:?}, phase={}, wire={wire}",
                result.as_ref().err().map(|error| error.code.as_str()),
                observed_phase(&workflow)
            ),
        );
    }

    let label = "T07.empty.error_field_rejected_at_typed_boundary";
    if let Some(workflow) = record_fixture(
        &mut failures,
        label,
        try_transcribing_workflow("run-07-illegal-error"),
    ) {
        let (mailbox, _events) = UiEventMailbox::for_test();
        let before = wire_view(&workflow);
        let result = workflow.apply_event(
            &mailbox,
            display_event(
                "terminal-07-illegal-error",
                "workflow.stopped.empty",
                "run-07-illegal-error",
                "completed",
                json!({"error": {"code": "E_ILLEGAL"}}),
            ),
        );
        let after = wire_view(&workflow);
        failures.check(
            label,
            result.is_err() && after == before,
            format!(
                "accepted={}, error={:?}, before={before}, after={after}",
                result.is_ok(),
                result.as_ref().err().map(|error| error.code.as_str())
            ),
        );
    }

    failures.finish("T07 empty_terminal_is_typed_and_ordered");
}

#[test]
fn target_contract_t08_failure_timeout_or_abnormal_exit_is_terminal_once() {
    let effects = [
        ("context_capture", "contextCapture"),
        ("recording", "contextCapture"),
        ("provider_session", "contextCapture"),
        ("record_finalize", "recordFinalize"),
        ("preprocess", "preprocess"),
        ("transcribe", "transcribe"),
        ("rewrite", "rewrite"),
        ("insert_prepare", "insertPrepare"),
        ("history", "history"),
        ("copy", "copy"),
        ("auto_paste", "autoPaste"),
        ("finalize", "finalize"),
    ];
    let failure_kinds = [("error", "E_EFFECT_ERROR"), ("timeout", "E_EFFECT_TIMEOUT")];
    let mut failures = TargetSubcases::default();

    for (effect_index, (effect, stage)) in effects.iter().copied().enumerate() {
        for (kind, code_prefix) in failure_kinds {
            let label = format!("T08.effect.{effect}.{kind}");
            let run_id = format!("run-08-{effect_index}-{kind}");
            let Some(workflow) = record_fixture(
                &mut failures,
                &label,
                try_workflow_for_target_stage(stage, &run_id),
            ) else {
                continue;
            };
            let code = format!("{code_prefix}_{}", effect.to_ascii_uppercase());
            let message = format!("scripted {effect} {kind}");
            workflow.fail_for_test(&code, &message);
            let snapshot = workflow.snapshot();
            let wire = wire_view(&workflow);
            failures.check(
                &label,
                observed_phase(&workflow) == "ready"
                    && snapshot.session.is_none()
                    && wire.pointer("/lastRun/runId") == Some(&json!(run_id))
                    && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                        == Some(&json!(code))
                    && wire.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64)),
                format!(
                    "stage={stage}, phase={}, session_live={}, wire={wire}",
                    observed_phase(&workflow),
                    snapshot.session.is_some()
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
    .iter()
    .copied()
    .enumerate()
    {
        let winner = if supervisor_wins {
            "supervisor_wins"
        } else {
            "ordinary_terminal_wins"
        };
        let label = format!("T08.abnormal_exit.{exit}.{winner}");
        let run_id = format!("run-08-abnormal-{index}");
        let Some(workflow) = record_fixture(&mut failures, &label, try_inserting_workflow(&run_id))
        else {
            continue;
        };
        if supervisor_wins {
            workflow.fail_for_test("E_EXECUTOR_ABNORMAL_EXIT", exit);
        } else if let Err(error) = workflow.complete_insert_for_test() {
            failures.fixture_error(
                &label,
                format!("ordinary terminal fixture failed: {}", error.render()),
            );
            continue;
        } else {
            let (mailbox, _events) = UiEventMailbox::for_test();
            let _ = workflow.apply_event(
                &mailbox,
                display_event(
                    &format!("terminal-08-abnormal-{index}"),
                    "workflow.task.failed",
                    &run_id,
                    "failed",
                    json!({"cause": exit}),
                ),
            );
        }
        let snapshot = workflow.snapshot();
        let wire = wire_view(&workflow);
        let outcome_matches_winner = if supervisor_wins {
            wire.pointer("/lastRun/outcome/failed/primaryError/code")
                == Some(&json!("E_EXECUTOR_ABNORMAL_EXIT"))
        } else {
            wire.pointer("/lastRun/outcome/completed").is_some()
        };
        failures.check(
            &label,
            observed_phase(&workflow) == "ready"
                && snapshot.session.is_none()
                && outcome_matches_winner
                && wire.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64)),
            format!(
                "exit={exit}, supervisor_wins={supervisor_wins}, phase={}, session_live={}, wire={wire}",
                observed_phase(&workflow),
                snapshot.session.is_some()
            ),
        );
    }

    failures.finish("T08 failure_timeout_or_abnormal_exit_is_terminal_once");
}

#[test]
fn target_contract_t09_plan_without_rewrite_still_copies_once() {
    let mut failures = TargetSubcases::default();
    let label = "T09.plan.rewrite_disabled_auto_paste_disabled";
    if let Some(workflow) = record_fixture(
        &mut failures,
        label,
        try_transcribed_workflow("run-09", "asr text"),
    ) {
        let wire = wire_view(&workflow);
        failures.check(
            label,
            observed_phase(&workflow) == "ready"
                && wire.pointer("/lastRun/outcome/completed/finalText") == Some(&json!("asr text"))
                && wire.pointer("/lastRun/outcome/completed/insertResult/copied")
                    == Some(&json!(true))
                && wire.pointer("/lastRun/outcome/completed/insertResult/autoPasteAttempted")
                    == Some(&json!(false)),
            format!("phase={}, wire={wire}", observed_phase(&workflow)),
        );
    }
    failures.finish("T09 plan_without_rewrite_still_copies_once");
}

#[test]
fn target_contract_t10_plan_with_rewrite_and_autopaste_runs_once() {
    let mut failures = TargetSubcases::default();
    let label = "T10.plan.rewrite_enabled_auto_paste_enabled";
    if let Some(workflow) = record_fixture(&mut failures, label, try_rewriting_workflow("run-10")) {
        match workflow.complete_rewrite_for_test(RewriteResult {
            transcript_id: "run-10".to_string(),
            final_text: "rewritten text".to_string(),
            rewrite_ms: 3,
        }) {
            Ok(()) => {
                let wire = wire_view(&workflow);
                failures.check(
                    label,
                    observed_phase(&workflow) == "ready"
                        && wire.pointer("/lastRun/outcome/completed/finalText")
                            == Some(&json!("rewritten text"))
                        && wire
                            .pointer("/lastRun/outcome/completed/insertResult/autoPasteAttempted")
                            == Some(&json!(true))
                        && wire.pointer("/lastRun/outcome/completed/insertResult/autoPasteOk")
                            == Some(&json!(true)),
                    format!("phase={}, wire={wire}", observed_phase(&workflow)),
                );
            }
            Err(error) => failures.fixture_error(
                label,
                format!("complete rewrite fixture failed: {}", error.render()),
            ),
        }
    }
    failures.finish("T10 plan_with_rewrite_and_autopaste_runs_once");
}

#[test]
fn target_contract_t11_rewrite_failure_preserves_asr_and_double_faults() {
    let mut failures = TargetSubcases::default();

    for (index, (label, history_succeeds)) in [
        ("T11.recovery_history.success", true),
        ("T11.recovery_history.failure", false),
    ]
    .iter()
    .copied()
    .enumerate()
    {
        let run_id = format!("run-11-history-{index}");
        let Some(workflow) = record_fixture(&mut failures, label, try_rewriting_workflow(&run_id))
        else {
            continue;
        };
        workflow.fail_for_test("E_REWRITE_FAILED", "scripted rewrite failure");
        let wire = wire_view(&workflow);
        let recovery_errors = wire
            .pointer("/lastRun/outcome/failed/recoveryErrors")
            .and_then(Value::as_array);
        let history_outcome_observed = if history_succeeds {
            recovery_errors.is_some_and(Vec::is_empty)
        } else {
            recovery_errors.is_some_and(|errors| {
                errors.iter().any(|error| {
                    error.pointer("/code") == Some(&json!("E_HISTORY_RECOVERY_FAILED"))
                })
            }) && wire.pointer("/lastRun/outcome/failed/recordSaved") == Some(&json!(false))
        };
        failures.check(
            label,
            observed_phase(&workflow) == "ready"
                && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                    == Some(&json!("E_REWRITE_FAILED"))
                && wire.pointer("/lastRun/outcome/failed/recoveredResult/finalText")
                    == Some(&json!("recoverable asr"))
                && history_outcome_observed,
            format!(
                "history_succeeds={history_succeeds}, phase={}, wire={wire}",
                observed_phase(&workflow)
            ),
        );
    }

    let label = "T11.manual_copy.recovered_result";
    if let Some(workflow) = record_fixture(
        &mut failures,
        label,
        try_rewriting_workflow("run-11-manual-copy"),
    ) {
        workflow.fail_for_test("E_REWRITE_FAILED", "scripted rewrite failure");
        let export_request = serde_json::from_value::<WorkflowCommandRequest>(json!({
            "command": "copyLast",
            "taskId": "run-11-manual-copy",
        }));
        let wire = wire_view(&workflow);
        failures.check(
            label,
            export_request.is_ok()
                && wire.get("canCopy") == Some(&json!(true))
                && wire.pointer("/lastRun/outcome/failed/recoveredResult/finalText")
                    == Some(&json!("recoverable asr")),
            format!(
                "export_request_error={}, phase={}, wire={wire}",
                export_request
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                observed_phase(&workflow)
            ),
        );
    }

    failures.finish("T11 rewrite_failure_preserves_asr_and_double_faults");
}

#[test]
fn target_contract_t12_paste_warning_keeps_completed_copy() {
    let mut failures = TargetSubcases::default();
    let label = "T12.completed.copy_succeeds_auto_paste_fails";
    if let Some(workflow) = record_fixture(&mut failures, label, try_inserting_workflow("run-12")) {
        let insert_result = InsertResult::paste_failed(
            "E_EXPORT_TARGET_UNAVAILABLE",
            "scripted native input failure",
        );
        let (mailbox, _events) = UiEventMailbox::for_test();
        let result = workflow.apply_event(
            &mailbox,
            display_event(
                "terminal-12-completed-warning",
                "workflow.stopped.completed",
                "run-12",
                "completed",
                completed_terminal_payload("run-12", "completed text", insert_result.clone()),
            ),
        );
        let wire = wire_view(&workflow);
        failures.check(
            label,
            result.is_ok()
                && insert_result.copied
                && insert_result.auto_paste_attempted
                && !insert_result.auto_paste_ok
                && observed_phase(&workflow) == "ready"
                && wire.pointer("/lastRun/outcome/completed/finalText")
                    == Some(&json!("completed text"))
                && wire.pointer("/lastRun/outcome/completed/insertResult/copied")
                    == Some(&json!(true))
                && wire.pointer("/lastRun/outcome/completed/insertResult/autoPasteAttempted")
                    == Some(&json!(true))
                && wire.pointer("/lastRun/outcome/completed/insertResult/autoPasteOk")
                    == Some(&json!(false))
                && wire.pointer("/lastRun/outcome/completed/warning/code")
                    == Some(&json!("E_EXPORT_TARGET_UNAVAILABLE")),
            format!(
                "apply_error={:?}, current_insert_result={insert_result:?}, phase={}, wire={wire}",
                result.as_ref().err().map(|error| error.code.as_str()),
                observed_phase(&workflow)
            ),
        );
    }
    failures.finish("T12 paste_warning_keeps_completed_copy");
}

#[test]
fn target_contract_t13_cancel_recording_resources_and_context_are_serial() {
    let mut failures = SubcaseFailures::default();

    for (label, launch_before_cancel) in [
        ("resource_launch_then_cancel", true),
        ("cancel_then_resource_launch", false),
    ] {
        let run_id = format!("run-13-{label}");
        let workflow = recording_workflow(&run_id);
        let (mailbox, _events) = UiEventMailbox::for_test();
        let resource_signal = || {
            workflow.apply_event(
                &mailbox,
                display_event(
                    &format!("resource-{label}"),
                    "executor.resource.started",
                    &run_id,
                    "started",
                    json!({"resource": "recording+context"}),
                ),
            )
        };
        if launch_before_cancel {
            resource_signal().expect("current event boundary accepts resource-start evidence");
        }
        workflow
            .cancel_current_recording_for_test()
            .expect("current recording cancellation must be accepted");
        if !launch_before_cancel {
            resource_signal().expect("current event boundary accepts late resource-start evidence");
        }
        let cancelling_wire = wire_view(&workflow);
        let session_live_while_cancelling = workflow.snapshot().session.is_some();
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("cancelled-{label}"),
                    "workflow.stopped.cancelled",
                    &run_id,
                    "cancelled",
                    json!({"cleanupElapsedMs": 300}),
                ),
            )
            .expect("current event boundary accepts cleanup-proving Cancelled evidence");
        let wire = wire_view(&workflow);
        let cleanup_within_deadline = wire
            .pointer("/lastRun/cleanupDiagnostic/elapsedMs")
            .and_then(Value::as_u64)
            .is_some_and(|elapsed| elapsed <= 300);
        let expected_resources_at_cancel = if launch_before_cancel { 2_u64 } else { 0_u64 };
        failures.check(
            label,
            cancelling_wire.get("mode") == Some(&json!("cancelling"))
                && session_live_while_cancelling
                && cancelling_wire.pointer("/activeRun/arbiter/cancelWinner")
                    == Some(&json!(true))
                && cancelling_wire.pointer("/activeRun/resourceCounts/live")
                    == Some(&json!(expected_resources_at_cancel))
                && wire.get("mode") == Some(&json!("ready"))
                && workflow.snapshot().session.is_none()
                && wire.pointer("/lastRun/outcome/cancelled").is_some()
                && cleanup_within_deadline,
            format!(
                "Accepted cancel must linearize with resource launch, reject post-cancel launch, retain the handle in Cancelling, then prove cleanup in <=300ms; session_live_while_cancelling={session_live_while_cancelling}, cleanup_within_deadline={cleanup_within_deadline}, cancelling={cancelling_wire}, terminal={wire}, final_phase={}",
                observed_phase(&workflow),
            ),
        );
    }

    for (label, abnormal_kind) in [
        ("accepted_then_inner_panic", "executor.inner.panic"),
        ("accepted_then_control_close", "executor.control.closed"),
    ] {
        let run_id = format!("run-13-{label}");
        let workflow = recording_workflow(&run_id);
        workflow
            .cancel_current_recording_for_test()
            .expect("current recording cancellation must be accepted");
        let (mailbox, _events) = UiEventMailbox::for_test();
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("abnormal-{label}"),
                    abnormal_kind,
                    &run_id,
                    "failed",
                    json!({"afterCancelAccepted": true}),
                ),
            )
            .expect("current event boundary accepts abnormal-exit evidence");
        let wire = wire_view(&workflow);
        let cleanup_within_deadline = wire
            .pointer("/lastRun/cleanupDiagnostic/elapsedMs")
            .and_then(Value::as_u64)
            .is_some_and(|elapsed| elapsed <= 300);
        failures.check(
            label,
            observed_phase(&workflow) == "ready"
                && workflow.snapshot().session.is_none()
                && wire.pointer("/lastRun/outcome/cancelled").is_some()
                && wire.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64))
                && wire.pointer("/lastRun/cleanupDiagnostic").is_some()
                && cleanup_within_deadline,
            format!(
                "supervisor exit after an Accepted cancel must append cleanup diagnostics, preserve the Cancelled winner exactly once, and finish in <=300ms; observed phase={}, session_live={}, cleanup_within_deadline={cleanup_within_deadline}, wire={wire}",
                observed_phase(&workflow),
                workflow.snapshot().session.is_some()
            ),
        );
    }

    let workflow = recording_workflow("run-13-direct-cancelled");
    let (mailbox, _events) = UiEventMailbox::for_test();
    workflow
        .apply_event(
            &mailbox,
            display_event(
                "direct-cancelled-13",
                "workflow.stopped.cancelled",
                "run-13-direct-cancelled",
                "cancelled",
                json!({}),
            ),
        )
        .expect("current display-event boundary accepts direct Cancelled evidence");
    let wire = wire_view(&workflow);
    failures.check(
        "direct_recording_cancelled_is_protocol_failure",
        observed_phase(&workflow) == "ready"
            && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                == Some(&json!("E_EXECUTOR_CANCEL_UNACKNOWLEDGED")),
        format!(
            "Recording may reach Cancelled only through G2/C3; observed phase={} wire={wire}",
            observed_phase(&workflow)
        ),
    );

    failures.finish("T13 cancel_recording_resources_and_context_are_serial");
}

#[test]
fn target_contract_t14_cancel_each_processing_stage_and_reject_direct_terminal() {
    const STAGES: [&str; 6] = [
        "contextCapture",
        "recordFinalize",
        "preprocess",
        "transcribe",
        "rewrite",
        "insertPrepare",
    ];
    let mut failures = SubcaseFailures::default();

    for stage in STAGES {
        for (order, launch_first) in [("beforeLaunch", false), ("afterLaunch", true)] {
            for intent in ["primary", "cancel"] {
                let label = format!("{stage}_{order}_{intent}");
                let run_id = format!("run-14-{label}");
                let workflow = workflow_at_current_stage(stage, &run_id);
                let (mailbox, _events) = UiEventMailbox::for_test();
                let before = wire_view(&workflow);
                let launch = || {
                    workflow.apply_event(
                        &mailbox,
                        display_event(
                            &format!("launch-{label}"),
                            "workflow.progress",
                            &run_id,
                            "started",
                            json!({"stage": stage}),
                        ),
                    )
                };
                let cancel = || {
                    workflow.apply_event(
                        &mailbox,
                        display_event(
                            &format!("intent-{label}"),
                            &format!("workflow.intent.{intent}"),
                            &run_id,
                            "requested",
                            json!({"targetRunId": run_id}),
                        ),
                    )
                };
                let at_cancel = if launch_first {
                    launch().expect("current event boundary accepts launch evidence");
                    let view = wire_view(&workflow);
                    cancel().expect("current event boundary accepts cancellation evidence");
                    view
                } else {
                    let view = wire_view(&workflow);
                    cancel().expect("current event boundary accepts cancellation evidence");
                    launch().expect("current event boundary accepts post-cancel launch evidence");
                    view
                };
                let after_cancel = wire_view(&workflow);
                let revision_advanced = at_cancel
                    .get("revision")
                    .and_then(Value::as_u64)
                    .zip(after_cancel.get("revision").and_then(Value::as_u64))
                    .is_some_and(|(old, new)| new == old + 1);
                workflow
                    .apply_event(
                        &mailbox,
                        display_event(
                            &format!("cancelled-{label}"),
                            "workflow.stopped.cancelled",
                            &run_id,
                            "cancelled",
                            json!({"cleanupElapsedMs": 300}),
                        ),
                    )
                    .expect("current event boundary accepts stage cleanup evidence");
                let terminal = wire_view(&workflow);
                let cleanup_within_deadline = terminal
                    .pointer("/lastRun/cleanupDiagnostic/elapsedMs")
                    .and_then(Value::as_u64)
                    .is_some_and(|elapsed| elapsed <= 300);
                let expected_status = if launch_first { "started" } else { "pending" };
                failures.check(
                    &label,
                    at_cancel.pointer("/stage/name") == Some(&json!(stage))
                        && at_cancel.pointer("/stage/status") == Some(&json!(expected_status))
                        && at_cancel.get("cancelEnabled") == Some(&json!(true))
                        && after_cancel.get("mode") == Some(&json!("cancelling"))
                        && after_cancel.get("cancelEnabled") == Some(&json!(false))
                        && revision_advanced
                        && terminal.get("mode") == Some(&json!("ready"))
                        && terminal.pointer("/lastRun/outcome/cancelled").is_some()
                        && terminal.pointer("/lastRun/effects/historyCommitCount")
                            == Some(&json!(0_u64))
                        && terminal.pointer("/lastRun/effects/copyCount")
                            == Some(&json!(0_u64))
                        && cleanup_within_deadline,
                    format!(
                        "P1/C3 must accept {intent} at the {stage} {order} boundary, prevent later launch, cancel effects, and reach Ready(Cancelled) in <=300ms; cleanup_within_deadline={cleanup_within_deadline}, initial={before}, at_cancel={at_cancel}, after_cancel={after_cancel}, terminal={terminal}"
                    ),
                );
            }
        }

        let label = format!("{stage}_direct_cancelled_terminal");
        let run_id = format!("run-14-{label}");
        let workflow = workflow_at_current_stage(stage, &run_id);
        let (mailbox, _events) = UiEventMailbox::for_test();
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("terminal-{label}"),
                    "workflow.stopped.cancelled",
                    &run_id,
                    "cancelled",
                    json!({}),
                ),
            )
            .expect("current event boundary accepts direct Cancelled evidence");
        let wire = wire_view(&workflow);
        failures.check(
            &label,
            observed_phase(&workflow) == "ready"
                && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                    == Some(&json!("E_EXECUTOR_CANCEL_UNACKNOWLEDGED")),
            format!(
                "P9 must convert direct Cancelled without an Accepted winner into protocol Failed; observed phase={} wire={wire}",
                observed_phase(&workflow)
            ),
        );
    }

    failures.finish("T14 cancel_each_processing_stage_and_reject_direct_terminal");
}

#[test]
fn target_contract_t15_cancel_terminal_and_finalization_are_linearizable() {
    let mut failures = SubcaseFailures::default();

    let workflow = recording_workflow("run-15-stopped-first");
    let (terminal_mailbox, _terminal_events) = UiEventMailbox::for_test();
    workflow
        .apply_event(
            &terminal_mailbox,
            display_event(
                "stopped-first-15",
                "workflow.stopped.failed",
                "run-15-stopped-first",
                "failed",
                json!({"error": {"code": "E_SCRIPTED"}}),
            ),
        )
        .expect("current event boundary accepts terminal-first evidence");
    let before_old_cancel = wire_view(&workflow);
    let (cancel_mailbox, cancel_events) = UiEventMailbox::for_test();
    workflow
        .apply_event(
            &cancel_mailbox,
            display_event(
                "old-cancel-15",
                "workflow.intent.cancel",
                "run-15-stopped-first",
                "requested",
                json!({"targetRunId": "run-15-stopped-first"}),
            ),
        )
        .expect("current event boundary accepts old-cancel evidence");
    let after_old_cancel = wire_view(&workflow);
    let old_cancel_broadcast = !matches!(cancel_events.try_recv(), Err(TryRecvError::Empty));
    failures.check(
        "stopped_then_old_target_cancel_is_a1_noop",
        before_old_cancel.get("mode") == Some(&json!("ready"))
            && before_old_cancel.pointer("/lastRun/runId")
                == Some(&json!("run-15-stopped-first"))
            && before_old_cancel.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64))
            && before_old_cancel == after_old_cancel
            && !old_cancel_broadcast,
        format!(
            "a Cancel after Stopped must be a stale-target A1 NoOp without a new snapshot; state_same={}, broadcast={old_cancel_broadcast}, wire={after_old_cancel}",
            before_old_cancel == after_old_cancel
        ),
    );

    let workflow = recording_workflow("run-15-processing-race");
    workflow
        .prepare_stop_for_test()
        .expect("current stop fixture must enter its processing analogue");
    let (mailbox, _events) = UiEventMailbox::for_test();
    workflow
        .apply_event(
            &mailbox,
            display_event(
                "processing-cancel-15",
                "workflow.intent.cancel",
                "run-15-processing-race",
                "requested",
                json!({"targetRunId": "run-15-processing-race"}),
            ),
        )
        .expect("current event boundary accepts processing-race evidence");
    let wire = wire_view(&workflow);
    failures.check(
        "recording_to_processing_same_run_cancel_reaches_arbiter",
        matches!(observed_phase(&workflow).as_str(), "processing" | "cancelling")
            && wire.pointer("/commandDisposition").is_some()
            && wire.pointer("/activeRun/arbiter").is_some(),
        format!(
            "same-run Cancel racing G1 must return Accepted or TooLate instead of projection-stale NoOp; observed phase={} wire={wire}",
            observed_phase(&workflow)
        ),
    );

    let workflow = transcribed_workflow("run-15-finalize-race", "final text");
    workflow
        .begin_insert_for_task_for_test("run-15-finalize-race")
        .expect("current finalize fixture must enter insert phase");
    let (mailbox, _events) = UiEventMailbox::for_test();
    workflow
        .apply_event(
            &mailbox,
            display_event(
                "finalize-started-15",
                "workflow.progress",
                "run-15-finalize-race",
                "started",
                json!({"stage": "finalize"}),
            ),
        )
        .expect("current event boundary accepts finalization evidence");
    workflow
        .apply_event(
            &mailbox,
            display_event(
                "finalize-cancel-15",
                "workflow.intent.cancel",
                "run-15-finalize-race",
                "requested",
                json!({"targetRunId": "run-15-finalize-race"}),
            ),
        )
        .expect("current event boundary accepts finalization-race evidence");
    let wire = wire_view(&workflow);
    failures.check(
        "finalize_started_then_same_run_cancel_is_too_late",
        observed_phase(&workflow) == "processing"
            && wire.get("cancelEnabled") == Some(&json!(false))
            && wire.pointer("/commandDisposition") == Some(&json!("cancelTooLate")),
        format!(
            "Finalize ownership must linearize before Cancel and return TooLate; observed phase={} wire={wire}",
            observed_phase(&workflow)
        ),
    );

    let workflow = recording_workflow("run-15-cancel-winner");
    workflow
        .cancel_current_recording_for_test()
        .expect("current cancellation must be accepted");
    let (mailbox, _events) = UiEventMailbox::for_test();
    for (event_id, kind, status, payload) in [
        (
            "duplicate-cancel-15",
            "workflow.intent.cancel",
            "requested",
            json!({"targetRunId": "run-15-cancel-winner"}),
        ),
        (
            "progress-after-cancel-15",
            "workflow.progress",
            "completed",
            json!({"stage": "transcribe"}),
        ),
        (
            "cancelled-winner-15",
            "workflow.stopped.cancelled",
            "cancelled",
            json!({}),
        ),
    ] {
        workflow
            .apply_event(
                &mailbox,
                display_event(event_id, kind, "run-15-cancel-winner", status, payload),
            )
            .expect("current event boundary accepts cancel-winner sequence evidence");
    }
    let wire = wire_view(&workflow);
    failures.check(
        "accepted_cancel_duplicate_and_progress_end_once_cancelled",
        observed_phase(&workflow) == "ready"
            && workflow.snapshot().session.is_none()
            && wire.pointer("/lastRun/outcome/cancelled").is_some()
            && wire.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64)),
        format!(
            "C1/C2 must ignore duplicates/progress and C3 must publish one Cancelled terminal; observed phase={}, session_live={}, wire={wire}",
            observed_phase(&workflow),
            workflow.snapshot().session.is_some()
        ),
    );

    for (label, claim_kind, terminal_kind, expected_outcome) in [
        (
            "begin_terminal_then_cancel_too_late",
            "executor.begin_terminal",
            "workflow.stopped.failed",
            "failed",
        ),
        (
            "begin_finalization_then_cancel_too_late",
            "executor.begin_finalization",
            "workflow.stopped.completed",
            "completed",
        ),
    ] {
        let run_id = format!("run-15-{label}");
        let workflow = if claim_kind.ends_with("finalization") {
            let workflow = transcribed_workflow(&run_id, "final text");
            workflow
                .begin_insert_for_task_for_test(&run_id)
                .expect("finalization claim fixture must enter current insert phase");
            workflow
        } else {
            transcribing_workflow(&run_id)
        };
        let (mailbox, _events) = UiEventMailbox::for_test();
        for (suffix, kind, status, payload) in [
            ("claim", claim_kind, "accepted", json!({})),
            (
                "cancel",
                "workflow.intent.cancel",
                "requested",
                json!({"targetRunId": run_id}),
            ),
            (
                "terminal",
                terminal_kind,
                expected_outcome,
                json!({"result": {"finalText": "final text"}, "error": {"code": "E_SCRIPTED"}}),
            ),
        ] {
            workflow
                .apply_event(
                    &mailbox,
                    display_event(&format!("{label}-{suffix}"), kind, &run_id, status, payload),
                )
                .expect("current event boundary accepts claim/cancel/terminal evidence");
        }
        let wire = wire_view(&workflow);
        failures.check(
            label,
            observed_phase(&workflow) == "ready"
                && wire.pointer("/lastCommand/disposition") == Some(&json!("cancelTooLate"))
                && wire.pointer(&format!("/lastRun/outcome/{expected_outcome}"))
                    .is_some(),
            format!(
                "a terminal/finalization claim must make the racing Cancel TooLate and preserve the claimed terminal variant; observed phase={} wire={wire}",
                observed_phase(&workflow)
            ),
        );
    }

    for terminal_kind in ["completed", "empty", "failed"] {
        let label = format!("accepted_cancel_then_illegal_{terminal_kind}");
        let run_id = format!("run-15-{label}");
        let workflow = recording_workflow(&run_id);
        workflow
            .cancel_current_recording_for_test()
            .expect("current cancellation must be accepted");
        let (mailbox, _events) = UiEventMailbox::for_test();
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("illegal-terminal-{label}"),
                    &format!("workflow.stopped.{terminal_kind}"),
                    &run_id,
                    terminal_kind,
                    json!({
                        "result": {"finalText": "preserved result"},
                        "error": {"code": "E_ORIGINAL"},
                        "recoveryErrors": [{"code": "E_RECOVERY"}]
                    }),
                ),
            )
            .expect("current event boundary accepts illegal post-cancel terminal evidence");
        let wire = wire_view(&workflow);
        failures.check(
            &label,
            observed_phase(&workflow) == "ready"
                && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                    == Some(&json!("E_EXECUTOR_TERMINAL_AFTER_CANCEL"))
                && wire.pointer("/lastRun/outcome/failed/protocolContext/originalVariant")
                    == Some(&json!(terminal_kind))
                && wire.pointer("/lastRun/outcome/failed/recoveryErrors/0/code")
                    == Some(&json!("E_RECOVERY")),
            format!(
                "C4 must expose the protocol violation while preserving original variant/result/error/recovery diagnostics; observed phase={} wire={wire}",
                observed_phase(&workflow)
            ),
        );
    }

    failures.finish("T15 cancel_terminal_and_finalization_are_linearizable");
}

#[test]
fn target_contract_t16_late_or_duplicate_signal_has_no_effect() {
    let mut failures = SubcaseFailures::default();

    for mode in ["ready", "active", "cancelling"] {
        for signal in ["progress", "terminal"] {
            let label = format!("{mode}_receives_old_run_{signal}");
            let workflow = match mode {
                "ready" => VoiceWorkflow::new(),
                "active" => recording_workflow("run-16-current"),
                "cancelling" => {
                    let workflow = recording_workflow("run-16-current");
                    workflow
                        .cancel_current_recording_for_test()
                        .expect("current cancellation fixture must complete");
                    workflow
                }
                _ => unreachable!("documented T16 modes are exhaustive"),
            };
            let (mailbox, events) = UiEventMailbox::for_test();
            let before = wire_view(&workflow);
            let (kind, status, payload) = if signal == "progress" {
                (
                    "workflow.progress",
                    "completed",
                    json!({"stage": "transcribe", "text": "old"}),
                )
            } else {
                (
                    "workflow.stopped.completed",
                    "completed",
                    json!({"result": {"finalText": "old"}}),
                )
            };
            workflow
                .apply_event(
                    &mailbox,
                    display_event(
                        &format!("late-{label}"),
                        kind,
                        "run-16-old",
                        status,
                        payload,
                    ),
                )
                .expect("current display-event boundary accepts late-signal evidence");
            let after = wire_view(&workflow);
            let emitted = !matches!(events.try_recv(), Err(TryRecvError::Empty));
            let expected_mode = if mode == "active" { "recording" } else { mode };
            let target_shape = before.get("mode") == Some(&json!(expected_mode))
                && before.get("revision").and_then(Value::as_u64).is_some()
                && if mode == "ready" {
                    before.get("activeRun") == Some(&Value::Null)
                } else {
                    before.pointer("/activeRun/runId") == Some(&json!("run-16-current"))
                };
            failures.check(
                &label,
                target_shape && before == after && !emitted,
                format!(
                    "R3/G9/P10/C5 require an old run signal to leave state, revision, storage/insertion effects and snapshot emission untouched; target_shape={target_shape}, state_same={}, broadcast={emitted}, before={before}, after={after}",
                    before == after
                ),
            );
        }
    }

    for (outcome, outcome_name) in OutcomeFixture::ALL {
        let run_id = format!("run-16-duplicate-{outcome_name}");
        let workflow = workflow_after_outcome(outcome, &run_id);
        let (mailbox, events) = UiEventMailbox::for_test();
        let before = wire_view(&workflow);
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("duplicate-terminal-{outcome_name}"),
                    &format!("workflow.stopped.{outcome_name}"),
                    &run_id,
                    outcome_name,
                    json!({"duplicate": true}),
                ),
            )
            .expect("current display-event boundary accepts duplicate-terminal evidence");
        let after = wire_view(&workflow);
        let emitted = !matches!(events.try_recv(), Err(TryRecvError::Empty));
        failures.check(
            &format!("ready_{outcome_name}_receives_duplicate_terminal"),
            before == after
                && !emitted
                && after.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64)),
            format!(
                "a duplicate {outcome_name} terminal must retain the unique lastRun and emit no second snapshot/effect; state_same={}, broadcast={emitted}, wire={after}",
                before == after
            ),
        );
    }

    failures.finish("T16 late_or_duplicate_signal_has_no_effect");
}

#[test]
fn target_contract_t17_new_run_after_any_outcome_gets_new_identity() {
    let mut failures = SubcaseFailures::default();

    for (outcome, outcome_name) in OutcomeFixture::ALL {
        let old_run_id = format!("run-17-{outcome_name}");
        let workflow = workflow_after_outcome(outcome, &old_run_id);
        let terminal_wire = wire_view(&workflow);
        let reopened = workflow
            .open_recording_for_test(&old_run_id, &format!("recording-{old_run_id}-reused"));
        let active_id = workflow
            .snapshot()
            .session
            .as_ref()
            .map(|session| session.session_id.clone());
        let active_wire = wire_view(&workflow);
        failures.check(
            &format!("fresh_primary_after_{outcome_name}"),
            reopened.is_ok()
                && active_id
                    .as_deref()
                    .is_some_and(|id| id != old_run_id.as_str())
                && terminal_wire.pointer("/lastRun/runId") == Some(&json!(old_run_id))
                && active_wire
                    .pointer("/activeRun/runId")
                    .and_then(Value::as_str)
                    == active_id.as_deref(),
            format!(
                "R1 after {outcome_name} must allocate a non-reused controller-owned runId while retaining lastRun traceability; reopened={:?}, active_id={active_id:?}, terminal={terminal_wire}, active={active_wire}",
                reopened.as_ref().err().map(|error| error.code.as_str())
            ),
        );
    }

    failures.finish("T17 new_run_after_any_outcome_gets_new_identity");
}

#[test]
fn target_contract_t18_finalization_gate_excludes_partial_or_cancelled_commit() {
    let mut failures = SubcaseFailures::default();

    let workflow = recording_workflow("run-18-cancel-first");
    workflow
        .cancel_current_recording_for_test()
        .expect("current cancellation must be accepted");
    let (mailbox, _events) = UiEventMailbox::for_test();
    for (suffix, kind) in [
        ("late-finalization", "executor.begin_finalization"),
        ("late-completed", "workflow.stopped.completed"),
    ] {
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("cancel-first-{suffix}-18"),
                    kind,
                    "run-18-cancel-first",
                    "completed",
                    json!({"result": {"finalText": "must not commit"}}),
                ),
            )
            .expect("current event boundary accepts cancel-first finalization evidence");
    }
    let wire = wire_view(&workflow);
    failures.check(
        "cancel_wins_before_finalization",
        observed_phase(&workflow) == "ready"
            && wire.pointer("/lastRun/outcome/cancelled").is_some()
            && wire.pointer("/lastRun/effects/historyCommitCount") == Some(&json!(0_u64))
            && wire.pointer("/lastRun/effects/copyCount") == Some(&json!(0_u64)),
        format!(
            "an Accepted cancel must exclude finalization and all irreversible effects; observed phase={} wire={wire}",
            observed_phase(&workflow)
        ),
    );

    let workflow = transcribed_workflow("run-18-finalization-first", "final text");
    workflow
        .begin_insert_for_task_for_test("run-18-finalization-first")
        .expect("current finalization fixture must enter insert phase");
    let (mailbox, _events) = UiEventMailbox::for_test();
    for (suffix, kind, status, payload) in [
        (
            "claim",
            "executor.begin_finalization",
            "accepted",
            json!({}),
        ),
        (
            "cancel",
            "workflow.intent.cancel",
            "requested",
            json!({"targetRunId": "run-18-finalization-first"}),
        ),
        (
            "completed",
            "workflow.stopped.completed",
            "completed",
            json!({"result": {"finalText": "final text"}}),
        ),
    ] {
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("finalization-first-{suffix}-18"),
                    kind,
                    "run-18-finalization-first",
                    status,
                    payload,
                ),
            )
            .expect("current event boundary accepts finalization-first evidence");
    }
    let wire = wire_view(&workflow);
    failures.check(
        "finalization_wins_before_cancel",
        observed_phase(&workflow) == "ready"
            && wire.pointer("/lastRun/outcome/completed").is_some()
            && wire.pointer("/lastCommand/disposition") == Some(&json!("cancelTooLate"))
            && wire.pointer("/lastRun/effects/historyCommitCount") == Some(&json!(1_u64))
            && wire.pointer("/lastRun/effects/copyCount") == Some(&json!(1_u64)),
        format!(
            "a finalization winner must make Cancel TooLate and commit History/copy at most once; observed phase={} wire={wire}",
            observed_phase(&workflow)
        ),
    );

    let workflow = transcribed_workflow("run-18-duplicate", "final text");
    workflow
        .begin_insert_for_task_for_test("run-18-duplicate")
        .expect("current duplicate fixture must enter insert phase");
    workflow
        .complete_insert_for_test()
        .expect("current first insert callback must complete");
    let duplicate = workflow.complete_insert_for_test();
    let wire = wire_view(&workflow);
    failures.check(
        "duplicate_finalization_callback_is_noop",
        duplicate.is_ok()
            && wire.pointer("/lastRun/finalization/commitCount") == Some(&json!(1_u64))
            && wire.pointer("/lastRun/effects/historyCommitCount") == Some(&json!(1_u64))
            && wire.pointer("/lastRun/effects/copyCount") == Some(&json!(1_u64)),
        format!(
            "a duplicate callback must be idempotent after the unique finalization commit; duplicate={:?}, wire={wire}",
            duplicate.as_ref().err().map(|error| error.code.as_str())
        ),
    );

    let workflow = transcribed_workflow("run-18-history-failure", "recoverable text");
    workflow
        .begin_insert_for_task_for_test("run-18-history-failure")
        .expect("current history-failure fixture must enter insert phase");
    let (mailbox, _events) = UiEventMailbox::for_test();
    workflow
        .apply_event(
            &mailbox,
            display_event(
                "history-failure-18",
                "workflow.stopped.failed",
                "run-18-history-failure",
                "failed",
                json!({
                    "error": {"code": "E_HISTORY_WRITE"},
                    "recoveredResult": {"finalText": "recoverable text"}
                }),
            ),
        )
        .expect("current event boundary accepts History-failure evidence");
    let wire = wire_view(&workflow);
    failures.check(
        "history_failure_stops_copy_and_preserves_result",
        observed_phase(&workflow) == "ready"
            && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                == Some(&json!("E_HISTORY_WRITE"))
            && wire.pointer("/lastRun/outcome/failed/recoveredResult/finalText")
                == Some(&json!("recoverable text"))
            && wire.pointer("/lastRun/effects/historyCommitCount") == Some(&json!(1_u64))
            && wire.pointer("/lastRun/effects/copyCount") == Some(&json!(0_u64)),
        format!(
            "History failure must terminate with recovered text and prevent copy/paste; observed phase={} wire={wire}",
            observed_phase(&workflow)
        ),
    );

    for terminal_kind in ["completed", "empty", "failed"] {
        let label = format!("cancel_winner_rejects_{terminal_kind}");
        let run_id = format!("run-18-{label}");
        let workflow = recording_workflow(&run_id);
        workflow
            .cancel_current_recording_for_test()
            .expect("current cancellation must be accepted");
        let (mailbox, _events) = UiEventMailbox::for_test();
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("terminal-{label}"),
                    &format!("workflow.stopped.{terminal_kind}"),
                    &run_id,
                    terminal_kind,
                    json!({
                        "result": {"finalText": "preserved"},
                        "error": {"code": "E_ORIGINAL"},
                        "recoveryErrors": [{"code": "E_RECOVERY"}]
                    }),
                ),
            )
            .expect("current event boundary accepts post-cancel terminal evidence");
        let wire = wire_view(&workflow);
        failures.check(
            &label,
            observed_phase(&workflow) == "ready"
                && wire.pointer("/lastRun/outcome/failed/primaryError/code")
                    == Some(&json!("E_EXECUTOR_TERMINAL_AFTER_CANCEL"))
                && wire.pointer("/lastRun/outcome/failed/protocolContext/originalVariant")
                    == Some(&json!(terminal_kind))
                && wire.pointer("/lastRun/outcome/failed/recoveryErrors/0/code")
                    == Some(&json!("E_RECOVERY")),
            format!(
                "C4 must retain original terminal diagnostics under a protocol Failed outcome; observed phase={} wire={wire}",
                observed_phase(&workflow)
            ),
        );
    }

    failures.finish("T18 finalization_gate_excludes_partial_or_cancelled_commit");
}

#[test]
fn target_contract_t19_projection_bootstrap_and_revision_order_are_total() {
    let mut failures = SubcaseFailures::default();
    let ready = VoiceWorkflow::new();
    let ready_wire = wire_view(&ready);
    failures.check(
        "initial_snapshot_revision_zero",
        ready_wire.get("mode") == Some(&json!("ready"))
            && ready_wire.get("revision") == Some(&json!(0_u64))
            && ready_wire.get("actionKey") == Some(&json!("Start(Initial)")),
        format!(
            "the first complete projection must preserve revision 0 and its derived Initial action; observed wire={ready_wire}"
        ),
    );

    let recording = recording_workflow("run-19-order");
    let recording_wire = wire_view(&recording);
    failures.check(
        "r1_commit_begin_sink_reply_projection",
        recording_wire.get("mode") == Some(&json!("recording"))
            && recording_wire.get("revision") == Some(&json!(1_u64))
            && recording_wire.pointer("/activeRun/runId") == Some(&json!("run-19-order"))
            && recording_wire.pointer("/activeRun/beginAcceptedAtMs").is_some()
            && recording_wire.pointer("/projectionOrder")
                == Some(&json!(["commit", "beginAccepted", "snapshot", "reply"])),
        format!(
            "R1 must expose one post-BeginAccepted revision after commit and before reply; observed wire={recording_wire}"
        ),
    );

    let (mailbox, events) = UiEventMailbox::for_test();
    let before = wire_view(&recording);
    let legacy_report = recording.apply_event(
        &mailbox,
        display_event(
            "legacy-report-19",
            "workflow.stopped.completed",
            "run-19-order",
            "completed",
            json!({"result": {"finalText": "frontend-owned"}}),
        ),
    );
    let after = wire_view(&recording);
    let emitted = !matches!(events.try_recv(), Err(TryRecvError::Empty));
    failures.check(
        "frontend_report_apply_path_is_absent",
        legacy_report.is_err() && before == after && !emitted,
        format!(
            "the UI must not be able to advance business state through the legacy report/apply boundary; accepted={}, state_same={}, broadcast={emitted}, wire={after}",
            legacy_report.is_ok(),
            before == after
        ),
    );

    failures.finish("T19 projection_bootstrap_and_revision_order_are_total");
}

#[test]
fn target_contract_t20_ready_implies_no_live_run_resources() {
    let mut failures = SubcaseFailures::default();

    for (outcome, outcome_name) in OutcomeFixture::ALL {
        let run_id = format!("run-20-{outcome_name}");
        let workflow = workflow_after_outcome(outcome, &run_id);
        let snapshot = workflow.snapshot();
        let wire = wire_view(&workflow);
        let resources_zero = [
            "runHandle",
            "ffmpeg",
            "providerRequest",
            "cancellationToken",
            "temporaryAsset",
        ]
        .into_iter()
        .all(|resource| {
            wire.pointer(&format!("/resourceCounts/{resource}")) == Some(&json!(0_u64))
        });
        failures.check(
            &format!("ready_after_{outcome_name}_has_no_live_resources"),
            wire.get("mode") == Some(&json!("ready"))
                && wire.get("activeRun") == Some(&Value::Null)
                && snapshot.session.is_none()
                && resources_zero,
            format!(
                "I2 requires Ready after {outcome_name} to prove zero RunHandle/FFmpeg/request/token/temp assets; observed phase={}, session_live={}, resources_zero={resources_zero}, wire={wire}",
                observed_phase(&workflow),
                snapshot.session.is_some()
            ),
        );
    }

    failures.finish("T20 ready_implies_no_live_run_resources");
}

#[test]
fn target_contract_t21_cleanup_timeout_escalates_or_fails_closed() {
    let mut failures = SubcaseFailures::default();

    for (winner, terminal_kind) in [("failed", "failed"), ("cancelled", "cancelled")] {
        let run_id = format!("run-21-force-success-{winner}");
        let workflow = recording_workflow(&run_id);
        if winner == "failed" {
            workflow.fail_for_test("E_EFFECT_TIMEOUT", "scripted cooperative timeout");
        } else {
            workflow
                .cancel_current_recording_for_test()
                .expect("current cancellation must be accepted");
        }
        let (mailbox, _events) = UiEventMailbox::for_test();
        workflow
            .apply_event(
                &mailbox,
                display_event(
                    &format!("cleanup-force-success-{winner}"),
                    "executor.cleanup.completed",
                    &run_id,
                    "completed",
                    json!({
                        "gracefulTimedOut": true,
                        "forceAttempted": true,
                        "releaseProven": true
                    }),
                ),
            )
            .expect("current event boundary accepts successful force-cleanup evidence");
        let wire = wire_view(&workflow);
        failures.check(
            &format!("cooperative_timeout_force_success_{winner}"),
            observed_phase(&workflow) == "ready"
                && workflow.snapshot().session.is_none()
                && wire.pointer(&format!("/lastRun/outcome/{terminal_kind}"))
                    .is_some()
                && wire.pointer("/lastRun/cleanupDiagnostic/gracefulTimedOut")
                    == Some(&json!(true))
                && wire.pointer("/lastRun/cleanupDiagnostic/forceAttempted")
                    == Some(&json!(true))
                && wire.pointer("/lastRun/stoppedCount") == Some(&json!(1_u64)),
            format!(
                "force cleanup that proves release must preserve the {winner} winner and publish one Stopped only after resources reach zero; observed phase={}, session_live={}, wire={wire}",
                observed_phase(&workflow),
                workflow.snapshot().session.is_some()
            ),
        );
    }

    let workflow = recording_workflow("run-21-force-failure");
    workflow
        .cancel_current_recording_for_test()
        .expect("current cancellation must be accepted");
    let (mailbox, _events) = UiEventMailbox::for_test();
    workflow
        .apply_event(
            &mailbox,
            display_event(
                "cleanup-force-failure-21",
                "executor.cleanup.failed",
                "run-21-force-failure",
                "failed",
                json!({
                    "gracefulTimedOut": true,
                    "forceAttempted": true,
                    "releaseProven": false
                }),
            ),
        )
        .expect("current event boundary accepts failed force-cleanup evidence");
    let fatal_wire = wire_view(&workflow);
    let accepted_next_run = workflow
        .open_recording_for_test("run-21-next", "recording-run-21-next")
        .is_ok();
    failures.check(
        "cooperative_and_force_timeout_fail_closed",
        fatal_wire.get("mode") == Some(&json!("fatal"))
            && fatal_wire.pointer("/fatal/error/code")
                == Some(&json!("E_RESOURCE_RELEASE_UNPROVEN"))
            && fatal_wire.get("lastRun") == Some(&Value::Null)
            && !accepted_next_run,
        format!(
            "unproven release must trigger fatal containment without Ready, a pseudo-Stopped, or a new run; accepted_next_run={accepted_next_run}, wire={fatal_wire}"
        ),
    );

    failures.finish("T21 cleanup_timeout_escalates_or_fails_closed");
}
