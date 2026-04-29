use std::{fs::OpenOptions, io::Write};

use serde_json::json;

use super::schema::now_ms;

pub fn startup_trace_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("startup_trace.jsonl")
}

pub fn mark_best_effort(stage: &str) {
    let Some(dir) = crate::obs::runtime_data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = startup_trace_path(&dir);
    let line = json!({
        "ts_ms": now_ms(),
        "type": "startup_stage",
        "stage": stage,
    })
    .to_string();
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = f.write_all(line.as_bytes());
    let _ = f.write_all(b"\n");
}
