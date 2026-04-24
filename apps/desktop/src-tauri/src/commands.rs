use serde::{Deserialize, Serialize};
use tauri::State;

use crate::audio_capture::RecordingRegistry;
use crate::insertion::{InsertResult, InsertTextRequest};
use crate::ports::PortError;
use crate::record_input_cache::RecordInputCacheState;
use crate::rewrite::{RewriteResult, RewriteTextRequest};
use crate::transcription::{
    TranscribeFixtureRequest, TranscriptionInput, TranscriptionResult, TranscriptionService,
};
use crate::ui_events::UiEventMailbox;
use crate::{data_dir, insertion, rewrite, RuntimeState};

#[cfg(test)]
pub fn command_names() -> &'static [&'static str] {
    &[
        "record_transcribe_start",
        "record_transcribe_stop",
        "record_transcribe_cancel",
        "rewrite_text",
        "insert_text",
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
    audio: State<'_, RecordingRegistry>,
    mailbox: State<'_, UiEventMailbox>,
    record_input_cache: State<'_, RecordInputCacheState>,
    req: RecordTranscribeStartRequest,
) -> Result<RecordTranscribeStartResult, String> {
    ensure_toolchain_ready(&runtime)?;
    let session_id = audio
        .start_recording(
            &mailbox,
            &record_input_cache,
            normalize_task_id(req.task_id)?,
        )
        .map_err(|e| e.render())?;
    Ok(RecordTranscribeStartResult { session_id })
}

#[tauri::command]
pub async fn record_transcribe_stop(
    runtime: State<'_, RuntimeState>,
    audio: State<'_, RecordingRegistry>,
    transcriber: State<'_, TranscriptionService>,
    mailbox: State<'_, UiEventMailbox>,
    req: RecordTranscribeStopRequest,
) -> Result<TranscriptionResult, String> {
    let asset = audio
        .stop_recording(&req.session_id)
        .map_err(|e| e.render())?;
    let consumed = audio.take_asset(&asset.asset_id).unwrap_or(asset);
    if let Err(e) = ensure_runtime_ready(&runtime) {
        let _ = std::fs::remove_file(&consumed.output_path);
        return Err(e);
    }
    transcriber
        .transcribe_audio(
            &mailbox,
            TranscriptionInput {
                task_id: consumed.task_id,
                input_path: consumed.output_path,
                record_elapsed_ms: consumed.record_elapsed_ms,
                record_label: "Record (backend)".to_string(),
            },
        )
        .await
        .map_err(render_port_error)
}

#[tauri::command]
pub fn record_transcribe_cancel(
    audio: State<'_, RecordingRegistry>,
    transcriber: State<'_, TranscriptionService>,
    _mailbox: State<'_, UiEventMailbox>,
    req: RecordTranscribeCancelRequest,
) -> Result<(), String> {
    let mut result: Result<(), String> = Ok(());
    if audio.has_active_recording() {
        result = audio
            .abort_recording(req.session_id.clone())
            .map_err(|e| e.render());
    }
    if transcriber.has_active_task() {
        result = transcriber
            .cancel(req.transcript_id.as_deref())
            .map_err(render_port_error);
    }
    result
}

#[tauri::command]
pub async fn transcribe_fixture(
    runtime: State<'_, RuntimeState>,
    transcriber: State<'_, TranscriptionService>,
    mailbox: State<'_, UiEventMailbox>,
    req: TranscribeFixtureRequest,
) -> Result<TranscriptionResult, String> {
    ensure_runtime_ready(&runtime)?;
    transcriber
        .transcribe_fixture(&mailbox, req)
        .await
        .map_err(render_port_error)
}

#[tauri::command]
pub async fn rewrite_text(
    mailbox: State<'_, UiEventMailbox>,
    task_state: State<'_, crate::task_manager::TaskManager>,
    req: RewriteTextRequest,
) -> Result<RewriteResult, String> {
    rewrite::rewrite_text(&mailbox, &task_state, req)
        .await
        .map_err(render_port_error)
}

#[tauri::command]
pub async fn insert_text(req: InsertTextRequest) -> Result<InsertResult, String> {
    insertion::insert_text(req).await.map_err(render_port_error)
}

fn ensure_toolchain_ready(runtime: &RuntimeState) -> Result<(), String> {
    let tc = runtime.get_toolchain();
    if !tc.ready {
        return Err(tc
            .message
            .unwrap_or_else(|| "E_TOOLCHAIN_NOT_READY: toolchain is not ready".to_string()));
    }
    Ok(())
}

fn ensure_runtime_ready(runtime: &RuntimeState) -> Result<(), String> {
    ensure_toolchain_ready(runtime)?;
    let py = runtime.get_python();
    if !py.ready {
        return Err(py
            .message
            .unwrap_or_else(|| "E_PYTHON_NOT_READY: python runtime is not ready".to_string()));
    }
    Ok(())
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

fn render_port_error(err: PortError) -> String {
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
        assert!(names.contains(&"transcribe_fixture"));
        assert!(!names.contains(&"start_task"));
        assert!(!names.contains(&"export_text"));
    }
}
