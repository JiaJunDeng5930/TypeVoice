#[derive(Debug, Clone)]
pub struct ExportError {
    pub code: String,
    pub message: String,
}

impl ExportError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::InsertionTarget;
#[cfg(windows)]
pub use windows::InsertionTarget;

#[cfg(not(any(windows, target_os = "linux")))]
#[derive(Debug, Clone)]
pub struct InsertionTarget;

pub fn copy_text_to_clipboard(text: &str) -> Result<(), ExportError> {
    if text.trim().is_empty() {
        return Err(ExportError::new(
            "E_EXPORT_EMPTY_TEXT",
            "empty text cannot be exported",
        ));
    }

    let mut clipboard = arboard::Clipboard::new().map_err(|e| {
        ExportError::new(
            "E_EXPORT_CLIPBOARD_UNAVAILABLE",
            format!("clipboard init failed: {e}"),
        )
    })?;

    clipboard.set_text(text.to_string()).map_err(|e| {
        ExportError::new(
            "E_EXPORT_COPY_FAILED",
            format!("clipboard write failed: {e}"),
        )
    })
}

pub async fn capture_insertion_target() -> Result<InsertionTarget, ExportError> {
    #[cfg(windows)]
    {
        windows::capture_insertion_target()
    }

    #[cfg(target_os = "linux")]
    {
        linux::capture_insertion_target().await
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(ExportError::new(
            "E_EXPORT_TARGET_UNSUPPORTED",
            "insertion target capture is only supported on Linux and Windows",
        ))
    }
}

pub async fn auto_paste_text(target: &InsertionTarget, text: &str) -> Result<(), ExportError> {
    if text.trim().is_empty() {
        return Err(ExportError::new(
            "E_EXPORT_EMPTY_TEXT",
            "empty text cannot be exported",
        ));
    }

    #[cfg(windows)]
    {
        windows::auto_input_text(target, text)
    }

    #[cfg(target_os = "linux")]
    {
        linux::auto_input_text(target, text).await
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(ExportError::new(
            "E_EXPORT_PASTE_UNSUPPORTED",
            "auto input is only supported on Linux and Windows",
        ))
    }
}

#[cfg(test)]
pub(crate) async fn native_input_contract_probe(text: &str) -> bool {
    #[cfg(windows)]
    {
        windows::native_input_contract_probe(text)
    }
    #[cfg(target_os = "linux")]
    {
        linux::native_input_contract_probe(text).await
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = text;
        false
    }
}

#[cfg(windows)]
fn utf16_code_units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[cfg(windows)]
mod windows {
    use super::{utf16_code_units, ExportError};
    use std::mem::{self, size_of};
    use windows_sys::Win32::Foundation::{GetLastError, HWND};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsWindow,
        SetForegroundWindow, GUITHREADINFO,
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InsertionTarget {
        foreground_hwnd: isize,
        focus_hwnd: isize,
        foreground_pid: u32,
        focus_pid: u32,
    }

    pub fn capture_insertion_target() -> Result<InsertionTarget, ExportError> {
        let target = resolve_external_focus_target()?;
        Ok(InsertionTarget {
            foreground_hwnd: target.foreground_hwnd as isize,
            focus_hwnd: target.hwnd as isize,
            foreground_pid: target.foreground_pid,
            focus_pid: target.focus_pid,
        })
    }

