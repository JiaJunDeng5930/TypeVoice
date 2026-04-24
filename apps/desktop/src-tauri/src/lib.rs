mod asr_service;
mod audio_capture;
mod audio_device_notifications_windows;
mod audio_devices_windows;
mod commands;
mod context_capture;
#[cfg(windows)]
mod context_capture_windows;
mod context_pack;
mod data_dir;
mod doubao_asr;
mod export;
mod history;
mod hotkeys;
mod insertion;
mod llm;
mod model;
mod obs;
mod pipeline;
mod ports;
mod python_runtime;
mod record_input;
mod record_input_cache;
mod remote_asr;
mod rewrite;
mod safe_print;
mod settings;
mod subprocess;
mod task_manager;
mod templates;
mod toolchain;
mod transcription;
mod transcription_actor;
mod ui_events;
mod voice_tasks;
mod voice_workflow;

use history::HistoryItem;
use llm::ApiKeyStatus;
use model::ModelStatus;
use obs::Span;
use settings::Settings;
use settings::SettingsPatch;
use task_manager::TaskManager;
use tauri::Emitter;
use tauri::Manager;
use templates::PromptTemplate;

pub(crate) struct RuntimeState {
    toolchain: std::sync::Mutex<toolchain::ToolchainStatus>,
    python: std::sync::Mutex<python_runtime::PythonStatus>,
}

impl RuntimeState {
    fn new() -> Self {
        Self {
            toolchain: std::sync::Mutex::new(toolchain::ToolchainStatus::pending()),
            python: std::sync::Mutex::new(python_runtime::PythonStatus::pending()),
        }
    }

    fn set_toolchain(&self, st: toolchain::ToolchainStatus) {
        let mut g = self.toolchain.lock().unwrap();
        *g = st;
    }

    pub(crate) fn get_toolchain(&self) -> toolchain::ToolchainStatus {
        self.toolchain.lock().unwrap().clone()
    }

    fn set_python(&self, st: python_runtime::PythonStatus) {
        let mut g = self.python.lock().unwrap();
        *g = st;
    }

