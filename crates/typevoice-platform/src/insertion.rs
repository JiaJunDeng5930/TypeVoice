use serde::{Deserialize, Serialize};
use std::future::Future;

use crate::ports::{PortError, PortResult};
use crate::{data_dir, export, obs, settings};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertTextRequest {
    pub transcript_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    insert_text_after_focus(req, None).await
}

pub async fn insert_text_after_focus(
    req: InsertTextRequest,
    target_hwnd: Option<isize>,
) -> PortResult<InsertResult> {
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

    let result = run_insertion_contract(
        &req.text,
        |text| export::copy_text_to_clipboard(text).map_err(|e| PortError::new(&e.code, e.message)),
        || {
            let current_settings = settings::load_settings_strict(&dir)
                .map_err(|e| PortError::from_message("E_SETTINGS_INVALID", e.to_string()))?;
            Ok(settings::resolve_auto_paste_enabled(&current_settings))
        },
        |text| async move {
            let _ = export::focus_window_best_effort(target_hwnd);
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            export::auto_paste_text(text).await
        },
    )
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if error.code != "E_SETTINGS_INVALID" {
                span.err("insert", &error.code, &error.message, None);
            }
            return Err(error);
        }
    };

    if !result.auto_paste_attempted {
        span.ok(Some(serde_json::json!({
            "copied": true,
            "auto_paste_enabled": false,
            "auto_paste_attempted": false,
        })));
        return Ok(result);
    }

    if result.auto_paste_ok {
        span.ok(Some(serde_json::json!({
            "copied": true,
            "auto_paste_enabled": true,
            "auto_paste_attempted": true,
            "auto_paste_ok": true,
        })));
    } else {
        span.err(
            "insert",
            result
                .error_code
                .as_deref()
                .unwrap_or("E_EXPORT_PASTE_FAILED"),
            result
                .error_message
                .as_deref()
                .unwrap_or("native input failed"),
            Some(serde_json::json!({
                "copied": true,
                "auto_paste_enabled": true,
                "auto_paste_attempted": true,
            })),
        );
    }

    Ok(result)
}

async fn run_insertion_contract<'a, Copy, ResolveAutoPaste, Paste, PasteFuture>(
    text: &'a str,
    copy: Copy,
    resolve_auto_paste: ResolveAutoPaste,
    paste: Paste,
) -> PortResult<InsertResult>
where
    Copy: FnOnce(&'a str) -> PortResult<()>,
    ResolveAutoPaste: FnOnce() -> PortResult<bool>,
    Paste: FnOnce(&'a str) -> PasteFuture,
    PasteFuture: Future<Output = Result<(), export::ExportError>>,
{
    copy(text)?;
    if !resolve_auto_paste()? {
        return Ok(InsertResult::copy_only());
    }

    match paste(text).await {
        Ok(()) => Ok(InsertResult::pasted()),
        Err(error) => Ok(InsertResult::paste_failed(&error.code, error.message)),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    #[test]
    fn insert_result_preserves_copy_success_when_paste_fails() {
        let result = InsertResult::paste_failed("E_EXPORT_PASTE_FAILED", "target unavailable");

        assert!(result.copied);
        assert!(result.auto_paste_attempted);
        assert!(!result.auto_paste_ok);
        assert_eq!(result.error_code.as_deref(), Some("E_EXPORT_PASTE_FAILED"));
    }

    async fn assert_shared_insertion_port_contract() {
        const CONTRACT_TEXT: &str = "TypeVoice 世界\n";
        let disabled_trace = Rc::new(RefCell::new(Vec::new()));
        let trace = disabled_trace.clone();
        let settings_trace = disabled_trace.clone();
        let paste_trace = disabled_trace.clone();
        let disabled = run_insertion_contract(
            CONTRACT_TEXT,
            move |text| {
                trace.borrow_mut().push(format!("copy:{text}"));
                Ok(())
            },
            move || {
                settings_trace.borrow_mut().push("settings".to_string());
                Ok(false)
            },
            move |text| async move {
                paste_trace.borrow_mut().push(format!("paste:{text}"));
                Ok(())
            },
        )
        .await
        .expect("copy-only contract must succeed");
        assert_eq!(
            &*disabled_trace.borrow(),
            &[format!("copy:{CONTRACT_TEXT}"), "settings".to_string()]
        );
        assert_eq!(disabled, InsertResult::copy_only());

        let enabled_trace = Rc::new(RefCell::new(Vec::new()));
        let trace = enabled_trace.clone();
        let settings_trace = enabled_trace.clone();
        let paste_trace = enabled_trace.clone();
        let enabled = run_insertion_contract(
            CONTRACT_TEXT,
            move |text| {
                trace.borrow_mut().push(format!("copy:{text}"));
                Ok(())
            },
            move || {
                settings_trace.borrow_mut().push("settings".to_string());
                Ok(true)
            },
            move |text| async move {
                paste_trace.borrow_mut().push(format!("paste:{text}"));
                Ok(())
            },
        )
        .await
        .expect("copy-and-paste contract must succeed");
        assert_eq!(
            &*enabled_trace.borrow(),
            &[
                format!("copy:{CONTRACT_TEXT}"),
                "settings".to_string(),
                format!("paste:{CONTRACT_TEXT}"),
            ]
        );
        assert_eq!(enabled, InsertResult::pasted());

        let paste_failed = run_insertion_contract(
            CONTRACT_TEXT,
            |text| {
                assert_eq!(text, CONTRACT_TEXT);
                Ok(())
            },
            || Ok(true),
            |text| async move {
                assert_eq!(text, CONTRACT_TEXT);
                Err(export::ExportError::new(
                    "E_EXPORT_PASTE_FAILED",
                    "scripted native input failure",
                ))
            },
        )
        .await
        .expect("paste failure remains a completed copy with warning");
        assert_eq!(
            paste_failed,
            InsertResult::paste_failed("E_EXPORT_PASTE_FAILED", "scripted native input failure")
        );

        let paste_called = Rc::new(RefCell::new(false));
        let observed = paste_called.clone();
        let copy_failed = run_insertion_contract(
            CONTRACT_TEXT,
            |text| {
                assert_eq!(text, CONTRACT_TEXT);
                Err(PortError::new(
                    "E_EXPORT_COPY_FAILED",
                    "scripted copy failure",
                ))
            },
            || Ok(true),
            move |text| async move {
                assert_eq!(text, CONTRACT_TEXT);
                *observed.borrow_mut() = true;
                Ok(())
            },
        )
        .await;
        assert_eq!(
            copy_failed
                .expect_err("copy failure must remain terminal")
                .code,
            "E_EXPORT_COPY_FAILED"
        );
        assert!(
            !*paste_called.borrow(),
            "paste must not run after copy failure"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn target_contract_t23_insertion_port_contract_windows() {
        assert_shared_insertion_port_contract().await;
        assert!(export::native_input_contract_probe("TypeVoice 世界\n").await);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn target_contract_t23_insertion_port_contract_linux() {
        assert_shared_insertion_port_contract().await;
        assert!(export::native_input_contract_probe("TypeVoice 世界\n").await);
    }
}
