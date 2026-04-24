use std::{collections::HashMap, path::Path, sync::Mutex};

use crate::audio_capture::RecordingRegistry;
use crate::context_capture;
use crate::context_pack::ContextSnapshot;
use crate::insertion::{InsertResult, InsertTextRequest};
use crate::ports::PortError;
use crate::record_input_cache::RecordInputCacheState;
use crate::rewrite::{RewriteResult, RewriteTextRequest};
use crate::task_manager::TaskManager;
use crate::transcription::{
    TranscribeFixtureRequest, TranscriptionInput, TranscriptionResult, TranscriptionService,
};
use crate::ui_events::{UiEvent, UiEventMailbox, UiEventStatus};
use crate::{insertion, rewrite, RuntimeState};

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

    fn from_message(code: &str, message: impl Into<String>) -> Self {
        Self::new(code, message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSession {
    pub session_id: String,
    pub recording_session_id: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WorkflowSnapshot {
    pub phase: WorkflowPhase,
    pub session: Option<WorkflowSession>,
    pub transcription: Option<TranscriptionResult>,
    pub rewrite: Option<RewriteResult>,
    pub last_error: Option<WorkflowError>,
}

#[derive(Debug, Clone)]
struct WorkflowState {
    phase: WorkflowPhase,
    session: Option<WorkflowSession>,
    transcription: Option<TranscriptionResult>,
    rewrite: Option<RewriteResult>,
    pending_contexts: HashMap<String, PendingWorkflowContext>,
    last_error: Option<WorkflowError>,
}

#[derive(Debug, Clone)]
struct PendingWorkflowContext {
    created_at_ms: i64,
    snapshot: ContextSnapshot,
}

impl WorkflowState {
    fn idle() -> Self {
        Self {
            phase: WorkflowPhase::Idle,
            session: None,
            transcription: None,
            rewrite: None,
            pending_contexts: HashMap::new(),
            last_error: None,
        }
    }

    fn snapshot(&self) -> WorkflowSnapshot {
        WorkflowSnapshot {
            phase: self.phase,
            session: self.session.clone(),
            transcription: self.transcription.clone(),
            rewrite: self.rewrite.clone(),
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

    pub fn start_record_transcribe(
        &self,
        runtime: &RuntimeState,
        audio: &RecordingRegistry,
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

        match audio.start_recording(mailbox, record_input_cache, Some(transcript_id.clone())) {
            Ok(recording_session_id) => {
                self.attach_recording_session(&transcript_id, &recording_session_id)?;
                Ok(recording_session_id)
            }
            Err(err) => {
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

    pub async fn stop_record_transcribe(
        &self,
        runtime: &RuntimeState,
        audio: &RecordingRegistry,
        transcriber: &TranscriptionService,
        mailbox: &UiEventMailbox,
        recording_session_id: &str,
    ) -> WorkflowResult<TranscriptionResult> {
        let session = self.begin_transcribing(recording_session_id)?;
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
        mailbox: &UiEventMailbox,
        session_id: Option<String>,
        transcript_id: Option<String>,
    ) -> WorkflowResult<()> {
        let snapshot = self.snapshot();
        match snapshot.phase {
            WorkflowPhase::Recording => {
                let expected = session_id.as_deref();
                self.cancel_recording_state(expected)?;
                audio
                    .abort_recording(session_id)
                    .map_err(|err| WorkflowError::new(&err.code, err.message))?;
                if let Some(session) = snapshot.session {
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
                self.mark_cancelled();
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
        let previous = self.begin_insert()?;
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
        self.complete_insert(previous);
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
            return Err(WorkflowError::new(
                "E_WORKFLOW_BUSY",
                "workflow already has an active session",
            ));
        }
        let session = WorkflowSession {
            session_id: session_id.into(),
            recording_session_id: recording_session_id.into(),
        };
        state.phase = WorkflowPhase::Recording;
        state.session = Some(session.clone());
        state.transcription = None;
        state.rewrite = None;
        state.last_error = None;
        Ok(session)
    }

    fn reserve_recording(&self, transcript_id: &str) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed
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
        });
        state.transcription = None;
        state.rewrite = None;
        state.last_error = None;
        Ok(())
    }

    fn reserve_transcribing(&self, transcript_id: &str) -> WorkflowResult<()> {
        let mut state = self.state.lock().unwrap();
        if !matches!(
            state.phase,
            WorkflowPhase::Idle | WorkflowPhase::Cancelled | WorkflowPhase::Failed
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
        });
        state.transcription = None;
        state.rewrite = None;
        state.last_error = None;
        Ok(())
    }

    fn attach_recording_session(
        &self,
        transcript_id: &str,
        recording_session_id: &str,
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
        });
        state.transcription = Some(result);
        state.rewrite = None;
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
        });
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
        state.last_error = None;
        Ok(())
    }

    fn begin_insert(&self) -> WorkflowResult<WorkflowPhase> {
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
        let previous = state.phase;
        state.phase = WorkflowPhase::Inserting;
        state.last_error = None;
        Ok(previous)
    }

    fn complete_insert(&self, previous: WorkflowPhase) {
        let mut state = self.state.lock().unwrap();
        if state.phase == WorkflowPhase::Inserting {
            state.phase = previous;
        }
    }

    fn mark_cancelled(&self) {
        let mut state = self.state.lock().unwrap();
        state.phase = WorkflowPhase::Cancelled;
        state.last_error = None;
    }

    fn mark_failed(&self, err: WorkflowError) {
        let mut state = self.state.lock().unwrap();
        state.phase = WorkflowPhase::Failed;
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
    fn begin_insert_for_test(&self) -> WorkflowResult<WorkflowPhase> {
        self.begin_insert()
    }

    #[cfg(test)]
    fn complete_insert_for_test(&self, previous: WorkflowPhase) {
        self.complete_insert(previous);
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
    fn start_recording_moves_idle_to_recording() {
        let workflow = VoiceWorkflow::new();

        let session = workflow
            .open_recording_for_test("task-1", "recording-1")
            .expect("recording starts");

        assert_eq!(session.session_id, "task-1");
        assert_eq!(workflow.phase(), WorkflowPhase::Recording);
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

        let previous = workflow.begin_insert_for_test().expect("insert starts");
        assert_eq!(workflow.phase(), WorkflowPhase::Inserting);
        workflow.complete_insert_for_test(previous);

        assert_eq!(workflow.phase(), WorkflowPhase::Transcribed);
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
}
