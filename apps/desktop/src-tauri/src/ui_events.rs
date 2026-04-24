use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const UI_EVENT_CHANNEL: &str = "ui_event";

static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEventStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

impl UiEventStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiEvent {
    pub kind: String,
    pub effect: String,
    pub event_id: String,
    pub sequence: u64,
    pub task_id: Option<String>,
    pub stage: Option<String>,
    pub status: Option<String>,
    pub message: String,
    pub elapsed_ms: Option<u128>,
    pub error_code: Option<String>,
    pub payload: Option<serde_json::Value>,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
struct OverlayState {
    visible: bool,
    status: String,
    detail: Option<String>,
    ts_ms: i64,
}

impl UiEvent {
    pub fn stage(
        task_id: impl Into<String>,
        stage: impl Into<String>,
        status: UiEventStatus,
        message: impl Into<String>,
    ) -> Self {
        Self::stage_with_elapsed(task_id, stage, status, message, None, None)
    }

    pub fn stage_with_elapsed(
        task_id: impl Into<String>,
        stage: impl Into<String>,
        status: UiEventStatus,
        message: impl Into<String>,
        elapsed_ms: Option<u128>,
        error_code: Option<String>,
    ) -> Self {
        Self {
            kind: "transcription.stage".to_string(),
            effect: "displayOnly".to_string(),
            event_id: new_event_id(),
            sequence: next_sequence(),
            task_id: Some(task_id.into()),
            stage: Some(stage.into()),
            status: Some(status.as_str().to_string()),
            message: message.into(),
            elapsed_ms,
            error_code,
            payload: None,
            ts_ms: now_ms(),
        }
    }

    pub fn error(
        task_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind: "diagnostic.error".to_string(),
            effect: "displayOnly".to_string(),
            event_id: new_event_id(),
            sequence: next_sequence(),
            task_id: Some(task_id.into()),
            stage: None,
            status: Some("failed".to_string()),
            message: message.into(),
            elapsed_ms: None,
            error_code: Some(code.into()),
            payload: None,
            ts_ms: now_ms(),
        }
    }

    pub fn completed(
        task_id: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self::completed_with_effect(task_id, kind, message, payload, "displayOnly")
    }

    pub fn state_completed(
        task_id: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self::completed_with_effect(task_id, kind, message, payload, "stateChanging")
    }

    fn completed_with_effect(
        task_id: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
        payload: serde_json::Value,
        effect: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            effect: effect.into(),
            event_id: new_event_id(),
            sequence: next_sequence(),
            task_id: Some(task_id.into()),
            stage: None,
            status: Some("completed".to_string()),
            message: message.into(),
            elapsed_ms: None,
            error_code: None,
            payload: Some(payload),
            ts_ms: now_ms(),
        }
    }

    pub fn workflow_state(payload: impl Serialize) -> Self {
        Self {
            kind: "workflow.state".to_string(),
            effect: "displayOnly".to_string(),
            event_id: new_event_id(),
            sequence: next_sequence(),
            task_id: None,
            stage: Some("Workflow".to_string()),
            status: None,
            message: "workflow state".to_string(),
            elapsed_ms: None,
            error_code: None,
            payload: Some(serde_json::to_value(payload).unwrap_or_default()),
            ts_ms: now_ms(),
        }
    }

    pub fn audio_level(recording_id: impl Into<String>, rms: f64, peak: f64) -> Self {
        Self {
            kind: "audio.level".to_string(),
            effect: "displayOnly".to_string(),
            event_id: new_event_id(),
            sequence: next_sequence(),
            task_id: None,
            stage: Some("Record".to_string()),
            status: Some("recording".to_string()),
            message: "audio level".to_string(),
            elapsed_ms: None,
            error_code: None,
            payload: Some(serde_json::json!({
                "recordingId": recording_id.into(),
                "rms": rms.clamp(0.0, 1.0),
                "peak": peak.clamp(0.0, 1.0),
            })),
            ts_ms: now_ms(),
        }
    }

    pub fn state_failed(
        task_id: impl Into<String>,
        stage: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind: "workflow.task.failed".to_string(),
            effect: "stateChanging".to_string(),
            event_id: new_event_id(),
            sequence: next_sequence(),
            task_id: Some(task_id.into()),
            stage: Some(stage.into()),
            status: Some("failed".to_string()),
            message: message.into(),
            elapsed_ms: None,
            error_code: Some(code.into()),
            payload: None,
            ts_ms: now_ms(),
        }
    }

    pub fn state_cancelled(task_id: impl Into<String>, stage: impl Into<String>) -> Self {
        Self {
            kind: "workflow.task.cancelled".to_string(),
            effect: "stateChanging".to_string(),
            event_id: new_event_id(),
            sequence: next_sequence(),
            task_id: Some(task_id.into()),
            stage: Some(stage.into()),
            status: Some("cancelled".to_string()),
            message: "cancelled".to_string(),
            elapsed_ms: None,
            error_code: None,
            payload: None,
            ts_ms: now_ms(),
        }
    }
}

