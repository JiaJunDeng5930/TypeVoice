use std::{collections::HashMap, path::Path, sync::Mutex};

use crate::audio_capture::RecordingRegistry;
use crate::context_capture;
use crate::context_pack::ContextSnapshot;
use serde::{Deserialize, Serialize};

use crate::insertion::{InsertResult, InsertTextRequest};
use crate::ports::PortError;
use crate::record_input_cache::RecordInputCacheState;
use crate::rewrite::{RewriteResult, RewriteTextRequest};
use crate::task_manager::TaskManager;
use crate::transcription::{
    TranscribeFixtureRequest, TranscriptionInput, TranscriptionMetrics, TranscriptionResult,
    TranscriptionService,
};
use crate::transcription_actor::{StreamingProviderKind, TranscriptionActor};
use crate::ui_events::{UiEvent, UiEventMailbox, UiEventStatus};
use crate::{data_dir, export, history, insertion, rewrite, RuntimeState};

pub type WorkflowResult<T> = Result<T, WorkflowError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPhase {
    Idle,
    Recording,
    Transcribing,
    Transcribed,
    Rewriting,
    Rewritten,
    Inserting,
    Cancelled,
    Failed,
}

impl WorkflowPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Transcribed => "transcribed",
            Self::Rewriting => "rewriting",
            Self::Rewritten => "rewritten",
            Self::Inserting => "inserting",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCommandRequest {
    pub command: WorkflowCommand,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowCommand {
    Primary,
    RewriteLast,
    InsertLast,
    CopyLast,
    Cancel,
}

#[derive(Debug, Clone)]
pub enum WorkflowTaskRequest {
    StopRecordTranscribe {
        task_id: String,
        recording_session_id: String,
    },
    Rewrite {
        task_id: String,
        pending_context: Option<ContextSnapshot>,
        req: RewriteTextRequest,
    },
    Insert {
        task_id: String,
        req: InsertTextRequest,
    },
}

#[derive(Debug, Clone)]
pub struct WorkflowCommandOutcome {
    pub view: WorkflowView,
    pub task: Option<WorkflowTaskRequest>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowView {
    pub phase: String,
    pub task_id: Option<String>,
    pub recording_session_id: Option<String>,
    pub last_transcript_id: Option<String>,
    pub last_asr_text: String,
    pub last_text: String,
    pub last_created_at_ms: Option<i64>,
    pub diagnostic_code: Option<String>,
    pub diagnostic_line: String,
    pub primary_label: String,
    pub primary_disabled: bool,
    pub can_rewrite: bool,
    pub can_insert: bool,
    pub can_copy: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowApplyEventRequest {
    pub event_id: String,
    pub kind: String,
    pub task_id: Option<String>,
    pub status: Option<String>,
    pub message: String,
    pub error_code: Option<String>,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowError {
    pub code: String,
    pub message: String,
}

impl WorkflowError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn render(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }

    fn from_port(err: PortError) -> Self {
        Self::new(&err.code, err.message)
    }

    pub(crate) fn from_message(code: &str, message: impl Into<String>) -> Self {
        Self::new(code, message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSession {
    pub session_id: String,
    pub recording_session_id: String,
    pub streaming_transcription: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WorkflowSnapshot {
    pub phase: WorkflowPhase,
    pub session: Option<WorkflowSession>,
    pub transcription: Option<TranscriptionResult>,
    pub rewrite: Option<RewriteResult>,
    pub last_created_at_ms: Option<i64>,
    pub last_error: Option<WorkflowError>,
}

#[derive(Debug, Clone)]
struct WorkflowState {
    phase: WorkflowPhase,
    session: Option<WorkflowSession>,
    transcription: Option<TranscriptionResult>,
    rewrite: Option<RewriteResult>,
    last_created_at_ms: Option<i64>,
    pending_contexts: HashMap<String, PendingWorkflowContext>,
    insert_previous_phase: Option<WorkflowPhase>,
    applied_event_views: HashMap<String, WorkflowView>,
    last_error: Option<WorkflowError>,
}

#[derive(Debug, Clone)]
struct PendingWorkflowContext {
    created_at_ms: i64,
    snapshot: ContextSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowActionText {
    transcript_id: String,
    asr_text: String,
    final_text: String,
    created_at_ms: Option<i64>,
}

impl WorkflowState {
    fn idle() -> Self {
        Self {
            phase: WorkflowPhase::Idle,
            session: None,
            transcription: None,
            rewrite: None,
            last_created_at_ms: None,
            pending_contexts: HashMap::new(),
            insert_previous_phase: None,
            applied_event_views: HashMap::new(),
            last_error: None,
        }
    }

    fn snapshot(&self) -> WorkflowSnapshot {
        WorkflowSnapshot {
            phase: self.phase,
            session: self.session.clone(),
            transcription: self.transcription.clone(),
            rewrite: self.rewrite.clone(),
            last_created_at_ms: self.last_created_at_ms,
            last_error: self.last_error.clone(),
        }
    }
}

pub struct VoiceWorkflow {
    state: Mutex<WorkflowState>,
}

impl VoiceWorkflow {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(WorkflowState::idle()),
        }
    }

    pub fn phase(&self) -> WorkflowPhase {
        self.state.lock().unwrap().phase
    }

    pub fn snapshot(&self) -> WorkflowSnapshot {
        self.state.lock().unwrap().snapshot()
    }

    pub fn snapshot_view(&self) -> WorkflowResult<WorkflowView> {
        self.hydrate_latest_history()?;
        Ok(self.view())
    }

    pub fn apply_event(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowApplyEventRequest,
    ) -> WorkflowResult<WorkflowView> {
        let view = self.apply_event_to_state(req)?;
        self.emit_state(mailbox);
        Ok(view)
    }

    fn apply_event_to_state(&self, req: WorkflowApplyEventRequest) -> WorkflowResult<WorkflowView> {
        let event_id = req.event_id.trim().to_string();
        if event_id.is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_EVENT_ID_MISSING",
                "event_id is required",
            ));
        }
        if let Some(view) = self.applied_event_view(&event_id) {
            return Ok(view);
        }

        let status = req.status.as_deref().unwrap_or("").to_string();
        match req.kind.as_str() {
            "transcription.completed" => {
                ensure_event_status(&status, "completed")?;
                let result: TranscriptionResult = decode_event_payload(req.payload)?;
                self.ensure_event_task(req.task_id.as_deref(), &result.transcript_id)?;
                self.complete_transcription(result)?;
            }
            "rewrite.completed" => {
                ensure_event_status(&status, "completed")?;
                let result: RewriteResult = decode_event_payload(req.payload)?;
                self.ensure_event_task(req.task_id.as_deref(), &result.transcript_id)?;
                self.complete_rewrite(result)?;
            }
            "insertion.completed" => {
                ensure_event_status(&status, "completed")?;
                let task_id = req.task_id.as_deref().ok_or_else(|| {
                    WorkflowError::new("E_WORKFLOW_EVENT_TASK_MISSING", "task_id is required")
                })?;
                self.ensure_active_task(task_id)?;
                self.complete_insert()?;
            }
            "workflow.task.failed" => {
                ensure_event_status(&status, "failed")?;
                let task_id = req.task_id.as_deref().ok_or_else(|| {
                    WorkflowError::new("E_WORKFLOW_EVENT_TASK_MISSING", "task_id is required")
                })?;
                self.ensure_active_task(task_id)?;
                let code = req
                    .error_code
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("E_WORKFLOW_TASK_FAILED");
                let err = WorkflowError::new(code, req.message);
                self.mark_failed(err);
            }
            "workflow.task.cancelled" => {
                ensure_event_status(&status, "cancelled")?;
                let task_id = req.task_id.as_deref().ok_or_else(|| {
                    WorkflowError::new("E_WORKFLOW_EVENT_TASK_MISSING", "task_id is required")
                })?;
                if self.phase() != WorkflowPhase::Cancelled {
                    self.ensure_active_task(task_id)?;
                    self.mark_cancelled();
                }
            }
            _ => {
                return Err(WorkflowError::new(
                    "E_WORKFLOW_EVENT_KIND_UNSUPPORTED",
                    format!("unsupported workflow event kind: {}", req.kind),
                ));
            }
        }

        let view = self.view();
        self.remember_applied_event(event_id, view.clone());
        Ok(view)
    }

    pub async fn run_command(
        &self,
        runtime: &RuntimeState,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
        record_input_cache: &RecordInputCacheState,
        task_state: &TaskManager,
        req: WorkflowCommandRequest,
    ) -> WorkflowResult<WorkflowCommandOutcome> {
        self.hydrate_latest_history()?;
        let result = match req.command {
            WorkflowCommand::Primary => {
                self.run_primary(
                    runtime,
                    audio,
                    transcriber,
                    streaming_actor,
                    mailbox,
                    record_input_cache,
                    normalize_optional_task_id(req.task_id)?,
                )
                .await
            }
            WorkflowCommand::RewriteLast => self.run_rewrite_last(mailbox, task_state).await,
            WorkflowCommand::InsertLast => self.run_insert_last(mailbox).await,
            WorkflowCommand::CopyLast => self.run_copy_last().map(|()| None),
            WorkflowCommand::Cancel => {
                self.run_cancel(audio, transcriber, streaming_actor, mailbox)
            }
        };

        match result {
            Ok(task) => {
                let view = self.view();
                self.emit_state(mailbox);
                Ok(WorkflowCommandOutcome { view, task })
            }
            Err(err) => {
                self.remember_error(err.clone());
                self.emit_state(mailbox);
                Err(err)
            }
        }
    }

    pub fn has_active_task(&self) -> bool {
        matches!(
            self.phase(),
            WorkflowPhase::Recording
                | WorkflowPhase::Transcribing
                | WorkflowPhase::Rewriting
                | WorkflowPhase::Inserting
        )
    }

    pub fn active_task_id_best_effort(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .session
            .as_ref()
            .map(|session| session.session_id.clone())
    }

    async fn run_primary(
        &self,
        runtime: &RuntimeState,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
        record_input_cache: &RecordInputCacheState,
        task_id: Option<String>,
    ) -> WorkflowResult<Option<WorkflowTaskRequest>> {
        let snapshot = self.snapshot();
        match snapshot.phase {
            WorkflowPhase::Idle
            | WorkflowPhase::Transcribed
            | WorkflowPhase::Rewritten
            | WorkflowPhase::Cancelled
            | WorkflowPhase::Failed => {
                self.start_record_transcribe(
                    runtime,
                    audio,
                    streaming_actor,
                    mailbox,
                    record_input_cache,
                    task_id,
                )?;
                Ok(None)
            }
            WorkflowPhase::Recording => {
                let session = snapshot.session.as_ref().ok_or_else(|| {
                    WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "recording session missing")
                })?;
                let recording_session_id = (!session.recording_session_id.trim().is_empty())
                    .then_some(session.recording_session_id.as_str())
                    .ok_or_else(|| {
                        WorkflowError::new(
                            "E_WORKFLOW_SESSION_MISSING",
                            "recording session missing",
                        )
                    })?;
                if session.streaming_transcription {
                    self.stop_streaming_record_transcribe(audio, mailbox, recording_session_id)?;
                    Ok(None)
                } else {
                    self.prepare_stop_record_transcribe(recording_session_id)
                        .map(Some)
                }
            }
            WorkflowPhase::Transcribing => {
                self.run_cancel(audio, transcriber, streaming_actor, mailbox)
            }
            WorkflowPhase::Rewriting | WorkflowPhase::Inserting => Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            )),
        }
    }

    async fn run_rewrite_last(
        &self,
        _mailbox: &UiEventMailbox,
        _task_state: &TaskManager,
    ) -> WorkflowResult<Option<WorkflowTaskRequest>> {
        let last = self.last_asr_for_action()?;
        let transcript_id = last.transcript_id;
        self.begin_rewrite(&transcript_id)?;
        let pending_context = self.take_pending_context(&transcript_id);
        Ok(Some(WorkflowTaskRequest::Rewrite {
            task_id: transcript_id.clone(),
            pending_context,
            req: RewriteTextRequest {
                transcript_id,
                text: last.asr_text,
                template_id: None,
            },
        }))
    }

    async fn run_insert_last(
        &self,
        _mailbox: &UiEventMailbox,
    ) -> WorkflowResult<Option<WorkflowTaskRequest>> {
        let last = self.last_text_for_action()?;
        let task_id = last.transcript_id;
        self.begin_insert()?;
        Ok(Some(WorkflowTaskRequest::Insert {
            task_id: task_id.clone(),
            req: InsertTextRequest {
                transcript_id: Some(task_id),
                text: last.final_text,
            },
        }))
    }

    fn run_copy_last(&self) -> WorkflowResult<()> {
        let last = self.last_text_for_action()?;
        export::copy_text_to_clipboard(&last.final_text)
            .map_err(|err| WorkflowError::new(&err.code, err.message))
    }

    fn run_cancel(
        &self,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
    ) -> WorkflowResult<Option<WorkflowTaskRequest>> {
        let snapshot = self.snapshot();
        let session_id = snapshot
            .session
            .as_ref()
            .map(|session| session.recording_session_id.clone());
        let transcript_id = snapshot
            .session
            .as_ref()
            .map(|session| session.session_id.clone());
        self.cancel_record_transcribe(
            audio,
            transcriber,
            streaming_actor,
            mailbox,
            session_id,
            transcript_id,
        )
        .map(|()| None)
    }

    pub fn start_record_transcribe(
        &self,
        runtime: &RuntimeState,
        audio: &RecordingRegistry,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
        record_input_cache: &RecordInputCacheState,
        task_id: Option<String>,
    ) -> WorkflowResult<String> {
        ensure_toolchain_ready(runtime)?;
        let transcript_id = task_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.reserve_recording(&transcript_id)?;
        mailbox.send(UiEvent::stage(
            &transcript_id,
            "Record",
            UiEventStatus::Started,
            "recording",
        ));

        let streaming_config = match streaming_actor.session_config_for_current_settings() {
            Ok(v) => v,
            Err(e) => {
                let workflow_err =
                    WorkflowError::new("E_STREAMING_TRANSCRIBE_CONFIG", e.to_string());
                self.mark_failed(workflow_err.clone());
                return Err(workflow_err);
            }
        };
        let streaming_enabled = streaming_config.provider != StreamingProviderKind::Remote;
        if streaming_enabled {
            if let Err(e) = streaming_actor.start_session(&transcript_id, streaming_config.clone())
            {
                let workflow_err =
                    WorkflowError::new("E_STREAMING_TRANSCRIBE_START", e.to_string());
                self.mark_failed(workflow_err.clone());
                return Err(workflow_err);
            }
        }

        match audio.start_recording(
            mailbox,
            streaming_enabled.then_some(streaming_actor),
            streaming_enabled.then_some(streaming_config),
            record_input_cache,
            Some(transcript_id.clone()),
        ) {
            Ok(recording_session_id) => {
                self.attach_recording_session(
                    &transcript_id,
                    &recording_session_id,
                    streaming_enabled,
                )?;
                self.emit_state(mailbox);
                Ok(recording_session_id)
            }
            Err(err) => {
                let _ = streaming_actor.cancel_session(&transcript_id);
                let workflow_err = WorkflowError::new(&err.code, err.message);
                self.mark_failed(workflow_err.clone());
                mailbox.send(UiEvent::stage_with_elapsed(
                    &transcript_id,
                    "Record",
                    UiEventStatus::Failed,
                    workflow_err.message.clone(),
                    None,
                    Some(workflow_err.code.clone()),
                ));
                Err(workflow_err)
            }
        }
    }

    pub fn prepare_stop_record_transcribe(
        &self,
        recording_session_id: &str,
    ) -> WorkflowResult<WorkflowTaskRequest> {
        let session = self.begin_transcribing(recording_session_id)?;
        Ok(WorkflowTaskRequest::StopRecordTranscribe {
            task_id: session.session_id,
            recording_session_id: recording_session_id.to_string(),
        })
    }

    pub fn stop_streaming_record_transcribe(
        &self,
        audio: &RecordingRegistry,
        mailbox: &UiEventMailbox,
        recording_session_id: &str,
    ) -> WorkflowResult<()> {
        let session = self.begin_transcribing(recording_session_id)?;
        self.emit_state(mailbox);
        let asset = match audio.stop_recording(recording_session_id) {
            Ok(asset) => asset,
            Err(err) => {
                let workflow_err = WorkflowError::new(&err.code, err.message);
                self.mark_failed(workflow_err.clone());
                mailbox.send(UiEvent::stage_with_elapsed(
                    session.session_id,
                    "Record",
                    UiEventStatus::Failed,
                    workflow_err.message.clone(),
                    None,
                    Some(workflow_err.code.clone()),
                ));
                return Err(workflow_err);
            }
        };
        let consumed = audio.take_asset(&asset.asset_id).unwrap_or(asset);
        mailbox.send(UiEvent::stage_with_elapsed(
            &session.session_id,
            "Record",
            UiEventStatus::Completed,
            "ok",
            Some(consumed.record_elapsed_ms),
            None,
        ));
        Ok(())
    }

    pub async fn stop_record_transcribe(
        &self,
        runtime: &RuntimeState,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        mailbox: &UiEventMailbox,
        recording_session_id: &str,
    ) -> WorkflowResult<TranscriptionResult> {
        let session = self.begin_transcribing(recording_session_id)?;
        self.emit_state(mailbox);
        let asset = match audio.stop_recording(recording_session_id) {
            Ok(asset) => asset,
            Err(err) => {
                let workflow_err = WorkflowError::new(&err.code, err.message);
                self.mark_failed(workflow_err.clone());
                mailbox.send(UiEvent::stage_with_elapsed(
                    session.session_id,
                    "Record",
                    UiEventStatus::Failed,
                    workflow_err.message.clone(),
                    None,
                    Some(workflow_err.code.clone()),
                ));
                return Err(workflow_err);
            }
        };
        let consumed = audio.take_asset(&asset.asset_id).unwrap_or(asset);
        mailbox.send(UiEvent::stage_with_elapsed(
            &session.session_id,
            "Record",
            UiEventStatus::Completed,
            "ok",
            Some(consumed.record_elapsed_ms),
            None,
        ));
        if let Err(err) = ensure_runtime_ready(runtime) {
            let _ = std::fs::remove_file(&consumed.output_path);
            self.mark_failed(err.clone());
            mailbox.send(UiEvent::stage_with_elapsed(
                session.session_id,
                "Transcribe",
                UiEventStatus::Failed,
                err.message.clone(),
                None,
                Some(err.code.clone()),
            ));
            return Err(err);
        }

        mailbox.send(UiEvent::stage(
            &session.session_id,
            "Transcribe",
            UiEventStatus::Started,
            "asr",
        ));
        let result = match transcriber
            .transcribe_audio(TranscriptionInput {
                task_id: consumed.task_id,
                input_path: consumed.output_path,
                record_elapsed_ms: consumed.record_elapsed_ms,
                record_label: "Record (backend)".to_string(),
            })
            .await
        {
            Ok(result) => result,
            Err(err) => {
                let workflow_err = WorkflowError::from_port(err);
                self.mark_failed(workflow_err.clone());
                mailbox.send(UiEvent::stage_with_elapsed(
                    &session.session_id,
                    "Transcribe",
                    UiEventStatus::Failed,
                    workflow_err.message.clone(),
                    None,
                    Some(workflow_err.code.clone()),
                ));
                return Err(workflow_err);
            }
        };
        self.complete_transcription(result.clone())?;
        self.emit_state(mailbox);
        mailbox.send(UiEvent::stage_with_elapsed(
            &result.transcript_id,
            "Transcribe",
            UiEventStatus::Completed,
            "ok",
            Some(result.metrics.asr_ms),
            None,
        ));
        mailbox.send(UiEvent::completed(
            &result.transcript_id,
            "transcription.completed",
            "transcription completed",
            serde_json::to_value(&result).unwrap_or_default(),
        ));
        Ok(result)
    }

    pub fn cancel_record_transcribe(
        &self,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
        session_id: Option<String>,
        transcript_id: Option<String>,
    ) -> WorkflowResult<()> {
        let snapshot = self.snapshot();
        match snapshot.phase {
            WorkflowPhase::Recording => {
                let expected = session_id.as_deref();
                self.cancel_recording_state(expected)?;
                self.emit_state(mailbox);
                audio
                    .abort_recording(session_id)
                    .map_err(|err| WorkflowError::new(&err.code, err.message))?;
                if let Some(session) = snapshot.session {
                    let _ = streaming_actor.cancel_session(&session.session_id);
                    mailbox.send(UiEvent::stage(
                        session.session_id,
                        "Record",
                        UiEventStatus::Cancelled,
                        "cancelled",
                    ));
                }
                Ok(())
            }
            WorkflowPhase::Transcribing => {
                let expected = transcript_id.as_deref();
                if let Some(expected) = expected {
                    if let Some(session) = snapshot.session.as_ref() {
                        if !expected.trim().is_empty() && expected != session.session_id {
                            return Err(WorkflowError::new(
                                "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                                "transcript id mismatch",
                            ));
                        }
                    }
                }
                transcriber
                    .cancel(expected)
                    .map_err(WorkflowError::from_port)?;
                if let Some(session) = snapshot.session.as_ref() {
                    let _ = streaming_actor.cancel_session(&session.session_id);
                }
                self.mark_cancelled();
                self.emit_state(mailbox);
                if let Some(session) = snapshot.session {
                    mailbox.send(UiEvent::stage(
                        session.session_id,
                        "Transcribe",
                        UiEventStatus::Cancelled,
                        "cancelled",
                    ));
                }
                Ok(())
            }
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed => {
                if let Some(transcript_id) = transcript_id {
                    self.abort_pending_task(&transcript_id);
                }
                Ok(())
            }
            _ => Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow cannot cancel the current phase",
            )),
        }
    }

    pub async fn transcribe_fixture(
        &self,
        runtime: &RuntimeState,
        transcriber: &TranscriptionService,
        mailbox: &UiEventMailbox,
        req: TranscribeFixtureRequest,
    ) -> WorkflowResult<TranscriptionResult> {
        ensure_runtime_ready(runtime)?;
        let task_id = req
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.reserve_transcribing(&task_id)?;
        self.emit_state(mailbox);
        mailbox.send(UiEvent::stage(
            &task_id,
            "Transcribe",
            UiEventStatus::Started,
            "fixture",
        ));
        let result = match transcriber
            .transcribe_fixture(TranscribeFixtureRequest {
                fixture_name: req.fixture_name,
                task_id: Some(task_id.clone()),
            })
            .await
        {
            Ok(result) => result,
            Err(err) => {
                let workflow_err = WorkflowError::from_port(err);
                self.mark_failed(workflow_err.clone());
                mailbox.send(UiEvent::stage_with_elapsed(
                    &task_id,
                    "Transcribe",
                    UiEventStatus::Failed,
                    workflow_err.message.clone(),
                    None,
                    Some(workflow_err.code.clone()),
                ));
                return Err(workflow_err);
            }
        };
        self.complete_transcription(result.clone())?;
        self.emit_state(mailbox);
        mailbox.send(UiEvent::stage_with_elapsed(
            &result.transcript_id,
            "Transcribe",
            UiEventStatus::Completed,
            "ok",
            Some(result.metrics.asr_ms),
            None,
        ));
        Ok(result)
    }

    pub async fn rewrite_text(
        &self,
        mailbox: &UiEventMailbox,
        task_state: &TaskManager,
        req: RewriteTextRequest,
    ) -> WorkflowResult<RewriteResult> {
        let transcript_id = req.transcript_id.trim().to_string();
        if transcript_id.is_empty() {
            return Err(WorkflowError::new(
                "E_REWRITE_TRANSCRIPT_ID_MISSING",
                "transcript_id is required",
            ));
        }
        self.begin_rewrite(&transcript_id)?;
        self.emit_state(mailbox);
        let pending_context = self.take_pending_context(&transcript_id);
        mailbox.send(UiEvent::stage(
            &transcript_id,
            "Rewrite",
            UiEventStatus::Started,
            "llm",
        ));
        let result = match rewrite::rewrite_text(task_state, pending_context, req).await {
            Ok(result) => result,
            Err(err) => {
                let workflow_err = WorkflowError::from_port(err);
                self.mark_failed(workflow_err.clone());
                mailbox.send(UiEvent::stage_with_elapsed(
                    &transcript_id,
                    "Rewrite",
                    UiEventStatus::Failed,
                    workflow_err.message.clone(),
                    None,
                    Some(workflow_err.code.clone()),
                ));
                return Err(workflow_err);
            }
        };
        self.complete_rewrite(result.clone())?;
        self.emit_state(mailbox);
        mailbox.send(UiEvent::stage_with_elapsed(
            &transcript_id,
            "Rewrite",
            UiEventStatus::Completed,
            "ok",
            Some(result.rewrite_ms),
            None,
        ));
        mailbox.send(UiEvent::completed(
            &transcript_id,
            "rewrite.completed",
            "rewrite completed",
            serde_json::to_value(&result).unwrap_or_default(),
        ));
        Ok(result)
    }

    pub async fn insert_text(
        &self,
        mailbox: &UiEventMailbox,
        req: InsertTextRequest,
    ) -> WorkflowResult<InsertResult> {
        self.begin_insert()?;
        self.emit_state(mailbox);
        let transcript_id = req.transcript_id.clone();
        let event_task_id = transcript_id.as_deref().unwrap_or("insert");
        mailbox.send(UiEvent::stage(
            event_task_id,
            "Insert",
            UiEventStatus::Started,
            "insert",
        ));
        let result = match insertion::insert_text(req).await {
            Ok(result) => result,
            Err(err) => {
                let workflow_err = WorkflowError::from_port(err);
                self.mark_failed(workflow_err.clone());
                mailbox.send(UiEvent::stage_with_elapsed(
                    event_task_id,
                    "Insert",
                    UiEventStatus::Failed,
                    workflow_err.message.clone(),
                    None,
                    Some(workflow_err.code.clone()),
                ));
                return Err(workflow_err);
            }
        };
        self.complete_insert()?;
        self.emit_state(mailbox);
        mailbox.send(UiEvent::stage(
            event_task_id,
            "Insert",
            UiEventStatus::Completed,
            "ok",
        ));
        mailbox.send(UiEvent::completed(
            event_task_id,
            "insertion.completed",
            "insertion completed",
            serde_json::to_value(&result).unwrap_or_default(),
        ));
        Ok(result)
    }

    pub fn abort_pending_task(&self, task_id: &str) -> bool {
        self.take_pending_context(task_id).is_some()
    }

    pub fn open_hotkey_task(
        &self,
        task_state: &TaskManager,
        data_dir: &Path,
        context_cfg: &context_capture::ContextConfig,
        capture_required: bool,
    ) -> WorkflowResult<String> {
        self.cleanup_orphan_pending_contexts(60_000);
        if self.has_active_task() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        let task_id = uuid::Uuid::new_v4().to_string();
        if capture_required {
            let snapshot = task_state
                .capture_hotkey_context(data_dir, context_cfg)
                .map_err(|e| WorkflowError::from_message("E_HOTKEY_TASK_OPEN", e.to_string()))?;
            self.store_pending_context(task_id.clone(), snapshot);
        }
        Ok(task_id)
    }

    fn emit_state(&self, mailbox: &UiEventMailbox) {
        mailbox.send(UiEvent::workflow_state(self.view()));
    }

    fn view(&self) -> WorkflowView {
        let snapshot = self.snapshot();
        let phase = snapshot.phase;
        let task_id = snapshot
            .session
            .as_ref()
            .map(|session| session.session_id.clone());
        let recording_session_id = snapshot.session.as_ref().and_then(|session| {
            if session.recording_session_id.trim().is_empty() {
                None
            } else {
                Some(session.recording_session_id.clone())
            }
        });
        let last = last_result_from_snapshot(&snapshot);
        let diagnostic_code = snapshot.last_error.as_ref().map(|err| err.code.clone());
        let diagnostic_line = snapshot
            .last_error
            .as_ref()
            .map(|err| format!("{}: {}", err.code, err.message))
            .unwrap_or_default();
        let active = matches!(
            phase,
            WorkflowPhase::Recording
                | WorkflowPhase::Transcribing
                | WorkflowPhase::Rewriting
                | WorkflowPhase::Inserting
        );
        let has_asr = last
            .as_ref()
            .map(|result| !result.asr_text.trim().is_empty())
            .unwrap_or(false);
        let has_text = last
            .as_ref()
            .map(|result| !result.final_text.trim().is_empty())
            .unwrap_or(false);

        WorkflowView {
            phase: phase.as_str().to_string(),
            task_id,
            recording_session_id,
            last_transcript_id: last.as_ref().map(|result| result.transcript_id.clone()),
            last_asr_text: last
                .as_ref()
                .map(|result| result.asr_text.clone())
                .unwrap_or_default(),
            last_text: last
                .as_ref()
                .map(|result| result.final_text.clone())
                .unwrap_or_default(),
            last_created_at_ms: last.as_ref().and_then(|result| result.created_at_ms),
            diagnostic_code,
            diagnostic_line,
            primary_label: primary_label(phase).to_string(),
            primary_disabled: matches!(phase, WorkflowPhase::Rewriting | WorkflowPhase::Inserting),
            can_rewrite: has_asr && !active,
            can_insert: has_text && !active,
            can_copy: has_text,
        }
    }

    fn hydrate_latest_history(&self) -> WorkflowResult<()> {
        let should_hydrate = {
            let state = self.state.lock().unwrap();
            matches!(
                state.phase,
                WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed
            ) && state.transcription.is_none()
                && state.rewrite.is_none()
        };
        if !should_hydrate {
            return Ok(());
        }

        let dir = data_dir::data_dir()
            .map_err(|e| WorkflowError::from_message("E_DATA_DIR", e.to_string()))?;
        let rows = history::list(&dir.join("history.sqlite3"), 1, None)
            .map_err(|e| WorkflowError::from_message("E_HISTORY_LIST", e.to_string()))?;
        let Some(item) = rows.into_iter().next() else {
            return Ok(());
        };

        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed
        ) || state.transcription.is_some()
            || state.rewrite.is_some()
        {
            return Ok(());
        }
        state.phase = WorkflowPhase::Transcribed;
        state.session = Some(WorkflowSession {
            session_id: item.task_id.clone(),
            recording_session_id: String::new(),
            streaming_transcription: false,
        });
        let created_at_ms = item.created_at_ms;
        state.transcription = Some(history_item_to_transcription_result(item));
        state.rewrite = None;
        state.last_created_at_ms = Some(created_at_ms);
        state.last_error = None;
        Ok(())
    }

    fn last_text_for_action(&self) -> WorkflowResult<WorkflowActionText> {
        self.hydrate_latest_history()?;
        let snapshot = self.snapshot();
        let last = last_result_from_snapshot(&snapshot).ok_or_else(|| {
            WorkflowError::new("E_WORKFLOW_LAST_RESULT_MISSING", "last result is missing")
        })?;
        if last.final_text.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_LAST_TEXT_MISSING",
                "last text is missing",
            ));
        }
        Ok(last)
    }

    fn last_asr_for_action(&self) -> WorkflowResult<WorkflowActionText> {
        self.hydrate_latest_history()?;
        let snapshot = self.snapshot();
        let last = last_result_from_snapshot(&snapshot).ok_or_else(|| {
            WorkflowError::new("E_WORKFLOW_LAST_RESULT_MISSING", "last result is missing")
        })?;
        if last.asr_text.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_LAST_ASR_TEXT_MISSING",
                "last ASR text is missing",
            ));
        }
        Ok(last)
    }

    fn remember_error(&self, err: WorkflowError) {
        let mut state = self.state.lock().unwrap();
        state.last_error = Some(err);
    }

    fn applied_event_view(&self, event_id: &str) -> Option<WorkflowView> {
        self.state
            .lock()
            .unwrap()
            .applied_event_views
            .get(event_id)
            .cloned()
    }

    fn remember_applied_event(&self, event_id: String, view: WorkflowView) {
        let mut state = self.state.lock().unwrap();
        state.applied_event_views.insert(event_id, view);
    }

    fn ensure_event_task(
        &self,
        event_task_id: Option<&str>,
        result_task_id: &str,
    ) -> WorkflowResult<()> {
        let event_task_id = event_task_id.ok_or_else(|| {
            WorkflowError::new("E_WORKFLOW_EVENT_TASK_MISSING", "task_id is required")
        })?;
        if event_task_id != result_task_id {
            return Err(WorkflowError::new(
                "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                "event task does not match result task",
            ));
        }
        self.ensure_active_task(event_task_id)
    }

    fn ensure_active_task(&self, task_id: &str) -> WorkflowResult<()> {
        let snapshot = self.snapshot();
        let active = snapshot
            .session
            .as_ref()
            .map(|session| session.session_id.as_str())
            .ok_or_else(|| WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing"))?;
        if !task_id.trim().is_empty() && active != task_id {
            return Err(WorkflowError::new(
                "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                "event task does not match active workflow task",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn begin_recording(
        &self,
        session_id: impl Into<String>,
        recording_session_id: impl Into<String>,
    ) -> WorkflowResult<WorkflowSession> {
        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Idle
                | WorkflowPhase::Transcribed
                | WorkflowPhase::Rewritten
                | WorkflowPhase::Cancelled
                | WorkflowPhase::Failed
        ) {
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        let session = WorkflowSession {
            session_id: session_id.into(),
            recording_session_id: recording_session_id.into(),
            streaming_transcription: true,
        };
        state.phase = WorkflowPhase::Recording;
        state.session = Some(session.clone());
        state.transcription = None;
        state.rewrite = None;
        state.last_created_at_ms = None;
        state.last_error = None;
        Ok(session)
    }

    fn reserve_recording(&self, transcript_id: &str) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Idle
                | WorkflowPhase::Transcribed
                | WorkflowPhase::Rewritten
                | WorkflowPhase::Cancelled
                | WorkflowPhase::Failed
        ) {
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        state.phase = WorkflowPhase::Recording;
        state.session = Some(WorkflowSession {
            session_id: transcript_id.to_string(),
            recording_session_id: String::new(),
            streaming_transcription: true,
        });
        state.transcription = None;
        state.rewrite = None;
        state.last_created_at_ms = None;
        state.insert_previous_phase = None;
        state.last_error = None;
        Ok(())
    }

    fn reserve_transcribing(&self, transcript_id: &str) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Idle
                | WorkflowPhase::Transcribed
                | WorkflowPhase::Rewritten
                | WorkflowPhase::Cancelled
                | WorkflowPhase::Failed
        ) {
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        state.phase = WorkflowPhase::Transcribing;
        state.session = Some(WorkflowSession {
            session_id: transcript_id.to_string(),
            recording_session_id: String::new(),
            streaming_transcription: true,
        });
        state.transcription = None;
        state.rewrite = None;
        state.last_created_at_ms = None;
        state.insert_previous_phase = None;
        state.last_error = None;
        Ok(())
    }

    fn attach_recording_session(
        &self,
        transcript_id: &str,
        recording_session_id: &str,
        streaming_transcription: bool,
    ) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if state.phase != WorkflowPhase::Recording {
            return Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow is not recording",
            ));
        }
        let session = state
            .session
            .as_mut()
            .ok_or_else(|| WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing"))?;
        if session.session_id != transcript_id {
            return Err(WorkflowError::new(
                "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                "transcript id mismatch",
            ));
        }
        session.recording_session_id = recording_session_id.to_string();
        session.streaming_transcription = streaming_transcription;
        Ok(())
    }

    fn begin_transcribing(&self, recording_session_id: &str) -> WorkflowResult<WorkflowSession> {
        let mut state = self.state.lock().unwrap();
        if state.phase != WorkflowPhase::Recording {
            return Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow is not recording",
            ));
        }
        let session = state
            .session
            .clone()
            .ok_or_else(|| WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing"))?;
        if session.recording_session_id != recording_session_id {
            return Err(WorkflowError::new(
                "E_WORKFLOW_SESSION_MISMATCH",
                "recording session mismatch",
            ));
        }
        state.phase = WorkflowPhase::Transcribing;
        Ok(session)
    }

    fn complete_transcription(&self, result: TranscriptionResult) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if state.phase != WorkflowPhase::Transcribing {
            return Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow is not transcribing",
            ));
        }
        let session_id = state
            .session
            .as_ref()
            .map(|session| session.session_id.as_str())
            .ok_or_else(|| WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing"))?;
        if session_id != result.transcript_id {
            return Err(WorkflowError::new(
                "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                "transcription result does not match active session",
            ));
        }
        state.phase = WorkflowPhase::Transcribed;
        state.transcription = Some(result);
        state.last_created_at_ms = Some(now_ms());
        state.insert_previous_phase = None;
        state.last_error = None;
        Ok(())
    }

    #[cfg(test)]
    fn open_transcribed_session(
        &self,
        transcript_id: impl Into<String>,
        asr_text: impl Into<String>,
    ) -> WorkflowResult<()> {
        let transcript_id = transcript_id.into();
        let result = TranscriptionResult::new(
            transcript_id.clone(),
            asr_text,
            crate::transcription::TranscriptionMetrics {
                rtf: 0.0,
                device_used: "test".to_string(),
                preprocess_ms: 0,
                asr_ms: 0,
            },
        );
        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Idle
                | WorkflowPhase::Transcribed
                | WorkflowPhase::Rewritten
                | WorkflowPhase::Cancelled
                | WorkflowPhase::Failed
        ) {
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        state.phase = WorkflowPhase::Transcribed;
        state.session = Some(WorkflowSession {
            session_id: transcript_id,
            recording_session_id: String::new(),
            streaming_transcription: false,
        });
        state.transcription = Some(result);
        state.rewrite = None;
        state.last_created_at_ms = Some(now_ms());
        state.last_error = None;
        Ok(())
    }

    fn begin_rewrite(&self, transcript_id: &str) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if matches!(
            state.phase,
            WorkflowPhase::Recording
                | WorkflowPhase::Transcribing
                | WorkflowPhase::Rewriting
                | WorkflowPhase::Inserting
        ) {
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        if let Some(session) = state.session.as_ref() {
            if !session.session_id.is_empty() && session.session_id != transcript_id {
                return Err(WorkflowError::new(
                    "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                    "rewrite transcript does not match workflow session",
                ));
            }
        }
        state.phase = WorkflowPhase::Rewriting;
        state.session = Some(WorkflowSession {
            session_id: transcript_id.to_string(),
            recording_session_id: state
                .session
                .as_ref()
                .map(|session| session.recording_session_id.clone())
                .unwrap_or_default(),
            streaming_transcription: state
                .session
                .as_ref()
                .map(|session| session.streaming_transcription)
                .unwrap_or(false),
        });
        state.insert_previous_phase = None;
        state.last_error = None;
        Ok(())
    }

    fn complete_rewrite(&self, result: RewriteResult) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if state.phase != WorkflowPhase::Rewriting {
            return Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow is not rewriting",
            ));
        }
        let session_id = state
            .session
            .as_ref()
            .map(|session| session.session_id.as_str())
            .ok_or_else(|| WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing"))?;
        if session_id != result.transcript_id {
            return Err(WorkflowError::new(
                "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                "rewrite result does not match workflow session",
            ));
        }
        state.phase = WorkflowPhase::Rewritten;
        state.rewrite = Some(result);
        state.insert_previous_phase = None;
        state.last_error = None;
        Ok(())
    }

    fn begin_insert(&self) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if matches!(
            state.phase,
            WorkflowPhase::Recording
                | WorkflowPhase::Transcribing
                | WorkflowPhase::Rewriting
                | WorkflowPhase::Inserting
        ) {
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        state.insert_previous_phase = Some(state.phase);
        state.phase = WorkflowPhase::Inserting;
        state.last_error = None;
        Ok(())
    }

    fn complete_insert(&self) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if state.phase != WorkflowPhase::Inserting {
            return Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow is not inserting",
            ));
        }
        let previous = state.insert_previous_phase.take().ok_or_else(|| {
            WorkflowError::new(
                "E_WORKFLOW_INSERT_PREVIOUS_MISSING",
                "insert previous phase missing",
            )
        })?;
        state.phase = previous;
        state.last_error = None;
        Ok(())
    }

    fn mark_cancelled(&self) {
        let mut state = self.state.lock().unwrap();
        state.phase = WorkflowPhase::Cancelled;
        state.insert_previous_phase = None;
        state.last_error = None;
    }

    fn mark_failed(&self, err: WorkflowError) {
        let mut state = self.state.lock().unwrap();
        state.phase = WorkflowPhase::Failed;
        state.insert_previous_phase = None;
        state.last_error = Some(err);
    }

    fn cancel_recording_state(
        &self,
        expected_recording_session_id: Option<&str>,
    ) -> WorkflowResult<()> {
        let state = self.state.lock().unwrap();
        if state.phase != WorkflowPhase::Recording {
            return Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow is not recording",
            ));
        }
        if let Some(expected) = expected_recording_session_id {
            let actual = state
                .session
                .as_ref()
                .map(|session| session.recording_session_id.as_str())
                .ok_or_else(|| {
                    WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing")
                })?;
            if !expected.trim().is_empty() && actual != expected {
                return Err(WorkflowError::new(
                    "E_WORKFLOW_SESSION_MISMATCH",
                    "recording session mismatch",
                ));
            }
        }
        drop(state);
        self.mark_cancelled();
        Ok(())
    }

    fn store_pending_context(&self, task_id: impl Into<String>, snapshot: ContextSnapshot) {
        let mut state = self.state.lock().unwrap();
        state.pending_contexts.insert(
            task_id.into(),
            PendingWorkflowContext {
                created_at_ms: now_ms(),
                snapshot,
            },
        );
    }

    fn take_pending_context(&self, task_id: &str) -> Option<ContextSnapshot> {
        let mut state = self.state.lock().unwrap();
        state
            .pending_contexts
            .remove(task_id)
            .map(|ctx| ctx.snapshot)
    }

    fn cleanup_orphan_pending_contexts(&self, max_age_ms: i64) {
        let now = now_ms();
        let mut state = self.state.lock().unwrap();
        state
            .pending_contexts
            .retain(|_, ctx| now.saturating_sub(ctx.created_at_ms) <= max_age_ms);
    }

    #[cfg(test)]
    fn open_recording_for_test(
        &self,
        session_id: &str,
        recording_session_id: &str,
    ) -> WorkflowResult<WorkflowSession> {
        self.begin_recording(session_id, recording_session_id)
    }

    #[cfg(test)]
    fn begin_transcribing_for_test(
        &self,
        recording_session_id: &str,
    ) -> WorkflowResult<WorkflowSession> {
        self.begin_transcribing(recording_session_id)
    }

    #[cfg(test)]
    fn complete_transcription_for_test(&self, result: TranscriptionResult) -> WorkflowResult<()> {
        self.complete_transcription(result)
    }

    #[cfg(test)]
    fn open_transcribed_session_for_test(
        &self,
        transcript_id: &str,
        asr_text: &str,
    ) -> WorkflowResult<()> {
        self.open_transcribed_session(transcript_id, asr_text)
    }

    #[cfg(test)]
    fn begin_rewrite_for_test(&self, transcript_id: &str) -> WorkflowResult<()> {
        self.begin_rewrite(transcript_id)
    }

    #[cfg(test)]
    fn complete_rewrite_for_test(&self, result: RewriteResult) -> WorkflowResult<()> {
        self.complete_rewrite(result)
    }

    #[cfg(test)]
    fn begin_insert_for_test(&self) -> WorkflowResult<()> {
        self.begin_insert()
    }

    #[cfg(test)]
    fn complete_insert_for_test(&self) -> WorkflowResult<()> {
        self.complete_insert()
    }

    #[cfg(test)]
    fn cancel_recording_for_test(
        &self,
        expected_recording_session_id: Option<&str>,
    ) -> WorkflowResult<()> {
        self.cancel_recording_state(expected_recording_session_id)
    }

    #[cfg(test)]
    fn store_pending_context_for_test(&self, task_id: &str, snapshot: ContextSnapshot) {
        self.store_pending_context(task_id, snapshot);
    }

    #[cfg(test)]
    fn take_pending_context_for_test(&self, task_id: &str) -> Option<ContextSnapshot> {
        self.take_pending_context(task_id)
    }
}