    pub(crate) fn get_python(&self) -> python_runtime::PythonStatus {
        self.python.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OverlayState {
    visible: bool,
    status: String,
    detail: Option<String>,
    ts_ms: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiLogEventRequest {
    kind: String,
    code: Option<String>,
    title: Option<String>,
    message: Option<String>,
    detail: Option<String>,
    action_hint: Option<String>,
    tone: Option<String>,
    tab: Option<String>,
    screen: Option<String>,
    command: Option<String>,
    trigger_source: Option<String>,
    task_id: Option<String>,
    ts_ms: Option<i64>,
    extra: Option<serde_json::Value>,
}

fn sanitize_ui_text(raw: Option<String>, max_chars: usize) -> Option<String> {
    let input = raw?;
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return None;
    }
    let redacted = crate::obs::trace::redact_user_paths(&compact).replace('\0', "");
    let mut out = String::with_capacity(std::cmp::min(redacted.len(), max_chars));
    for (idx, ch) in redacted.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        out.push(ch);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parse_ui_error_code(raw: &str) -> Option<String> {
    for token in raw.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if token.starts_with("E_") && token.len() > 2 {
            return Some(token.to_string());
        }
        if token.starts_with("HTTP_")
            && token.len() > 5
            && token[5..].chars().all(|c| c.is_ascii_digit())
        {
            return Some(token.to_string());
        }
    }
    None
}

#[tauri::command]
fn ui_log_event(req: UiLogEventRequest) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;

    let UiLogEventRequest {
        kind,
        code,
        title,
        message,
        detail,
        action_hint,
        tone,
        tab,
        screen,
        command,
        trigger_source,
        task_id,
        ts_ms,
        extra,
    } = req;

    let norm_kind = sanitize_ui_text(Some(kind.clone()), 40).unwrap_or_else(|| "event".to_string());
    let step_id = match norm_kind.as_str() {
        "toast" => "UI.toast",
        "diagnostic" => "UI.diagnostic",
        "invoke_error" => "UI.invoke_error",
        _ => "UI.event",
    };
    let fallback_code = match step_id {
        "UI.toast" => "E_UI_TOAST",
        "UI.diagnostic" => "E_UI_DIAGNOSTIC",
        "UI.invoke_error" => "E_UI_INVOKE",
        _ => "E_UI_EVENT",
    };

    let norm_message = sanitize_ui_text(message, 240);
    let norm_detail = sanitize_ui_text(detail, 320);
    let norm_title = sanitize_ui_text(title, 80);
    let norm_action_hint = sanitize_ui_text(action_hint, 160);
    let norm_tone = sanitize_ui_text(tone, 24);
    let norm_tab = sanitize_ui_text(tab, 24);
    let norm_screen = sanitize_ui_text(screen, 48);
    let norm_command = sanitize_ui_text(command, 80);
    let norm_trigger_source = sanitize_ui_text(trigger_source, 32);
    let mut norm_code = sanitize_ui_text(code, 64);
    if norm_code.is_none() {
        if let Some(ref message_text) = norm_message {
            norm_code = parse_ui_error_code(message_text);
        }
    }
    if norm_code.is_none() {
        if let Some(ref detail_text) = norm_detail {
            norm_code = parse_ui_error_code(detail_text);
        }
    }
    let final_code = norm_code.unwrap_or_else(|| fallback_code.to_string());
    let final_message = norm_message
        .or_else(|| norm_detail.clone())
        .or_else(|| norm_title.clone())
        .unwrap_or_else(|| "ui event".to_string());

    let mut ctx = serde_json::Map::new();
    ctx.insert("kind".to_string(), serde_json::json!(norm_kind));
    if let Some(v) = norm_title {
        ctx.insert("title".to_string(), serde_json::json!(v));
    }
    if let Some(v) = norm_detail {
        ctx.insert("detail".to_string(), serde_json::json!(v));
    }
    if let Some(v) = norm_action_hint {
        ctx.insert("action_hint".to_string(), serde_json::json!(v));
    }
    if let Some(v) = norm_tone {
        ctx.insert("tone".to_string(), serde_json::json!(v));
    }
    if let Some(v) = norm_tab {
        ctx.insert("tab".to_string(), serde_json::json!(v));
    }
    if let Some(v) = norm_screen {
        ctx.insert("screen".to_string(), serde_json::json!(v));
    }
    if let Some(v) = norm_command {
        ctx.insert("command".to_string(), serde_json::json!(v));
    }
    if let Some(v) = norm_trigger_source {
        ctx.insert("trigger_source".to_string(), serde_json::json!(v));
    }
    if let Some(v) = ts_ms {
        ctx.insert("ui_ts_ms".to_string(), serde_json::json!(v));
    }
    if let Some(v) = extra {
        ctx.insert("extra".to_string(), v);
    }

    crate::obs::event_err(
        &dir,
        task_id.as_deref(),
        "UI",
        step_id,
        "ui",
        &final_code,
        &final_message,
        Some(serde_json::Value::Object(ctx)),
    );
    Ok(())
}

#[tauri::command]
fn overlay_set_state(app: tauri::AppHandle, state: OverlayState) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.overlay_set_state",
        Some(serde_json::json!({
            "visible": state.visible,
            "status": state.status,
            "has_detail": state.detail.as_deref().map(|s| !s.is_empty()).unwrap_or(false),
        })),
    );

    if let Some(w) = app.get_webview_window("overlay") {
        if state.visible {
            let _ = w.show();
        } else {
            let _ = w.hide();
        }
    }

    // Broadcast: the overlay window listens and updates its UI.
    let _ = app.emit("tv_overlay_state", state);
    span.ok(None);
    Ok(())
}

fn cmd_span(
    data_dir: &std::path::Path,
    task_id: Option<&str>,
    step_id: &str,
    ctx: Option<serde_json::Value>,
) -> Span {
    Span::start(data_dir, task_id, "Cmd", step_id, ctx)
}

fn repo_root() -> Result<std::path::PathBuf, String> {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "repo root not found".to_string())
}

#[tauri::command]
fn runtime_toolchain_status(
    runtime: tauri::State<'_, RuntimeState>,
) -> Result<toolchain::ToolchainStatus, String> {
    Ok(runtime.get_toolchain())
}

#[tauri::command]
fn runtime_python_status(
    runtime: tauri::State<'_, RuntimeState>,
) -> Result<python_runtime::PythonStatus, String> {
    Ok(runtime.get_python())
}

