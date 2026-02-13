use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::settings::Settings;
use crate::trace::Span;

#[derive(Debug, Clone)]
struct HotkeyConfig {
    enabled: bool,
    ptt: Option<String>,
    toggle: Option<String>,
}

fn hotkey_config_from_settings(s: &Settings) -> anyhow::Result<HotkeyConfig> {
    let cfg = crate::settings::resolve_hotkey_config(s)?;
    Ok(HotkeyConfig {
        enabled: cfg.enabled,
        ptt: cfg.ptt,
        toggle: cfg.toggle,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyRecordEvent {
    pub kind: String,  // ptt|toggle
    pub state: String, // Pressed|Released
    pub shortcut: String,
    pub ts_ms: i64,
    pub recording_session_id: Option<String>,
    pub capture_status: Option<String>, // ok|err
    pub capture_error_code: Option<String>,
    pub capture_error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyAvailability {
    pub available: bool,
    pub reason: Option<String>,
    pub reason_code: Option<String>,
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn normalized_shortcut(raw: &str) -> String {
    raw.split('+')
        .map(|part| part.trim().to_ascii_uppercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("+")
}

#[tauri::command]
pub fn check_hotkey_available(
    app: AppHandle,
    shortcut: &str,
    ignore_self: Option<&str>,
) -> Result<HotkeyAvailability, String> {
    let candidate = normalized_shortcut(shortcut);
    if candidate.is_empty() {
        return Ok(HotkeyAvailability {
            available: false,
            reason: Some("shortcut is empty".to_string()),
            reason_code: Some("E_HOTKEY_SHORTCUT_EMPTY".to_string()),
        });
    }

    if let Some(ignore_self) = ignore_self {
        if !ignore_self.trim().is_empty()
            && candidate.eq_ignore_ascii_case(&normalized_shortcut(ignore_self))
        {
            return Ok(HotkeyAvailability {
                available: true,
                reason: None,
                reason_code: None,
            });
        }
    }

    let gs = app.global_shortcut();

    match gs.register(candidate.as_str()) {
        Ok(()) => match gs.unregister(candidate.as_str()) {
            Ok(()) => Ok(HotkeyAvailability {
                available: true,
                reason: None,
                reason_code: None,
            }),
            Err(e) => Ok(HotkeyAvailability {
                available: false,
                reason: Some(format!(
                    "registered but cleanup failed: {}",
                    e
                )),
                reason_code: Some("E_HOTKEY_CLEANUP_FAILED".to_string()),
            }),
        },
        Err(e) => Ok(HotkeyAvailability {
            available: false,
            reason: Some(e.to_string()),
            reason_code: Some("E_HOTKEY_REGISTER_FAILED".to_string()),
        }),
    }
}

pub struct HotkeyManager {
    // Ensures apply is serialized (settings updates may come quickly).
    lock: Mutex<()>,
    registered: Mutex<Vec<String>>,
}

impl Default for HotkeyManager {
    fn default() -> Self {
        Self {
            lock: Mutex::new(()),
            registered: Mutex::new(Vec::new()),
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
                "ptt": cfg.ptt.as_deref(),
                "toggle": cfg.toggle.as_deref(),
            })),
        );

        let gs = app.global_shortcut();

        // Only unregister shortcuts that this module registered before.
        {
            let mut prev = self.registered.lock().unwrap();
            for shortcut in prev.iter() {
                if let Err(e) = gs.unregister(shortcut.as_str()) {
                    crate::trace::event(
                        data_dir,
                        None,
                        "Hotkeys",
                        "HK.unregister_scoped",
                        "err",
                        Some(
                            serde_json::json!({"code": "E_HK_UNREGISTER_SCOPED", "shortcut": shortcut, "error": e.to_string()}),
                        ),
                    );
                }
            }
            prev.clear();
        }

        if !cfg.enabled {
            span.ok(Some(serde_json::json!({"status": "disabled"})));
            return;
        }

        let capture_required = matches!(
            crate::settings::resolve_rewrite_start_config(s),
            Ok((true, Some(_)))
        );
        let mut registered_now: Vec<String> = Vec::new();

        if let Some(ptt) = cfg.ptt.clone() {
            let ctx_cfg = crate::context_capture::config_from_settings(s);
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = gs.on_shortcut(ptt.as_str(), move |app, shortcut, event| {
                let (recording_session_id, capture_status, capture_error_code, capture_error_message) =
                    if event.state == ShortcutState::Pressed {
                        let tm = app.state::<crate::task_manager::TaskManager>();
                        if tm.has_active_task() {
                            (None, None, None, None)
                        } else {
                            match tm.open_recording_session(&data_dir_buf, &ctx_cfg, capture_required) {
                                Ok(id) => (Some(id), Some("ok".to_string()), None, None),
                                Err(e) => (
                                    None,
                                    Some("err".to_string()),
                                    Some("E_RECORDING_SESSION_OPEN".to_string()),
                                    Some(e.to_string()),
                                ),
                            }
                        }
                    } else {
                        (None, None, None, None)
                    };
                let payload = HotkeyRecordEvent {
                    kind: "ptt".to_string(),
                    state: match event.state {
                        ShortcutState::Pressed => "Pressed".to_string(),
                        ShortcutState::Released => "Released".to_string(),
                    },
                    shortcut: shortcut.into_string(),
                    ts_ms: now_ms(),
                    recording_session_id,
                    capture_status,
                    capture_error_code,
                    capture_error_message,
                };
                let _ = app.emit("tv_hotkey_record", payload);
            }) {
                crate::trace::event(
                    data_dir,
                    None,
                    "Hotkeys",
                    "HK.register.ptt",
                    "err",
                    Some(
                        serde_json::json!({"code": "E_HK_REGISTER_PTT", "ptt": ptt, "error": e.to_string()}),
                    ),
                );
            } else {
                registered_now.push(ptt);
            }
        }

        if let Some(toggle) = cfg.toggle.clone() {
            let ctx_cfg = crate::context_capture::config_from_settings(s);
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = gs.on_shortcut(toggle.as_str(), move |app, shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                let tm = app.state::<crate::task_manager::TaskManager>();
                let (recording_session_id, capture_status, capture_error_code, capture_error_message) =
                    if tm.has_active_task() {
                        (None, None, None, None)
                    } else {
                        match tm.open_recording_session(&data_dir_buf, &ctx_cfg, capture_required) {
                            Ok(id) => (Some(id), Some("ok".to_string()), None, None),
                            Err(e) => (
                                None,
                                Some("err".to_string()),
                                Some("E_RECORDING_SESSION_OPEN".to_string()),
                                Some(e.to_string()),
                            ),
                        }
                    };
                let payload = HotkeyRecordEvent {
                    kind: "toggle".to_string(),
                    state: "Pressed".to_string(),
                    shortcut: shortcut.into_string(),
                    ts_ms: now_ms(),
                    recording_session_id,
                    capture_status,
                    capture_error_code,
                    capture_error_message,
                };
                let _ = app.emit("tv_hotkey_record", payload);
            }) {
                crate::trace::event(
                    data_dir,
                    None,
                    "Hotkeys",
                    "HK.register.toggle",
                    "err",
                    Some(
                        serde_json::json!({"code": "E_HK_REGISTER_TOGGLE", "toggle": toggle, "error": e.to_string()}),
                    ),
                );
            } else {
                registered_now.push(toggle);
            }
        }

        {
            let mut current = self.registered.lock().unwrap();
            *current = registered_now;
        }
        span.ok(Some(serde_json::json!({"status": "ok"})));
    }
}

#[cfg(test)]
mod tests {
    use super::{hotkey_config_from_settings, HotkeyConfig};
    use crate::settings::Settings;

    fn cfg(s: Settings) -> anyhow::Result<HotkeyConfig> {
        hotkey_config_from_settings(&s)
    }

    #[test]
    fn config_requires_explicit_fields() {
        let err = cfg(Settings::default()).expect_err("should fail");
        assert!(err
            .to_string()
            .contains("E_SETTINGS_HOTKEYS_ENABLED_MISSING"));
    }

    #[test]
    fn disabled_means_no_keys() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(false);
        let c = cfg(s).expect("cfg");
        assert!(!c.enabled);
        assert!(c.ptt.is_none());
        assert!(c.toggle.is_none());
    }

    #[test]
    fn empty_shortcuts_are_invalid() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(true);
        s.hotkey_ptt = Some("   ".to_string());
        s.hotkey_toggle = Some("\n".to_string());
        let err = cfg(s).expect_err("should fail");
        assert!(err.to_string().contains("E_SETTINGS_HOTKEY_PTT_MISSING"));
    }

    #[test]
    fn same_key_is_invalid() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(true);
        s.hotkey_ptt = Some("F9".to_string());
        s.hotkey_toggle = Some("f9".to_string());
        let err = cfg(s).expect_err("should fail");
        assert!(err.to_string().contains("E_SETTINGS_HOTKEY_CONFLICT"));
    }
}