#[derive(Clone)]
pub struct UiEventMailbox {
    tx: mpsc::Sender<UiEvent>,
}

impl UiEventMailbox {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel::<UiEvent>();
        std::thread::Builder::new()
            .name("ui_event_actor".to_string())
            .spawn(move || {
                while let Ok(event) = rx.recv() {
                    let overlay = overlay_state_from_event(&event);
                    let _ = app.emit(UI_EVENT_CHANNEL, event);
                    if let Some(state) = overlay {
                        apply_overlay_state(&app, state);
                    }
                }
            })
            .expect("failed to start ui event actor");
        Self { tx }
    }

    pub fn send(&self, event: UiEvent) {
        let _ = self.tx.send(event);
    }
}

fn overlay_state_from_event(event: &UiEvent) -> Option<OverlayState> {
    if event.kind != "workflow.state" {
        return None;
    }
    if !overlay_enabled() {
        return Some(OverlayState {
            visible: false,
            status: "IDLE".to_string(),
            detail: None,
            ts_ms: now_ms(),
        });
    }
    let payload = event.payload.as_ref()?.as_object()?;
    let phase = payload
        .get("phase")
        .and_then(|v| v.as_str())
        .unwrap_or("idle");
    let diagnostic_code = payload
        .get("diagnosticCode")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .map(ToOwned::to_owned);
    let (visible, status, detail) = match phase {
        "recording" => (true, "REC".to_string(), None),
        "transcribing" => (true, "TRANSCRIBING".to_string(), None),
        "rewriting" => (true, "REWRITING".to_string(), None),
        "inserting" => (true, "INSERTING".to_string(), None),
        _ => (false, "IDLE".to_string(), diagnostic_code),
    };
    Some(OverlayState {
        visible,
        status,
        detail,
        ts_ms: now_ms(),
    })
}

fn overlay_enabled() -> bool {
    let Ok(dir) = crate::data_dir::data_dir() else {
        return false;
    };
    let Ok(s) = crate::settings::load_settings_strict(&dir) else {
        return false;
    };
    s.hotkeys_show_overlay.unwrap_or(false)
}

fn apply_overlay_state(app: &AppHandle, state: OverlayState) {
    if let Some(w) = app.get_webview_window("overlay") {
        if state.visible {
            let _ = w.show();
        } else {
            let _ = w.hide();
        }
    }
    let _ = app.emit("tv_overlay_state", state);
}

fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => dur.as_millis() as i64,
        Err(_) => 0,
    }
}

fn new_event_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn next_sequence() -> u64 {
    EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_events_use_single_frontend_channel() {
        let event = UiEvent::stage("task-1", "Transcribe", UiEventStatus::Started, "asr(local)");

        assert_eq!(UI_EVENT_CHANNEL, "ui_event");
        assert_eq!(event.kind, "transcription.stage");
        assert_eq!(event.task_id.as_deref(), Some("task-1"));
    }

    #[test]
    fn error_events_keep_code_and_message() {
        let event = UiEvent::error("task-1", "E_ASR_FAILED", "asr failed");

        assert_eq!(event.kind, "diagnostic.error");
        assert_eq!(event.error_code.as_deref(), Some("E_ASR_FAILED"));
        assert_eq!(event.message, "asr failed");
    }
}
