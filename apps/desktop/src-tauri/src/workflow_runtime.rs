use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use tauri::AppHandle;
use typevoice_core::workflow::{
    AsrPlan, ContextPlan, InsertionPlan, RewritePlan, RunOutcomeView, RunPlanSeed, StageKind,
    WorkflowError, WorkflowMode, WorkflowView,
};
use typevoice_engine::{
    ui_events::{UiEvent, UiEventMailbox},
    workflow_controller::{WorkflowResult, WorkflowSnapshotSink},
};
use typevoice_storage::settings::{self, Settings};

pub struct UiWorkflowSnapshotSink {
    mailbox: UiEventMailbox,
    app: AppHandle,
    previous: Mutex<Option<WorkflowView>>,
}

impl UiWorkflowSnapshotSink {
    pub fn new(mailbox: UiEventMailbox, app: AppHandle) -> Self {
        Self {
            mailbox,
            app,
            previous: Mutex::new(None),
        }
    }
}

impl WorkflowSnapshotSink for UiWorkflowSnapshotSink {
    fn publish(&self, view: &WorkflowView) -> WorkflowResult<()> {
        let previous = self.previous.lock().unwrap().replace(view.clone());
        self.mailbox.send(UiEvent::workflow_state(view));
        if let Some(previous) = previous {
            trace_workflow_transition(&previous, view);
        }
        Ok(())
    }

    fn diagnostic(&self, run_id: &str, code: &str, message: &str, mut context: serde_json::Value) {
        if let Some(object) = context.as_object_mut() {
            object.insert("code".to_string(), serde_json::json!(code));
            object.insert("message".to_string(), serde_json::json!(message));
        }
        if let Ok(dir) = typevoice_storage::data_dir::data_dir() {
            typevoice_observability::obs::event(
                &dir,
                Some(run_id),
                "Workflow",
                "workflow.signal_diagnostic",
                "ignored",
                Some(context),
            );
        }
    }

    fn fatal(
        &self,
        run_id: &str,
        error: &WorkflowError,
        context: serde_json::Value,
        deadline: std::time::Instant,
    ) {
        if let Ok(dir) = typevoice_storage::data_dir::data_dir() {
            let persisted = typevoice_observability::obs::event_err_durable(
                &dir,
                typevoice_observability::obs::ErrorEvent {
                    task_id: Some(run_id),
                    stage: "Workflow",
                    step_id: "workflow.fatal",
                    kind: "workflow",
                    code: &error.code,
                    ctx: Some(context),
                },
                &error.message,
                deadline,
            );
            if !persisted {
                typevoice_observability::safe_eprintln!(
                    "fatal trace fallback: run_id={run_id} code={}",
                    error.code
                );
            }
        } else {
            typevoice_observability::safe_eprintln!(
                "fatal trace fallback: data directory unavailable; run_id={run_id} code={}",
                error.code
            );
        }
        self.mailbox.send(UiEvent::error(
            run_id,
            error.code.clone(),
            error.message.clone(),
        ));
        self.app.exit(1);
    }
}

fn trace_workflow_transition(previous: &WorkflowView, current: &WorkflowView) {
    if current.revision <= previous.revision {
        return;
    }
    let terminal = (previous.last_run != current.last_run)
        .then_some(current.last_run.as_ref())
        .flatten();
    let run_id = current
        .active_run
        .as_ref()
        .map(|run| run.run_id.as_str())
        .or_else(|| terminal.map(|run| run.run_id.as_str()))
        .or_else(|| previous.active_run.as_ref().map(|run| run.run_id.as_str()));
    let Some(run_id) = run_id else {
        return;
    };
    let stage = current
        .active_run
        .as_ref()
        .and_then(|run| run.stage.as_ref())
        .or_else(|| {
            previous
                .active_run
                .as_ref()
                .and_then(|run| run.stage.as_ref())
        })
        .map(|stage| stage_name(stage.kind));
    let cause = transition_cause(previous, current, terminal.is_some());
    let mut context = serde_json::json!({
        "runId": run_id,
        "revision": current.revision,
        "from": mode_name(previous.mode),
        "to": mode_name(current.mode),
        "cause": cause,
        "stage": stage,
        "outcome": serde_json::Value::Null,
        "errorCode": serde_json::Value::Null,
        "recoveryErrorCodes": [],
        "protocolTerminal": serde_json::Value::Null,
        "protocolErrorCode": serde_json::Value::Null,
    });
    if let (Some(terminal), Some(object)) = (terminal, context.as_object_mut()) {
        match &terminal.outcome {
            RunOutcomeView::Completed { warning, .. } => {
                object.insert("outcome".to_string(), serde_json::json!("completed"));
                if let Some(warning) = warning {
                    object.insert("warningCode".to_string(), serde_json::json!(warning.code));
                }
            }
            RunOutcomeView::Empty { .. } => {
                object.insert("outcome".to_string(), serde_json::json!("empty"));
            }
            RunOutcomeView::Cancelled { .. } => {
                object.insert("outcome".to_string(), serde_json::json!("cancelled"));
            }
            RunOutcomeView::Failed {
                primary_error,
                recovery_errors,
                protocol_context,
                ..
            } => {
                object.insert("outcome".to_string(), serde_json::json!("failed"));
                object.insert(
                    "errorCode".to_string(),
                    serde_json::json!(primary_error.code),
                );
                object.insert(
                    "recoveryErrorCodes".to_string(),
                    serde_json::json!(recovery_errors
                        .iter()
                        .map(|error| error.code.as_str())
                        .collect::<Vec<_>>()),
                );
                if let Some(protocol) = protocol_context {
                    object.insert(
                        "protocolTerminal".to_string(),
                        serde_json::json!(protocol.original_variant),
                    );
                    object.insert(
                        "protocolErrorCode".to_string(),
                        serde_json::json!(protocol
                            .original_error
                            .as_ref()
                            .map(|error| error.code.as_str())),
                    );
                }
            }
        }
    }
    if let Ok(dir) = typevoice_storage::data_dir::data_dir() {
        typevoice_observability::obs::event(
            &dir,
            Some(run_id),
            "Workflow",
            "workflow.transition",
            "committed",
            Some(context),
        );
    }
}

