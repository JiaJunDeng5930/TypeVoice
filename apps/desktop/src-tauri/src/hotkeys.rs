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
    pub capture_id: Option<String>,
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
fn check_hotkey_available(
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
            let ctx_cfg = crate::context_capture::config_from_settings(s);
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = gs.on_shortcut(ptt.as_str(), move |app, shortcut, event| {
                let (capture_id, capture_status, capture_error_code, capture_error_message) =
                    if event.state == ShortcutState::Pressed {
                        let tm = app.state::<crate::task_manager::TaskManager>();
                        match tm.capture_hotkey_context_now(&data_dir_buf, &ctx_cfg) {
                            Ok(id) => (Some(id), Some("ok".to_string()), None, None),
                            Err(e) => (
                                None,
                                Some("err".to_string()),
                                Some("E_HOTKEY_CAPTURE".to_string()),
                                Some(e.to_string()),
                            ),
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
                    capture_id,
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
                let (capture_id, capture_status, capture_error_code, capture_error_message) =
                    match tm.capture_hotkey_context_now(&data_dir_buf, &ctx_cfg) {
                        Ok(id) => (Some(id), Some("ok".to_string()), None, None),
                        Err(e) => (
                            None,
                            Some("err".to_string()),
                            Some("E_HOTKEY_CAPTURE".to_string()),
                            Some(e.to_string()),
                        ),
                    };
                let payload = HotkeyRecordEvent {
                    kind: "toggle".to_string(),
                    state: "Pressed".to_string(),
                    shortcut: shortcut.into_string(),
                    ts_ms: now_ms(),
                    capture_id,
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
            }
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
