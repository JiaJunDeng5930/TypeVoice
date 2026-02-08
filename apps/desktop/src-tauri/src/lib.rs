mod pipeline;

use pipeline::TranscribeResult;

#[tauri::command]
fn transcribe_fixture(fixture_name: &str) -> Result<TranscribeResult, String> {
    pipeline::run_fixture_pipeline(fixture_name).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![transcribe_fixture])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
