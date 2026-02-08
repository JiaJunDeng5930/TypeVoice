mod pipeline;

use pipeline::TranscribeResult;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            transcribe_fixture,
            transcribe_recording_base64
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
