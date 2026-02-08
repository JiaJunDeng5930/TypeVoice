use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub model_dir: String,
    pub ok: bool,
    pub reason: Option<String>,
}

pub fn default_model_dir(repo_root: &Path) -> PathBuf {
    repo_root.join("models").join("Qwen3-ASR-0.6B")
}

pub fn verify_model_dir(model_dir: &Path) -> Result<ModelStatus> {
    if !model_dir.exists() {
        return Ok(ModelStatus {
            model_dir: model_dir.display().to_string(),
            ok: false,
            reason: Some("model_dir_missing".to_string()),
        });
    }
    let cfg = model_dir.join("config.json");
    if !cfg.exists() {
        return Ok(ModelStatus {
            model_dir: model_dir.display().to_string(),
            ok: false,
            reason: Some("config.json_missing".to_string()),
        });
    }
    Ok(ModelStatus {
        model_dir: model_dir.display().to_string(),
        ok: true,
        reason: None,
    })
}

pub fn download_model(
    repo_root: &Path,
    venv_python: &Path,
    model_dir: &Path,
) -> Result<ModelStatus> {
    std::fs::create_dir_all(model_dir.parent().unwrap_or(model_dir)).ok();

    let status = Command::new(venv_python)
        .current_dir(repo_root)
        .env("TYPEVOICE_ASR_MODEL_DIR", model_dir)
        .arg("scripts/download_asr_model.py")
        .status()
        .context("failed to run download_asr_model.py")?;
    if !status.success() {
        return Err(anyhow!("model download failed: exit={status}"));
    }
    verify_model_dir(model_dir)
}
