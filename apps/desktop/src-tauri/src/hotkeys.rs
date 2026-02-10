use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::settings::Settings;
use crate::trace::Span;

const DEFAULT_PTT: &str = "F9";
const DEFAULT_TOGGLE: &str = "F10";

#[derive(Debug, Clone)]
struct HotkeyConfig {
    enabled: bool,
    ptt: Option<String>,
    toggle: Option<String>,
}

fn hotkey_config_from_settings(s: &Settings) -> HotkeyConfig {
    let enabled = s.hotkeys_enabled.unwrap_or(true);
    if !enabled {
        return HotkeyConfig {
            enabled: false,
            ptt: None,
            toggle: None,
        };
    }

    let ptt = s
        .hotkey_ptt
        .as_deref()
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .unwrap_or(DEFAULT_PTT)
        .to_string();

    let toggle = s
        .hotkey_toggle
        .as_deref()
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .unwrap_or(DEFAULT_TOGGLE)
        .to_string();

    // If both are the same, prefer PTT and disable toggle to avoid ambiguous behavior.
    if ptt.eq_ignore_ascii_case(&toggle) {
        return HotkeyConfig {
            enabled: true,
            ptt: Some(ptt),
            toggle: None,
        };
    }

    HotkeyConfig {
        enabled: true,
        ptt: Some(ptt),
        toggle: Some(toggle),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyRecordEvent {
    pub kind: String,  // ptt|toggle
    pub state: String, // Pressed|Released
    pub shortcut: String,
    pub ts_ms: i64,
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct HotkeyManager {
    // Ensures apply is serialized (settings updates may come quickly).
    lock: Mutex<()>,
}

impl Default for HotkeyManager {
    fn default() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }
}

impl HotkeyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_from_settings_best_effort<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        data_dir: &Path,
        s: &Settings,
    ) {
        let _g = self.lock.lock().unwrap();

        let cfg = hotkey_config_from_settings(s);
        let span = Span::start(
            data_dir,
            None,
            "Hotkeys",
            "HK.apply",
            Some(serde_json::json!({
                "enabled": cfg.enabled,
                "ptt": cfg.ptt.as_deref(),
                "toggle": cfg.toggle.as_deref(),
            })),
        );

        let gs = app.global_shortcut();

        // Clean slate. This app owns all global shortcuts.
        if let Err(e) = gs.unregister_all() {
            crate::trace::event(
                data_dir,
                None,
                "Hotkeys",
                "HK.unregister_all",
                "err",
                Some(serde_json::json!({"code": "E_HK_UNREGISTER_ALL", "error": e.to_string()})),
            );
            // Keep going: attempt to register anyway; worst case, register fails and we surface it.
        }

        if !cfg.enabled {
            span.ok(Some(serde_json::json!({"status": "disabled"})));
            return;
        }

        if let Some(ptt) = cfg.ptt.clone() {
            if let Err(e) = gs.on_shortcut(ptt.as_str(), |app, shortcut, event| {
                let payload = HotkeyRecordEvent {
                    kind: "ptt".to_string(),
                    state: match event.state {
                        ShortcutState::Pressed => "Pressed".to_string(),
                        ShortcutState::Released => "Released".to_string(),
                    },
                    shortcut: shortcut.into_string(),
                    ts_ms: now_ms(),
                };
                let _ = app.emit("tv_hotkey_record", payload);
            }) {
                crate::trace::event(
                    data_dir,
                    None,
                    "Hotkeys",
                    "HK.register.ptt",
                    "err",
                    Some(serde_json::json!({"code": "E_HK_REGISTER_PTT", "ptt": ptt, "error": e.to_string()})),
                );
            }
        }

        if let Some(toggle) = cfg.toggle.clone() {
            if let Err(e) = gs.on_shortcut(toggle.as_str(), |app, shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                let payload = HotkeyRecordEvent {
                    kind: "toggle".to_string(),
                    state: "Pressed".to_string(),
                    shortcut: shortcut.into_string(),
                    ts_ms: now_ms(),
                };
                let _ = app.emit("tv_hotkey_record", payload);
            }) {
                crate::trace::event(
                    data_dir,
                    None,
                    "Hotkeys",
                    "HK.register.toggle",
                    "err",
                    Some(serde_json::json!({"code": "E_HK_REGISTER_TOGGLE", "toggle": toggle, "error": e.to_string()})),
                );
            }
        }

        // Surface the "same key" normalization explicitly in trace.
        if cfg.ptt.is_some() && cfg.toggle.is_none() {
            if let (Some(raw_ptt), Some(raw_toggle)) = (s.hotkey_ptt.as_deref(), s.hotkey_toggle.as_deref()) {
                if raw_ptt.trim().eq_ignore_ascii_case(raw_toggle.trim()) && !raw_ptt.trim().is_empty() {
                    crate::trace::event(
                        data_dir,
                        None,
                        "Hotkeys",
                        "HK.same_key",
                        "ok",
                        Some(serde_json::json!({"key": raw_ptt.trim(), "note": "toggle_disabled"})),
                    );
                }
            }
        }

        span.ok(Some(serde_json::json!({"status": "ok"})));
    }
}

#[cfg(test)]
mod tests {
    use super::{hotkey_config_from_settings, HotkeyConfig, DEFAULT_PTT, DEFAULT_TOGGLE};
    use crate::settings::Settings;

    fn cfg(s: Settings) -> HotkeyConfig {
        hotkey_config_from_settings(&s)
    }

    #[test]
    fn defaults_are_present_when_enabled() {
        let c = cfg(Settings::default());
        assert!(c.enabled);
        assert_eq!(c.ptt.as_deref(), Some(DEFAULT_PTT));
        assert_eq!(c.toggle.as_deref(), Some(DEFAULT_TOGGLE));
    }

    #[test]
    fn disabled_means_no_keys() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(false);
        let c = cfg(s);
        assert!(!c.enabled);
        assert!(c.ptt.is_none());
        assert!(c.toggle.is_none());
    }

    #[test]
    fn trims_and_uses_defaults_on_empty() {
        let mut s = Settings::default();
        s.hotkey_ptt = Some("   ".to_string());
        s.hotkey_toggle = Some("\n".to_string());
        let c = cfg(s);
        assert_eq!(c.ptt.as_deref(), Some(DEFAULT_PTT));
        assert_eq!(c.toggle.as_deref(), Some(DEFAULT_TOGGLE));
    }

    #[test]
    fn same_key_disables_toggle() {
        let mut s = Settings::default();
        s.hotkey_ptt = Some("F9".to_string());
        s.hotkey_toggle = Some("f9".to_string());
        let c = cfg(s);
        assert_eq!(c.ptt.as_deref(), Some("F9"));
        assert!(c.toggle.is_none());
    }
}
