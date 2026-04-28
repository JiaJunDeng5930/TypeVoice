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
    primary: KeyKind,
}

fn hotkey_config_from_settings(s: &Settings) -> anyhow::Result<HotkeyConfig> {
    let cfg = crate::settings::resolve_hotkey_config(s)?;
    Ok(HotkeyConfig {
        enabled: cfg.enabled,
        primary: KeyKind::from_config_value(&cfg.primary)?,
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
    Primary,
}

impl HotkeyAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Alt,
    Ctrl,
    Shift,
    Function(u8),
    Other,
}

impl KeyKind {
    fn from_config_value(value: &str) -> anyhow::Result<Self> {
        match value {
            "Alt" => Ok(Self::Alt),
            "Ctrl" => Ok(Self::Ctrl),
            "Shift" => Ok(Self::Shift),
            f if f.len() >= 2 && f.starts_with('F') => {
                let number = f[1..].parse::<u8>()?;
                if (1..=12).contains(&number) {
                    Ok(Self::Function(number))
                } else {
                    Err(anyhow::anyhow!(
                        "E_SETTINGS_HOTKEY_PRIMARY_INVALID: unsupported primary hotkey '{value}'"
                    ))
                }
            }
            _ => Err(anyhow::anyhow!(
                "E_SETTINGS_HOTKEY_PRIMARY_INVALID: unsupported primary hotkey '{value}'"
            )),
        }
    }
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
    primary: KeyKind,
    primary_down_at_ms: Option<i64>,
    primary_clean: bool,
}

impl HotkeyDetector {
    fn new(primary: KeyKind) -> Self {
        Self {
            primary,
            primary_down_at_ms: None,
            primary_clean: false,
        }
    }

    fn apply(&mut self, signal: KeySignal) -> Option<HotkeyAction> {
        if signal.key == self.primary {
            return match signal.state {
                KeyState::Down => {
                    if self.primary_down_at_ms.is_none() {
                        self.primary_down_at_ms = Some(signal.ts_ms);
                        self.primary_clean = true;
                    }
                    None
                }
                KeyState::Up => {
                    let Some(started_at) = self.primary_down_at_ms.take() else {
                        return None;
                    };
                    let clean = self.primary_clean;
                    self.primary_clean = false;
                    if clean && signal.ts_ms.saturating_sub(started_at) <= ALT_TAP_MAX_MS {
                        Some(HotkeyAction::Primary)
                    } else {
                        None
                    }
                }
            };
        }
        if signal.state == KeyState::Down && self.primary_down_at_ms.is_some() {
            self.primary_clean = false;
        }
        None
    }
}

impl Default for KeyKind {
    fn default() -> Self {
        Self::Alt
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
                "mode": "primary",
            })),
        );

        if !cfg.enabled {
            self.stop_listener();
            span.ok(Some(serde_json::json!({"status": "disabled"})));
            return;
        }

        let mut listener = self.listener.lock().unwrap();
        if let Some(mut current) = listener.take() {
            current.stop();
        }
        match PlatformKeyboardListener::start(app.clone(), cfg.primary) {
            Ok(next) => {
                *listener = Some(next);
                span.ok(Some(serde_json::json!({"status": "ok"})));
            }
            Err(e) => {
                span.err_anyhow("hook", "E_HK_LISTENER_START", &e, None);
            }
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
    fn start(app: AppHandle, primary: KeyKind) -> anyhow::Result<Self> {
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
                VK_CONTROL, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6,
                VK_F7, VK_F8, VK_F9, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL,
                VK_RETURN, VK_RMENU, VK_RSHIFT, VK_SHIFT,
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
                        key if key == VK_SHIFT as u32
                            || key == VK_LSHIFT as u32
                            || key == VK_RSHIFT as u32 =>
                        {
                            KeyKind::Shift
                        }
                        key if key == VK_F1 as u32 => KeyKind::Function(1),
                        key if key == VK_F2 as u32 => KeyKind::Function(2),
                        key if key == VK_F3 as u32 => KeyKind::Function(3),
                        key if key == VK_F4 as u32 => KeyKind::Function(4),
                        key if key == VK_F5 as u32 => KeyKind::Function(5),
                        key if key == VK_F6 as u32 => KeyKind::Function(6),
                        key if key == VK_F7 as u32 => KeyKind::Function(7),
                        key if key == VK_F8 as u32 => KeyKind::Function(8),
                        key if key == VK_F9 as u32 => KeyKind::Function(9),
                        key if key == VK_F10 as u32 => KeyKind::Function(10),
                        key if key == VK_F11 as u32 => KeyKind::Function(11),
                        key if key == VK_F12 as u32 => KeyKind::Function(12),
                        key if key == VK_RETURN as u32 => KeyKind::Other,
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
                let mut detector = HotkeyDetector::new(primary);
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
    fn start(_app: AppHandle, _primary: KeyKind) -> anyhow::Result<Self> {
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
        assert_eq!(cfg.primary, KeyKind::Alt);
    }

    #[test]
    fn config_accepts_single_function_key() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(true);
        s.hotkey_primary = Some("F9".to_string());
        let cfg = hotkey_config_from_settings(&s).expect("config");
        assert!(cfg.enabled);
        assert_eq!(cfg.primary, KeyKind::Function(9));
    }

    #[test]
    fn config_rejects_combo_key() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(true);
        s.hotkey_primary = Some("Ctrl+Alt".to_string());
        assert!(hotkey_config_from_settings(&s).is_err());
    }

    #[test]
    fn alt_tap_within_threshold_triggers() {
        let mut detector = HotkeyDetector::new(KeyKind::Alt);
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Alt, KeyState::Up, 1200)),
            Some(HotkeyAction::Primary)
        );
    }

    #[test]
    fn long_alt_press_is_ignored() {
        let mut detector = HotkeyDetector::new(KeyKind::Alt);
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
        let mut detector = HotkeyDetector::new(KeyKind::Alt);
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
        let mut detector = HotkeyDetector::new(KeyKind::Alt);
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
            Some(HotkeyAction::Primary)
        );
    }

    #[test]
    fn configured_function_key_triggers_primary() {
        let mut detector = HotkeyDetector::new(KeyKind::Function(9));
        assert_eq!(
            detector.apply(signal(KeyKind::Function(9), KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Function(9), KeyState::Up, 1010)),
            Some(HotkeyAction::Primary)
        );
    }

    #[test]
    fn ctrl_enter_does_not_trigger_shortcut_when_primary_is_alt() {
        let mut detector = HotkeyDetector::new(KeyKind::Alt);
        assert_eq!(
            detector.apply(signal(KeyKind::Ctrl, KeyState::Down, 1000)),
            None
        );
        assert_eq!(
            detector.apply(signal(KeyKind::Other, KeyState::Down, 1010)),
            None
        );
    }
}
