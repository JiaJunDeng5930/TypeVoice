use serde::{Deserialize, Serialize};
use tauri::State;

use crate::audio_capture::RecordingRegistry;
use crate::insertion::{InsertResult, InsertTextRequest};
use crate::record_input_cache::RecordInputCacheState;
use crate::rewrite::{RewriteResult, RewriteTextRequest};
use crate::transcription::{TranscribeFixtureRequest, TranscriptionResult, TranscriptionService};
use crate::ui_events::UiEventMailbox;
use crate::voice_workflow::{VoiceWorkflow, WorkflowCommandRequest, WorkflowError, WorkflowView};
use crate::{data_dir, RuntimeState};

#[cfg(test)]
pub fn command_names() -> &'static [&'static str] {
    &[
        "record_transcribe_start",
        "record_transcribe_stop",
        "record_transcribe_cancel",
        "rewrite_text",
        "insert_text",
        "workflow_snapshot",
        "workflow_command",
        "transcribe_fixture",
    ]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTranscribeStartRequest {
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTranscribeStartResult {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTranscribeStopRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTranscribeCancelRequest {
    pub session_id: Option<String>,
    pub transcript_id: Option<String>,
}

#[tauri::command]
pub fn record_transcribe_start(
    runtime: State<'_, RuntimeState>,
    workflow: State<'_, VoiceWorkflow>,
    audio: State<'_, RecordingRegistry>,
    mailbox: State<'_, UiEventMailbox>,
    record_input_cache: State<'_, RecordInputCacheState>,
    req: RecordTranscribeStartRequest,
) -> Result<RecordTranscribeStartResult, String> {
    let session_id = workflow
        .start_record_transcribe(
            &runtime,
            &audio,
            &mailbox,
            &record_input_cache,
            normalize_task_id(req.task_id)?,
        )
        .map_err(render_workflow_error)?;
    Ok(RecordTranscribeStartResult { session_id })
}

#[tauri::command]
pub fn workflow_snapshot(workflow: State<'_, VoiceWorkflow>) -> Result<WorkflowView, String> {
    workflow.snapshot_view().map_err(render_workflow_error)
}

#[tauri::command]
pub async fn workflow_command(
    runtime: State<'_, RuntimeState>,
    workflow: State<'_, VoiceWorkflow>,
    audio: State<'_, RecordingRegistry>,
    transcriber: State<'_, TranscriptionService>,
    mailbox: State<'_, UiEventMailbox>,
    record_input_cache: State<'_, RecordInputCacheState>,
    task_state: State<'_, crate::task_manager::TaskManager>,
    req: WorkflowCommandRequest,
) -> Result<WorkflowView, String> {
    workflow
        .run_command(
            &runtime,
            &audio,
            &transcriber,
            &mailbox,
            &record_input_cache,
            &task_state,
            req,
        )
        .await
        .map_err(render_workflow_error)
}

#[tauri::command]
pub async fn record_transcribe_stop(
    runtime: State<'_, RuntimeState>,
    workflow: State<'_, VoiceWorkflow>,
    audio: State<'_, RecordingRegistry>,
    transcriber: State<'_, TranscriptionService>,
    mailbox: State<'_, UiEventMailbox>,
    req: RecordTranscribeStopRequest,
) -> Result<TranscriptionResult, String> {
    workflow
        .stop_record_transcribe(&runtime, &audio, &transcriber, &mailbox, &req.session_id)
        .await
        .map_err(render_workflow_error)
}

#[tauri::command]
pub fn record_transcribe_cancel(
    workflow: State<'_, VoiceWorkflow>,
    audio: State<'_, RecordingRegistry>,
    transcriber: State<'_, TranscriptionService>,
    mailbox: State<'_, UiEventMailbox>,
    req: RecordTranscribeCancelRequest,
) -> Result<(), String> {
    workflow
        .cancel_record_transcribe(
            &audio,
            &transcriber,
            &mailbox,
            req.session_id,
            req.transcript_id,
        )
        .map_err(render_workflow_error)
}

#[tauri::command]
pub async fn transcribe_fixture(
    runtime: State<'_, RuntimeState>,
    workflow: State<'_, VoiceWorkflow>,
    transcriber: State<'_, TranscriptionService>,
    mailbox: State<'_, UiEventMailbox>,
    req: TranscribeFixtureRequest,
) -> Result<TranscriptionResult, String> {
    workflow
        .transcribe_fixture(&runtime, &transcriber, &mailbox, req)
        .await
        .map_err(render_workflow_error)
}

#[tauri::command]
pub async fn rewrite_text(
    workflow: State<'_, VoiceWorkflow>,
    mailbox: State<'_, UiEventMailbox>,
    task_state: State<'_, crate::task_manager::TaskManager>,
    req: RewriteTextRequest,
) -> Result<RewriteResult, String> {
    workflow
        .rewrite_text(&mailbox, &task_state, req)
        .await
        .map_err(render_workflow_error)
}

#[tauri::command]
pub async fn insert_text(
    workflow: State<'_, VoiceWorkflow>,
    mailbox: State<'_, UiEventMailbox>,
    req: InsertTextRequest,
) -> Result<InsertResult, String> {
    workflow
        .insert_text(&mailbox, req)
        .await
        .map_err(render_workflow_error)
}

fn normalize_task_id(task_id: Option<String>) -> Result<Option<String>, String> {
    let raw = match task_id {
        Some(v) => v.trim().to_string(),
        None => return Ok(None),
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let parsed = uuid::Uuid::parse_str(&raw)
        .map_err(|e| format!("E_TASK_ID_INVALID: invalid task_id ({e})"))?;
    Ok(Some(parsed.to_string()))
}

fn render_workflow_error(err: WorkflowError) -> String {
    format!("{}: {}", err.code, err.message)
}

#[allow(dead_code)]
fn command_span(step_id: &str) {
    if let Ok(dir) = data_dir::data_dir() {
        let span = crate::obs::Span::start(&dir, None, "Cmd", step_id, None);
        span.ok(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_command_names_exclude_removed_pipeline_entrypoints() {
        let names = command_names();

        assert!(names.contains(&"record_transcribe_start"));
        assert!(names.contains(&"record_transcribe_stop"));
        assert!(names.contains(&"record_transcribe_cancel"));
        assert!(names.contains(&"rewrite_text"));
        assert!(names.contains(&"insert_text"));
        assert!(names.contains(&"workflow_snapshot"));
        assert!(names.contains(&"workflow_command"));
        assert!(names.contains(&"transcribe_fixture"));
        assert!(!names.contains(&"start_task"));
        assert!(!names.contains(&"export_text"));
    }
}
