use std::{
    fs::OpenOptions,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

// Minimal, non-sensitive startup breadcrumbs to help diagnose early crashes on Windows.
// This is intentionally always-on and best-effort.
pub fn mark_best_effort(stage: &str) {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let Ok(dir) = crate::data_dir::data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("startup_trace.log");
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = writeln!(f, "ts_ms={ts_ms} stage={stage}");
}