#[tauri::command]
fn abort_pending_task(
    workflow: tauri::State<voice_workflow::VoiceWorkflow>,
    task_id: &str,
) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.abort_pending_task",
        Some(serde_json::json!({"has_task_id": !task_id.trim().is_empty()})),
    );
    if task_id.trim().is_empty() {
        span.ok(Some(serde_json::json!({"removed": false})));
        return Ok(());
    }
    let removed = workflow.abort_pending_task(task_id.trim());
    span.ok(Some(serde_json::json!({"removed": removed})));
    Ok(())
}

#[tauri::command]
fn list_templates() -> Result<Vec<PromptTemplate>, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.list_templates", None);
    match templates::load_templates(&dir) {
        Ok(v) => {
            span.ok(Some(serde_json::json!({"count": v.len()})));
            Ok(v)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_LIST", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn upsert_template(tpl: PromptTemplate) -> Result<PromptTemplate, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let tpl_id = tpl.id.clone();
    let has_id = !tpl_id.trim().is_empty();
    let name_chars = tpl.name.len();
    let prompt_chars = tpl.system_prompt.len();
    let span = cmd_span(
        &dir,
        None,
        "CMD.upsert_template",
        Some(
            serde_json::json!({"has_id": has_id, "id": tpl_id, "name_chars": name_chars, "prompt_chars": prompt_chars}),
        ),
    );
    match templates::upsert_template(&dir, tpl) {
        Ok(v) => {
            span.ok(Some(serde_json::json!({"id": v.id})));
            Ok(v)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_UPSERT", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn delete_template(id: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.delete_template",
        Some(serde_json::json!({"id": id})),
    );
    match templates::delete_template(&dir, id) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_DELETE", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn templates_export_json() -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.templates_export_json", None);
    match templates::export_templates_json(&dir) {
        Ok(s) => {
            span.ok(Some(serde_json::json!({"bytes": s.len()})));
            Ok(s)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_EXPORT", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn templates_import_json(json: &str, mode: &str) -> Result<usize, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.templates_import_json",
        Some(serde_json::json!({"mode": mode, "json_chars": json.len()})),
    );
    match templates::import_templates_json(&dir, json, mode) {
        Ok(n) => {
            span.ok(Some(serde_json::json!({"count": n})));
            Ok(n)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_IMPORT", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn set_llm_api_key(api_key: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.set_llm_api_key",
        Some(serde_json::json!({"api_key_chars": api_key.len()})),
    );
    match llm::set_api_key(api_key) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_SET_KEY", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn clear_llm_api_key() -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.clear_llm_api_key", None);
    match llm::clear_api_key() {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_CLEAR_KEY", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn llm_api_key_status() -> Result<ApiKeyStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.llm_api_key_status", None);
    let st = llm::api_key_status();
    span.ok(Some(
        serde_json::json!({"configured": st.configured, "source": st.source, "reason": st.reason}),
    ));
    Ok(st)
}

#[tauri::command]
fn set_remote_asr_api_key(api_key: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.set_remote_asr_api_key",
        Some(serde_json::json!({"api_key_chars": api_key.len()})),
    );
    match remote_asr::set_api_key(api_key) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_SET_REMOTE_ASR_KEY", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn clear_remote_asr_api_key() -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.clear_remote_asr_api_key", None);
    match remote_asr::clear_api_key() {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_CLEAR_REMOTE_ASR_KEY", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn remote_asr_api_key_status() -> Result<ApiKeyStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.remote_asr_api_key_status", None);
    let st = remote_asr::api_key_status();
    span.ok(Some(
        serde_json::json!({"configured": st.configured, "source": st.source, "reason": st.reason}),
    ));
    Ok(st)
}

