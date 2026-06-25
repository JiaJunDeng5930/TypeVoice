use std::path::PathBuf;

use anyhow::{anyhow, Result};

const APP_DATA_DIR: &str = "com.typevoice.typevoice";
const APP_DATA_SUBDIR: &str = "data";

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
    Ok(app_data_dir(base))
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Result<PathBuf> {
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            return Ok(app_data_dir(PathBuf::from(trimmed)));
        }
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| anyhow!("HOME is not set"))?;
    Ok(app_data_dir(home.join(".local").join("share")))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn platform_data_dir() -> Result<PathBuf> {
    Err(anyhow!("unsupported platform data directory"))
}

fn app_data_dir(base: PathBuf) -> PathBuf {
    base.join(APP_DATA_DIR).join(APP_DATA_SUBDIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_uses_identifier_data_subdir() {
        assert_eq!(
            app_data_dir(PathBuf::from("base")),
            PathBuf::from("base")
                .join("com.typevoice.typevoice")
                .join("data")
        );
    }
}
