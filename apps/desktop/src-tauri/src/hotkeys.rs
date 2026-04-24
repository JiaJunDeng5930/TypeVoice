use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::obs::Span;
use crate::settings::Settings;

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
pub struct HotkeyAvailability {
    pub available: bool,
    pub reason: Option<String>,
    pub reason_code: Option<String>,
}

fn normalized_shortcut(raw: &str) -> String {
    raw.split('+')
        .map(|part| part.trim().to_ascii_uppercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("+")
}

fn spawn_primary_workflow_command<R: Runtime>(
    app: AppHandle<R>,
    data_dir: std::path::PathBuf,
    task_id: Option<String>,
    kind: &'static str,
    state: &'static str,
    shortcut: String,
) {
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<crate::RuntimeState>();
        let workflow = app.state::<crate::voice_workflow::VoiceWorkflow>();
        let audio = app.state::<crate::audio_capture::RecordingRegistry>();
        let transcriber = app.state::<crate::transcription::TranscriptionService>();
        let streaming_actor = app.state::<crate::transcription_actor::TranscriptionActor>();
        let mailbox = app.state::<crate::ui_events::UiEventMailbox>();
        let record_input_cache = app.state::<crate::record_input_cache::RecordInputCacheState>();
        let task_state = app.state::<crate::task_manager::TaskManager>();
        let result = workflow
            .run_command(
                &runtime,
                &audio,
                &transcriber,
                &streaming_actor,
                &mailbox,
                &record_input_cache,
                &task_state,
                crate::voice_workflow::WorkflowCommandRequest {
                    command: crate::voice_workflow::WorkflowCommand::Primary,
                    task_id,
                },
            )
            .await;
        match result {
            Ok(outcome) => {
                if let Some(task) = outcome.task {
                    crate::voice_tasks::spawn(app.clone(), task);
                }
            }
            Err(err) => {
                let message = err.render();
                crate::obs::event_err(
                    &data_dir,
                    None,
                    "Hotkeys",
                    "HK.workflow_command_failed",
                    "logic",
                    &err.code,
                    &message,
                    Some(serde_json::json!({
                        "kind": kind,
                        "state": state,
                        "shortcut": shortcut,
                    })),
                );
                mailbox.send(crate::ui_events::UiEvent::error(
                    "hotkey", err.code, message,
                ));
            }
        }
    });
}

fn open_hotkey_task_or_report<R: Runtime>(
    app: &AppHandle<R>,
    data_dir: &Path,
    ctx_cfg: &crate::context_capture::ContextConfig,
    capture_required: bool,
    kind: &str,
    shortcut: &str,
) -> Option<String> {
    let tm = app.state::<crate::task_manager::TaskManager>();
    let workflow = app.state::<crate::voice_workflow::VoiceWorkflow>();
    if workflow.has_active_task() {
        let active_task_id = workflow.active_task_id_best_effort();
        crate::obs::event(
            data_dir,
            active_task_id.as_deref(),
            "Hotkeys",
            "HK.hotkey_active_forwarded",
            "ok",
            Some(serde_json::json!({
                "kind": kind,
                "shortcut": shortcut,
                "capture_required": capture_required,
                "active_task_id": active_task_id,
            })),
        );
        return None;
    }

    match workflow.open_hotkey_task(&tm, data_dir, ctx_cfg, capture_required) {
        Ok(id) => Some(id),
        Err(e) => {
            let code = e.code.clone();
            let message = e.render();
            crate::obs::event_err(
                data_dir,
                None,
                "Hotkeys",
                "HK.task_open_failed",
                "logic",
                &code,
                &message,
                Some(serde_json::json!({
                    "kind": kind,
                    "shortcut": shortcut,
                    "capture_required": capture_required,
                })),
            );
            let mailbox = app.state::<crate::ui_events::UiEventMailbox>();
            mailbox.send(crate::ui_events::UiEvent::error("hotkey", code, message));
            None
        }
    }
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
                reason: Some(format!("registered but cleanup failed: {}", e)),
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
                    crate::obs::event(
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

        let capture_required = s.rewrite_enabled.unwrap_or(false)
            && s.rewrite_template_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some();
        let mut registered_now: Vec<String> = Vec::new();

        if let Some(ptt) = cfg.ptt.clone() {
            let ctx_cfg = crate::context_capture::config_from_settings(s);
            let data_dir_buf = data_dir.to_path_buf();
            if let Err(e) = gs.on_shortcut(ptt.as_str(), move |app, shortcut, event| {
                let shortcut_value = shortcut.into_string();
                match event.state {
                    ShortcutState::Pressed => {
                        let task_id = open_hotkey_task_or_report(
                            &app,
                            &data_dir_buf,
                            &ctx_cfg,
                            capture_required,
                            "ptt",
                            &shortcut_value,
                        );
                        if task_id.is_some() {
                            spawn_primary_workflow_command(
                                app.clone(),
                                data_dir_buf.clone(),
                                task_id,
                                "ptt",
                                "Pressed",
                                shortcut_value,
                            );
                        }
                    }
                    ShortcutState::Released => {
                        let workflow = app.state::<crate::voice_workflow::VoiceWorkflow>();
                        if workflow.phase() == crate::voice_workflow::WorkflowPhase::Recording {
                            spawn_primary_workflow_command(
                                app.clone(),
                                data_dir_buf.clone(),
                                None,
                                "ptt",
                                "Released",
                                shortcut_value,
                            );
                        }
                    }
                }
            }) {
                crate::obs::event(
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
                let shortcut_value = shortcut.into_string();
                if event.state != ShortcutState::Pressed {
                    return;
                }
                let workflow = app.state::<crate::voice_workflow::VoiceWorkflow>();
                let task_id = if workflow.has_active_task() {
                    None
                } else {
                    open_hotkey_task_or_report(
                        &app,
                        &data_dir_buf,
                        &ctx_cfg,
                        capture_required,
                        "toggle",
                        &shortcut_value,
                    )
                };
                if workflow.has_active_task() || task_id.is_some() {
                    spawn_primary_workflow_command(
                        app.clone(),
                        data_dir_buf.clone(),
                        task_id,
                        "toggle",
                        "Pressed",
                        shortcut_value,
                    );
                }
            }) {
                crate::obs::event(
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
