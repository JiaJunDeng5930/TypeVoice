use serde::{Deserialize, Serialize};

use crate::ports::{PortError, PortResult};
use crate::{data_dir, export, obs, settings};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertTextRequest {
    pub transcript_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertResult {
    pub copied: bool,
    pub auto_paste_attempted: bool,
    pub auto_paste_ok: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl InsertResult {
    pub fn copy_only() -> Self {
        Self {
            copied: true,
            auto_paste_attempted: false,
            auto_paste_ok: true,
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

pub async fn insert_text(req: InsertTextRequest) -> PortResult<InsertResult> {
    let dir =
        data_dir::data_dir().map_err(|e| PortError::from_message("E_DATA_DIR", e.to_string()))?;
    let span = obs::Span::start(
        &dir,
        req.transcript_id.as_deref(),
        "Cmd",
        "CMD.insert_text",
        Some(serde_json::json!({
            "chars": req.text.chars().count(),
            "has_transcript_id": req.transcript_id.as_deref().map(|v| !v.is_empty()).unwrap_or(false),
        })),
    );

    if let Err(e) = export::copy_text_to_clipboard(&req.text) {
        span.err("insert", &e.code, &e.message, None);
        return Err(PortError::new(&e.code, e.message));
    }

    let current_settings = settings::load_settings_strict(&dir)
        .map_err(|e| PortError::from_message("E_SETTINGS_INVALID", e.to_string()))?;
    if !settings::resolve_auto_paste_enabled(&current_settings) {
        span.ok(Some(serde_json::json!({
            "copied": true,
            "auto_paste_enabled": false,
            "auto_paste_attempted": false,
        })));
        return Ok(InsertResult::copy_only());
    }

    match export::auto_paste_text(&req.text).await {
        Ok(()) => {
            span.ok(Some(serde_json::json!({
                "copied": true,
                "auto_paste_enabled": true,
                "auto_paste_attempted": true,
                "auto_paste_ok": true,
            })));
            Ok(InsertResult::pasted())
        }
        Err(e) => {
            span.err(
                "insert",
                &e.code,
                &e.message,
                Some(serde_json::json!({
                    "copied": true,
                    "auto_paste_enabled": true,
                    "auto_paste_attempted": true,
                })),
            );
            Ok(InsertResult::paste_failed(&e.code, e.message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_result_preserves_copy_success_when_paste_fails() {
        let result = InsertResult::paste_failed("E_EXPORT_PASTE_FAILED", "target unavailable");

        assert!(result.copied);
        assert!(result.auto_paste_attempted);
        assert!(!result.auto_paste_ok);
        assert_eq!(result.error_code.as_deref(), Some("E_EXPORT_PASTE_FAILED"));
    }
}
