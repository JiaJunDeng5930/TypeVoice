use std::path::PathBuf;

use anyhow::{anyhow, Result};

pub fn data_dir() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("TYPEVOICE_DATA_DIR") {
        return Ok(PathBuf::from(p));
    }
    platform_data_dir()
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Result<PathBuf> {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .map_err(|_| anyhow!("LOCALAPPDATA is not set"))?;
    Ok(base.join("TypeVoice"))
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Result<PathBuf> {
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed).join("TypeVoice"));
        }
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| anyhow!("HOME is not set"))?;
    Ok(home.join(".local").join("share").join("TypeVoice"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn platform_data_dir() -> Result<PathBuf> {
    Err(anyhow!("unsupported platform data directory"))
}
