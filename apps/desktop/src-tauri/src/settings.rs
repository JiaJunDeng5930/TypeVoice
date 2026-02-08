use std::{fs, path::{Path, PathBuf}};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub asr_model: Option<String>, // local dir or HF repo id
}

pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

pub fn load_settings(data_dir: &Path) -> Result<Settings> {
    let p = settings_path(data_dir);
    if !p.exists() {
        return Ok(Settings::default());
    }
    let s = fs::read_to_string(&p).context("read settings.json failed")?;
    let v: Settings = serde_json::from_str(&s).context("parse settings.json failed")?;
    Ok(v)
}

pub fn save_settings(data_dir: &Path, settings: &Settings) -> Result<()> {
    std::fs::create_dir_all(data_dir).ok();
    let p = settings_path(data_dir);
    let s = serde_json::to_string_pretty(settings).context("serialize settings failed")?;
    fs::write(&p, s).context("write settings.json failed")?;
    Ok(())
}

