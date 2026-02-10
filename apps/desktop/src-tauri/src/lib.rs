mod asr_service;
mod context_capture;
#[cfg(windows)]
mod context_capture_windows;
mod context_pack;
mod data_dir;
mod debug_log;
mod history;
mod llm;
mod metrics;
mod model;
mod panic_log;
mod pipeline;
mod safe_print;
mod settings;
mod startup_trace;
mod task_manager;
mod templates;

use history::HistoryItem;
use llm::ApiKeyStatus;
use model::ModelStatus;
use pipeline::TranscribeResult;
use settings::Settings;
use settings::SettingsPatch;
use task_manager::TaskManager;
use tauri::Manager;
use templates::PromptTemplate;

#[tauri::command]
fn transcribe_fixture(fixture_name: &str) -> Result<TranscribeResult, String> {
    pipeline::run_fixture_pipeline(fixture_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn transcribe_recording_base64(b64: &str, ext: &str) -> Result<TranscribeResult, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let input = pipeline::save_base64_file(&task_id, b64, ext).map_err(|e| e.to_string())?;
    pipeline::run_audio_pipeline_with_task_id(task_id, &input, "Qwen/Qwen3-ASR-0.6B")
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_transcribe_fixture(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskManager>,
    fixture_name: &str,
    rewrite_enabled: bool,
    template_id: Option<String>,
) -> Result<String, String> {
    state
        .start_fixture(
            app,
            fixture_name.to_string(),
            task_manager::StartOpts {
                rewrite_enabled,
                template_id,
            },
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_transcribe_recording_base64(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskManager>,
    b64: &str,
    ext: &str,
    rewrite_enabled: bool,
    template_id: Option<String>,
) -> Result<String, String> {
    state
        .start_recording_base64(
            app,
            b64.to_string(),
            ext.to_string(),
            task_manager::StartOpts {
                rewrite_enabled,
                template_id,
            },
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_task(state: tauri::State<TaskManager>, task_id: &str) -> Result<(), String> {
    state.cancel(task_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_templates() -> Result<Vec<PromptTemplate>, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    templates::load_templates(&dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn upsert_template(tpl: PromptTemplate) -> Result<PromptTemplate, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    templates::upsert_template(&dir, tpl).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_template(id: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    templates::delete_template(&dir, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn templates_export_json() -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    templates::export_templates_json(&dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn templates_import_json(json: &str, mode: &str) -> Result<usize, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    templates::import_templates_json(&dir, json, mode).map_err(|e| e.to_string())
}

#[tauri::command]
async fn rewrite_text(template_id: &str, asr_text: &str) -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let tpl = templates::get_template(&dir, template_id).map_err(|e| e.to_string())?;
    let task_id = uuid::Uuid::new_v4().to_string();
    llm::rewrite(&dir, &task_id, &tpl.system_prompt, asr_text)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_llm_api_key(api_key: &str) -> Result<(), String> {
    llm::set_api_key(api_key).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_llm_api_key() -> Result<(), String> {
    llm::clear_api_key().map_err(|e| e.to_string())
}

#[tauri::command]
fn llm_api_key_status() -> Result<ApiKeyStatus, String> {
    Ok(llm::api_key_status())
}

fn history_db_path() -> Result<std::path::PathBuf, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("history.sqlite3"))
}

#[tauri::command]
fn history_append(item: HistoryItem) -> Result<(), String> {
    let db = history_db_path()?;
    history::append(&db, &item).map_err(|e| e.to_string())
}

#[tauri::command]
fn history_list(limit: i64, before_ms: Option<i64>) -> Result<Vec<HistoryItem>, String> {
    let db = history_db_path()?;
    history::list(&db, limit, before_ms).map_err(|e| e.to_string())
}

#[tauri::command]
fn history_clear() -> Result<(), String> {
    let db = history_db_path()?;
    history::clear(&db).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings() -> Result<Settings, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    Ok(settings::load_settings_or_recover(&dir))
}

#[tauri::command]
fn set_settings(s: Settings) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    settings::save_settings(&dir, &s).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_settings(
    state: tauri::State<TaskManager>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let cur = settings::load_settings_or_recover(&dir);
    let asr_model_changed = patch.asr_model.is_some();
    let next = settings::apply_patch(cur, patch);
    settings::save_settings(&dir, &next).map_err(|e| e.to_string())?;
    // If ASR model changed, restart the resident ASR runner.
    // We do this best-effort; errors are surfaced later via task events.
    if asr_model_changed {
        state.restart_asr_best_effort("settings_changed");
    }
    Ok(next)
}

#[tauri::command]
fn asr_model_status() -> Result<ModelStatus, String> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .ok_or_else(|| "repo root not found".to_string())?
        .to_path_buf();
    let model_dir = model::default_model_dir(&root);
    model::verify_model_dir(&model_dir).map_err(|e| e.to_string())
}

#[tauri::command]
async fn download_asr_model() -> Result<ModelStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .ok_or_else(|| "repo root not found".to_string())?
        .to_path_buf();
    let model_dir = model::default_model_dir(&root);
    let py = if cfg!(windows) {
        root.join(".venv").join("Scripts").join("python.exe")
    } else {
        root.join(".venv").join("bin").join("python")
    };
    let root2 = root.clone();
    let py2 = py.clone();
    let model_dir2 = model_dir.clone();
    let st = tauri::async_runtime::spawn_blocking(move || {
        model::download_model(&root2, &py2, &model_dir2)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    // Set settings.asr_model to local dir if ok.
    if st.ok {
        let mut s = settings::load_settings_or_recover(&dir);
        s.asr_model = Some(model_dir.display().to_string());
        let _ = settings::save_settings(&dir, &s);
    }
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
        .setup(|app| {
            startup_trace::mark_best_effort("setup_enter");
            // Warm up the ASR runner in background so first transcription is fast.
            let state = app.state::<TaskManager>();
            state.warmup_asr_best_effort();
            state.warmup_context_best_effort();
            startup_trace::mark_best_effort("setup_exit");
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
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
            asr_model_status,
            download_asr_model
        ])
        .run(ctx)
        .expect("error while running tauri application");
}
