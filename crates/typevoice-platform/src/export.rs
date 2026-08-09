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
#[cfg(target_os = "macos")]
pub use macos::InsertionTarget;
#[cfg(windows)]
pub use windows::InsertionTarget;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
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

    #[cfg(target_os = "macos")]
    {
        macos::capture_insertion_target()
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err(ExportError::new(
            "E_EXPORT_TARGET_UNSUPPORTED",
            "insertion target capture is unsupported on this platform",
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

    #[cfg(target_os = "macos")]
    {
        macos::auto_input_text(target, text)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Err(ExportError::new(
            "E_EXPORT_PASTE_UNSUPPORTED",
            "auto input is unsupported on this platform",
        ))
    }
}

#[cfg(test)]
pub(crate) async fn native_input_contract_probe(text: &str) -> Result<(), ExportError> {
    #[cfg(windows)]
    {
        windows::native_input_contract_probe(text).await
    }
    #[cfg(target_os = "linux")]
    {
        linux::native_input_contract_probe(text).await
    }
    #[cfg(target_os = "macos")]
    {
        macos::native_input_contract_probe(text).await
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = text;
        Err(ExportError::new(
            "E_EXPORT_NATIVE_CONTRACT_UNSUPPORTED",
            "native input contract is only supported on Linux, macOS, and Windows",
        ))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use macos_accessibility_client::accessibility::application_is_trusted_with_prompt;

    #[cfg(test)]
    use super::NativeContractChild;

    use super::ExportError;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InsertionTarget {
        pid: i32,
    }

    pub fn capture_insertion_target() -> Result<InsertionTarget, ExportError> {
        if !application_is_trusted_with_prompt() {
            return Err(ExportError::new(
                "E_EXPORT_ACCESSIBILITY_PERMISSION",
                "macOS Accessibility permission is required for automatic paste",
            ));
        }
        let application = crate::macos_app::frontmost_application().ok_or_else(|| {
            ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "no frontmost macOS application is available for automatic paste",
            )
        })?;
        if application.pid == std::process::id() as i32 {
            return Err(ExportError::new(
                "E_EXPORT_TARGET_UNAVAILABLE",
                "the frontmost application is TypeVoice",
            ));
        }
        Ok(InsertionTarget {
            pid: application.pid,
        })
    }

    pub fn auto_input_text(target: &InsertionTarget, text: &str) -> Result<(), ExportError> {
        let current = crate::macos_app::frontmost_application();
        if current.as_ref().map(|application| application.pid) != Some(target.pid) {
            if !crate::macos_app::activate_application(target.pid) {
                return Err(ExportError::new(
                    "E_EXPORT_TARGET_UNAVAILABLE",
                    "the frozen macOS insertion target no longer exists",
                ));
            }
            let activated = crate::macos_app::frontmost_application();
            if activated.as_ref().map(|application| application.pid) != Some(target.pid) {
                return Err(ExportError::new(
                    "E_EXPORT_TARGET_MISMATCH",
                    "macOS did not activate the frozen insertion target",
                ));
            }
        }

        dispatch_unicode_text(target.pid, text)
    }

    fn dispatch_unicode_text(pid: i32, text: &str) -> Result<(), ExportError> {
        for character in text.chars() {
            let unicode = character.to_string();
            post_unicode_keyboard_event(pid, &unicode, true)?;
            post_unicode_keyboard_event(pid, &unicode, false)?;
        }
        Ok(())
    }

    fn post_unicode_keyboard_event(
        pid: i32,
        unicode: &str,
        key_down: bool,
    ) -> Result<(), ExportError> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| {
            ExportError::new(
                "E_EXPORT_PASTE_FAILED",
                "failed to create a macOS keyboard event source",
            )
        })?;
        let event = CGEvent::new_keyboard_event(source, 0, key_down).map_err(|_| {
            ExportError::new(
                "E_EXPORT_PASTE_FAILED",
                "failed to create a macOS Unicode keyboard event",
            )
        })?;
        event.set_string(unicode);
        event.post_to_pid(pid);
        Ok(())
    }

    #[cfg(test)]
    pub(super) async fn native_input_contract_probe(text: &str) -> Result<(), ExportError> {
        let mut child =
            NativeContractChild::spawn("export::macos::t23_native_input_target_helper", text)?;
        child.wait_ready().await?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let target = loop {
            match capture_insertion_target() {
                Ok(target) if target.pid == child.id() as i32 => break target,
                Ok(_) | Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Ok(_) => {
                    return Err(ExportError::new(
                        "E_EXPORT_NATIVE_CONTRACT_TIMEOUT",
                        "isolated AppKit target did not become the insertion target",
                    ));
                }
                Err(error) => return Err(error),
            }
        };
        super::auto_paste_text(&target, text).await?;
        child.wait_success().await
    }
}

