pub mod debug;
pub mod metrics;
pub mod panic;
pub mod schema;
pub mod startup;
pub mod trace;
mod writer;

pub use trace::{event, event_err, event_err_anyhow, event_err_durable, ErrorEvent, Span};

const APP_DATA_DIR: &str = "com.typevoice.typevoice";
const APP_DATA_SUBDIR: &str = "data";

pub fn runtime_data_dir() -> Option<std::path::PathBuf> {
    if let Some(path) = configured_data_dir(std::env::var("TYPEVOICE_DATA_DIR").ok()) {
        return Some(path);
    }
    platform_data_dir()
}

fn configured_data_dir(raw: Option<String>) -> Option<std::path::PathBuf> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<std::path::PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(std::path::PathBuf::from)
        .map(app_data_dir)
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return Some(app_data_dir(std::path::PathBuf::from(trimmed)));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .map(|home| app_data_dir(home.join(".local").join("share")))
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .map(|home| app_data_dir(home.join("Library").join("Application Support")))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn platform_data_dir() -> Option<std::path::PathBuf> {
    None
}

fn app_data_dir(base: std::path::PathBuf) -> std::path::PathBuf {
    base.join(APP_DATA_DIR).join(APP_DATA_SUBDIR)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn flush(timeout_ms: u64) -> bool {
    writer::flush(timeout_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_uses_identifier_data_subdir() {
        assert_eq!(
            app_data_dir(std::path::PathBuf::from("base")),
            std::path::PathBuf::from("base")
                .join("com.typevoice.typevoice")
                .join("data")
        );
    }

    #[test]
    fn configured_data_dir_ignores_blank_values() {
        assert_eq!(configured_data_dir(Some("   ".to_string())), None);
        assert_eq!(
            configured_data_dir(Some(" custom-data ".to_string())),
            Some(std::path::PathBuf::from("custom-data"))
        );
    }
}
