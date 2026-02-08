mod pipeline;
mod data_dir;
mod templates;
mod llm;
mod history;

use pipeline::TranscribeResult;
use templates::PromptTemplate;
use history::HistoryItem;

#[tauri::command]
fn transcribe_fixture(fixture_name: &str) -> Result<TranscribeResult, String> {
    pipeline::run_fixture_pipeline(fixture_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn transcribe_recording_base64(b64: &str, ext: &str) -> Result<TranscribeResult, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let input = pipeline::save_base64_file(&task_id, b64, ext).map_err(|e| e.to_string())?;
    pipeline::run_audio_pipeline_with_task_id(task_id, &input, "Qwen/Qwen3-ASR-0.6B").map_err(|e| e.to_string())
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
async fn rewrite_text(template_id: &str, asr_text: &str) -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let tpl = templates::get_template(&dir, template_id).map_err(|e| e.to_string())?;
    llm::rewrite(&tpl.system_prompt, asr_text)
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
fn history_list(limit: i64) -> Result<Vec<HistoryItem>, String> {
    let db = history_db_path()?;
    history::list(&db, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn history_clear() -> Result<(), String> {
    let db = history_db_path()?;
    history::clear(&db).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            transcribe_fixture,
            transcribe_recording_base64,
            list_templates,
            upsert_template,
            delete_template,
            rewrite_text,
            set_llm_api_key,
            clear_llm_api_key,
            history_append,
            history_list,
            history_clear
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
