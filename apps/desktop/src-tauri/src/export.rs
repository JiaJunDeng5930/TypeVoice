use std::cmp;

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

pub async fn auto_paste_text(text: &str) -> Result<(), ExportError> {
    if text.trim().is_empty() {
        return Err(ExportError::new(
            "E_EXPORT_EMPTY_TEXT",
            "empty text cannot be exported",
        ));
    }

    #[cfg(windows)]
    {
        return windows::auto_paste_text();
    }

    #[cfg(target_os = "linux")]
    {
        return linux::auto_paste_text(text).await;
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(ExportError::new(
            "E_EXPORT_PASTE_UNSUPPORTED",
            "auto paste is only supported on Linux and Windows",
        ))
    }
}

#[cfg(windows)]
mod windows {
    use super::ExportError;
    use std::mem;
    use windows_sys::Win32::Foundation::{GetLastError, HWND};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsWindow,
        SendMessageTimeoutW, GUITHREADINFO, SMTO_ABORTIFHUNG, WM_PASTE,
    };

    pub fn auto_paste_text() -> Result<(), ExportError> {
        let target = resolve_foreground_focus_window().ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "no focused foreground window available for auto paste",
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

        let mut result: usize = 0;
        let ok = unsafe {
            SendMessageTimeoutW(target.hwnd, WM_PASTE, 0, 0, SMTO_ABORTIFHUNG, 1200, &mut result)
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(ExportError::new(
                "E_EXPORT_PASTE_FAILED",
                format!(
                    "SendMessageTimeoutW(WM_PASTE) failed: last_error={err}, focus_hwnd={:p}, foreground_hwnd={:p}, foreground_pid={}, focus_pid={}",
                    target.hwnd, target.foreground_hwnd, target.foreground_pid, target.focus_pid
                ),
            ));
        }
        Ok(())
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
    use super::{cmp, ExportError};
    use atspi::proxy::accessible::ObjectRefExt;
    use atspi::proxy::proxy_ext::ProxyExt;
    use atspi::{AccessibilityConnection, Interface, ObjectRefOwned, State};

    const MAX_TRAVERSE_NODES: usize = 2048;

    pub async fn auto_paste_text(text: &str) -> Result<(), ExportError> {
        let conn = AccessibilityConnection::new().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_PASTE_UNAVAILABLE",
                format!("failed to connect to AT-SPI bus: {e}"),
            )
        })?;

        let target = find_focused_editable_object(&conn).await?.ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_TARGET_NOT_EDITABLE",
                "focused editable target not found via AT-SPI",
            )
        })?;

        let accessible = target
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

        if let Ok(component) = proxies.component().await {
            let _ = component.grab_focus().await;
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

        let ok = editable
            .insert_text(insert_pos, text, utf8_char_count_i32(text))
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

    fn utf8_char_count_i32(text: &str) -> i32 {
        let n = text.chars().count();
        cmp::min(n, i32::MAX as usize) as i32
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
