use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub model_dir: String,
    pub ok: bool,
    pub reason: Option<String>,
    pub model_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    #[allow(dead_code)]
    repo_id: Option<String>,
    #[allow(dead_code)]
    revision: Option<String>,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    path: String,
    size: u64,
    sha256: String,
}

pub fn default_model_dir(repo_root: &Path) -> PathBuf {
    repo_root.join("models").join("Qwen3-ASR-0.6B")
}

fn read_model_version(model_dir: &Path) -> Option<String> {
    let p = model_dir.join("REVISION.txt");
    let s = fs::read_to_string(p).ok()?;
    let line = s.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

fn sha256_file_hex(path: &Path) -> Result<String> {
    let mut f =
        fs::File::open(path).with_context(|| format!("open file failed: {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)
            .with_context(|| format!("read file failed: {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

pub fn verify_model_dir(model_dir: &Path) -> Result<ModelStatus> {
    if !model_dir.exists() {
        return Ok(ModelStatus {
            model_dir: model_dir.display().to_string(),
            ok: false,
            reason: Some("model_dir_missing".to_string()),
            model_version: None,
        });
    }
    let cfg = model_dir.join("config.json");
    if !cfg.exists() {
        return Ok(ModelStatus {
            model_dir: model_dir.display().to_string(),
            ok: false,
            reason: Some("config.json_missing".to_string()),
            model_version: read_model_version(model_dir),
        });
    }
    let revision = model_dir.join("REVISION.txt");
    if !revision.exists() {
        return Ok(ModelStatus {
            model_dir: model_dir.display().to_string(),
            ok: false,
            reason: Some("REVISION.txt_missing".to_string()),
            model_version: None,
        });
    }
    let manifest_path = model_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Ok(ModelStatus {
            model_dir: model_dir.display().to_string(),
            ok: true,
            reason: Some("manifest.json_missing".to_string()),
            model_version: read_model_version(model_dir),
        });
    }

    let manifest_str = fs::read_to_string(&manifest_path).context("read manifest.json failed")?;
    let manifest: Manifest =
        serde_json::from_str(&manifest_str).context("parse manifest.json failed")?;
    for f in manifest.files.iter() {
        let full = model_dir.join(&f.path);
        if !full.exists() {
            return Ok(ModelStatus {
                model_dir: model_dir.display().to_string(),
                ok: false,
                reason: Some(format!("file_missing:{}", f.path)),
                model_version: read_model_version(model_dir),
            });
        }
        let st = fs::metadata(&full).with_context(|| format!("stat failed: {}", full.display()))?;
        let size = st.len();
        if size != f.size {
            return Ok(ModelStatus {
                model_dir: model_dir.display().to_string(),
                ok: false,
                reason: Some(format!("size_mismatch:{}:{}!={}", f.path, size, f.size)),
                model_version: read_model_version(model_dir),
            });
        }
        // Verify sha256 for small files (fast); large weights are checked by size to keep UI snappy.
        if size <= 2_000_000 {
            let got = sha256_file_hex(&full)?;
            if !got.eq_ignore_ascii_case(&f.sha256) {
                return Ok(ModelStatus {
                    model_dir: model_dir.display().to_string(),
                    ok: false,
                    reason: Some(format!("sha256_mismatch:{}", f.path)),
                    model_version: read_model_version(model_dir),
                });
            }
        }
    }
    Ok(ModelStatus {
        model_dir: model_dir.display().to_string(),
        ok: true,
        reason: None,
        model_version: read_model_version(model_dir),
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
