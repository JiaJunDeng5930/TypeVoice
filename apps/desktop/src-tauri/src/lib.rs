mod asr_service;
mod context_capture;
#[cfg(windows)]
mod context_capture_windows;
mod context_pack;
mod data_dir;
mod debug_log;
mod history;
mod hotkeys;
mod llm;
mod metrics;
mod model;
mod panic_log;
mod pipeline;
mod python_runtime;
mod safe_print;
mod settings;
mod startup_trace;
mod task_manager;
mod templates;
mod toolchain;
mod trace;

use history::HistoryItem;
use llm::ApiKeyStatus;
use model::ModelStatus;
use pipeline::TranscribeResult;
use settings::Settings;
use settings::SettingsPatch;
use task_manager::TaskManager;
use tauri::Emitter;
use tauri::Manager;
use templates::PromptTemplate;
use trace::Span;

struct RuntimeState {
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

    fn get_toolchain(&self) -> toolchain::ToolchainStatus {
        self.toolchain.lock().unwrap().clone()
    }

    fn set_python(&self, st: python_runtime::PythonStatus) {
        let mut g = self.python.lock().unwrap();
        *g = st;
    }

    fn get_python(&self) -> python_runtime::PythonStatus {
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

fn runtime_not_ready(runtime: &RuntimeState) -> Option<(&'static str, String)> {
    let tc = runtime.get_toolchain();
    if !tc.ready {
        let msg = tc
            .message
            .unwrap_or_else(|| "E_TOOLCHAIN_NOT_READY: toolchain is not ready".to_string());
        return Some(("E_TOOLCHAIN_NOT_READY", msg));
    }

    let py = runtime.get_python();
    if !py.ready {
        let msg = py
            .message
            .unwrap_or_else(|| "E_PYTHON_NOT_READY: python runtime is not ready".to_string());
        return Some(("E_PYTHON_NOT_READY", msg));
    }
    None
}

fn start_opts_from_settings(data_dir: &std::path::Path) -> Result<task_manager::StartOpts, String> {
    let s = settings::load_settings_strict(data_dir).map_err(|e| e.to_string())?;
    let (rewrite_enabled, template_id) =
        settings::resolve_rewrite_start_config(&s).map_err(|e| e.to_string())?;
    let asr_preprocess = resolve_asr_preprocess_config(&s);
    Ok(task_manager::StartOpts {
        rewrite_enabled,
        template_id,
        context_cfg: context_capture::config_from_settings(&s),
        rewrite_glossary: sanitize_rewrite_glossary(s.rewrite_glossary),
        rewrite_include_glossary: s.rewrite_include_glossary.unwrap_or(true),
        asr_preprocess,
        pre_captured_context: None,
        recording_session_id: None,
    })
}

fn abort_recording_session_if_present(
    state: &tauri::State<'_, TaskManager>,
    recording_session_id: &Option<String>,
) {
    if let Some(id) = recording_session_id.as_deref() {
        state.abort_recording_session(id);
    }
}

fn resolve_asr_preprocess_config(s: &settings::Settings) -> pipeline::PreprocessConfig {
    let mut cfg = pipeline::PreprocessConfig::default();
    if let Some(v) = s.asr_preprocess_silence_trim_enabled {
        cfg.silence_trim_enabled = v;
    }
    if let Some(v) = s.asr_preprocess_silence_threshold_db {
        cfg.silence_threshold_db = v;
    }
    if let Some(v) = s.asr_preprocess_silence_start_ms {
        cfg.silence_trim_start_ms = v;
    }
    if let Some(v) = s.asr_preprocess_silence_end_ms {
        cfg.silence_trim_end_ms = v;
    }
    cfg
}

fn resolve_asr_preprocess_config_strict(
    data_dir: &std::path::Path,
) -> anyhow::Result<pipeline::PreprocessConfig> {
    settings::load_settings_strict(data_dir).map(|s| resolve_asr_preprocess_config(&s))
}

fn sanitize_rewrite_glossary(glossary: Option<Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for item in glossary.unwrap_or_default() {
        let v = item.trim();
        if !v.is_empty() {
            out.push(v.to_string());
        }
    }
    out
}

#[tauri::command]
fn transcribe_fixture(
    runtime: tauri::State<'_, RuntimeState>,
    fixture_name: &str,
) -> Result<TranscribeResult, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.transcribe_fixture",
        Some(serde_json::json!({"fixture_name": fixture_name})),
    );
    if let Some((code, msg)) = runtime_not_ready(&runtime) {
        span.err("config", code, &msg, None);
        return Err(msg);
    }
    let model_id = match pipeline::resolve_asr_model_id(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("pipeline", "E_PIPELINE_RESOLVE_ASR", &e, None);
            return Err(e.to_string());
        }
    };
    let preprocess_cfg = match resolve_asr_preprocess_config_strict(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("pipeline", "E_CMD_RESOLVE_ASR_PREPROCESS", &e, None);
            return Err(e.to_string());
        }
    };
    match pipeline::run_fixture_pipeline(fixture_name, &model_id, &preprocess_cfg) {
        Ok(r) => {
            span.ok(None);
            Ok(r)
        }
        Err(e) => {
            span.err_anyhow("pipeline", "E_CMD_TRANSCRIBE_FIXTURE", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn transcribe_recording_base64(
    runtime: tauri::State<'_, RuntimeState>,
    b64: &str,
    ext: &str,
) -> Result<TranscribeResult, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        Some(&task_id),
        "CMD.transcribe_recording_base64",
        Some(serde_json::json!({"ext": ext, "b64_chars": b64.len()})),
    );
    if let Some((code, msg)) = runtime_not_ready(&runtime) {
        span.err("config", code, &msg, None);
        return Err(msg);
    }
    let input = match pipeline::save_base64_file(&task_id, b64, ext) {
        Ok(p) => p,
        Err(e) => {
            span.err_anyhow("io", "E_CMD_SAVE_B64", &e, None);
            return Err(e.to_string());
        }
    };
    let model_id = match pipeline::resolve_asr_model_id(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("pipeline", "E_PIPELINE_RESOLVE_ASR", &e, None);
            return Err(e.to_string());
        }
    };
    let preprocess_cfg = match resolve_asr_preprocess_config_strict(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("pipeline", "E_CMD_RESOLVE_ASR_PREPROCESS", &e, None);
            return Err(e.to_string());
        }
    };
    match pipeline::run_audio_pipeline_with_task_id(task_id, &input, &model_id, &preprocess_cfg) {
        Ok(r) => {
            span.ok(None);
            Ok(r)
        }
        Err(e) => {
            span.err_anyhow("pipeline", "E_CMD_TRANSCRIBE_B64", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn start_transcribe_fixture(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskManager>,
    runtime: tauri::State<'_, RuntimeState>,
    fixture_name: &str,
) -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let opts = match start_opts_from_settings(&dir) {
        Ok(v) => v,
        Err(e) => {
            let span = cmd_span(&dir, None, "CMD.start_transcribe_fixture.settings", None);
            span.err("config", "E_SETTINGS_INVALID", &e, None);
            return Err(e);
        }
    };
    let span = cmd_span(
        &dir,
        None,
        "CMD.start_transcribe_fixture",
        Some(serde_json::json!({
            "fixture_name": fixture_name,
            "rewrite_enabled": opts.rewrite_enabled,
            "template_id": opts.template_id.as_deref(),
            "source": "settings",
        })),
    );
    if let Some((code, msg)) = runtime_not_ready(&runtime) {
        span.err("config", code, &msg, None);
        return Err(msg);
    }
    match state.start_fixture(app, fixture_name.to_string(), opts) {
        Ok(task_id) => {
            span.ok(Some(serde_json::json!({"task_id": task_id})));
            Ok(task_id)
        }
        Err(e) => {
            span.err_anyhow("task", "E_CMD_START_FIXTURE", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn start_transcribe_recording_base64(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskManager>,
    runtime: tauri::State<'_, RuntimeState>,
    b64: &str,
    ext: &str,
    recording_session_id: Option<String>,
) -> Result<String, String> {
    let session_id = recording_session_id;
    let dir = match data_dir::data_dir() {
        Ok(v) => v,
        Err(e) => {
            abort_recording_session_if_present(&state, &session_id);
            return Err(e.to_string());
        }
    };
    let mut opts = match start_opts_from_settings(&dir) {
        Ok(v) => v,
        Err(e) => {
            let span = cmd_span(
                &dir,
                None,
                "CMD.start_transcribe_recording_base64.settings",
                None,
            );
            span.err("config", "E_SETTINGS_INVALID", &e, None);
            abort_recording_session_if_present(&state, &session_id);
            return Err(e);
        }
    };
    opts.recording_session_id = session_id;
    let span = cmd_span(
        &dir,
        None,
        "CMD.start_transcribe_recording_base64",
        Some(serde_json::json!({
            "ext": ext,
            "b64_chars": b64.len(),
            "rewrite_enabled": opts.rewrite_enabled,
            "template_id": opts.template_id.as_deref(),
            "has_recording_session_id": opts
                .recording_session_id
                .as_ref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            "context_include_prev_window_screenshot": opts.context_cfg.include_prev_window_screenshot,
            "context_include_prev_window_meta": opts.context_cfg.include_prev_window_meta,
            "asr_preprocess_silence_trim_enabled": opts.asr_preprocess.silence_trim_enabled,
            "source": "settings",
        })),
    );
    let session_id_for_cleanup = opts.recording_session_id.clone();
    if let Some((code, msg)) = runtime_not_ready(&runtime) {
        span.err("config", code, &msg, None);
        abort_recording_session_if_present(&state, &session_id_for_cleanup);
        return Err(msg);
    }
    match state.start_recording_base64(app, b64.to_string(), ext.to_string(), opts) {
        Ok(task_id) => {
            span.ok(Some(serde_json::json!({"task_id": task_id})));
            Ok(task_id)
        }
        Err(e) => {
            span.err_anyhow("task", "E_CMD_START_B64", &e, None);
            abort_recording_session_if_present(&state, &session_id_for_cleanup);
            Err(e.to_string())
        }
    }
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
fn cancel_task(state: tauri::State<TaskManager>, task_id: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        Some(task_id),
        "CMD.cancel_task",
        Some(serde_json::json!({"task_id": task_id})),
    );
    match state.cancel(task_id) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("task", "E_CMD_CANCEL", &e, None);
            Err(e.to_string())
        }
    }
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
async fn rewrite_text(template_id: &str, asr_text: &str) -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let task_id = uuid::Uuid::new_v4().to_string();
    let span = cmd_span(
        &dir,
        Some(&task_id),
        "CMD.rewrite_text",
        Some(serde_json::json!({"template_id": template_id, "asr_chars": asr_text.len()})),
    );
    let tpl = match templates::get_template(&dir, template_id) {
        Ok(t) => t,
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_GET", &e, None);
            return Err(e.to_string());
        }
    };
    let glossary = match settings::load_settings_strict(&dir) {
        Ok(s) => {
            if s.rewrite_include_glossary.unwrap_or(true) {
                sanitize_rewrite_glossary(s.rewrite_glossary)
            } else {
                Vec::new()
            }
        }
        Err(e) => {
            let ae = anyhow::anyhow!("rewrite_text load settings failed: {e}");
            span.err_anyhow("settings", "E_CMD_REWRITE_SETTINGS", &ae, None);
            return Err(ae.to_string());
        }
    };
    let include_glossary = !glossary.is_empty();
    let policy = llm::RewriteContextPolicy {
        include_history: false,
        include_clipboard: false,
        include_prev_window_meta: false,
        include_prev_window_screenshot: false,
        include_glossary,
    };
    match llm::rewrite_with_context(
        &dir,
        &task_id,
        &tpl.system_prompt,
        asr_text,
        None,
        &glossary,
        &policy,
    )
    .await
    {
        Ok(s) => {
            span.ok(Some(serde_json::json!({"out_chars": s.len()})));
            Ok(s)
        }
        Err(e) => {
            span.err_anyhow("llm", "E_CMD_REWRITE", &e, None);
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
fn set_settings(s: Settings) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.set_settings", None);
    match settings::save_settings(&dir, &s) {
        Ok(()) => {
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
    state: tauri::State<TaskManager>,
    hotkeys: tauri::State<hotkeys::HotkeyManager>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let patch_summary = serde_json::json!({
        "asr_model": patch.asr_model.is_some(),
        "llm_base_url": patch.llm_base_url.is_some(),
        "llm_model": patch.llm_model.is_some(),
        "llm_reasoning_effort": patch.llm_reasoning_effort.is_some(),
        "rewrite_enabled": patch.rewrite_enabled.is_some(),
        "rewrite_template_id": patch.rewrite_template_id.is_some(),
        "rewrite_glossary": patch.rewrite_glossary.is_some(),
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
    let next = settings::apply_patch(cur, patch);
    if let Err(e) = settings::save_settings(&dir, &next) {
        span.err_anyhow("settings", "E_CMD_UPDATE_SETTINGS", &e, None);
        return Err(e.to_string());
    }
    // If ASR model changed, restart the resident ASR runner.
    // We do this best-effort; errors are surfaced later via task events.
    if asr_model_changed {
        state.restart_asr_best_effort("settings_changed");
    }

    // Hotkeys are also best-effort; failures are traced and should not break settings.
    hotkeys.apply_from_settings_best_effort(&app, &dir, &next);

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
    startup_trace::mark_best_effort("run_enter");
    panic_log::install_best_effort();
    startup_trace::mark_best_effort("panic_hook_installed");
    let ctx = tauri::generate_context!();
    startup_trace::mark_best_effort("context_generated");
    tauri::Builder::default()
        .manage(TaskManager::new())
        .manage(RuntimeState::new())
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
                trace::event(
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
            startup_trace::mark_best_effort("setup_enter");

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
            }

            // Warm up the ASR runner in background so first transcription is fast.
            // If runtime preflight failed, skip warmup to avoid noisy startup failures.
            if toolchain_ready && python_ready {
                let state = app.state::<TaskManager>();
                state.warmup_asr_best_effort();
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
                        trace::event(
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

            startup_trace::mark_best_effort("setup_exit");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            transcribe_fixture,
            transcribe_recording_base64,
            start_transcribe_fixture,
            start_transcribe_recording_base64,
            cancel_task,
            list_templates,
            upsert_template,
            delete_template,
            templates_export_json,
            templates_import_json,
            rewrite_text,
            set_llm_api_key,
            clear_llm_api_key,
            llm_api_key_status,
            history_append,
            history_list,
            history_clear,
            get_settings,
            set_settings,
            update_settings,
            hotkeys::check_hotkey_available,
            runtime_toolchain_status,
            runtime_python_status,
            overlay_set_state,
            asr_model_status,
            download_asr_model
        ])
        .run(ctx)
        .expect("error while running tauri application");
}
