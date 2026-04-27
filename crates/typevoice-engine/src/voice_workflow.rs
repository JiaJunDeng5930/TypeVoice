use std::{collections::HashMap, path::Path, sync::Mutex};

use crate::audio_capture::{RecordingRegistry, RecordingStopOutcome};
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAsrCompletedRequest {
    pub transcript_id: String,
    pub text: String,
    pub metrics: TranscriptionMetrics,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAsrEmptyRequest {
    pub transcript_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTaskFailedRequest {
    pub transcript_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTextCommandRequest {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRewriteCompletedRequest {
    pub transcript_id: String,
    pub text: String,
    pub rewrite_ms: u128,
    pub template_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInsertCompletedRequest {
    pub transcript_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowError {
    pub code: String,
    pub message: String,
    pub raw: Option<String>,
}

impl WorkflowError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: code.to_string(),
            raw: Some(message.clone()),
            message,
        }
    }

    pub fn render(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }

    fn from_port(err: PortError) -> Self {
        let PortError { code, message, raw } = err;
        Self {
            code,
            raw: Some(raw.unwrap_or_else(|| message.clone())),
            message,
        }
    }

    pub(crate) fn from_message(code: &str, message: impl Into<String>) -> Self {
        Self::new(code, message)
    }

    pub fn raw_message(&self) -> &str {
        self.raw.as_deref().unwrap_or(&self.message)
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
        _transcriber: &TranscriptionService,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
        record_input_cache: &RecordInputCacheState,
        task_id: Option<String>,
    ) -> WorkflowResult<Option<WorkflowTaskRequest>> {
        let snapshot = self.snapshot();
        match snapshot.phase {
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed => {
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
                if session.streaming_transcription {
                    self.stop_streaming_record_transcribe(audio, streaming_actor, mailbox)?;
                    Ok(None)
                } else {
                    self.prepare_stop_record_transcribe().map(Some)
                }
            }
            WorkflowPhase::Transcribing
            | WorkflowPhase::Transcribed
            | WorkflowPhase::Rewriting
            | WorkflowPhase::Rewritten
            | WorkflowPhase::Inserting => Err(primary_phase_error(snapshot.phase)),
        }
    }

    async fn run_rewrite_last(
        &self,
        _mailbox: &UiEventMailbox,
        _task_state: &TaskManager,
    ) -> WorkflowResult<Option<WorkflowTaskRequest>> {
        Err(rewrite_phase_error(self.phase()))
    }

    async fn run_insert_last(
        &self,
        _mailbox: &UiEventMailbox,
    ) -> WorkflowResult<Option<WorkflowTaskRequest>> {
        Err(insert_phase_error(self.phase()))
    }

    fn run_copy_last(&self) -> WorkflowResult<()> {
        let last = self.current_action_text()?;
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
        self.cancel_record_transcribe(audio, transcriber, streaming_actor, mailbox)
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
            None,
            None,
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

    pub fn prepare_stop_record_transcribe(&self) -> WorkflowResult<WorkflowTaskRequest> {
        let session = self.begin_transcribing_current()?;
        Ok(WorkflowTaskRequest::StopRecordTranscribe {
            task_id: session.session_id,
            recording_session_id: session.recording_session_id,
        })
    }

    pub fn stop_streaming_record_transcribe(
        &self,
        audio: &RecordingRegistry,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
    ) -> WorkflowResult<()> {
        let session = self.begin_transcribing_current()?;
        self.emit_state(mailbox);
        let asset = match audio.stop_recording(&session.recording_session_id) {
            Ok(RecordingStopOutcome::Completed(asset)) => asset,
            Ok(RecordingStopOutcome::Stale) => return Ok(()),
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
        if let Err(e) =
            streaming_actor.send_wav_file_and_finish(&session.session_id, &consumed.output_path)
        {
            let workflow_err = WorkflowError::new("E_STREAMING_TRANSCRIBE_SEND", e.to_string());
            self.mark_failed(workflow_err.clone());
            mailbox.send(UiEvent::stage_with_elapsed(
                session.session_id,
                "Transcribe",
                UiEventStatus::Failed,
                workflow_err.message.clone(),
                None,
                Some(workflow_err.code.clone()),
            ));
            return Err(workflow_err);
        }
        Ok(())
    }

    pub async fn stop_record_transcribe(
        &self,
        runtime: &RuntimeState,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        mailbox: &UiEventMailbox,
    ) -> WorkflowResult<Option<TranscriptionResult>> {
        let session = self.begin_transcribing_current()?;
        self.emit_state(mailbox);
        let asset = match audio.stop_recording(&session.recording_session_id) {
            Ok(RecordingStopOutcome::Completed(asset)) => asset,
            Ok(RecordingStopOutcome::Stale) => return Ok(None),
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
            Err(err) if err.code == "E_TASK_STALE" => return Ok(None),
            Err(err) if is_empty_asr_failure(&err.code, &err.message) => {
                self.complete_empty_transcription(&session.session_id)?;
                self.emit_state(mailbox);
                mailbox.send(UiEvent::stage(
                    &session.session_id,
                    "Transcribe",
                    UiEventStatus::Completed,
                    "empty",
                ));
                mailbox.send(UiEvent::transcription_empty(&session.session_id));
                return Ok(None);
            }
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
        if result.asr_text.trim().is_empty() {
            self.complete_empty_transcription(&result.transcript_id)?;
            self.emit_state(mailbox);
            mailbox.send(UiEvent::stage_with_elapsed(
                &result.transcript_id,
                "Transcribe",
                UiEventStatus::Completed,
                "empty",
                Some(result.metrics.asr_ms),
                None,
            ));
            mailbox.send(UiEvent::transcription_empty(&result.transcript_id));
            return Ok(None);
        }
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
        Ok(Some(result))
    }

    pub fn report_asr_completed(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowAsrCompletedRequest,
    ) -> WorkflowResult<WorkflowView> {
        let transcript_id = req.transcript_id.trim().to_string();
        if transcript_id.is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_ASR_TRANSCRIPT_ID_MISSING",
                "transcript_id is required",
            ));
        }
        if req.text.trim().is_empty() {
            return self.report_asr_empty(
                mailbox,
                WorkflowAsrEmptyRequest {
                    transcript_id: transcript_id.clone(),
                },
            );
        }
        self.ensure_transcribing_task(&transcript_id)?;
        let result = TranscriptionResult::new(transcript_id, req.text, req.metrics);
        self.complete_transcription(result.clone())?;
        self.persist_transcription_result(&result)?;
        let view = self.view();
        self.emit_state(mailbox);
        Ok(view)
    }

    pub fn report_asr_empty(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowAsrEmptyRequest,
    ) -> WorkflowResult<WorkflowView> {
        let transcript_id = req.transcript_id.trim().to_string();
        if transcript_id.is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_ASR_TRANSCRIPT_ID_MISSING",
                "transcript_id is required",
            ));
        }
        let snapshot = self.snapshot();
        if snapshot.phase == WorkflowPhase::Idle && snapshot.session.is_none() {
            return Ok(self.view());
        }
        self.ensure_transcribing_task(&transcript_id)?;
        self.complete_empty_transcription(&transcript_id)?;
        let view = self.view();
        self.emit_state(mailbox);
        Ok(view)
    }

    pub fn report_asr_failed(
        &self,
        audio: &RecordingRegistry,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
        req: WorkflowTaskFailedRequest,
    ) -> WorkflowResult<WorkflowView> {
        self.ensure_asr_report_task(&req.transcript_id)?;
        let code = req.code.trim();
        if code.is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_FAILURE_CODE_MISSING",
                "code is required",
            ));
        }
        if is_empty_asr_failure(code, &req.message) {
            return self.report_asr_empty(
                mailbox,
                WorkflowAsrEmptyRequest {
                    transcript_id: req.transcript_id,
                },
            );
        }
        let snapshot = self.snapshot();
        let err = WorkflowError::new(code, req.message);
        self.mark_failed(err);
        if snapshot.phase == WorkflowPhase::Recording {
            if let Some(session) = snapshot.session {
                audio
                    .abort_recording(Some(session.recording_session_id.clone()))
                    .map_err(|err| WorkflowError::new(&err.code, err.message))?;
                let _ = streaming_actor.cancel_session(&session.session_id);
            }
        }
        let view = self.view();
        self.emit_state(mailbox);
        Ok(view)
    }

    pub fn cancel_record_transcribe(
        &self,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        streaming_actor: &TranscriptionActor,
        mailbox: &UiEventMailbox,
    ) -> WorkflowResult<()> {
        let snapshot = self.snapshot();
        match snapshot.phase {
            WorkflowPhase::Recording => {
                let session = snapshot.session.clone().ok_or_else(|| {
                    WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing")
                })?;
                self.cancel_current_recording_state()?;
                self.emit_state(mailbox);
                audio
                    .abort_recording(Some(session.recording_session_id.clone()))
                    .map_err(|err| WorkflowError::new(&err.code, err.message))?;
                let _ = streaming_actor.cancel_session(&session.session_id);
                mailbox.send(UiEvent::stage(
                    session.session_id,
                    "Record",
                    UiEventStatus::Cancelled,
                    "cancelled",
                ));
                Ok(())
            }
            WorkflowPhase::Transcribing => {
                let session = snapshot.session.clone().ok_or_else(|| {
                    WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing")
                })?;
                transcriber
                    .cancel(Some(session.session_id.as_str()))
                    .map_err(WorkflowError::from_port)?;
                let _ = streaming_actor.cancel_session(&session.session_id);
                self.mark_cancelled();
                self.emit_state(mailbox);
                mailbox.send(UiEvent::stage(
                    session.session_id,
                    "Transcribe",
                    UiEventStatus::Cancelled,
                    "cancelled",
                ));
                Ok(())
            }
            WorkflowPhase::Idle
            | WorkflowPhase::Transcribed
            | WorkflowPhase::Rewritten
            | WorkflowPhase::Cancelled
            | WorkflowPhase::Failed => Err(cancel_phase_error(snapshot.phase)),
            WorkflowPhase::Rewriting | WorkflowPhase::Inserting => {
                Err(cancel_phase_error(snapshot.phase))
            }
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
        self.persist_transcription_result(&result)?;
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

    pub async fn rewrite_current_text(
        &self,
        mailbox: &UiEventMailbox,
        task_state: &TaskManager,
        req: WorkflowTextCommandRequest,
    ) -> WorkflowResult<RewriteResult> {
        self.rewrite_text(mailbox, task_state, self.current_rewrite_request(req)?)
            .await
    }

    pub fn report_rewrite_completed(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowRewriteCompletedRequest,
    ) -> WorkflowResult<WorkflowView> {
        let transcript_id = req.transcript_id.trim().to_string();
        if transcript_id.is_empty() {
            return Err(WorkflowError::new(
                "E_REWRITE_TRANSCRIPT_ID_MISSING",
                "transcript_id is required",
            ));
        }
        if req.text.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_REWRITE_EMPTY_TEXT",
                "text is required",
            ));
        }
        let result = RewriteResult {
            transcript_id,
            final_text: req.text,
            rewrite_ms: req.rewrite_ms,
            template_id: req.template_id,
        };
        self.complete_rewrite(result.clone())?;
        self.persist_rewrite_result(&result)?;
        let view = self.view();
        self.emit_state(mailbox);
        Ok(view)
    }

    pub fn report_rewrite_failed(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowTaskFailedRequest,
    ) -> WorkflowResult<WorkflowView> {
        self.ensure_rewriting_task(&req.transcript_id)?;
        let code = req.code.trim();
        if code.is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_FAILURE_CODE_MISSING",
                "code is required",
            ));
        }
        self.mark_failed(WorkflowError::new(code, req.message));
        let view = self.view();
        self.emit_state(mailbox);
        Ok(view)
    }

    pub async fn insert_text(
        &self,
        mailbox: &UiEventMailbox,
        req: InsertTextRequest,
    ) -> WorkflowResult<InsertResult> {
        self.insert_text_after_focus(mailbox, req, None).await
    }

    pub async fn insert_text_after_focus(
        &self,
        mailbox: &UiEventMailbox,
        req: InsertTextRequest,
        target_hwnd: Option<isize>,
    ) -> WorkflowResult<InsertResult> {
        let transcript_id = req
            .transcript_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                WorkflowError::new(
                    "E_INSERT_TRANSCRIPT_ID_MISSING",
                    "transcript_id is required",
                )
            })?
            .to_string();
        if req.text.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_INSERT_EMPTY_TEXT",
                "text is required",
            ));
        }
        self.begin_insert(&transcript_id)?;
        self.emit_state(mailbox);
        let event_task_id = transcript_id.as_str();
        mailbox.send(UiEvent::stage(
            event_task_id,
            "Insert",
            UiEventStatus::Started,
            "insert",
        ));
        let result = match insertion::insert_text_after_focus(req.clone(), target_hwnd).await {
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
        self.persist_inserted_text(&transcript_id, &req.text)?;
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

    pub async fn insert_current_text_after_focus(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowTextCommandRequest,
        target_hwnd: Option<isize>,
    ) -> WorkflowResult<InsertResult> {
        self.insert_text_after_focus(mailbox, self.current_insert_request(req)?, target_hwnd)
            .await
    }

    pub fn report_insert_completed(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowInsertCompletedRequest,
    ) -> WorkflowResult<WorkflowView> {
        let transcript_id = req.transcript_id.trim().to_string();
        if transcript_id.is_empty() {
            return Err(WorkflowError::new(
                "E_INSERT_TRANSCRIPT_ID_MISSING",
                "transcript_id is required",
            ));
        }
        if req.text.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_INSERT_EMPTY_TEXT",
                "text is required",
            ));
        }
        self.ensure_inserting_task(&transcript_id)?;
        self.persist_inserted_text(&transcript_id, &req.text)?;
        self.complete_insert()?;
        let view = self.view();
        self.emit_state(mailbox);
        Ok(view)
    }

    pub fn report_insert_failed(
        &self,
        mailbox: &UiEventMailbox,
        req: WorkflowTaskFailedRequest,
    ) -> WorkflowResult<WorkflowView> {
        self.ensure_inserting_task(&req.transcript_id)?;
        let code = req.code.trim();
        if code.is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_FAILURE_CODE_MISSING",
                "code is required",
            ));
        }
        self.mark_failed(WorkflowError::new(code, req.message));
        let view = self.view();
        self.emit_state(mailbox);
        Ok(view)
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
            .map(user_facing_error_line)
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

    fn current_action_text(&self) -> WorkflowResult<WorkflowActionText> {
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

    fn current_rewrite_request(
        &self,
        req: WorkflowTextCommandRequest,
    ) -> WorkflowResult<RewriteTextRequest> {
        if req.text.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_REWRITE_EMPTY_TEXT",
                "text is required",
            ));
        }
        let current = self.current_action_text()?;
        Ok(RewriteTextRequest {
            transcript_id: current.transcript_id,
            text: req.text,
            template_id: None,
        })
    }

    fn current_insert_request(
        &self,
        req: WorkflowTextCommandRequest,
    ) -> WorkflowResult<InsertTextRequest> {
        if req.text.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_INSERT_EMPTY_TEXT",
                "text is required",
            ));
        }
        let current = self.current_action_text()?;
        Ok(InsertTextRequest {
            transcript_id: Some(current.transcript_id),
            text: req.text,
        })
    }

    fn persist_transcription_result(&self, result: &TranscriptionResult) -> WorkflowResult<()> {
        let dir = data_dir::data_dir()
            .map_err(|e| WorkflowError::from_message("E_DATA_DIR", e.to_string()))?;
        history::append(
            &dir.join("history.sqlite3"),
            &history::HistoryItem {
                task_id: result.transcript_id.clone(),
                created_at_ms: now_ms(),
                asr_text: result.asr_text.clone(),
                rewritten_text: String::new(),
                inserted_text: String::new(),
                final_text: result.asr_text.clone(),
                template_id: None,
                rtf: result.metrics.rtf,
                device_used: result.metrics.device_used.clone(),
                preprocess_ms: result.metrics.preprocess_ms as i64,
                asr_ms: result.metrics.asr_ms as i64,
            },
        )
        .map_err(|e| WorkflowError::from_message("E_HISTORY_APPEND", e.to_string()))
    }

    fn persist_rewrite_result(&self, result: &RewriteResult) -> WorkflowResult<()> {
        let dir = data_dir::data_dir()
            .map_err(|e| WorkflowError::from_message("E_DATA_DIR", e.to_string()))?;
        history::update_final_text(
            &dir.join("history.sqlite3"),
            &result.transcript_id,
            &result.final_text,
            result.template_id.as_deref(),
        )
        .map_err(|e| WorkflowError::from_message("E_HISTORY_UPDATE", e.to_string()))
    }

    fn persist_inserted_text(&self, transcript_id: &str, text: &str) -> WorkflowResult<()> {
        let dir = data_dir::data_dir()
            .map_err(|e| WorkflowError::from_message("E_DATA_DIR", e.to_string()))?;
        history::update_inserted_text(&dir.join("history.sqlite3"), transcript_id, text)
            .map_err(|e| WorkflowError::from_message("E_HISTORY_UPDATE", e.to_string()))
    }

    fn remember_error(&self, err: WorkflowError) {
        let task_id = {
            let state = self.state.lock().unwrap();
            state
                .session
                .as_ref()
                .map(|session| session.session_id.clone())
        };
        log_workflow_error(task_id.as_deref(), "WF.remember_error", &err);
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

    fn ensure_phase_task(
        &self,
        expected_phase: WorkflowPhase,
        task_id: &str,
        code: &str,
    ) -> WorkflowResult<()> {
        let snapshot = self.snapshot();
        if snapshot.phase != expected_phase {
            return Err(WorkflowError::new(
                code,
                format!("workflow phase is {}", snapshot.phase.as_str()),
            ));
        }
        self.ensure_active_task(task_id)
    }

    fn ensure_transcribing_task(&self, task_id: &str) -> WorkflowResult<()> {
        self.ensure_phase_task(
            WorkflowPhase::Transcribing,
            task_id,
            "E_WORKFLOW_ASR_REPORT_INVALID_PHASE",
        )
    }

    fn ensure_asr_report_task(&self, task_id: &str) -> WorkflowResult<()> {
        let snapshot = self.snapshot();
        if !matches!(
            snapshot.phase,
            WorkflowPhase::Recording | WorkflowPhase::Transcribing
        ) {
            return Err(WorkflowError::new(
                "E_WORKFLOW_ASR_REPORT_INVALID_PHASE",
                format!("workflow phase is {}", snapshot.phase.as_str()),
            ));
        }
        self.ensure_active_task(task_id)
    }

    fn ensure_rewriting_task(&self, task_id: &str) -> WorkflowResult<()> {
        self.ensure_phase_task(
            WorkflowPhase::Rewriting,
            task_id,
            "E_WORKFLOW_REWRITE_REPORT_INVALID_PHASE",
        )
    }

    fn ensure_inserting_task(&self, task_id: &str) -> WorkflowResult<()> {
        self.ensure_phase_task(
            WorkflowPhase::Inserting,
            task_id,
            "E_WORKFLOW_INSERT_REPORT_INVALID_PHASE",
        )
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
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed
        ) {
            return Err(primary_phase_error(state.phase));
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
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed
        ) {
            return Err(primary_phase_error(state.phase));
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
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed
        ) {
            return Err(primary_phase_error(state.phase));
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

    #[cfg(test)]
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

    fn begin_transcribing_current(&self) -> WorkflowResult<WorkflowSession> {
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
        if session.recording_session_id.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_SESSION_MISSING",
                "recording session missing",
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

    fn complete_empty_transcription(&self, transcript_id: &str) -> WorkflowResult<()> {
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
        if session_id != transcript_id {
            return Err(WorkflowError::new(
                "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                "empty transcription does not match active session",
            ));
        }
        state.phase = WorkflowPhase::Idle;
        state.session = None;
        state.transcription = None;
        state.rewrite = None;
        state.insert_previous_phase = None;
        state.last_created_at_ms = None;
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
        if !matches!(
            state.phase,
            WorkflowPhase::Transcribed | WorkflowPhase::Rewritten
        ) {
            return Err(rewrite_phase_error(state.phase));
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

    fn begin_insert(&self, transcript_id: &str) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Transcribed | WorkflowPhase::Rewritten
        ) {
            return Err(insert_phase_error(state.phase));
        }
        if let Some(session) = state.session.as_ref() {
            if !session.session_id.is_empty() && session.session_id != transcript_id {
                return Err(WorkflowError::new(
                    "E_WORKFLOW_TRANSCRIPT_MISMATCH",
                    "insert transcript does not match workflow session",
                ));
            }
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
        state.insert_previous_phase = None;
        state.phase = WorkflowPhase::Idle;
        state.session = None;
        state.transcription = None;
        state.rewrite = None;
        state.last_created_at_ms = None;
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
        let task_id = {
            let state = self.state.lock().unwrap();
            state
                .session
                .as_ref()
                .map(|session| session.session_id.clone())
        };
        log_workflow_error(task_id.as_deref(), "WF.mark_failed", &err);
        let mut state = self.state.lock().unwrap();
        state.phase = WorkflowPhase::Failed;
        state.insert_previous_phase = None;
        state.last_error = Some(err);
    }

    fn cancel_current_recording_state(&self) -> WorkflowResult<()> {
        let state = self.state.lock().unwrap();
        if state.phase != WorkflowPhase::Recording {
            return Err(WorkflowError::new(
                "E_WORKFLOW_INVALID_PHASE",
                "workflow is not recording",
            ));
        }
        let session = state
            .session
            .as_ref()
            .ok_or_else(|| WorkflowError::new("E_WORKFLOW_SESSION_MISSING", "session missing"))?;
        if session.recording_session_id.trim().is_empty() {
            return Err(WorkflowError::new(
                "E_WORKFLOW_SESSION_MISSING",
                "recording session missing",
            ));
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
        self.begin_insert("task-1")
    }

    #[cfg(test)]
    fn complete_insert_for_test(&self) -> WorkflowResult<()> {
        self.complete_insert()
    }

    #[cfg(test)]
    fn cancel_current_recording_for_test(&self) -> WorkflowResult<()> {
        self.cancel_current_recording_state()
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
        WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed => "START",
        WorkflowPhase::Recording => "STOP",
        WorkflowPhase::Transcribing => "TRANSCRIBING",
        WorkflowPhase::Transcribed => "TRANSCRIBED",
        WorkflowPhase::Rewriting => "REWRITING",
        WorkflowPhase::Rewritten => "REWRITTEN",
        WorkflowPhase::Inserting => "INSERTING",
    }
}

fn user_facing_error_line(err: &WorkflowError) -> String {
    let title = user_facing_error_title(&err.code);
    let action = user_facing_error_action(&err.code);
    format!("{title}. {action}")
}

fn user_facing_error_title(code: &str) -> &'static str {
    if code.starts_with("E_TOOLCHAIN_") {
        return "Local audio tools need repair";
    }
    if code.starts_with("E_RECORD_")
        || code.starts_with("E_STREAMING_TRANSCRIBE_")
        || code.starts_with("E_DOUBAO_ASR_")
        || code.starts_with("E_ASR_")
    {
        return "Speech recognition could not start";
    }
    if code.starts_with("E_REWRITE_") || code.starts_with("HTTP_") {
        return "Text improvement failed";
    }
    if code.starts_with("E_INSERT_")
        || code.starts_with("E_EXPORT_")
        || code.starts_with("E_OVERLAY_")
    {
        return "Text could not be pasted";
    }
    if code == "E_TASK_ALREADY_ACTIVE" || code == "E_RECORD_ALREADY_ACTIVE" {
        return "An action is already running";
    }
    if code.starts_with("E_SETTINGS_") {
        return "Settings need attention";
    }
    "Something went wrong"
}

fn user_facing_error_action(code: &str) -> &'static str {
    if code.starts_with("E_TOOLCHAIN_") {
        return "Repair the local audio tools, then restart the app.";
    }
    if code.starts_with("E_RECORD_")
        || code.starts_with("E_STREAMING_TRANSCRIBE_")
        || code.starts_with("E_DOUBAO_ASR_")
        || code.starts_with("E_ASR_")
    {
        return "Check the selected microphone and speech recognition settings.";
    }
    if code.starts_with("E_REWRITE_") || code.starts_with("HTTP_") {
        return "Check text improvement settings and try again.";
    }
    if code.starts_with("E_INSERT_")
        || code.starts_with("E_EXPORT_")
        || code.starts_with("E_OVERLAY_")
    {
        return "Select the target app and try again.";
    }
    if code == "E_TASK_ALREADY_ACTIVE" || code == "E_RECORD_ALREADY_ACTIVE" {
        return "Wait for the current action to finish.";
    }
    "Check settings and try again."
}

fn primary_phase_error(phase: WorkflowPhase) -> WorkflowError {
    match phase {
        WorkflowPhase::Transcribing => WorkflowError::new(
            "E_WORKFLOW_PRIMARY_TRANSCRIBING",
            "workflow is transcribing",
        ),
        WorkflowPhase::Transcribed => {
            WorkflowError::new("E_WORKFLOW_PRIMARY_TRANSCRIBED", "workflow is transcribed")
        }
        WorkflowPhase::Rewriting => {
            WorkflowError::new("E_WORKFLOW_PRIMARY_REWRITING", "workflow is rewriting")
        }
        WorkflowPhase::Rewritten => {
            WorkflowError::new("E_WORKFLOW_PRIMARY_REWRITTEN", "workflow is rewritten")
        }
        WorkflowPhase::Inserting => {
            WorkflowError::new("E_WORKFLOW_PRIMARY_INSERTING", "workflow is inserting")
        }
        WorkflowPhase::Recording => {
            WorkflowError::new("E_WORKFLOW_PRIMARY_RECORDING", "workflow is recording")
        }
        WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed => {
            WorkflowError::new("E_WORKFLOW_PRIMARY_ALLOWED", "primary is allowed")
        }
    }
}

fn rewrite_phase_error(phase: WorkflowPhase) -> WorkflowError {
    let code = match phase {
        WorkflowPhase::Idle => "E_WORKFLOW_REWRITE_IDLE",
        WorkflowPhase::Recording => "E_WORKFLOW_REWRITE_RECORDING",
        WorkflowPhase::Transcribing => "E_WORKFLOW_REWRITE_TRANSCRIBING",
        WorkflowPhase::Rewriting => "E_WORKFLOW_REWRITE_REWRITING",
        WorkflowPhase::Inserting => "E_WORKFLOW_REWRITE_INSERTING",
        WorkflowPhase::Cancelled => "E_WORKFLOW_REWRITE_CANCELLED",
        WorkflowPhase::Failed => "E_WORKFLOW_REWRITE_FAILED",
        WorkflowPhase::Transcribed | WorkflowPhase::Rewritten => "E_WORKFLOW_REWRITE_ALLOWED",
    };
    WorkflowError::new(code, format!("workflow is {}", phase.as_str()))
}

fn insert_phase_error(phase: WorkflowPhase) -> WorkflowError {
    let code = match phase {
        WorkflowPhase::Idle => "E_WORKFLOW_INSERT_IDLE",
        WorkflowPhase::Recording => "E_WORKFLOW_INSERT_RECORDING",
        WorkflowPhase::Transcribing => "E_WORKFLOW_INSERT_TRANSCRIBING",
        WorkflowPhase::Rewriting => "E_WORKFLOW_INSERT_REWRITING",
        WorkflowPhase::Inserting => "E_WORKFLOW_INSERT_INSERTING",
        WorkflowPhase::Cancelled => "E_WORKFLOW_INSERT_CANCELLED",
        WorkflowPhase::Failed => "E_WORKFLOW_INSERT_FAILED",
        WorkflowPhase::Transcribed | WorkflowPhase::Rewritten => "E_WORKFLOW_INSERT_ALLOWED",
    };
    WorkflowError::new(code, format!("workflow is {}", phase.as_str()))
}

fn cancel_phase_error(phase: WorkflowPhase) -> WorkflowError {
    let code = match phase {
        WorkflowPhase::Idle => "E_WORKFLOW_CANCEL_IDLE",
        WorkflowPhase::Transcribed => "E_WORKFLOW_CANCEL_TRANSCRIBED",
        WorkflowPhase::Rewritten => "E_WORKFLOW_CANCEL_REWRITTEN",
        WorkflowPhase::Cancelled => "E_WORKFLOW_CANCEL_CANCELLED",
        WorkflowPhase::Failed => "E_WORKFLOW_CANCEL_FAILED",
        WorkflowPhase::Recording | WorkflowPhase::Transcribing => "E_WORKFLOW_CANCEL_ALLOWED",
        WorkflowPhase::Rewriting => "E_WORKFLOW_CANCEL_REWRITING",
        WorkflowPhase::Inserting => "E_WORKFLOW_CANCEL_INSERTING",
    };
    WorkflowError::new(code, format!("workflow is {}", phase.as_str()))
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

fn is_empty_asr_failure(code: &str, message: &str) -> bool {
    matches!(
        code,
        "E_ASR_EMPTY_TEXT" | "E_REMOTE_ASR_EMPTY_TEXT" | "E_WORKFLOW_ASR_TEXT_MISSING"
    ) || message.contains("Empty ASR text")
        || message.contains("empty transcription")
        || message.contains("merged text is empty")
        || message.contains("response.text is missing or empty")
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

fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => dur.as_millis() as i64,
        Err(_) => 0,
    }
}

fn log_workflow_error(task_id: Option<&str>, step_id: &str, err: &WorkflowError) {
    if let Ok(dir) = data_dir::data_dir() {
        crate::obs::event_err(
            &dir,
            task_id,
            "Workflow",
            step_id,
            "workflow",
            &err.code,
            &err.message,
            Some(serde_json::json!({
                "raw": err.raw_message(),
                "rendered": err.render(),
            })),
        );
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
    ensure_toolchain_ready(runtime)
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
    fn diagnostic_line_uses_user_facing_text() {
        let err = WorkflowError::new("E_ASR_FAILED", "provider returned E_ASR_FAILED");

        let line = user_facing_error_line(&err);

        assert_eq!(
            line,
            "Speech recognition could not start. Check the selected microphone and speech recognition settings.",
        );
        assert!(!line.contains("E_ASR_FAILED"));
    }

    #[test]
    fn workflow_error_preserves_port_error_raw_message() {
        let port = PortError::from_message(
            "E_PORT_DEFAULT",
            "E_PORT_REAL: provider returned raw diagnostic",
        );

        let err = WorkflowError::from_port(port);

        assert_eq!(err.code, "E_PORT_REAL");
        assert_eq!(err.message, "E_PORT_REAL: provider returned raw diagnostic");
        assert_eq!(
            err.raw_message(),
            "E_PORT_REAL: provider returned raw diagnostic"
        );
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

        assert_eq!(err.code, "E_WORKFLOW_PRIMARY_RECORDING");
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
    fn insert_completes_session_to_idle() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");

        workflow.begin_insert_for_test().expect("insert starts");
        assert_eq!(workflow.phase(), WorkflowPhase::Inserting);
        workflow
            .complete_insert_for_test()
            .expect("insert completes");

        assert_eq!(workflow.phase(), WorkflowPhase::Idle);
    }

    #[test]
    fn rewrite_allows_empty_asr_when_command_text_is_external() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "")
            .expect("transcribed");

        workflow
            .begin_rewrite_for_test("task-1")
            .expect("rewrite starts from external command text");
        assert_eq!(workflow.phase(), WorkflowPhase::Rewriting);
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
            .current_action_text()
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
            .cancel_current_recording_for_test()
            .expect("recording cancels");

        assert_eq!(workflow.phase(), WorkflowPhase::Cancelled);
    }

    #[test]
    fn current_action_text_uses_current_workflow_result() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");

        let current = workflow
            .current_action_text()
            .expect("current action text exists");

        assert_eq!(current.transcript_id, "task-1");
        assert_eq!(current.final_text, "asr text");
    }

    #[test]
    fn rewrite_current_request_uses_current_transcript_id() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");

        let req = workflow
            .current_rewrite_request(WorkflowTextCommandRequest {
                text: "edited text".to_string(),
            })
            .expect("request is built from current state");

        assert_eq!(req.transcript_id, "task-1");
        assert_eq!(req.text, "edited text");
    }

    #[test]
    fn insert_current_request_uses_current_transcript_id() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_transcribed_session_for_test("task-1", "asr text")
            .expect("transcribed");

        let req = workflow
            .current_insert_request(WorkflowTextCommandRequest {
                text: "edited text".to_string(),
            })
            .expect("request is built from current state");

        assert_eq!(req.transcript_id.as_deref(), Some("task-1"));
        assert_eq!(req.text, "edited text");
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
            .prepare_stop_record_transcribe()
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
    fn report_failed_event_rejects_mismatched_task_id() {
        let (mailbox, _rx) = UiEventMailbox::for_test();
        let audio = RecordingRegistry::new();
        let actor = TranscriptionActor::new(mailbox.clone());
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let err = workflow
            .report_asr_failed(
                &audio,
                &actor,
                &mailbox,
                WorkflowTaskFailedRequest {
                    transcript_id: "task-2".to_string(),
                    code: "E_ASR_FAILED".to_string(),
                    message: "failed".to_string(),
                },
            )
            .expect_err("wrong task id is rejected");

        assert_eq!(err.code, "E_WORKFLOW_TRANSCRIPT_MISMATCH");
        assert_eq!(workflow.phase(), WorkflowPhase::Transcribing);
    }

    #[test]
    fn report_failed_event_accepts_recording_phase_for_active_task() {
        let (mailbox, _rx) = UiEventMailbox::for_test();
        let audio = RecordingRegistry::new();
        let actor = TranscriptionActor::new(mailbox.clone());
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");

        let view = workflow
            .report_asr_failed(
                &audio,
                &actor,
                &mailbox,
                WorkflowTaskFailedRequest {
                    transcript_id: "task-1".to_string(),
                    code: "E_DOUBAO_ASR_CREDENTIALS_MISSING".to_string(),
                    message: "missing credentials".to_string(),
                },
            )
            .expect("recording phase ASR failure is accepted");

        assert_eq!(view.phase, "failed");
        assert_eq!(
            view.diagnostic_code.as_deref(),
            Some("E_DOUBAO_ASR_CREDENTIALS_MISSING")
        );
        assert_eq!(workflow.phase(), WorkflowPhase::Failed);
    }

    #[test]
    fn report_empty_event_clears_transcribing_without_failure() {
        let (mailbox, _rx) = UiEventMailbox::for_test();
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let view = workflow
            .report_asr_empty(
                &mailbox,
                WorkflowAsrEmptyRequest {
                    transcript_id: "task-1".to_string(),
                },
            )
            .expect("empty transcription is accepted");

        assert_eq!(view.phase, "idle");
        assert_eq!(view.diagnostic_code.as_deref(), None);
        assert_eq!(view.last_text, "");
        assert_eq!(workflow.phase(), WorkflowPhase::Idle);
    }

    #[test]
    fn apply_transcription_completed_event_is_display_only() {
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

        assert_eq!(view.phase, "transcribing");
        assert_eq!(view.last_text, "");
    }

    #[test]
    fn apply_rewrite_completed_event_is_display_only() {
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

        assert_eq!(view.phase, "rewriting");
        assert_eq!(view.last_text, "asr text");
    }

    #[test]
    fn apply_insert_completed_event_is_display_only() {
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

        assert_eq!(view.phase, "inserting");
    }

    #[test]
    fn apply_failed_event_is_display_only() {
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

        assert_eq!(view.phase, "transcribing");
        assert_eq!(view.diagnostic_code.as_deref(), None);
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
    fn task_mismatch_display_event_keeps_state() {
        let workflow = VoiceWorkflow::new();
        workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");
        workflow
            .begin_transcribing_for_test("recording-1")
            .expect("transcribing starts");

        let view = workflow
            .apply_event_to_state(completed_event(
                "event-1",
                "transcription.completed",
                "task-2",
                serde_json::to_value(transcription_result("task-2", "asr text"))
                    .expect("payload serializes"),
            ))
            .expect("display event is accepted");

        assert_eq!(view.phase, "transcribing");
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
