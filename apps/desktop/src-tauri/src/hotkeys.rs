use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::obs::Span;
use crate::settings::Settings;

pub const GLOBAL_HOTKEY_EVENT: &str = "tv_global_hotkey";
const ALT_TAP_MAX_MS: i64 = 350;

#[derive(Debug, Clone)]
struct HotkeyConfig {
    enabled: bool,
}

fn hotkey_config_from_settings(s: &Settings) -> anyhow::Result<HotkeyConfig> {
    let cfg = crate::settings::resolve_hotkey_config(s)?;
    Ok(HotkeyConfig {
        enabled: cfg.enabled,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyAvailability {
    pub available: bool,
    pub reason: Option<String>,
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GlobalHotkeyEvent {
    action: &'static str,
    ts_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HotkeyAction {
    AltTap,
    InsertOverlay,
}

impl HotkeyAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::AltTap => "altTap",
            Self::InsertOverlay => "insertOverlay",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Alt,
    Ctrl,
    Enter,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyState {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeySignal {
    key: KeyKind,
    state: KeyState,
    ts_ms: i64,
}

#[derive(Debug, Default)]
struct HotkeyDetector {
    alt_down_at_ms: Option<i64>,
    alt_clean: bool,
    ctrl_down: bool,
    enter_down: bool,
}

impl HotkeyDetector {
    fn apply(&mut self, signal: KeySignal) -> Option<HotkeyAction> {
        match (signal.key, signal.state) {
            (KeyKind::Alt, KeyState::Down) => {
                if self.alt_down_at_ms.is_none() {
                    self.alt_down_at_ms = Some(signal.ts_ms);
                    self.alt_clean = true;
                }
                None
            }
            (KeyKind::Alt, KeyState::Up) => {
                let Some(started_at) = self.alt_down_at_ms.take() else {
                    return None;
                };
                let clean = self.alt_clean;
                self.alt_clean = false;
                if clean && signal.ts_ms.saturating_sub(started_at) <= ALT_TAP_MAX_MS {
                    Some(HotkeyAction::AltTap)
                } else {
                    None
                }
            }
            (KeyKind::Ctrl, KeyState::Down) => {
                self.ctrl_down = true;
                if self.alt_down_at_ms.is_some() {
                    self.alt_clean = false;
                }
                None
            }
            (KeyKind::Ctrl, KeyState::Up) => {
                self.ctrl_down = false;
                None
            }
            (KeyKind::Enter, KeyState::Down) => {
                if self.alt_down_at_ms.is_some() {
                    self.alt_clean = false;
                }
                if self.enter_down {
                    return None;
                }
                self.enter_down = true;
                self.ctrl_down.then_some(HotkeyAction::InsertOverlay)
            }
            (KeyKind::Enter, KeyState::Up) => {
                self.enter_down = false;
                None
            }
            (KeyKind::Other, KeyState::Down) => {
                if self.alt_down_at_ms.is_some() {
                    self.alt_clean = false;
                }
                None
            }
            (KeyKind::Other, KeyState::Up) => None,
        }
    }
}

#[tauri::command]
pub fn check_hotkey_available(
    _app: AppHandle,
    shortcut: &str,
    _ignore_self: Option<&str>,
) -> Result<HotkeyAvailability, String> {
    let available = !shortcut.trim().is_empty();
    Ok(HotkeyAvailability {
        available,
        reason: (!available).then(|| "shortcut is empty".to_string()),
        reason_code: (!available).then(|| "E_HOTKEY_SHORTCUT_EMPTY".to_string()),
    })
}

pub struct HotkeyManager {
    lock: Mutex<()>,
    listener: Mutex<Option<PlatformKeyboardListener>>,
}

impl Default for HotkeyManager {
    fn default() -> Self {
        Self {
            lock: Mutex::new(()),
            listener: Mutex::new(None),
        }
    }
}

impl HotkeyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_from_settings_best_effort(&self, app: &AppHandle, data_dir: &Path, s: &Settings) {
        let _g = self.lock.lock().unwrap();

        let cfg = match hotkey_config_from_settings(s) {
            Ok(v) => v,
            Err(e) => {
                let span = Span::start(data_dir, None, "Hotkeys", "HK.apply", None);
                span.err_anyhow("config", "E_HK_CONFIG", &e, None);
                return;
            }
        };
        let span = Span::start(
            data_dir,
            None,
            "Hotkeys",
            "HK.apply",
            Some(serde_json::json!({
                "enabled": cfg.enabled,
                "mode": "alt_overlay",
            })),
        );

        if !cfg.enabled {
            self.stop_listener();
            span.ok(Some(serde_json::json!({"status": "disabled"})));
            return;
        }

        let mut listener = self.listener.lock().unwrap();
        if listener.is_none() {
            match PlatformKeyboardListener::start(app.clone()) {
                Ok(next) => {
                    *listener = Some(next);
                    span.ok(Some(serde_json::json!({"status": "ok"})));
                }
                Err(e) => {
                    span.err_anyhow("hook", "E_HK_LISTENER_START", &e, None);
                }
            }
        } else {
            span.ok(Some(serde_json::json!({"status": "ok"})));
        }
    }

    fn stop_listener(&self) {
        let mut listener = self.listener.lock().unwrap();
        if let Some(mut current) = listener.take() {
            current.stop();
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        if let Ok(mut listener) = self.listener.lock() {
            if let Some(mut current) = listener.take() {
                current.stop();
            }
        }
    }
}

#[cfg(windows)]
struct PlatformKeyboardListener {
    hook_thread_id: u32,
    hook_thread: Option<std::thread::JoinHandle<()>>,
    event_thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(windows))]
struct PlatformKeyboardListener;

impl PlatformKeyboardListener {
    #[cfg(windows)]
    fn start(app: AppHandle) -> anyhow::Result<Self> {
        use std::sync::mpsc;
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
            UnhookWindowsHookEx, HHOOK, MSG, WH_KEYBOARD_LL, WM_QUIT,
        };

        unsafe extern "system" fn keyboard_proc(
            code: i32,
            w_param: windows_sys::Win32::Foundation::WPARAM,
            l_param: windows_sys::Win32::Foundation::LPARAM,
        ) -> windows_sys::Win32::Foundation::LRESULT {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU,
            };
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                CallNextHookEx, HC_ACTION, KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
                WM_SYSKEYUP,
            };

            if code == HC_ACTION as i32 {
                let state = match w_param as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyState::Down),
                    WM_KEYUP | WM_SYSKEYUP => Some(KeyState::Up),
                    _ => None,
                };
                if let Some(state) = state {
                    let info = unsafe { *(l_param as *const KBDLLHOOKSTRUCT) };
                    let key = match info.vkCode {
                        key if key == VK_MENU as u32
                            || key == VK_LMENU as u32
                            || key == VK_RMENU as u32 =>
                        {
                            KeyKind::Alt
                        }
                        key if key == VK_CONTROL as u32
                            || key == VK_LCONTROL as u32
                            || key == VK_RCONTROL as u32 =>
                        {
                            KeyKind::Ctrl
                        }
                        key if key == VK_RETURN as u32 => KeyKind::Enter,
                        _ => KeyKind::Other,
                    };
                    let signal = KeySignal {
                        key,
                        state,
                        ts_ms: now_ms(),
                    };
                    if let Some(lock) = KEY_SIGNAL_SLOT.get() {
                        if let Some(tx) = lock.lock().unwrap().as_ref() {
                            let _ = tx.send(signal);
                        }
                    }
                }
            }

            unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
        }

        let (signal_tx, signal_rx) = mpsc::channel::<KeySignal>();
        let signal_slot = KEY_SIGNAL_SLOT.get_or_init(|| Mutex::new(None));
        *signal_slot.lock().unwrap() = Some(signal_tx);

        let event_thread = std::thread::Builder::new()
            .name("typevoice_hotkey_events".to_string())
            .spawn(move || {
                let mut detector = HotkeyDetector::default();
                while let Ok(signal) = signal_rx.recv() {
                    if let Some(action) = detector.apply(signal) {
                        let _ = app.emit(
                            GLOBAL_HOTKEY_EVENT,
                            GlobalHotkeyEvent {
                                action: action.as_str(),
                                ts_ms: now_ms(),
                            },
                        );
                    }
                }
            })?;

        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let hook_thread = std::thread::Builder::new()
            .name("typevoice_keyboard_hook".to_string())
            .spawn(move || {
                let thread_id = unsafe { GetCurrentThreadId() };
                let hook: HHOOK = unsafe {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), std::ptr::null_mut(), 0)
                };
                if hook.is_null() {
                    let _ = ready_tx.send(Err("SetWindowsHookExW failed".to_string()));
                    return;
                }
                let _ = ready_tx.send(Ok(thread_id));

                let mut msg: MSG = unsafe { std::mem::zeroed() };
                loop {
                    let ok = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
                    if ok <= 0 || msg.message == WM_QUIT {
                        break;
                    }
                    unsafe {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                unsafe {
                    UnhookWindowsHookEx(hook);
                }
            })?;

        match ready_rx.recv() {
            Ok(Ok(hook_thread_id)) => Ok(Self {
                hook_thread_id,
                hook_thread: Some(hook_thread),
                event_thread: Some(event_thread),
            }),
            Ok(Err(message)) => {
                *KEY_SIGNAL_SLOT
                    .get_or_init(|| Mutex::new(None))
                    .lock()
                    .unwrap() = None;
                let _ = hook_thread.join();
                let _ = event_thread.join();
                Err(anyhow::anyhow!(message))
            }
            Err(e) => {
                *KEY_SIGNAL_SLOT
                    .get_or_init(|| Mutex::new(None))
                    .lock()
                    .unwrap() = None;
                let _ = hook_thread.join();
                let _ = event_thread.join();
                Err(anyhow::anyhow!("keyboard hook thread failed: {e}"))
            }
        }
    }

    #[cfg(not(windows))]
    fn start(_app: AppHandle) -> anyhow::Result<Self> {
        Ok(Self)
    }

    #[cfg(windows)]
    fn stop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        unsafe {
            let _ = PostThreadMessageW(self.hook_thread_id, WM_QUIT, 0, 0);
        }
        if let Some(handle) = self.hook_thread.take() {
            let _ = handle.join();
        }
        if let Some(lock) = KEY_SIGNAL_SLOT.get() {
            *lock.lock().unwrap() = None;
        }
        if let Some(handle) = self.event_thread.take() {
            let _ = handle.join();
        }
    }

    #[cfg(not(windows))]
    fn stop(&mut self) {}
}

