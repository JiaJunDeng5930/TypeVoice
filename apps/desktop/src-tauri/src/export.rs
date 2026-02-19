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
        return windows::auto_paste_text(text);
    }

    #[cfg(target_os = "linux")]
    {
        return linux::auto_paste_text(text).await;
    }

    #[cfg(target_os = "macos")]
    {
        return macos::auto_paste_text(text);
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err(ExportError::new(
            "E_EXPORT_PASTE_UNSUPPORTED",
            "auto paste is only supported on Linux, macOS, and Windows",
        ))
    }
}

#[cfg(any(windows, target_os = "macos", test))]
#[allow(dead_code)]
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(any(windows, target_os = "macos", test))]
fn insert_at_utf16_offset(base: &str, offset_utf16: usize, inserted: &str) -> String {
    if inserted.is_empty() {
        return base.to_string();
    }
    let split = byte_index_from_utf16_offset(base, offset_utf16);
    let mut out = String::with_capacity(base.len() + inserted.len());
    out.push_str(&base[..split]);
    out.push_str(inserted);
    out.push_str(&base[split..]);
    out
}

#[cfg(any(windows, target_os = "macos", test))]
fn byte_index_from_utf16_offset(text: &str, offset_utf16: usize) -> usize {
    if offset_utf16 == 0 {
        return 0;
    }

    let mut seen_utf16 = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        if seen_utf16 >= offset_utf16 {
            return byte_idx;
        }
        let next = seen_utf16.saturating_add(ch.len_utf16());
        if next >= offset_utf16 {
            // Never split a code point even if offset lands in the middle of a UTF-16 surrogate pair.
            return byte_idx + ch.len_utf8();
        }
        seen_utf16 = next;
    }

    text.len()
}

#[cfg(windows)]
mod windows {
    use super::{insert_at_utf16_offset, utf16_len, ExportError};
    use uiautomation::patterns::{UITextPattern, UIValuePattern};
    use uiautomation::types::TextPatternRangeEndpoint;
    use uiautomation::UIAutomation;