fn transition_cause(
    previous: &WorkflowView,
    current: &WorkflowView,
    terminal_changed: bool,
) -> &'static str {
    if terminal_changed {
        return "terminal";
    }
    match (previous.mode, current.mode) {
        (WorkflowMode::Ready, WorkflowMode::Recording) => "start",
        (WorkflowMode::Recording, WorkflowMode::Processing) => "stop",
        (_, WorkflowMode::Cancelling) => "cancel",
        _ => "progress",
    }
}

fn mode_name(mode: WorkflowMode) -> &'static str {
    match mode {
        WorkflowMode::Ready => "ready",
        WorkflowMode::Recording => "recording",
        WorkflowMode::Processing => "processing",
        WorkflowMode::Cancelling => "cancelling",
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

pub fn run_plan_seed(settings_value: &Settings) -> Result<RunPlanSeed, WorkflowError> {
    let provider = settings::resolve_asr_provider(settings_value);
    let rewrite_enabled = settings_value.rewrite_enabled.unwrap_or(false);
    let base_url = resolve_value_or_env(
        settings_value.llm_base_url.as_deref(),
        "TYPEVOICE_LLM_BASE_URL",
    );
    let model = resolve_value_or_env(settings_value.llm_model.as_deref(), "TYPEVOICE_LLM_MODEL");
    let prompt = settings_value
        .llm_prompt
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if rewrite_enabled {
        if base_url.is_empty() {
            return Err(WorkflowError::new(
                "E_LLM_CONFIG_BASE_URL_MISSING",
                "rewrite is enabled but no LLM base URL is configured",
            ));
        }
        if model.is_empty() {
            return Err(WorkflowError::new(
                "E_LLM_CONFIG_MODEL_MISSING",
                "rewrite is enabled but no LLM model is configured",
            ));
        }
        if prompt.is_empty() {
            return Err(WorkflowError::new(
                "E_SETTINGS_LLM_PROMPT_MISSING",
                "rewrite is enabled but no LLM prompt is configured",
            ));
        }
    }

    let include_history = settings_value.context_include_history.unwrap_or(true);
    let include_clipboard = settings_value.context_include_clipboard.unwrap_or(true);
    let include_prev_window_meta = settings_value
        .context_include_prev_window_meta
        .unwrap_or(true);
    let include_prev_window_screenshot = settings_value
        .context_include_prev_window_screenshot
        .unwrap_or(true);
    let context_enabled = rewrite_enabled
        && (include_history
            || include_clipboard
            || include_prev_window_meta
            || include_prev_window_screenshot);

    let mut seed = RunPlanSeed {
        settings_revision: String::new(),
        asr: AsrPlan {
            provider,
            remote_url: settings::resolve_remote_asr_url(settings_value),
            remote_model: settings::resolve_remote_asr_model(settings_value),
            remote_concurrency: settings::resolve_remote_asr_concurrency(settings_value),
            silence_trim_enabled: settings_value
                .asr_preprocess_silence_trim_enabled
                .unwrap_or(false),
            silence_threshold_db: settings_value
                .asr_preprocess_silence_threshold_db
                .unwrap_or(-50.0)
                .round()
                .clamp(f64::from(i32::MIN), f64::from(i32::MAX))
                as i32,
            silence_start_ms: settings_value
                .asr_preprocess_silence_start_ms
                .unwrap_or(300),
            silence_end_ms: settings_value.asr_preprocess_silence_end_ms.unwrap_or(300),
        },
        recording: typevoice_platform::record_input_cache::recording_plan_from_settings(
            settings_value,
        ),
        rewrite: RewritePlan {
            enabled: rewrite_enabled,
            base_url,
            model,
            reasoning_effort: settings_value.llm_reasoning_effort.clone(),
            prompt,
            glossary: settings_value.rewrite_glossary.clone().unwrap_or_default(),
            include_glossary: settings_value.rewrite_include_glossary.unwrap_or(true),
            supports_vision: settings_value.llm_supports_vision.unwrap_or(true),
        },
        context: ContextPlan {
            enabled: context_enabled,
            include_history,
            history_n: settings_value
                .context_history_n
                .filter(|value| *value > 0)
                .unwrap_or(3) as usize,
            history_window_ms: settings_value
                .context_history_window_ms
                .filter(|value| *value > 0)
                .unwrap_or(30 * 60 * 1000),
            include_clipboard,
            include_prev_window_meta,
            include_prev_window_screenshot,
        },
        insertion: InsertionPlan {
            auto_paste: settings::resolve_auto_paste_enabled(settings_value),
        },
        intent_source: "ui".to_string(),
    };
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(&seed)
        .unwrap_or_default()
        .hash(&mut hasher);
    seed.settings_revision = format!("{:016x}", hasher.finish());
    Ok(seed)
}

fn resolve_value_or_env(value: Option<&str>, env_name: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            std::env::var(env_name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default()
}
