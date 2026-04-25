use tauri::{Manager, Runtime};

use crate::audio_capture::RecordingRegistry;
use crate::insertion;
use crate::rewrite;
use crate::task_manager::TaskManager;
use crate::transcription::{TranscriptionInput, TranscriptionService};
use crate::ui_events::{UiEvent, UiEventMailbox, UiEventStatus};
use crate::voice_workflow::{WorkflowError, WorkflowTaskRequest};
use crate::RuntimeState;

pub fn spawn<R: Runtime>(app: tauri::AppHandle<R>, task: WorkflowTaskRequest) {
    tauri::async_runtime::spawn(async move {
        match task {
            WorkflowTaskRequest::StopRecordTranscribe {
                task_id,
                recording_session_id,
            } => {
                let runtime = app.state::<RuntimeState>();
                let audio = app.state::<RecordingRegistry>();
                let transcriber = app.state::<TranscriptionService>();
                let mailbox = app.state::<UiEventMailbox>();
                run_stop_record_transcribe(
                    &runtime,
                    &audio,
                    &transcriber,
                    &mailbox,
                    task_id,
                    recording_session_id,
                )
                .await;
            }
            WorkflowTaskRequest::Rewrite {
                task_id,
                pending_context,
                req,
            } => {
                let task_state = app.state::<TaskManager>();
                let mailbox = app.state::<UiEventMailbox>();
                mailbox.send(UiEvent::stage(
                    &task_id,
                    "Rewrite",
                    UiEventStatus::Started,
                    "llm",
                ));
                match rewrite::rewrite_text(&task_state, pending_context, req).await {
                    Ok(result) => {
                        mailbox.send(UiEvent::stage_with_elapsed(
                            &task_id,
                            "Rewrite",
                            UiEventStatus::Completed,
                            "ok",
                            Some(result.rewrite_ms),
                            None,
                        ));
                        mailbox.send(UiEvent::state_completed(
                            &task_id,
                            "rewrite.completed",
                            "rewrite completed",
                            serde_json::to_value(&result).unwrap_or_default(),
                        ));
                    }
                    Err(err) => {
                        send_failed(&mailbox, &task_id, "Rewrite", &err.code, err.message);
                    }
                }
            }
            WorkflowTaskRequest::Insert { task_id, req } => {
                let mailbox = app.state::<UiEventMailbox>();
                mailbox.send(UiEvent::stage(
                    &task_id,
                    "Insert",
                    UiEventStatus::Started,
                    "insert",
                ));
                match insertion::insert_text(req).await {
                    Ok(result) => {
                        mailbox.send(UiEvent::stage(
                            &task_id,
                            "Insert",
                            UiEventStatus::Completed,
                            "ok",
                        ));
                        mailbox.send(UiEvent::state_completed(
                            &task_id,
                            "insertion.completed",
                            "insertion completed",
                            serde_json::to_value(&result).unwrap_or_default(),
                        ));
                    }
                    Err(err) => {
                        send_failed(&mailbox, &task_id, "Insert", &err.code, err.message);
                    }
                }
            }
        }
    });
}

async fn run_stop_record_transcribe(
    runtime: &RuntimeState,
    audio: &RecordingRegistry,
    transcriber: &TranscriptionService,
    mailbox: &UiEventMailbox,
    task_id: String,
    recording_session_id: String,
) {
    let asset = match audio.stop_recording(&recording_session_id) {
        Ok(asset) => asset,
        Err(err) => {
            send_failed(mailbox, &task_id, "Record", &err.code, err.message);
            return;
        }
    };
    let consumed = audio.take_asset(&asset.asset_id).unwrap_or(asset);
    mailbox.send(UiEvent::stage_with_elapsed(
        &task_id,
        "Record",
        UiEventStatus::Completed,
        "ok",
        Some(consumed.record_elapsed_ms),
        None,
    ));

    if let Err(err) = ensure_runtime_ready(runtime) {
        let _ = std::fs::remove_file(&consumed.output_path);
        send_failed(mailbox, &task_id, "Transcribe", &err.code, err.message);
        return;
    }

    mailbox.send(UiEvent::stage(
        &task_id,
        "Transcribe",
        UiEventStatus::Started,
        "asr",
    ));
    match transcriber
        .transcribe_audio(TranscriptionInput {
            task_id: consumed.task_id,
            input_path: consumed.output_path,
            record_elapsed_ms: consumed.record_elapsed_ms,
            record_label: "Record (backend)".to_string(),
        })
        .await
    {
        Ok(result) => {
            mailbox.send(UiEvent::stage_with_elapsed(
                &result.transcript_id,
                "Transcribe",
                UiEventStatus::Completed,
                "ok",
                Some(result.metrics.asr_ms),
                None,
            ));
            mailbox.send(UiEvent::state_completed(
                &result.transcript_id,
                "transcription.completed",
                "transcription completed",
                serde_json::to_value(&result).unwrap_or_default(),
            ));
        }
        Err(err) if err.code == "E_CANCELLED" => {
            mailbox.send(UiEvent::stage(
                &task_id,
                "Transcribe",
                UiEventStatus::Cancelled,
                "cancelled",
            ));
            mailbox.send(UiEvent::state_cancelled(&task_id, "Transcribe"));
        }
        Err(err) => {
            send_failed(mailbox, &task_id, "Transcribe", &err.code, err.message);
        }
    }
}

fn send_failed(mailbox: &UiEventMailbox, task_id: &str, stage: &str, code: &str, message: String) {
    mailbox.send(UiEvent::stage_with_elapsed(
        task_id,
        stage,
        UiEventStatus::Failed,
        message.clone(),
        None,
        Some(code.to_string()),
    ));
    mailbox.send(UiEvent::state_failed(task_id, stage, code, message));
}

fn ensure_runtime_ready(runtime: &RuntimeState) -> Result<(), WorkflowError> {
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
