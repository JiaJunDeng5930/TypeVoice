use serde::{Deserialize, Serialize};

pub type RunId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowMode {
    Ready,
    Recording,
    Processing,
    Cancelling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StageKind {
    ContextCapture,
    RecordFinalize,
    Preprocess,
    Transcribe,
    Rewrite,
    InsertPrepare,
    Finalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StageStatus {
    Pending,
    Started,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageView {
    pub kind: StageKind,
    pub status: StageStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u128>,
}

impl StageView {
    pub fn pending(kind: StageKind) -> Self {
        Self {
            kind,
            status: StageStatus::Pending,
            elapsed_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptionMetrics {
    pub rtf: f64,
    pub device_used: String,
    pub preprocess_ms: u128,
    pub asr_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
            history_id: transcript_id.clone(),
            transcript_id,
            final_text: asr_text.clone(),
            asr_text,
            metrics,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RewriteResult {
    pub transcript_id: String,
    pub final_text: String,
    pub rewrite_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InsertResult {
    pub copied: bool,
    pub auto_paste_attempted: bool,
    pub auto_paste_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl InsertResult {
    pub fn copy_only() -> Self {
        Self {
            copied: true,
            auto_paste_attempted: false,
            auto_paste_ok: false,
            error_code: None,
            error_message: None,
        }
    }

    pub fn pasted() -> Self {
        Self {
            copied: true,
            auto_paste_attempted: true,
            auto_paste_ok: true,
            error_code: None,
            error_message: None,
        }
    }

    pub fn paste_failed(code: &str, message: impl Into<String>) -> Self {
        Self {
            copied: true,
            auto_paste_attempted: true,
            auto_paste_ok: false,
            error_code: Some(code.to_string()),
            error_message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowError {
    pub code: String,
    pub message: String,
}

impl WorkflowError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn render(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

impl std::error::Error for WorkflowError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunTimings {
    pub total_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preprocess_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asr_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite_ms: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveredRunResult {
    pub asr_text: String,
    pub final_text: String,
    pub timings: RunTimings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<TranscriptionMetrics>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletedRunResult {
    pub asr_text: String,
    pub final_text: String,
    pub timings: RunTimings,
    pub insert_result: InsertResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<TranscriptionMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolContext {
    pub original_variant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_error: Option<WorkflowError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupDiagnostic {
    pub elapsed_ms: u128,
    pub graceful_timed_out: bool,
    pub force_attempted: bool,
    pub force_succeeded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectCounts {
    pub history_commit_count: u32,
    pub copy_count: u32,
    pub paste_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalizationAudit {
    pub commit_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAudit {
    pub effects: EffectCounts,
    pub finalization: FinalizationAudit,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<CleanupDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StoppedTerminal {
    Completed {
        result: CompletedRunResult,
        #[serde(skip_serializing_if = "Option::is_none")]
        warning: Option<WorkflowError>,
    },
    Empty {
        timings: RunTimings,
    },
    Failed {
        error: WorkflowError,
        #[serde(skip_serializing_if = "Option::is_none")]
        recovered_result: Option<RecoveredRunResult>,
        #[serde(default)]
        recovery_errors: Vec<WorkflowError>,
        #[serde(default)]
        record_saved: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol_context: Option<ProtocolContext>,
    },
    Cancelled {
        #[serde(skip_serializing_if = "Option::is_none")]
        recovered_result: Option<RecoveredRunResult>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cleanup_diagnostic: Option<CleanupDiagnostic>,
    },
}

impl StoppedTerminal {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Completed { .. } => "completed",
            Self::Empty { .. } => "empty",
            Self::Failed { .. } => "failed",
            Self::Cancelled { .. } => "cancelled",
        }
    }

    pub fn primary_error(&self) -> Option<&WorkflowError> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stopped {
    pub run_id: RunId,
    pub terminal: StoppedTerminal,
    #[serde(default)]
    pub audit: RunAudit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageProgress {
    pub status: StageStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TranscribeProgress {
    Started {
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
    Completed {
        result: TranscriptionResult,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RewriteProgress {
    Started {
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
    Completed {
        result: RewriteResult,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InsertPrepareResult {
    pub target: String,
    pub text_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InsertPrepareProgress {
    Started {
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
    Completed {
        result: InsertPrepareResult,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ProgressPayload {
    ContextCapture(StageProgress),
    RecordFinalize(StageProgress),
    Preprocess(StageProgress),
    Transcribe(TranscribeProgress),
    Rewrite(RewriteProgress),
    InsertPrepare(InsertPrepareProgress),
    Finalize {
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u128>,
    },
}

impl ProgressPayload {
    pub fn kind(&self) -> StageKind {
        match self {
            Self::ContextCapture(_) => StageKind::ContextCapture,
            Self::RecordFinalize(_) => StageKind::RecordFinalize,
            Self::Preprocess(_) => StageKind::Preprocess,
            Self::Transcribe(_) => StageKind::Transcribe,
            Self::Rewrite(_) => StageKind::Rewrite,
            Self::InsertPrepare(_) => StageKind::InsertPrepare,
            Self::Finalize { .. } => StageKind::Finalize,
        }
    }

    pub fn status(&self) -> StageStatus {
        match self {
            Self::ContextCapture(progress)
            | Self::RecordFinalize(progress)
            | Self::Preprocess(progress) => progress.status,
            Self::Transcribe(TranscribeProgress::Started { .. })
            | Self::Rewrite(RewriteProgress::Started { .. })
            | Self::InsertPrepare(InsertPrepareProgress::Started { .. })
            | Self::Finalize { .. } => StageStatus::Started,
            Self::Transcribe(TranscribeProgress::Completed { .. })
            | Self::Rewrite(RewriteProgress::Completed { .. })
            | Self::InsertPrepare(InsertPrepareProgress::Completed { .. }) => {
                StageStatus::Completed
            }
        }
    }

    pub fn elapsed_ms(&self) -> Option<u128> {
        match self {
            Self::ContextCapture(progress)
            | Self::RecordFinalize(progress)
            | Self::Preprocess(progress) => progress.elapsed_ms,
            Self::Transcribe(TranscribeProgress::Started { elapsed_ms })
            | Self::Transcribe(TranscribeProgress::Completed { elapsed_ms, .. })
            | Self::Rewrite(RewriteProgress::Started { elapsed_ms })
            | Self::Rewrite(RewriteProgress::Completed { elapsed_ms, .. })
            | Self::InsertPrepare(InsertPrepareProgress::Started { elapsed_ms })
            | Self::InsertPrepare(InsertPrepareProgress::Completed { elapsed_ms, .. })
            | Self::Finalize { elapsed_ms } => *elapsed_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Progress {
    pub run_id: RunId,
    pub payload: ProgressPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AsrPlan {
    pub provider: String,
    pub remote_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_model: Option<String>,
    pub remote_concurrency: usize,
    pub silence_trim_enabled: bool,
    pub silence_threshold_db: i32,
    pub silence_start_ms: u64,
    pub silence_end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingPlan {
    pub input_strategy: String,
    pub follow_default_role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RewritePlan {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub prompt: String,
    pub glossary: Vec<String>,
    pub include_glossary: bool,
    pub supports_vision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextPlan {
    pub enabled: bool,
    pub include_history: bool,
    pub history_n: usize,
    pub history_window_ms: i64,
    pub include_clipboard: bool,
    pub include_prev_window_meta: bool,
    pub include_prev_window_screenshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InsertionPlan {
    pub auto_paste: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunPlanSeed {
    pub settings_revision: String,
    pub asr: AsrPlan,
    pub recording: RecordingPlan,
    pub rewrite: RewritePlan,
    pub context: ContextPlan,
    pub insertion: InsertionPlan,
    pub intent_source: String,
}

impl Default for RunPlanSeed {
    fn default() -> Self {
        Self {
            settings_revision: "initial".to_string(),
            asr: AsrPlan {
                provider: "doubao".to_string(),
                remote_url: String::new(),
                remote_model: None,
                remote_concurrency: 4,
                silence_trim_enabled: false,
                silence_threshold_db: -45,
                silence_start_ms: 120,
                silence_end_ms: 250,
            },
            recording: RecordingPlan {
                input_strategy: "follow_default".to_string(),
                follow_default_role: "communications".to_string(),
                fixed_endpoint_id: None,
            },
            rewrite: RewritePlan {
                enabled: false,
                base_url: String::new(),
                model: String::new(),
                reasoning_effort: None,
                prompt: String::new(),
                glossary: Vec::new(),
                include_glossary: true,
                supports_vision: false,
            },
            context: ContextPlan {
                enabled: false,
                include_history: false,
                history_n: 3,
                history_window_ms: 30 * 60 * 1000,
                include_clipboard: false,
                include_prev_window_meta: false,
                include_prev_window_screenshot: false,
            },
            insertion: InsertionPlan { auto_paste: true },
            intent_source: "ui".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveRunView {
    pub run_id: RunId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<StageView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RecoveredRunResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunOutcomeView {
    Completed {
        #[serde(flatten)]
        result: CompletedRunResult,
        #[serde(skip_serializing_if = "Option::is_none")]
        warning: Option<WorkflowError>,
    },
    Empty {
        timings: RunTimings,
    },
    Failed {
        primary_error: WorkflowError,
        #[serde(skip_serializing_if = "Option::is_none")]
        recovered_result: Option<RecoveredRunResult>,
        recovery_errors: Vec<WorkflowError>,
        record_saved: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol_context: Option<ProtocolContext>,
    },
    Cancelled {
        #[serde(skip_serializing_if = "Option::is_none")]
        recovered_result: Option<RecoveredRunResult>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LastRunView {
    pub run_id: RunId,
    pub outcome: RunOutcomeView,
    pub stopped_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_diagnostic: Option<CleanupDiagnostic>,
    pub effects: EffectCounts,
    pub finalization: FinalizationAudit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowView {
    pub revision: u64,
    pub action_key: String,
    pub mode: WorkflowMode,
    pub active_run: Option<ActiveRunView>,
    pub last_run: Option<LastRunView>,
    pub primary_label: String,
    pub primary_disabled: bool,
    pub cancel_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkflowIntent {
    Primary { action_key: String },
    Cancel { target_run_id: Option<RunId> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandDisposition {
    Applied,
    NoOp,
    CancelTooLate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowCommandReply {
    pub disposition: CommandDisposition,
    pub view: WorkflowView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelDisposition {
    Accepted,
    TooLate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_union_rejects_missing_completed_result() {
        let value = serde_json::json!({
            "runId": "run-1",
            "terminal": {"completed": {"warning": null}},
            "audit": {"effects": {"historyCommitCount": 0, "copyCount": 0, "pasteCount": 0}, "finalization": {"commitCount": 0}}
        });
        assert!(serde_json::from_value::<Stopped>(value).is_err());
    }

    #[test]
    fn stopped_union_rejects_error_on_empty() {
        let value = serde_json::json!({
            "runId": "run-1",
            "terminal": {
                "empty": {
                    "timings": { "totalMs": 1 },
                    "error": { "code": "E_ILLEGAL", "message": "not allowed" }
                }
            }
        });
        assert!(serde_json::from_value::<Stopped>(value).is_err());
    }

    #[test]
    fn stopped_union_rejects_error_on_cancelled() {
        let value = serde_json::json!({
            "runId": "run-1",
            "terminal": {
                "cancelled": {
                    "error": { "code": "E_ILLEGAL", "message": "not allowed" }
                }
            }
        });
        assert!(serde_json::from_value::<Stopped>(value).is_err());
    }

    #[test]
    fn workflow_intent_uses_strict_camel_case_fields() {
        let value = serde_json::json!({
            "kind": "primary",
            "actionKey": "Start(Initial)"
        });
        assert!(serde_json::from_value::<WorkflowIntent>(value).is_ok());
        assert!(serde_json::from_value::<WorkflowIntent>(serde_json::json!({
            "kind": "primary",
            "action_key": "Start(Initial)"
        }))
        .is_err());
        assert!(serde_json::from_value::<WorkflowIntent>(serde_json::json!({
            "kind": "cancel"
        }))
        .is_ok());
    }

    #[test]
    fn run_outcome_serializes_camel_case_variant_fields() {
        let value = serde_json::to_value(RunOutcomeView::Failed {
            primary_error: WorkflowError::new("E_TEST", "failed"),
            recovered_result: None,
            recovery_errors: Vec::new(),
            record_saved: false,
            protocol_context: Some(ProtocolContext {
                original_variant: "completed".to_string(),
                original_error: None,
            }),
        })
        .expect("serialize failed outcome");
        let failed = value
            .get("failed")
            .and_then(serde_json::Value::as_object)
            .expect("failed variant object");
        assert!(failed.contains_key("primaryError"));
        assert!(failed.contains_key("recoveryErrors"));
        assert!(failed.contains_key("recordSaved"));
        assert!(failed.contains_key("protocolContext"));
        assert!(!failed.contains_key("primary_error"));
    }
}