impl Default for VoiceWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

fn primary_label(phase: WorkflowPhase) -> &'static str {
    match phase {
        WorkflowPhase::Idle
        | WorkflowPhase::Transcribed
        | WorkflowPhase::Rewritten
        | WorkflowPhase::Cancelled
        | WorkflowPhase::Failed => "START",
        WorkflowPhase::Recording => "STOP",
        WorkflowPhase::Transcribing => "CANCEL",
        WorkflowPhase::Rewriting => "REWRITING",
        WorkflowPhase::Inserting => "INSERTING",
    }
}

fn last_result_from_snapshot(snapshot: &WorkflowSnapshot) -> Option<WorkflowActionText> {
    let transcription = snapshot.transcription.as_ref()?;
    let final_text = snapshot
        .rewrite
        .as_ref()
        .map(|result| result.final_text.clone())
        .unwrap_or_else(|| transcription.final_text.clone());
    Some(WorkflowActionText {
        transcript_id: transcription.transcript_id.clone(),
        asr_text: transcription.asr_text.clone(),
        final_text,
        created_at_ms: snapshot.last_created_at_ms,
    })
}

fn history_item_to_transcription_result(item: history::HistoryItem) -> TranscriptionResult {
    let task_id = item.task_id;
    TranscriptionResult {
        transcript_id: task_id.clone(),
        asr_text: item.asr_text,
        final_text: item.final_text,
        metrics: TranscriptionMetrics {
            rtf: item.rtf,
            device_used: item.device_used,
            preprocess_ms: item.preprocess_ms.max(0) as u128,
            asr_ms: item.asr_ms.max(0) as u128,
        },
        history_id: task_id,
    }
}