    pub fn auto_input_text(expected: &InsertionTarget, text: &str) -> Result<(), ExportError> {
        let expected_foreground = expected.foreground_hwnd as HWND;
        if expected_foreground.is_null() || unsafe { IsWindow(expected_foreground) } == 0 {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "the frozen insertion target no longer exists",
            ));
        }
        let mut target = resolve_external_focus_target().ok();
        if !target_matches(target.as_ref(), expected) {
            let _ = unsafe { SetForegroundWindow(expected_foreground) };
            target = resolve_external_focus_target().ok();
        }
        let _validated_target = target
            .filter(|current| target_matches(Some(current), expected))
            .ok_or_else(|| {
                ExportError::new(
                    "E_EXPORT_TARGET_MISMATCH",
                    "current focus no longer matches the frozen insertion target",
                )
            })?;

        let target = resolve_external_focus_target()
            .ok()
            .filter(|current| target_matches(Some(current), expected))
            .ok_or_else(|| {
                ExportError::new(
                    "E_EXPORT_TARGET_MISMATCH",
                    "focus changed immediately before native Unicode input",
                )
            })?;
        let (expected_count, sent) = dispatch_unicode_inputs(text, |inputs| unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        });
        if sent != expected_count {
            let err = unsafe { GetLastError() };
            return Err(ExportError::new(
                "E_EXPORT_PASTE_FAILED",
                format!(
                    "SendInput(unicode) failed: last_error={err}, sent={sent}, expected={expected_count}, focus_hwnd={:p}, foreground_hwnd={:p}, foreground_pid={}, focus_pid={}",
                    target.hwnd, target.foreground_hwnd, target.foreground_pid, target.focus_pid,
                ),
            ));
        }
        Ok(())
    }

    fn target_matches(current: Option<&ForegroundFocusTarget>, expected: &InsertionTarget) -> bool {
        current.is_some_and(|current| {
            current.foreground_hwnd as isize == expected.foreground_hwnd
                && current.hwnd as isize == expected.focus_hwnd
                && current.foreground_pid == expected.foreground_pid
                && current.focus_pid == expected.focus_pid
        })
    }

    fn resolve_external_focus_target() -> Result<ForegroundFocusTarget, ExportError> {
        let target = resolve_foreground_focus_window().ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "no focused foreground window available for auto input",
            )
        })?;
        if target.foreground_pid == target.self_pid || target.focus_pid == target.self_pid {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!(
                    "focused target belongs to TypeVoice process (foreground_pid={}, focus_pid={}, self_pid={})",
                    target.foreground_pid, target.focus_pid, target.self_pid
                ),
            ));
        }
        Ok(target)
    }

    #[cfg(test)]
    pub(super) fn native_input_contract_probe(text: &str) -> bool {
        let calls = std::cell::Cell::new(0_u32);
        let valid_unicode_input = std::cell::Cell::new(false);
        let (expected, sent) = dispatch_unicode_inputs(text, |inputs| {
            calls.set(calls.get() + 1);
            valid_unicode_input.set(
                !inputs.is_empty()
                    && inputs.len() == super::utf16_code_units(text).len() * 2
                    && inputs.iter().all(|input| {
                        let keyboard = unsafe { input.Anonymous.ki };
                        keyboard.wVk == 0 && keyboard.dwFlags & KEYEVENTF_UNICODE != 0
                    }),
            );
            inputs.len() as u32
        });
        calls.get() == 1 && expected == sent && valid_unicode_input.get()
    }

    fn dispatch_unicode_inputs<Send>(text: &str, send: Send) -> (u32, u32)
    where
        Send: FnOnce(&[INPUT]) -> u32,
    {
        let inputs = build_unicode_key_inputs(text);
        let expected = inputs.len() as u32;
        let sent = send(&inputs);
        (expected, sent)
    }

    fn build_unicode_key_inputs(text: &str) -> Vec<INPUT> {
        let units = utf16_code_units(text);
        let mut inputs = Vec::with_capacity(units.len() * 2);
        for unit in units {
            inputs.push(key_input(unit, KEYEVENTF_UNICODE));
            inputs.push(key_input(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        inputs
    }

    fn key_input(scan: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    struct ForegroundFocusTarget {
        hwnd: HWND,
        foreground_hwnd: HWND,
        foreground_pid: u32,
        focus_pid: u32,
        self_pid: u32,
    }

    fn resolve_foreground_focus_window() -> Option<ForegroundFocusTarget> {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_null() || unsafe { IsWindow(foreground) } == 0 {
            return None;
        }
        let mut foreground_pid: u32 = 0;
        let thread_id = unsafe { GetWindowThreadProcessId(foreground, &mut foreground_pid) };
        if thread_id == 0 {
            return None;
        }
        let mut info: GUITHREADINFO = unsafe { mem::zeroed() };
        info.cbSize = mem::size_of::<GUITHREADINFO>() as u32;
        let ok = unsafe { GetGUIThreadInfo(thread_id, &mut info) };
        if ok == 0 || info.hwndFocus.is_null() {
            return None;
        }
        let focus = info.hwndFocus;
        if unsafe { IsWindow(focus) } == 0 {
            return None;
        }
        let mut focus_pid: u32 = 0;
        let _ = unsafe { GetWindowThreadProcessId(focus, &mut focus_pid) };
        let self_pid = unsafe { GetCurrentProcessId() };
        Some(ForegroundFocusTarget {
            hwnd: focus,
            foreground_hwnd: foreground,
            foreground_pid,
            focus_pid,
            self_pid,
        })
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::ExportError;
    use atspi::proxy::accessible::ObjectRefExt;
    use atspi::proxy::proxy_ext::ProxyExt;
    use atspi::{AccessibilityConnection, Interface, ObjectRefOwned, State};
    use std::{cmp, future::Future};

    const MAX_TRAVERSE_NODES: usize = 2048;

    #[derive(Clone)]
    pub struct InsertionTarget {
        object: ObjectRefOwned,
    }

    pub async fn capture_insertion_target() -> Result<InsertionTarget, ExportError> {
        let conn = connection().await?;
        let object = find_focused_editable_object(&conn).await?.ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_TARGET_NOT_EDITABLE",
                "focused editable target not found via AT-SPI",
            )
        })?;
        Ok(InsertionTarget { object })
    }

    async fn connection() -> Result<AccessibilityConnection, ExportError> {
        AccessibilityConnection::new().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_PASTE_UNAVAILABLE",
                format!("failed to connect to AT-SPI bus: {e}"),
            )
        })
    }

    pub async fn auto_input_text(
        expected: &InsertionTarget,
        text: &str,
    ) -> Result<(), ExportError> {
        let conn = connection().await?;
        let current = find_focused_editable_object(&conn).await?.ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_TARGET_NOT_EDITABLE",
                "focused editable target not found via AT-SPI",
            )
        })?;
        if current != expected.object {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_MISMATCH",
                "current focus no longer matches the frozen insertion target",
            ));
        }

        let accessible = expected
            .object
            .as_accessible_proxy(conn.connection())
            .await
            .map_err(|e| {
                ExportError::new(
                    "E_EXPORT_TARGET_UNAVAILABLE",
                    format!("failed to resolve focused object proxy: {e}"),
                )
            })?;

        let proxies = accessible.proxies().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!("failed to enumerate target interfaces: {e}"),
            )
        })?;

        let state = accessible.get_state().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!("failed to revalidate target focus: {e}"),
            )
        })?;
        if !state.contains(State::Focused) {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_MISMATCH",
                "the frozen insertion target lost focus before insertion",
            ));
        }

        let editable = proxies.editable_text().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_NOT_EDITABLE",
                format!("EditableText interface unavailable: {e}"),
            )
        })?;

        let insert_pos = match proxies.text().await {
            Ok(text_proxy) => text_proxy.caret_offset().await.unwrap_or(0).max(0),
            Err(_) => 0,
        };

        let ok = dispatch_editable_insert(insert_pos, text, |position, text, character_count| {
            editable.insert_text(position, text, character_count)
        })
        .await
        .map_err(|e| {
            ExportError::new(
                "E_EXPORT_PASTE_FAILED",
                format!("EditableText.InsertText call failed: {e}"),
            )
        })?;

        if !ok {
            return Err(ExportError::new(
                "E_EXPORT_PASTE_FAILED",
                "EditableText.InsertText returned false",
            ));
        }

        Ok(())
    }

    async fn dispatch_editable_insert<'a, Call, CallFuture, CallError>(
        insert_pos: i32,
        text: &'a str,
        call: Call,
    ) -> Result<bool, CallError>
    where
        Call: FnOnce(i32, &'a str, i32) -> CallFuture,
        CallFuture: Future<Output = Result<bool, CallError>>,
    {
        call(insert_pos, text, utf8_char_count_i32(text)).await
    }

    fn utf8_char_count_i32(text: &str) -> i32 {
        let n = text.chars().count();
        cmp::min(n, i32::MAX as usize) as i32
    }

    #[cfg(test)]
    pub(super) async fn native_input_contract_probe(text: &str) -> bool {
        let calls = std::cell::Cell::new(0_u32);
        let observed = std::cell::RefCell::new(None);
        let inserted = dispatch_editable_insert(7, text, |position, actual, character_count| {
            calls.set(calls.get() + 1);
            *observed.borrow_mut() = Some((position, actual.to_string(), character_count));
            std::future::ready(Ok::<bool, ()>(true))
        })
        .await;
        let expected_count = text.chars().count() as i32;
        calls.get() == 1
            && inserted == Ok(true)
            && observed.borrow().as_ref() == Some(&(7, text.to_string(), expected_count))
    }

    async fn find_focused_editable_object(
        conn: &AccessibilityConnection,
    ) -> Result<Option<ObjectRefOwned>, ExportError> {
        let root = conn.root_accessible_on_registry().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_PASTE_UNAVAILABLE",
                format!("failed to access AT-SPI registry root: {e}"),
            )
        })?;

        let mut stack = root.get_children().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_PASTE_UNAVAILABLE",
                format!("failed to query AT-SPI applications: {e}"),
            )
        })?;

        let mut visited = 0usize;
        while let Some(node) = stack.pop() {
            if visited >= MAX_TRAVERSE_NODES {
                break;
            }
            visited += 1;

            if node.is_null() {
                continue;
            }

            let accessible = match node.as_accessible_proxy(conn.connection()).await {
                Ok(v) => v,
                Err(_) => continue,
            };

            let interfaces = match accessible.get_interfaces().await {
                Ok(v) => v,
                Err(_) => continue,
            };

            let state = match accessible.get_state().await {
                Ok(v) => v,
                Err(_) => continue,
            };

            if interfaces.contains(Interface::EditableText) && state.contains(State::Focused) {
                return Ok(Some(node));
            }

            if let Ok(children) = accessible.get_children().await {
                for child in children {
                    if !child.is_null() {
                        stack.push(child);
                    }
                }
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::utf16_code_units;

    #[test]
    fn utf16_code_units_preserve_newline() {
        assert_eq!(utf16_code_units("a\nb"), vec![0x0061, 0x000A, 0x0062]);
    }

    #[test]
    fn utf16_code_units_support_surrogate_pairs() {
        assert_eq!(utf16_code_units("😀").len(), 2);
    }
}