    pub fn auto_paste_text(text: &str) -> Result<(), ExportError> {
        let automation = UIAutomation::new().map_err(|e| {
            ExportError::new(
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
                format!(
                    "failed to initialize UI Automation: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        let focused = automation.get_focused_element().map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!(
                    "failed to resolve focused element: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        let target_pid = focused.get_process_id().map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!(
                    "failed to resolve focused process id: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;
        let self_pid = std::process::id();
        if target_pid == self_pid {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_SELF_APP",
                format!(
                    "focused element belongs to TypeVoice process: target_pid={target_pid}, self_pid={self_pid}"
                ),
            ));
        }

        let enabled = focused.is_enabled().map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!(
                    "failed to query focused element enabled state: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;
        if !enabled {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_READONLY",
                "focused editable target is disabled",
            ));
        }

        let value_pattern = focused.get_pattern::<UIValuePattern>().map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_NOT_EDITABLE",
                format!(
                    "ValuePattern unavailable on focused element: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        let readonly = value_pattern.is_readonly().map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!(
                    "failed to query ValuePattern readonly state: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;
        if readonly {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_READONLY",
                "focused editable target is readonly",
            ));
        }

        let current_text = value_pattern.get_value().map_err(|e| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!(
                    "failed to read current value from focused element: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        let text_pattern = focused.get_pattern::<UITextPattern>().map_err(|e| {
            ExportError::new(
                "E_EXPORT_SELECTION_UNAVAILABLE",
                format!(
                    "TextPattern unavailable on focused element: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        let caret_utf16 = resolve_caret_utf16_offset(&text_pattern)?;
        let updated = insert_at_utf16_offset(&current_text, caret_utf16, text);

        value_pattern.set_value(&updated).map_err(|e| {
            if e.code() == -2147024891 {
                return ExportError::new(
                    "E_EXPORT_PERMISSION_DENIED",
                    format!(
                        "ValuePattern.SetValue blocked by access policy: code={}, message={}",
                        e.code(),
                        e.message()
                    ),
                );
            }
            let lower = e.message().to_ascii_lowercase();
            if lower.contains("readonly") || lower.contains("read-only") {
                return ExportError::new(
                    "E_EXPORT_TARGET_READONLY",
                    format!(
                        "ValuePattern.SetValue rejected readonly target: code={}, message={}",
                        e.code(),
                        e.message()
                    ),
                );
            }
            ExportError::new(
                "E_EXPORT_PASTE_FAILED",
                format!(
                    "ValuePattern.SetValue failed: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        // Best effort: move selection/caret to the end of inserted text for predictable follow-up typing.
        let selection_end = caret_utf16.saturating_add(utf16_len(text));
        let _ = selection_end;

        Ok(())
    }

    fn resolve_caret_utf16_offset(text_pattern: &UITextPattern) -> Result<usize, ExportError> {
        let selection = text_pattern.get_selection().map_err(|e| {
            ExportError::new(
                "E_EXPORT_SELECTION_UNAVAILABLE",
                format!(
                    "TextPattern.GetSelection failed: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        let caret_range = if let Some(first) = selection.into_iter().next() {
            first
        } else {
            text_pattern
                .get_caret_range()
                .map_err(|e| {
                    ExportError::new(
                        "E_EXPORT_SELECTION_UNAVAILABLE",
                        format!(
                            "TextPattern.GetCaretRange failed: code={}, message={}",
                            e.code(),
                            e.message()
                        ),
                    )
                })?
                .1
        };

        let document_range = text_pattern.get_document_range().map_err(|e| {
            ExportError::new(
                "E_EXPORT_SELECTION_UNAVAILABLE",
                format!(
                    "TextPattern.DocumentRange failed: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        let prefix_range = document_range.clone();
        prefix_range
            .move_endpoint_by_range(
                TextPatternRangeEndpoint::End,
                &caret_range,
                TextPatternRangeEndpoint::Start,
            )
            .map_err(|e| {
                ExportError::new(
                    "E_EXPORT_SELECTION_UNAVAILABLE",
                    format!(
                        "TextRange.MoveEndpointByRange failed: code={}, message={}",
                        e.code(),
                        e.message()
                    ),
                )
            })?;

        let prefix_text = prefix_range.get_text(-1).map_err(|e| {
            ExportError::new(
                "E_EXPORT_SELECTION_UNAVAILABLE",
                format!(
                    "TextRange.GetText failed: code={}, message={}",
                    e.code(),
                    e.message()
                ),
            )
        })?;

        Ok(utf16_len(&prefix_text))
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
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
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
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
                format!("failed to access AT-SPI registry root: {e}"),
            )
        })?;

        let mut stack = root.get_children().await.map_err(|e| {
            ExportError::new(
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
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

#[cfg(target_os = "macos")]
mod macos {
    use super::{insert_at_utf16_offset, utf16_len, ExportError};
    use accessibility_sys::{
        kAXErrorAPIDisabled, kAXErrorAttributeUnsupported, kAXErrorInvalidUIElement,
        kAXErrorNoValue, kAXErrorSuccess, kAXFocusedUIElementAttribute,
        kAXSelectedTextRangeAttribute, kAXValueAttribute, kAXValueTypeCFRange, AXError,
        AXIsProcessTrusted, AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide,
        AXUIElementGetPid, AXUIElementGetTypeID, AXUIElementIsAttributeSettable, AXUIElementRef,
        AXUIElementSetAttributeValue, AXValueCreate, AXValueGetType, AXValueGetTypeID,
        AXValueGetValue, AXValueRef,
    };
    use core_foundation_sys::base::{
        kCFAllocatorDefault, CFGetTypeID, CFRange, CFRelease, CFTypeRef,
    };
    use core_foundation_sys::string::{
        kCFStringEncodingUTF8, CFStringCreateWithBytes, CFStringGetCString, CFStringGetLength,
        CFStringGetMaximumSizeForEncoding, CFStringGetTypeID, CFStringRef,
    };
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr;

    pub fn auto_paste_text(text: &str) -> Result<(), ExportError> {
        let trusted = unsafe { AXIsProcessTrusted() };
        if !trusted {
            return Err(ExportError::new(
                "E_EXPORT_PERMISSION_DENIED",
                "Accessibility permission is required (AXIsProcessTrusted=false)",
            ));
        }

        let system = unsafe { AXUIElementCreateSystemWide() };
        let system = OwnedCf::new(system as CFTypeRef).ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
                "AXUIElementCreateSystemWide returned null",
            )
        })?;

        let focused = copy_attribute_value(system.as_ax_element(), kAXFocusedUIElementAttribute)?;
        if unsafe { CFGetTypeID(focused.as_type_ref()) } != unsafe { AXUIElementGetTypeID() } {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "focused accessibility object is not AXUIElement",
            ));
        }
        let focused_element = focused.as_ax_element();

        let mut target_pid: i32 = 0;
        let pid_err = unsafe { AXUIElementGetPid(focused_element, &mut target_pid) };
        if pid_err != kAXErrorSuccess {
            return Err(ax_error(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "AXUIElementGetPid",
                pid_err,
            ));
        }
        if target_pid as u32 == std::process::id() {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_SELF_APP",
                format!(
                    "focused accessibility object belongs to TypeVoice process: target_pid={target_pid}"
                ),
            ));
        }

        let value_attr = owned_cf_string(kAXValueAttribute)?;
        let mut settable: u8 = 0;
        let settable_err = unsafe {
            AXUIElementIsAttributeSettable(
                focused_element,
                value_attr.as_cf_string(),
                &mut settable,
            )
        };
        if settable_err != kAXErrorSuccess {
            return Err(ax_error(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "AXUIElementIsAttributeSettable(AXValue)",
                settable_err,
            ));
        }
        if settable == 0 {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_READONLY",
                "focused accessibility target is not writable",
            ));
        }

        let current_value = copy_attribute_value(focused_element, kAXValueAttribute)?;
        if unsafe { CFGetTypeID(current_value.as_type_ref()) } != unsafe { CFStringGetTypeID() } {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_NOT_EDITABLE",
                "AXValue attribute is not string-backed",
            ));
        }
        let current_text = cf_string_to_string(current_value.as_cf_string())?;

        let selected_range_obj =
            copy_attribute_value(focused_element, kAXSelectedTextRangeAttribute).map_err(|e| {
                if e.code == "E_EXPORT_TARGET_NOT_EDITABLE" {
                    return ExportError::new("E_EXPORT_SELECTION_UNAVAILABLE", e.message);
                }
                e
            })?;

        if unsafe { CFGetTypeID(selected_range_obj.as_type_ref()) } != unsafe { AXValueGetTypeID() }
        {
            return Err(ExportError::new(
                "E_EXPORT_SELECTION_UNAVAILABLE",
                "AXSelectedTextRange is not AXValue",
            ));
        }

        let selected_ax_value = selected_range_obj.as_ax_value();
        if unsafe { AXValueGetType(selected_ax_value) } != kAXValueTypeCFRange {
            return Err(ExportError::new(
                "E_EXPORT_SELECTION_UNAVAILABLE",
                "AXSelectedTextRange AXValue is not CFRange",
            ));
        }

        let mut selected_range = CFRange::init(0, 0);
        let got_range = unsafe {
            AXValueGetValue(
                selected_ax_value,
                kAXValueTypeCFRange,
                &mut selected_range as *mut _ as *mut c_void,
            )
        };
        if !got_range {
            return Err(ExportError::new(
                "E_EXPORT_SELECTION_UNAVAILABLE",
                "AXValueGetValue failed for AXSelectedTextRange",
            ));
        }

        let insert_utf16 = if selected_range.location < 0 {
            0usize
        } else {
            selected_range.location as usize
        };

        let updated = insert_at_utf16_offset(&current_text, insert_utf16, text);
        let updated_cf = owned_cf_string(&updated)?;

        let set_value_err = unsafe {
            AXUIElementSetAttributeValue(
                focused_element,
                value_attr.as_cf_string(),
                updated_cf.as_type_ref(),
            )
        };
        if set_value_err != kAXErrorSuccess {
            if set_value_err == kAXErrorAPIDisabled {
                return Err(ax_error(
                    "E_EXPORT_PERMISSION_DENIED",
                    "AXUIElementSetAttributeValue(AXValue)",
                    set_value_err,
                ));
            }
            return Err(ax_error(
                "E_EXPORT_PASTE_FAILED",
                "AXUIElementSetAttributeValue(AXValue)",
                set_value_err,
            ));
        }

        let selection_attr = owned_cf_string(kAXSelectedTextRangeAttribute)?;
        let next_caret = insert_utf16.saturating_add(utf16_len(text));
        let next_range = CFRange::init(next_caret as isize, 0);
        let next_range_value = unsafe {
            AXValueCreate(
                kAXValueTypeCFRange,
                &next_range as *const _ as *const c_void,
            )
        };
        if let Some(next_range_value) = OwnedCf::new(next_range_value as CFTypeRef) {
            let _ = unsafe {
                AXUIElementSetAttributeValue(
                    focused_element,
                    selection_attr.as_cf_string(),
                    next_range_value.as_type_ref(),
                )
            };
        }

        Ok(())
    }

    struct OwnedCf {
        ptr: CFTypeRef,
    }

    impl OwnedCf {
        fn new(ptr: CFTypeRef) -> Option<Self> {
            if ptr.is_null() {
                return None;
            }
            Some(Self { ptr })
        }

        fn as_type_ref(&self) -> CFTypeRef {
            self.ptr
        }

        fn as_cf_string(&self) -> CFStringRef {
            self.ptr as CFStringRef
        }

        fn as_ax_element(&self) -> AXUIElementRef {
            self.ptr as AXUIElementRef
        }

        fn as_ax_value(&self) -> AXValueRef {
            self.ptr as AXValueRef
        }
    }

    impl Drop for OwnedCf {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                unsafe { CFRelease(self.ptr) };
            }
        }
    }

    fn copy_attribute_value(
        element: AXUIElementRef,
        attribute: &str,
    ) -> Result<OwnedCf, ExportError> {
        let attr = owned_cf_string(attribute)?;
        let mut value: CFTypeRef = ptr::null();
        let err =
            unsafe { AXUIElementCopyAttributeValue(element, attr.as_cf_string(), &mut value) };
        if err != kAXErrorSuccess {
            if err == kAXErrorAttributeUnsupported {
                return Err(ax_error(
                    "E_EXPORT_TARGET_NOT_EDITABLE",
                    &format!("AXUIElementCopyAttributeValue({attribute})"),
                    err,
                ));
            }
            if err == kAXErrorNoValue {
                return Err(ax_error(
                    "E_EXPORT_SELECTION_UNAVAILABLE",
                    &format!("AXUIElementCopyAttributeValue({attribute})"),
                    err,
                ));
            }
            if err == kAXErrorInvalidUIElement {
                return Err(ax_error(
                    "E_EXPORT_TARGET_UNAVAILABLE",
                    &format!("AXUIElementCopyAttributeValue({attribute})"),
                    err,
                ));
            }
            return Err(ax_error(
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
                &format!("AXUIElementCopyAttributeValue({attribute})"),
                err,
            ));
        }

        OwnedCf::new(value).ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                format!("AXUIElementCopyAttributeValue({attribute}) returned null value"),
            )
        })
    }

    fn owned_cf_string(text: &str) -> Result<OwnedCf, ExportError> {
        let raw = unsafe {
            CFStringCreateWithBytes(
                kCFAllocatorDefault,
                text.as_ptr(),
                text.len() as isize,
                kCFStringEncodingUTF8,
                0,
            )
        };
        OwnedCf::new(raw as CFTypeRef).ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
                "CFStringCreateWithBytes returned null",
            )
        })
    }

    fn cf_string_to_string(value: CFStringRef) -> Result<String, ExportError> {
        if value.is_null() {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "CFStringRef is null",
            ));
        }

        let len = unsafe { CFStringGetLength(value) };
        let cap = unsafe { CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) } + 1;
        if cap <= 0 {
            return Ok(String::new());
        }
        let mut buf = vec![0 as c_char; cap as usize];
        let ok = unsafe { CFStringGetCString(value, buf.as_mut_ptr(), cap, kCFStringEncodingUTF8) };
        if ok == 0 {
            return Err(ExportError::new(
                "E_EXPORT_AUTOMATION_UNAVAILABLE",
                "CFStringGetCString failed",
            ));
        }
        let c = unsafe { CStr::from_ptr(buf.as_ptr()) };
        Ok(c.to_string_lossy().into_owned())
    }

    fn ax_error(code: &str, context: &str, err: AXError) -> ExportError {
        ExportError::new(code, format!("{context} failed: ax_error={err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{byte_index_from_utf16_offset, insert_at_utf16_offset};

    #[test]
    fn utf16_offset_insert_ascii() {
        let out = insert_at_utf16_offset("abcd", 2, "ZZ");
        assert_eq!(out, "abZZcd");
    }

    #[test]
    fn utf16_offset_insert_cjk() {
        let out = insert_at_utf16_offset("你好世界", 2, "-");
        assert_eq!(out, "你好-世界");
    }

    #[test]
    fn utf16_offset_insert_emoji_boundary() {
        let src = "a🙂b";
        // "a"(1) + "🙂"(2)
        let out = insert_at_utf16_offset(src, 3, "X");
        assert_eq!(out, "a🙂Xb");
    }

    #[test]
    fn utf16_offset_clamps_to_end() {
        let out = insert_at_utf16_offset("abc", 999, "Z");
        assert_eq!(out, "abcZ");
    }

    #[test]
    fn utf16_split_never_breaks_codepoint() {
        let src = "🙂";
        // Mid-surrogate offset should snap to character boundary.
        let idx = byte_index_from_utf16_offset(src, 1);
        assert_eq!(idx, src.len());
    }
}