fn normalize_optional_task_id(task_id: Option<String>) -> WorkflowResult<Option<String>> {
    let raw = match task_id {
        Some(v) => v.trim().to_string(),
        None => return Ok(None),
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let parsed = uuid::Uuid::parse_str(&raw)
        .map_err(|e| WorkflowError::new("E_TASK_ID_INVALID", format!("invalid task_id ({e})")))?;
    Ok(Some(parsed.to_string()))
}

fn decode_event_payload<T>(payload: Option<serde_json::Value>) -> WorkflowResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let payload = payload.ok_or_else(|| {
        WorkflowError::new(
            "E_WORKFLOW_EVENT_PAYLOAD_MISSING",
            "event payload is required",
        )
    })?;
    serde_json::from_value(payload).map_err(|e| {
        WorkflowError::new(
            "E_WORKFLOW_EVENT_PAYLOAD_INVALID",
            format!("invalid event payload: {e}"),
        )
    })
}

fn ensure_event_status(actual: &str, expected: &str) -> WorkflowResult<()> {
    if actual.is_empty() || actual == expected {
        return Ok(());
    }
    Err(WorkflowError::new(
        "E_WORKFLOW_EVENT_STATUS_INVALID",
        format!("expected event status {expected}, got {actual}"),
    ))
}

fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => dur.as_millis() as i64,
        Err(_) => 0,
    }
}

fn ensure_toolchain_ready(runtime: &RuntimeState) -> WorkflowResult<()> {
    let tc = runtime.get_toolchain();
    if !tc.ready {
        return Err(WorkflowError::from_message(
            "E_TOOLCHAIN_NOT_READY",
            tc.message
                .unwrap_or_else(|| "toolchain is not ready".to_string()),
        ));
    }
    Ok(())
}

fn ensure_runtime_ready(runtime: &RuntimeState) -> WorkflowResult<()> {
    ensure_toolchain_ready(runtime)?;
    let py = runtime.get_python();
    if !py.ready {
        return Err(WorkflowError::from_message(
            "E_PYTHON_NOT_READY",
            py.message
                .unwrap_or_else(|| "python runtime is not ready".to_string()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_starts_idle() {
        let workflow = VoiceWorkflow::new();

        assert_eq!(workflow.phase(), WorkflowPhase::Idle);
    }

    #[test]
    fn initial_workflow_view_allows_primary_start() {
        let workflow = VoiceWorkflow::new();

        let view = workflow.view();

        assert_eq!(view.phase, "idle");
        assert_eq!(view.primary_label, "START");
        assert!(!view.primary_disabled);
        assert!(!view.can_insert);
        assert!(!view.can_copy);
    }

    #[test]
    fn start_recording_moves_idle_to_recording() {
        let workflow = VoiceWorkflow::new();

        let session = workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");

        assert_eq!(session.session_id, "task-1");
        assert_eq!(workflow.phase(), WorkflowPhase::Recording);
        let view = workflow.view();
        assert_eq!(view.primary_label, "STOP");
        assert_eq!(view.task_id.as_deref(), Some("task-1"));
        assert_eq!(view.recording_session_id.as_deref(), Some("recording-1"));
    }

    #[test]
    fn duplicate_recording_start_fails_fast() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("first recording starts");

        let err = workflow
            .open_recording_for_test("task-2", "recording-2")
            .expect_err("second recording is rejected");

        assert_eq!(err.code, "E_WORKFLOW_BUSY");
        assert_eq!(workflow.phase(), WorkflowPhase::Recording);
    }

    #[test]
    fn stop_rejects_mismatched_recording_session() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");

        let err = workflow
            .begin_transcribing_for_test("recording-2")
            .expect_err("wrong recording session is rejected");

        assert_eq!(err.code, "E_WORKFLOW_SESSION_MISMATCH");
        assert_eq!(workflow.phase(), WorkflowPhase::Recording);
    }

    #[test]
    fn completed_transcription_is_saved_in_state() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let result = crate::transcription::TranscriptionResult::new(
            "task-1",
            "asr text",
            crate::transcription::TranscriptionMetrics {
                rtf: 0.4,
                device_used: "cuda".to_string(),
                preprocess_ms: 10,
                asr_ms: 20,
            },
        );
        workflow
            .complete_transcription_for_test(result.clone())
            .expect("transcription completes");

        assert_eq!(workflow.phase(), WorkflowPhase::Transcribed);
        let view = workflow.view();
        assert_eq!(view.phase, "transcribed");
        assert_eq!(view.last_transcript_id.as_deref(), Some("task-1"));
        assert_eq!(view.last_asr_text, "asr text");
        assert_eq!(view.last_text, "asr text");
        assert!(view.can_insert);
        assert!(view.can_copy);
        assert_eq!(
            workflow
                .snapshot()
                .transcription
                .as_ref()
                .map(|item| item.asr_text.as_str()),
            Some("asr text")
        );
    }

    #[test]
    fn completed_rewrite_is_saved_in_state() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");

        workflow
            .begin_rewrite_for_test("task-1")
            .expect("rewrite starts");
        workflow
            .complete_rewrite_for_test(crate::rewrite::RewriteResult {
                transcript_id: "task-1".to_string(),
                final_text: "final text".to_string(),
                rewrite_ms: 30,
                template_id: Some("template-1".to_string()),
            })
            .expect("rewrite completes");

        assert_eq!(workflow.phase(), WorkflowPhase::Rewritten);
        let view = workflow.view();
        assert_eq!(view.phase, "rewritten");
        assert_eq!(view.last_asr_text, "asr text");
        assert_eq!(view.last_text, "final text");
        assert_eq!(
            workflow
                .snapshot()
                .rewrite
                .as_ref()
                .map(|item| item.final_text.as_str()),
            Some("final text")
        );
    }

    #[test]
    fn insert_restores_previous_completion_phase() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");

        workflow.begin_insert_for_test().expect("insert starts");
        assert_eq!(workflow.phase(), WorkflowPhase::Inserting);
        workflow
            .complete_insert_for_test()
            .expect("insert completes");

        assert_eq!(workflow.phase(), WorkflowPhase::Transcribed);
    }

    #[test]
    fn rewrite_last_requires_asr_text() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "")
            .expect("transcribed");

        let err = workflow
            .last_asr_for_action()
            .expect_err("empty ASR text is rejected");

        assert_eq!(err.code, "E_WORKFLOW_LAST_ASR_TEXT_MISSING");
    }

    #[test]
    fn insert_and_copy_require_final_text() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");
        let mut result = crate::transcription::TranscriptionResult::new(
            "task-1",
            "asr text",
            crate::transcription::TranscriptionMetrics {
                rtf: 0.4,
                device_used: "cuda".to_string(),
                preprocess_ms: 10,
                asr_ms: 20,
            },
        );
        result.final_text.clear();
        workflow
            .complete_transcription_for_test(result)
            .expect("transcription completes");

        let err = workflow
            .last_text_for_action()
            .expect_err("empty final text is rejected");

        assert_eq!(err.code, "E_WORKFLOW_LAST_TEXT_MISSING");
    }

    #[test]
    fn cancel_recording_moves_to_cancelled() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");

        workflow
            .cancel_recording_for_test(Some("recording-1"))
            .expect("recording cancels");

        assert_eq!(workflow.phase(), WorkflowPhase::Cancelled);
    }

    #[test]
    fn pending_context_is_consumed_once() {
        let workflow = VoiceWorkflow::new();
        let snapshot = crate::context_pack::ContextSnapshot::default();

        workflow.store_pending_context_for_test("task-1", snapshot);

        assert!(workflow.take_pending_context_for_test("task-1").is_some());
        assert!(workflow.take_pending_context_for_test("task-1").is_none());
    }

    #[test]
    fn prepare_stop_moves_recording_to_transcribing() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");

        let task = workflow
            .prepare_stop_record_transcribe("recording-1")
            .expect("stop task is prepared");

        assert_eq!(workflow.phase(), WorkflowPhase::Transcribing);
        match task {
            WorkflowTaskRequest::StopRecordTranscribe {
                task_id,
                recording_session_id,
            } => {
                assert_eq!(task_id, "task-1");
                assert_eq!(recording_session_id, "recording-1");
            }
            _ => panic!("unexpected task"),
        }
    }

    #[test]
    fn apply_transcription_completed_event_updates_state() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let result = transcription_result("task-1", "asr text");
        let view = workflow
            .apply_event_to_state(completed_event(
                "event-1",
                "transcription.completed",
                "task-1",
                serde_json::to_value(result).expect("payload serializes"),
            ))
            .expect("event applies");

        assert_eq!(view.phase, "transcribed");
        assert_eq!(view.last_text, "asr text");
    }

    #[test]
    fn apply_rewrite_completed_event_updates_state() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");
        workflow
            .begin_rewrite_for_test("task-1")
            .expect("rewrite starts");

        let view = workflow
            .apply_event_to_state(completed_event(
                "event-1",
                "rewrite.completed",
                "task-1",
                serde_json::json!({
                    "transcriptId": "task-1",
                    "finalText": "final text",
                    "rewriteMs": 12,
                    "templateId": null,
                }),
            ))
            .expect("event applies");

        assert_eq!(view.phase, "rewritten");
        assert_eq!(view.last_text, "final text");
    }

    #[test]
    fn apply_insert_completed_event_restores_previous_phase() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");
        workflow.begin_insert_for_test().expect("insert starts");

        let view = workflow
            .apply_event_to_state(completed_event(
                "event-1",
                "insertion.completed",
                "task-1",
                serde_json::json!({
                    "copied": true,
                    "autoPasteAttempted": false,
                    "autoPasteOk": true,
                    "errorCode": null,
                    "errorMessage": null,
                }),
            ))
            .expect("event applies");

        assert_eq!(view.phase, "transcribed");
    }

    #[test]
    fn apply_failed_event_updates_diagnostic_state() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let view = workflow
            .apply_event_to_state(WorkflowApplyEventRequest {
                event_id: "event-1".to_string(),
                kind: "workflow.task.failed".to_string(),
                task_id: Some("task-1".to_string()),
                status: Some("failed".to_string()),
                message: "asr failed".to_string(),
                error_code: Some("E_ASR_FAILED".to_string()),
                payload: None,
            })
            .expect("event applies");

        assert_eq!(view.phase, "failed");
        assert_eq!(view.diagnostic_code.as_deref(), Some("E_ASR_FAILED"));
    }

    #[test]
    fn duplicate_event_id_returns_same_view() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let event = completed_event(
            "event-1",
            "transcription.completed",
            "task-1",
            serde_json::to_value(transcription_result("task-1", "asr text"))
                .expect("payload serializes"),
        );
        let first = workflow
            .apply_event_to_state(event.clone())
            .expect("first event applies");
        let second = workflow
            .apply_event_to_state(event)
            .expect("duplicate event applies");

        assert_eq!(first, second);
    }

    #[test]
    fn task_mismatch_event_fails_fast() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let err = workflow
            .apply_event_to_state(completed_event(
                "event-1",
                "transcription.completed",
                "task-2",
                serde_json::to_value(transcription_result("task-2", "asr text"))
                    .expect("payload serializes"),
            ))
            .expect_err("mismatched event is rejected");

        assert_eq!(err.code, "E_WORKFLOW_TRANSCRIPT_MISMATCH");
        assert_eq!(workflow.phase(), WorkflowPhase::Transcribing);
    }

    fn transcription_result(
        task_id: &str,
        text: &str,
    ) -> crate::transcription::TranscriptionResult {
        crate::transcription::TranscriptionResult::new(
            task_id,
            text,
            crate::transcription::TranscriptionMetrics {
                rtf: 0.4,
                device_used: "cuda".to_string(),
                preprocess_ms: 10,
                asr_ms: 20,
            },
        )
    }

    fn completed_event(
        event_id: &str,
        kind: &str,
        task_id: &str,
        payload: serde_json::Value,
    ) -> WorkflowApplyEventRequest {
        WorkflowApplyEventRequest {
            event_id: event_id.to_string(),
            kind: kind.to_string(),
            task_id: Some(task_id.to_string()),
            status: Some("completed".to_string()),
            message: "completed".to_string(),
            error_code: None,
            payload: Some(payload),
        }
    }
}