#[tauri::command]
fn set_doubao_asr_credentials(app_key: &str, access_key: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.set_doubao_asr_credentials",
        Some(serde_json::json!({
            "app_key_chars": app_key.len(),
            "access_key_chars": access_key.len(),
        })),
    );
    match doubao_asr::set_credentials(app_key, access_key) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_SET_DOUBAO_ASR_CREDENTIALS", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn clear_doubao_asr_credentials() -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.clear_doubao_asr_credentials", None);
    match doubao_asr::clear_credentials() {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_CLEAR_DOUBAO_ASR_CREDENTIALS", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn doubao_asr_credentials_status() -> Result<ApiKeyStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.doubao_asr_credentials_status", None);
    let st = doubao_asr::credentials_status();
    span.ok(Some(
        serde_json::json!({"configured": st.configured, "source": st.source, "reason": st.reason}),
    ));
    Ok(st)
}

fn history_db_path() -> Result<std::path::PathBuf, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("history.sqlite3"))
}

#[tauri::command]
fn history_append(item: HistoryItem) -> Result<(), String> {
    let db = history_db_path()?;
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        Some(item.task_id.as_str()),
        "CMD.history_append",
        None,
    );
    match history::append(&db, &item) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("history", "E_CMD_HISTORY_APPEND", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn history_list(limit: i64, before_ms: Option<i64>) -> Result<Vec<HistoryItem>, String> {
    let db = history_db_path()?;
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.history_list",
        Some(serde_json::json!({"limit": limit, "before_ms": before_ms})),
    );
    match history::list(&db, limit, before_ms) {
        Ok(v) => {
            span.ok(Some(serde_json::json!({"count": v.len()})));
            Ok(v)
        }
        Err(e) => {
            span.err_anyhow("history", "E_CMD_HISTORY_LIST", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn history_clear() -> Result<(), String> {
    let db = history_db_path()?;
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.history_clear", None);
    match history::clear(&db) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("history", "E_CMD_HISTORY_CLEAR", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn get_settings() -> Result<Settings, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.get_settings", None);
    match settings::load_settings_strict(&dir) {
        Ok(s) => {
            span.ok(Some(
                serde_json::json!({"rewrite_enabled": s.rewrite_enabled, "template_id": s.rewrite_template_id}),
            ));
            Ok(s)
        }
        Err(e) => {
            span.err_anyhow("settings", "E_CMD_GET_SETTINGS", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn list_audio_capture_devices() -> Result<Vec<record_input::AudioCaptureDeviceView>, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.list_audio_capture_devices", None);
    match record_input::list_audio_capture_devices_for_settings() {
        Ok(items) => {
            span.ok(Some(serde_json::json!({
                "count": items.len(),
            })));
            Ok(items)
        }
        Err(e) => {
            span.err("io", "E_RECORD_INPUT_ENUM_FAILED", &e, None);
            Err(e)
        }
    }
}

#[tauri::command]
fn set_settings(
    s: Settings,
    record_input_cache: tauri::State<'_, record_input_cache::RecordInputCacheState>,
) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.set_settings", None);
    match settings::save_settings(&dir, &s) {
        Ok(()) => {
            if cfg!(windows) {
                let _ = record_input_cache.refresh_blocking(&dir, "set_settings");
            }
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("settings", "E_CMD_SET_SETTINGS", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn update_settings(
    app: tauri::AppHandle,
    transcriber: tauri::State<transcription::TranscriptionService>,
    hotkeys: tauri::State<hotkeys::HotkeyManager>,
    record_input_cache: tauri::State<record_input_cache::RecordInputCacheState>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let patch_summary = serde_json::json!({
        "asr_model": patch.asr_model.is_some(),
        "asr_provider": patch.asr_provider.is_some(),
        "remote_asr_url": patch.remote_asr_url.is_some(),
        "remote_asr_model": patch.remote_asr_model.is_some(),
        "remote_asr_concurrency": patch.remote_asr_concurrency.is_some(),
        "llm_base_url": patch.llm_base_url.is_some(),
        "llm_model": patch.llm_model.is_some(),
        "llm_reasoning_effort": patch.llm_reasoning_effort.is_some(),
        "record_input_strategy": patch.record_input_strategy.is_some(),
        "record_follow_default_role": patch.record_follow_default_role.is_some(),
        "record_fixed_endpoint_id": patch.record_fixed_endpoint_id.is_some(),
        "record_fixed_friendly_name": patch.record_fixed_friendly_name.is_some(),
        "rewrite_enabled": patch.rewrite_enabled.is_some(),
        "rewrite_template_id": patch.rewrite_template_id.is_some(),
        "rewrite_glossary": patch.rewrite_glossary.is_some(),
        "auto_paste_enabled": patch.auto_paste_enabled.is_some(),
        "rewrite_include_glossary": patch.rewrite_include_glossary.is_some(),
        "context_include_history": patch.context_include_history.is_some(),
        "context_history_n": patch.context_history_n.is_some(),
        "context_history_window_ms": patch.context_history_window_ms.is_some(),
        "context_include_clipboard": patch.context_include_clipboard.is_some(),
        "context_include_prev_window_meta": patch.context_include_prev_window_meta.is_some(),
        "context_include_prev_window_screenshot": patch.context_include_prev_window_screenshot.is_some(),
        "llm_supports_vision": patch.llm_supports_vision.is_some(),
        "hotkeys_enabled": patch.hotkeys_enabled.is_some(),
        "hotkey_ptt": patch.hotkey_ptt.is_some(),
        "hotkey_toggle": patch.hotkey_toggle.is_some(),
        "hotkeys_show_overlay": patch.hotkeys_show_overlay.is_some(),
        "asr_preprocess_silence_trim_enabled": patch.asr_preprocess_silence_trim_enabled.is_some(),
        "asr_preprocess_silence_threshold_db": patch
            .asr_preprocess_silence_threshold_db
            .is_some(),
        "asr_preprocess_silence_start_ms": patch.asr_preprocess_silence_start_ms.is_some(),
        "asr_preprocess_silence_end_ms": patch.asr_preprocess_silence_end_ms.is_some(),
    });
    let span = cmd_span(&dir, None, "CMD.update_settings", Some(patch_summary));
    let cur = match settings::load_settings_strict(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("settings", "E_CMD_UPDATE_SETTINGS_LOAD", &e, None);
            return Err(e.to_string());
        }
    };
    let asr_model_changed = patch.asr_model.is_some();
    let asr_provider_changed = patch.asr_provider.is_some();
    let record_input_changed = patch.record_input_strategy.is_some()
        || patch.record_follow_default_role.is_some()
        || patch.record_fixed_endpoint_id.is_some()
        || patch.record_fixed_friendly_name.is_some()
        || patch.record_input_spec.is_some();
    let mut next = settings::apply_patch(cur, patch);
    next.record_input_strategy = Some(
        next.record_input_strategy
            .as_deref()
            .and_then(record_input::normalize_strategy_for_settings)
            .unwrap_or(record_input::default_strategy())
            .to_string(),
    );
    next.record_follow_default_role = Some(
        next.record_follow_default_role
            .as_deref()
            .and_then(record_input::normalize_default_role_for_settings)
            .unwrap_or(record_input::default_role())
            .to_string(),
    );
    if next.record_input_strategy.as_deref() != Some("fixed_device") {
        next.record_fixed_endpoint_id = None;
        next.record_fixed_friendly_name = None;
    } else {
        let fixed_id = next
            .record_fixed_endpoint_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
        if fixed_id.is_none() {
            let msg =
                "E_RECORD_INPUT_FIXED_MISSING: record_fixed_endpoint_id is required when strategy=fixed_device";
            span.err("config", "E_RECORD_INPUT_FIXED_MISSING", msg, None);
            return Err(msg.to_string());
        }
        next.record_fixed_endpoint_id = fixed_id;
        next.record_fixed_friendly_name = next
            .record_fixed_friendly_name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
    }
    if let Err(e) = settings::save_settings(&dir, &next) {
        span.err_anyhow("settings", "E_CMD_UPDATE_SETTINGS", &e, None);
        return Err(e.to_string());
    }
    let asr_provider = settings::resolve_asr_provider(&next);
    if asr_provider == "local" {
        if asr_model_changed || asr_provider_changed {
            transcriber.restart_asr_best_effort("settings_changed");
        }
    } else if asr_model_changed || asr_provider_changed {
        transcriber.kill_asr_best_effort("settings_changed_remote");
    }

    // Hotkeys are also best-effort; failures are traced and should not break settings.
    hotkeys.apply_from_settings_best_effort(&app, &dir, &next);
    if cfg!(windows) && record_input_changed {
        let _ = record_input_cache.refresh_blocking(&dir, "settings_changed");
    }

    span.ok(None);
    Ok(next)
}

#[tauri::command]
fn asr_model_status() -> Result<ModelStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.asr_model_status", None);
    let model_id = match pipeline::resolve_asr_model_id(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("model", "E_CMD_MODEL_ID", &e, None);
            return Err(e.to_string());
        }
    };

    let st = if std::path::Path::new(&model_id).exists() {
        match model::verify_model_dir(std::path::Path::new(&model_id)) {
            Ok(st) => st,
            Err(e) => {
                span.err_anyhow("model", "E_CMD_MODEL_STATUS", &e, None);
                return Err(e.to_string());
            }
        }
    } else {
        ModelStatus {
            model_dir: model_id,
            ok: true,
            reason: Some("remote_model_not_locally_verified".to_string()),
            model_version: None,
        }
    };
    let _ok = st.ok;
    span.ok(Some(
        serde_json::json!({"ok": st.ok, "reason": st.reason, "model_version": st.model_version}),
    ));
    Ok(st)
}

#[tauri::command]
async fn download_asr_model() -> Result<ModelStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.download_asr_model", None);
    let root = repo_root()?;
    let model_dir = model::default_model_dir(&root);
    let py = match python_runtime::resolve_python_binary(&root) {
        Ok(p) => p,
        Err(e) => {
            span.err_anyhow("config", "E_PYTHON_NOT_READY", &e, None);
            return Err(e.to_string());
        }
    };
    let root2 = root.clone();
    let py2 = py.clone();
    let model_dir2 = model_dir.clone();
    let st_res = tauri::async_runtime::spawn_blocking(move || {
        model::download_model(&root2, &py2, &model_dir2)
    })
    .await;
    let st = match st_res {
        Ok(Ok(st)) => st,
        Ok(Err(e)) => {
            span.err_anyhow("model", "E_CMD_MODEL_DOWNLOAD", &e, None);
            return Err(e.to_string());
        }
        Err(e) => {
            let ae = anyhow::anyhow!("spawn_blocking failed: {e}");
            span.err_anyhow("runtime", "E_CMD_JOIN", &ae, None);
            return Err(ae.to_string());
        }
    };
    // Set settings.asr_model to local dir if ok.
    if st.ok {
        let mut s = match settings::load_settings_strict(&dir) {
            Ok(v) => v,
            Err(e) => {
                span.err_anyhow("settings", "E_CMD_MODEL_DOWNLOAD_SETTINGS", &e, None);
                return Err(e.to_string());
            }
        };
        s.asr_model = Some(model_dir.display().to_string());
        if let Err(e) = settings::save_settings(&dir, &s) {
            span.err_anyhow("settings", "E_CMD_MODEL_DOWNLOAD_SAVE", &e, None);
            return Err(e.to_string());
        }
    }
    span.ok(Some(
        serde_json::json!({"ok": st.ok, "reason": st.reason, "model_version": st.model_version}),
    ));
    Ok(st)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    obs::startup::mark_best_effort("run_enter");
    obs::panic::install_best_effort();
    obs::startup::mark_best_effort("panic_hook_installed");
    let ctx = tauri::generate_context!();
    obs::startup::mark_best_effort("context_generated");
    tauri::Builder::default()
        .manage(TaskManager::new())
        .manage(voice_workflow::VoiceWorkflow::new())
        .manage(transcription::TranscriptionService::new())
        .manage(audio_capture::RecordingRegistry::new())
        .manage(RuntimeState::new())
        .manage(record_input_cache::RecordInputCacheState::new())
        .manage(audio_device_notifications_windows::AudioDeviceNotificationState::new())
        .manage(hotkeys::HotkeyManager::new())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            #[derive(Clone, serde::Serialize)]
            struct Payload {
                args: Vec<String>,
                cwd: String,
            }

            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            let _ = app.emit("tv_single_instance", Payload { args: argv, cwd });

            if let Ok(dir) = data_dir::data_dir() {
                obs::event(
                    &dir,
                    None,
                    "App",
                    "APP.single_instance",
                    "ok",
                    Some(serde_json::json!({"note": "second_instance_redirected"})),
                );
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            obs::startup::mark_best_effort("setup_enter");
            let mailbox = ui_events::UiEventMailbox::new(app.handle().clone());
            app.manage(transcription_actor::TranscriptionActor::new(mailbox.clone()));
            app.manage(mailbox);

            // Small always-on-top overlay window for hotkey-driven UX.
            // Keep it hidden by default; the frontend will invoke overlay_set_state to show/hide.
            let _overlay = tauri::WebviewWindowBuilder::new(
                app,
                "overlay",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("TypeVoice Overlay")
            .inner_size(240.0, 64.0)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .visible(false)
            .skip_taskbar(true)
            .focused(false)
            .build();

            let mut toolchain_ready = false;
            let mut python_ready = false;
            if let Ok(dir) = data_dir::data_dir() {
                let runtime = app.state::<RuntimeState>();
                let st = toolchain::initialize_and_verify(&app.handle(), &dir);
                toolchain_ready = st.ready;
                runtime.set_toolchain(st);
                if let Ok(root) = repo_root() {
                    let py = python_runtime::initialize_and_verify(&dir, &root);
                    python_ready = py.ready;
                    runtime.set_python(py);
                } else {
                    let py = python_runtime::PythonStatus {
                        ready: false,
                        code: Some("E_PYTHON_NOT_READY".to_string()),
                        message: Some("E_PYTHON_NOT_READY: repo root not found".to_string()),
                        python_path: None,
                        python_version: None,
                    };
                    runtime.set_python(py);
                }

                if cfg!(windows) {
                    let record_input_cache = app.state::<record_input_cache::RecordInputCacheState>();
                    if toolchain_ready {
                        let _ = record_input_cache.refresh_blocking(&dir, "app_startup");
                        let listener =
                            app.state::<audio_device_notifications_windows::AudioDeviceNotificationState>();
                        listener.start_best_effort(&dir, record_input_cache.inner().clone());
                    } else {
                        obs::event(
                            &dir,
                            None,
                            "App",
                            "APP.record_input_cache_refresh_skipped",
                            "ok",
                            Some(serde_json::json!({
                                "reason": "toolchain_not_ready",
                            })),
                        );
                    }
                }
            }

            // Warm up the ASR runner in background so first transcription is fast.
            // If runtime preflight failed, skip warmup to avoid noisy startup failures.
            if toolchain_ready && python_ready {
                let transcriber = app.state::<transcription::TranscriptionService>();
                transcriber.warmup_asr_best_effort();
                let state = app.state::<TaskManager>();
                state.warmup_context_best_effort();
            }

            // Apply hotkeys from persisted settings.
            if let Ok(dir) = data_dir::data_dir() {
                match settings::load_settings_strict(&dir) {
                    Ok(s) => {
                        let hk = app.state::<hotkeys::HotkeyManager>();
                        hk.apply_from_settings_best_effort(&app.handle(), &dir, &s);
                    }
                    Err(e) => {
                        obs::event(
                            &dir,
                            None,
                            "App",
                            "APP.hotkeys_init",
                            "err",
                            Some(serde_json::json!({
                                "code": "E_SETTINGS_INVALID",
                                "error": e.to_string()
                            })),
                        );
                    }
                }
            }

            obs::startup::mark_best_effort("setup_exit");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::record_transcribe_start,
            commands::record_transcribe_stop,
            commands::record_transcribe_cancel,
            commands::rewrite_text,
            commands::insert_text,
            commands::workflow_snapshot,
            commands::workflow_command,
            commands::workflow_apply_event,
            commands::transcribe_fixture,
            abort_pending_task,
            list_templates,
            upsert_template,
            delete_template,
            templates_export_json,
            templates_import_json,
            set_llm_api_key,
            clear_llm_api_key,
            llm_api_key_status,
            set_remote_asr_api_key,
            clear_remote_asr_api_key,
            remote_asr_api_key_status,
            set_doubao_asr_credentials,
            clear_doubao_asr_credentials,
            doubao_asr_credentials_status,
            history_append,
            history_list,
            history_clear,
            get_settings,
            list_audio_capture_devices,
            set_settings,
            update_settings,
            hotkeys::check_hotkey_available,
            runtime_toolchain_status,
            runtime_python_status,
            overlay_set_state,
            ui_log_event,
            asr_model_status,
            download_asr_model
        ])
        .run(ctx)
        .expect("error while running tauri application");
}
