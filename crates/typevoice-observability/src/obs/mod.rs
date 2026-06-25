pub mod debug;
pub mod metrics;
pub mod panic;
pub mod schema;
pub mod startup;
pub mod trace;
mod writer;

pub use trace::{event, event_err, event_err_anyhow, ErrorEvent, Span};

pub(crate) fn runtime_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("TYPEVOICE_DATA_DIR") {
        return Some(std::path::PathBuf::from(p));
    }
    platform_data_dir()
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<std::path::PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(std::path::PathBuf::from)
        .map(|base| base.join("TypeVoice"))
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed).join("TypeVoice"));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .map(|home| home.join(".local").join("share").join("TypeVoice"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn platform_data_dir() -> Option<std::path::PathBuf> {
    None
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn flush(timeout_ms: u64) -> bool {
    writer::flush(timeout_ms)
}
