use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::trace;

#[derive(Debug, Clone, Serialize)]
pub struct PythonStatus {
    pub ready: bool,
    pub code: Option<String>,
    pub message: Option<String>,
    pub python_path: Option<String>,
    pub python_version: Option<String>,
}

impl PythonStatus {
    pub fn pending() -> Self {
        Self {
            ready: false,
            code: Some("E_PYTHON_NOT_READY".to_string()),
            message: Some("python runtime not checked yet".to_string()),
            python_path: None,
            python_version: None,
        }
    }
}

pub fn default_python_path(repo_root: &Path) -> PathBuf {
    if cfg!(windows) {
        repo_root.join(".venv").join("Scripts").join("python.exe")
    } else {
        repo_root.join(".venv").join("bin").join("python")
    }
}

pub fn resolve_python_binary(repo_root: &Path) -> Result<PathBuf> {
    if let Ok(raw) = std::env::var("TYPEVOICE_PYTHON") {
        let t = raw.trim();
        if !t.is_empty() {
            let p = PathBuf::from(t);
            if p.exists() {
                return Ok(p);
            }
            return Err(anyhow!(
                "E_PYTHON_NOT_READY: TYPEVOICE_PYTHON points to missing file: {}",
                p.display()
            ));
        }
    }

    let p = default_python_path(repo_root);
    if p.exists() {
        return Ok(p);
    }
    Err(anyhow!(
        "E_PYTHON_NOT_READY: missing python interpreter at {} (set TYPEVOICE_PYTHON or create repo-local .venv)",
        p.display()
    ))
}

fn verify_python_version(python: &Path) -> Result<String> {
    let out = Command::new(python)
        .arg("--version")
        .output()
        .with_context(|| format!("run python --version failed: {}", python.display()))?;
    if !out.status.success() {
        return Err(anyhow!(
            "E_PYTHON_NOT_READY: python --version exited with {} ({})",
            out.status,
            python.display()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let merged = if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        stdout.to_string()
    };
    let line = merged.lines().next().unwrap_or("").trim().to_string();
    if line.is_empty() {
        return Err(anyhow!(
            "E_PYTHON_NOT_READY: python --version returned empty output ({})",
            python.display()
        ));
    }
    Ok(line)
}

pub fn initialize_and_verify(data_dir: &Path, repo_root: &Path) -> PythonStatus {
    let resolved = match resolve_python_binary(repo_root) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.to_string();
            trace::event(
                data_dir,
                None,
                "Python",
                "PY.verify",
                "err",
                Some(serde_json::json!({
                    "code": "E_PYTHON_NOT_READY",
                    "message": msg,
                })),
            );
            return PythonStatus {
                ready: false,
                code: Some("E_PYTHON_NOT_READY".to_string()),
                message: Some(msg),
                python_path: None,
                python_version: None,
            };
        }
    };

    match verify_python_version(&resolved) {
        Ok(version) => {
            std::env::set_var("TYPEVOICE_PYTHON", resolved.display().to_string());
            trace::event(
                data_dir,
                None,
                "Python",
                "PY.verify",
                "ok",
                Some(serde_json::json!({
                    "python": resolved.display().to_string(),
                    "version": version,
                })),
            );
            PythonStatus {
                ready: true,
                code: None,
                message: None,
                python_path: Some(resolved.display().to_string()),
                python_version: Some(version),
            }
        }
        Err(e) => {
            let msg = e.to_string();
            trace::event(
                data_dir,
                None,
                "Python",
                "PY.verify",
                "err",
                Some(serde_json::json!({
                    "code": "E_PYTHON_NOT_READY",
                    "message": msg,
                    "python": resolved.display().to_string(),
                })),
            );
            PythonStatus {
                ready: false,
                code: Some("E_PYTHON_NOT_READY".to_string()),
                message: Some(msg),
                python_path: Some(resolved.display().to_string()),
                python_version: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_python_binary;
    use std::{
        path::Path,
        sync::{Mutex, OnceLock},
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_python_binary_requires_config() {
        let _g = env_lock().lock().unwrap();
        std::env::remove_var("TYPEVOICE_PYTHON");
        let td = tempfile::tempdir().expect("tempdir");
        let err = resolve_python_binary(td.path()).unwrap_err();
        assert!(err.to_string().contains("E_PYTHON_NOT_READY"));
    }

    #[test]
    fn resolve_python_binary_prefers_explicit_env_path() {
        let _g = env_lock().lock().unwrap();
        let td = tempfile::tempdir().expect("tempdir");
        let py = td.path().join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        std::fs::write(&py, b"x").expect("write");
        std::env::set_var("TYPEVOICE_PYTHON", py.display().to_string());
        let got = resolve_python_binary(td.path()).expect("resolve");
        assert_eq!(got, py);
        std::env::remove_var("TYPEVOICE_PYTHON");
    }

    #[test]
    fn resolve_python_binary_uses_repo_venv() {
        let _g = env_lock().lock().unwrap();
        std::env::remove_var("TYPEVOICE_PYTHON");
        let td = tempfile::tempdir().expect("tempdir");
        let py = if cfg!(windows) {
            td.path().join(".venv").join("Scripts").join("python.exe")
        } else {
            td.path().join(".venv").join("bin").join("python")
        };
        std::fs::create_dir_all(py.parent().unwrap_or(Path::new("."))).expect("mkdir");
        std::fs::write(&py, b"x").expect("write");

        let got = resolve_python_binary(td.path()).expect("resolve");
        assert_eq!(got, py);
    }
}