#[cfg(test)]
const NATIVE_CONTRACT_ISOLATED_ENV: &str = "TYPEVOICE_T23_ISOLATED";
#[cfg(test)]
const NATIVE_CONTRACT_HELPER_ENV: &str = "TYPEVOICE_T23_HELPER";
#[cfg(test)]
const NATIVE_CONTRACT_TEXT_ENV: &str = "TYPEVOICE_T23_TEXT";
#[cfg(test)]
const NATIVE_CONTRACT_READY_ENV: &str = "TYPEVOICE_T23_READY_PATH";
#[cfg(test)]
const NATIVE_CONTRACT_SUCCESS_ENV: &str = "TYPEVOICE_T23_SUCCESS_PATH";
#[cfg(all(test, windows))]
const NATIVE_CONTRACT_WINDOWS_VM_ENV: &str = "TYPEVOICE_T23_WINDOWS_VM";

#[cfg(test)]
struct NativeContractChild {
    child: std::process::Child,
    ready_path: std::path::PathBuf,
    success_path: std::path::PathBuf,
    _temp: tempfile::TempDir,
}

#[cfg(test)]
impl NativeContractChild {
    fn spawn(test_name: &str, text: &str) -> Result<Self, ExportError> {
        if std::env::var(NATIVE_CONTRACT_ISOLATED_ENV).as_deref() != Ok("1") {
            return Err(ExportError::new(
                "E_EXPORT_NATIVE_CONTRACT_NOT_ISOLATED",
                format!(
                    "set {NATIVE_CONTRACT_ISOLATED_ENV}=1 only inside an isolated native-input environment"
                ),
            ));
        }
        #[cfg(windows)]
        if std::env::var(NATIVE_CONTRACT_WINDOWS_VM_ENV).as_deref() != Ok("1") {
            return Err(ExportError::new(
                "E_EXPORT_NATIVE_CONTRACT_NOT_ISOLATED",
                format!(
                    "Windows SendInput E2E requires explicit {NATIVE_CONTRACT_WINDOWS_VM_ENV}=1 for an isolated interactive VM"
                ),
            ));
        }
        let temp = tempfile::tempdir().map_err(|error| {
            ExportError::new(
                "E_EXPORT_NATIVE_CONTRACT_SETUP",
                format!("create native input contract temp dir failed: {error}"),
            )
        })?;
        let ready_path = temp.path().join("ready");
        let success_path = temp.path().join("success");

        #[cfg(target_os = "macos")]
        let child = {
            let _ = test_name;
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/macos_insertion_target.swift");
            let executable = temp.path().join("macos-insertion-target");
            let output = std::process::Command::new("xcrun")
                .arg("swiftc")
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .output()
                .map_err(|error| {
                    ExportError::new(
                        "E_EXPORT_NATIVE_CONTRACT_SETUP",
                        format!("launch Swift compiler for AppKit target failed: {error}"),
                    )
                })?;
            if !output.status.success() {
                return Err(ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_SETUP",
                    format!(
                        "compile AppKit target failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                ));
            }
            std::process::Command::new(executable)
                .env(NATIVE_CONTRACT_HELPER_ENV, "1")
                .env(NATIVE_CONTRACT_TEXT_ENV, text)
                .env(NATIVE_CONTRACT_READY_ENV, &ready_path)
                .env(NATIVE_CONTRACT_SUCCESS_ENV, &success_path)
                .spawn()
                .map_err(|error| {
                    ExportError::new(
                        "E_EXPORT_NATIVE_CONTRACT_SETUP",
                        format!("spawn AppKit input target failed: {error}"),
                    )
                })?
        };

        #[cfg(not(target_os = "macos"))]
        let child = {
            let executable = std::env::current_exe().map_err(|error| {
                ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_SETUP",
                    format!("resolve current test executable failed: {error}"),
                )
            })?;
            std::process::Command::new(executable)
                .args([
                    "--ignored",
                    "--exact",
                    test_name,
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(NATIVE_CONTRACT_HELPER_ENV, "1")
                .env(NATIVE_CONTRACT_TEXT_ENV, text)
                .env(NATIVE_CONTRACT_READY_ENV, &ready_path)
                .env(NATIVE_CONTRACT_SUCCESS_ENV, &success_path)
                .spawn()
                .map_err(|error| {
                    ExportError::new(
                        "E_EXPORT_NATIVE_CONTRACT_SETUP",
                        format!("spawn native input target failed: {error}"),
                    )
                })?
        };
        Ok(Self {
            child,
            ready_path,
            success_path,
            _temp: temp,
        })
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    async fn wait_ready(&mut self) -> Result<(), ExportError> {
        self.wait_for_path(&self.ready_path.clone(), "target readiness")
            .await
    }

    async fn wait_success(&mut self) -> Result<(), ExportError> {
        self.wait_for_path(&self.success_path.clone(), "native text readback")
            .await?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|error| {
                ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_CHILD",
                    format!("query native input target failed: {error}"),
                )
            })? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(ExportError::new(
                        "E_EXPORT_NATIVE_CONTRACT_CHILD",
                        format!("native input target exited with {status}"),
                    ))
                };
            }
            if std::time::Instant::now() >= deadline {
                return Err(ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_TIMEOUT",
                    "native input target did not exit after successful readback",
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_path(
        &mut self,
        path: &std::path::Path,
        step: &str,
    ) -> Result<(), ExportError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if path.is_file() {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait().map_err(|error| {
                ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_CHILD",
                    format!("query native input target failed: {error}"),
                )
            })? {
                return Err(ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_CHILD",
                    format!("native input target exited during {step} with {status}"),
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err(ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_TIMEOUT",
                    format!("timed out waiting for {step}"),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

#[cfg(test)]
impl Drop for NativeContractChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
fn native_contract_helper_config() -> (String, std::path::PathBuf, std::path::PathBuf) {
    assert_eq!(
        std::env::var(NATIVE_CONTRACT_HELPER_ENV).as_deref(),
        Ok("1"),
        "native input target helpers may only be launched by the T23 parent test"
    );
    let text = std::env::var(NATIVE_CONTRACT_TEXT_ENV).expect("T23 helper text");
    let ready_path = std::env::var_os(NATIVE_CONTRACT_READY_ENV)
        .map(std::path::PathBuf::from)
        .expect("T23 helper ready path");
    let success_path = std::env::var_os(NATIVE_CONTRACT_SUCCESS_ENV)
        .map(std::path::PathBuf::from)
        .expect("T23 helper success path");
    (text, ready_path, success_path)
}

#[cfg(windows)]
fn utf16_code_units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[cfg(windows)]
mod windows {
    #[cfg(test)]
    use super::{native_contract_helper_config, NativeContractChild};
    use super::{utf16_code_units, ExportError};
    use std::mem::{self, size_of};
    #[cfg(test)]
    use std::{fs, ptr, thread, time::Duration};
    use windows_sys::Win32::Foundation::{GetLastError, HWND};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    #[cfg(test)]
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetFocus, SetFocus, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };
    #[cfg(test)]
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowTextLengthW, GetWindowTextW,
        PeekMessageW, ShowWindow, TranslateMessage, ES_AUTOVSCROLL, ES_MULTILINE, MSG, PM_REMOVE,
        SW_SHOW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
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
    pub(super) async fn native_input_contract_probe(text: &str) -> Result<(), ExportError> {
        let mut child =
            NativeContractChild::spawn("export::windows::t23_native_input_target_helper", text)?;
        child.wait_ready().await?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let target = loop {
            if let Ok(target) = capture_insertion_target() {
                if target.foreground_pid == child.id() && target.focus_pid == child.id() {
                    break target;
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(ExportError::new(
                    "E_EXPORT_NATIVE_CONTRACT_TIMEOUT",
                    "isolated Win32 edit target did not become the focused insertion target",
                ));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        if modifier_key_is_down() {
            return Err(ExportError::new(
                "E_EXPORT_NATIVE_CONTRACT_INPUT_STATE",
                "isolated Windows input desktop has a pressed modifier key",
            ));
        }
        super::auto_paste_text(&target, text).await?;
        child.wait_success().await
    }

    #[cfg(test)]
    fn modifier_key_is_down() -> bool {
        [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
            .into_iter()
            .any(|key| unsafe { GetAsyncKeyState(i32::from(key)) } as u16 & 0x8000 != 0)
    }

    #[cfg(test)]
    #[test]
    #[ignore = "spawned by the isolated Windows T23 parent test"]
    fn t23_native_input_target_helper() {
        let (expected, ready_path, success_path) = native_contract_helper_config();
        let class_name = "EDIT\0".encode_utf16().collect::<Vec<_>>();
        let empty = [0_u16];
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                empty.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32,
                100,
                100,
                640,
                240,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        assert!(!hwnd.is_null(), "create isolated Win32 edit target");
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
        }

        let focus_deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            pump_window_messages();
            unsafe {
                SetForegroundWindow(hwnd);
                SetFocus(hwnd);
            }
            if unsafe { GetForegroundWindow() == hwnd && GetFocus() == hwnd } {
                break;
            }
            assert!(
                std::time::Instant::now() < focus_deadline,
                "isolated Win32 edit target could not acquire foreground focus"
            );
            thread::sleep(Duration::from_millis(20));
        }
        fs::write(&ready_path, b"ready").expect("write T23 ready marker");

        let input_deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            pump_window_messages();
            let actual = read_window_text(hwnd).replace("\r\n", "\n");
            if actual == expected {
                fs::write(&success_path, b"success").expect("write T23 success marker");
                break;
            }
            assert!(
                std::time::Instant::now() < input_deadline,
                "Win32 edit target did not receive the expected Unicode text: {actual:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            DestroyWindow(hwnd);
        }
    }

    #[cfg(test)]
    fn pump_window_messages() {
        let mut message: MSG = unsafe { mem::zeroed() };
        while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    #[cfg(test)]
    fn read_window_text(hwnd: HWND) -> String {
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        assert!(length >= 0, "read Win32 edit target length");
        let mut buffer = vec![0_u16; length as usize + 1];
        let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
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
    #[cfg(test)]
    use super::{native_contract_helper_config, NativeContractChild};
    use atspi::proxy::accessible::ObjectRefExt;
    use atspi::proxy::proxy_ext::ProxyExt;
    use atspi::{AccessibilityConnection, Interface, ObjectRefOwned, State};
    #[cfg(test)]
    use gtk::prelude::*;
    use std::{cmp, future::Future};
    #[cfg(test)]
    use std::{fs, time::Duration};

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

        let ok = dispatch_editable_insert(insert_pos, text, |position, text, byte_length| {
            editable.insert_text(position, text, byte_length)
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
        call(insert_pos, text, utf8_byte_len_i32(text)).await
    }

    fn utf8_byte_len_i32(text: &str) -> i32 {
        let n = text.len();
        cmp::min(n, i32::MAX as usize) as i32
    }

    #[cfg(test)]
    #[test]
    fn editable_text_length_uses_utf8_bytes() {
        assert_eq!(utf8_byte_len_i32("TypeVoice 世界\n"), 17);
    }

    #[cfg(test)]
    pub(super) async fn native_input_contract_probe(text: &str) -> Result<(), ExportError> {
        let mut child =
            NativeContractChild::spawn("export::linux::t23_native_input_target_helper", text)?;
        child.wait_ready().await?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let target = loop {
            match capture_insertion_target().await {
                Ok(target) => break target,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => {
                    return Err(ExportError::new(
                        "E_EXPORT_NATIVE_CONTRACT_TIMEOUT",
                        format!(
                            "isolated GTK target did not register with AT-SPI: {}",
                            error.message
                        ),
                    ));
                }
            }
        };
        super::auto_paste_text(&target, text).await?;
        child.wait_success().await
    }

    #[cfg(test)]
    #[test]
    #[ignore = "spawned by the isolated Linux T23 parent test"]
    fn t23_native_input_target_helper() {
        let (expected, ready_path, success_path) = native_contract_helper_config();
        gtk::init().expect("initialize GTK for T23 target");
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("TypeVoice T23 isolated target");
        window.set_default_size(640, 240);
        let text_view = gtk::TextView::new();
        window.add(&text_view);
        window.show_all();
        window.present();
        text_view.grab_focus();
        while gtk::events_pending() {
            gtk::main_iteration();
        }
        fs::write(&ready_path, b"ready").expect("write T23 ready marker");

        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let success_marker = success_path.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(20), move || {
            let actual = text_view
                .buffer()
                .and_then(|buffer| {
                    buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), true)
                        .map(|text| text.to_string())
                })
                .unwrap_or_default();
            if actual == expected {
                fs::write(&success_marker, b"success").expect("write T23 success marker");
                gtk::main_quit();
                return gtk::glib::ControlFlow::Break;
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("T23 GTK readback mismatch: expected={expected:?}, actual={actual:?}");
                gtk::main_quit();
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });
        gtk::main();
        assert!(
            success_path.is_file(),
            "GTK target did not receive the expected text through AT-SPI"
        );
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

#[cfg(all(test, windows))]
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
