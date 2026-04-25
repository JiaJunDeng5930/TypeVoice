use std::{fs::OpenOptions, io::Write};

use serde_json::json;

use super::schema::now_ms;
use super::trace::redact_user_paths;

pub fn panic_trace_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("panic.jsonl")
}

pub fn install_best_effort() {
    std::panic::set_hook(Box::new(|info| {
        let bt = format!("{:?}", std::backtrace::Backtrace::force_capture());
        let message = format!("{info}");
        let rec = json!({
            "ts_ms": now_ms(),
            "type": "panic",
            "message": redact_user_paths(&message),
            "backtrace": redact_user_paths(&bt),
        })
        .to_string();

        if let Some(dir) = crate::obs::runtime_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let path = panic_trace_path(&dir);
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
                let _ = f.write_all(rec.as_bytes());
                let _ = f.write_all(b"\n");
            }
        }
    }));
}
