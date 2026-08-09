#[cfg(test)]
use std::future::Future;

#[cfg(test)]
use crate::export;
#[cfg(test)]
use crate::ports::{PortError, PortResult};
#[cfg(test)]
use typevoice_core::workflow::InsertResult;

#[cfg(test)]
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
    #[ignore = "requires an isolated interactive Windows desktop"]
    async fn target_contract_t23_insertion_port_contract_windows() {
        assert_shared_insertion_port_contract().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            export::native_input_contract_probe("TypeVoice 世界\n"),
        )
        .await
        .expect("native Windows input contract timed out")
        .expect("native Windows input contract failed");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires an isolated Linux display and accessibility bus"]
    async fn target_contract_t23_insertion_port_contract_linux() {
        assert_shared_insertion_port_contract().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            export::native_input_contract_probe("TypeVoice 世界\n"),
        )
        .await
        .expect("native Linux input contract timed out")
        .expect("native Linux input contract failed");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires an isolated interactive macOS session with Accessibility permission"]
    async fn target_contract_t23_insertion_port_contract_macos() {
        assert_shared_insertion_port_contract().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            export::native_input_contract_probe("TypeVoice 世界\n"),
        )
        .await
        .expect("native macOS input contract timed out")
        .expect("native macOS input contract failed");
    }
}