#[cfg(windows)]
static KEY_SIGNAL_SLOT: std::sync::OnceLock<Mutex<Option<std::sync::mpsc::Sender<KeySignal>>>> =
    std::sync::OnceLock::new();

fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => dur.as_millis() as i64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        hotkey_config_from_settings, HotkeyAction, HotkeyDetector, KeyKind, KeySignal, KeyState,
    };
    use crate::settings::Settings;

    fn signal(key: KeyKind, state: KeyState, ts_ms: i64) -> KeySignal {
        KeySignal { key, state, ts_ms }
    }

    #[test]
    fn config_requires_only_enabled_flag() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(true);
        let cfg = hotkey_config_from_settings(&s).expect("config");
        assert!(cfg.enabled);
    }

    #[test]
    fn legacy_shortcuts_remain_compatible() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(true);
        s.hotkey_ptt = Some("F9".to_string());
        s.hotkey_toggle = Some("F10".to_string());
        let cfg = hotkey_config_from_settings(&s).expect("config");
        assert!(cfg.enabled);
    }

    #[test]
    fn alt_tap_within_threshold_triggers() {
        let mut detector = HotkeyDetector::default();
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Up, 1200)),
            Some(HotkeyAction::AltTap)
        );
    }

    #[test]
    fn long_alt_press_is_ignored() {
        let mut detector = HotkeyDetector::default();
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Up, 1401)),
            None
        );
    }

    #[test]
    fn alt_combo_is_ignored() {
        let mut detector = HotkeyDetector::default();
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Other, KeyState::Down, 1020)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Up, 1100)),
            None
        );
    }

    #[test]
    fn repeated_alt_down_keeps_first_press_time() {
        let mut detector = HotkeyDetector::default();
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Down, 1100)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Up, 1300)),
            Some(HotkeyAction::AltTap)
        );
    }

    #[test]
    fn ctrl_enter_triggers_insert_overlay_once() {
        let mut detector = HotkeyDetector::default();
        assert_eq!(
            detector.apply(signal(KeyKind::Ctrl, KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Enter, KeyState::Down, 1010)),
            Some(HotkeyAction::InsertOverlay)
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Enter, KeyState::Down, 1020)),
            None
        );
    }
}
