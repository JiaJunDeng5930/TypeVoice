use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::trace;

const MANIFEST_VERSION: &str = "7.0.2";

#[derive(Debug, Clone)]
struct PlatformSpec {
    id: &'static str,
    version: &'static str,
    ffmpeg_file: &'static str,
    ffmpeg_sha256: &'static str,
    ffprobe_file: &'static str,
    ffprobe_sha256: &'static str,
}

const WINDOWS_X86_64_SPEC: PlatformSpec = PlatformSpec {
    id: "windows-x86_64",
    version: MANIFEST_VERSION,
    ffmpeg_file: "ffmpeg.exe",
    ffmpeg_sha256: "33cf0d2a42486a59f74f3b3741d8ff71bed82169db7125e91804cf264b365a4a",
    ffprobe_file: "ffprobe.exe",
    ffprobe_sha256: "af3c38d4a25acf3bf0f16c3e36b7f0700bbf8fc1159057186a6f3f1fe7cd1611",
};

const LINUX_X86_64_SPEC: PlatformSpec = PlatformSpec {
    id: "linux-x86_64",
    version: MANIFEST_VERSION,
    ffmpeg_file: "ffmpeg",
    ffmpeg_sha256: "e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99",
    ffprobe_file: "ffprobe",
    ffprobe_sha256: "4f231a1960d83e403d08f7971e271707bec278a9ae18e21b8b5b03186668450d",
};

#[derive(Debug, Clone, Serialize)]
pub struct ToolchainStatus {
    pub ready: bool,
    pub code: Option<String>,
    pub message: Option<String>,
    pub toolchain_dir: Option<String>,
    pub platform: String,
    pub expected_version: String,
}

impl ToolchainStatus {
    pub fn pending() -> Self {
        Self {
            ready: false,
            code: Some("E_TOOLCHAIN_NOT_READY".to_string()),
            message: Some("toolchain not checked yet".to_string()),
            toolchain_dir: None,
            platform: current_platform_id().unwrap_or("unknown").to_string(),
            expected_version: current_expected_version().unwrap_or("unknown").to_string(),
        }
    }
}

pub fn current_platform_id() -> Result<&'static str> {
    Ok(current_spec()?.id)
}

pub fn current_expected_version() -> Result<&'static str> {
    Ok(current_spec()?.version)
}

fn current_spec() -> Result<&'static PlatformSpec> {
    if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        return Ok(&WINDOWS_X86_64_SPEC);
    }
    if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        return Ok(&LINUX_X86_64_SPEC);
    }
    Err(anyhow!(
        "E_TOOLCHAIN_PLATFORM_UNSUPPORTED: unsupported platform target"
    ))
}

fn source_toolchain_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("toolchain")
        .join("bin")
}

pub fn source_toolchain_dir_for(platform_id: &str) -> PathBuf {
    source_toolchain_root().join(platform_id)
}

fn source_toolchain_dir_current() -> Result<PathBuf> {
    Ok(source_toolchain_dir_for(current_platform_id()?))
}

fn resource_toolchain_dir(app: &AppHandle) -> Result<PathBuf> {
    let rel = format!("toolchain/{}", current_platform_id()?);
    app.path()
        .resolve(rel, BaseDirectory::Resource)
        .context("resolve resource toolchain dir failed")
}

fn env_toolchain_dir() -> Option<PathBuf> {
    let raw = std::env::var("TYPEVOICE_TOOLCHAIN_DIR").ok()?;
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    Some(PathBuf::from(t))
}

fn selected_toolchain_dir(app: &AppHandle) -> Result<PathBuf> {
    if let Some(dir) = env_toolchain_dir() {
        return Ok(dir);
    }

    // In dev mode, prefer workspace local binaries to keep behavior deterministic.
    if cfg!(debug_assertions) {
        let dev_dir = source_toolchain_dir_current()?;
        if dev_dir.exists() {
            return Ok(dev_dir);
        }
    }

    let res_dir = resource_toolchain_dir(app)?;
    if res_dir.exists() {
        return Ok(res_dir);
    }

    Err(anyhow!(
        "E_TOOLCHAIN_NOT_READY: toolchain directory not found (expected one of: TYPEVOICE_TOOLCHAIN_DIR, dev toolchain dir, bundled resource dir)"
    ))
}

fn tool_binary_from_dir(dir: &Path, file_name: &str) -> PathBuf {
    dir.join(file_name)
}

pub fn resolve_tool_binary(env_key: &str, file_name: &str) -> Result<PathBuf> {
    if let Ok(raw) = std::env::var(env_key) {
        let t = raw.trim();
        if !t.is_empty() {
            let p = PathBuf::from(t);
            if p.exists() {
                return Ok(p);
            }
            return Err(anyhow!(
                "E_TOOLCHAIN_NOT_READY: {} points to missing file: {}",
                env_key,
                p.display()
            ));
        }
    }

    let dir = env_toolchain_dir().ok_or_else(|| {
        anyhow!(
            "E_TOOLCHAIN_NOT_READY: TYPEVOICE_TOOLCHAIN_DIR is not set and {} is empty",
            env_key
        )
    })?;
    let p = tool_binary_from_dir(&dir, file_name);
    if !p.exists() {
        return Err(anyhow!(
            "E_TOOLCHAIN_NOT_READY: missing tool binary {}",
            p.display()
        ));
    }
    Ok(p)
}

pub fn initialize_and_verify(app: &AppHandle, data_dir: &Path) -> ToolchainStatus {
    let spec = match current_spec() {
        Ok(s) => s,
        Err(e) => {
            let msg = e.to_string();
            trace::event(
                data_dir,
                None,
                "Toolchain",
                "TC.init",
                "err",
                Some(serde_json::json!({"code":"E_TOOLCHAIN_PLATFORM_UNSUPPORTED","message":msg})),
            );
            return ToolchainStatus {
                ready: false,
                code: Some("E_TOOLCHAIN_PLATFORM_UNSUPPORTED".to_string()),
                message: Some(msg),
                toolchain_dir: None,
                platform: "unknown".to_string(),
                expected_version: "unknown".to_string(),
            };
        }
    };

    let dir = match selected_toolchain_dir(app) {
        Ok(d) => d,
        Err(e) => {
            let msg = e.to_string();
            trace::event(
                data_dir,
                None,
                "Toolchain",
                "TC.resolve_dir",
                "err",
                Some(serde_json::json!({"code":"E_TOOLCHAIN_NOT_READY","message":msg})),
            );
            return ToolchainStatus {
                ready: false,
                code: Some("E_TOOLCHAIN_NOT_READY".to_string()),
                message: Some(msg),
                toolchain_dir: None,
                platform: spec.id.to_string(),
                expected_version: spec.version.to_string(),
            };
        }
    };

    let ffmpeg = tool_binary_from_dir(&dir, spec.ffmpeg_file);
    let ffprobe = tool_binary_from_dir(&dir, spec.ffprobe_file);
    std::env::set_var("TYPEVOICE_TOOLCHAIN_DIR", dir.display().to_string());
    std::env::set_var("TYPEVOICE_FFMPEG", ffmpeg.display().to_string());
    std::env::set_var("TYPEVOICE_FFPROBE", ffprobe.display().to_string());

    match verify_toolchain_dir(&dir, spec) {
        Ok(()) => {
            trace::event(
                data_dir,
                None,
                "Toolchain",
                "TC.verify",
                "ok",
                Some(serde_json::json!({
                    "toolchain_dir": dir.display().to_string(),
                    "platform": spec.id,
                    "expected_version": spec.version,
                })),
            );
            ToolchainStatus {
                ready: true,
                code: None,
                message: None,
                toolchain_dir: Some(dir.display().to_string()),
                platform: spec.id.to_string(),
                expected_version: spec.version.to_string(),
            }
        }
        Err(e) => {
            let msg = e.to_string();
            let code = detect_code(&msg)
                .unwrap_or("E_TOOLCHAIN_NOT_READY")
                .to_string();
            trace::event(
                data_dir,
                None,
                "Toolchain",
                "TC.verify",
                "err",
                Some(serde_json::json!({
                    "code": code,
                    "message": msg,
                    "toolchain_dir": dir.display().to_string(),
                })),
            );
            ToolchainStatus {
                ready: false,
                code: Some(code),
                message: Some(msg),
                toolchain_dir: Some(dir.display().to_string()),
                platform: spec.id.to_string(),
                expected_version: spec.version.to_string(),
            }
        }
    }
}

fn detect_code(msg: &str) -> Option<&'static str> {
    if msg.contains("E_TOOLCHAIN_CHECKSUM_MISMATCH") {
        return Some("E_TOOLCHAIN_CHECKSUM_MISMATCH");
    }
    if msg.contains("E_TOOLCHAIN_VERSION_MISMATCH") {
        return Some("E_TOOLCHAIN_VERSION_MISMATCH");
    }
    if msg.contains("E_TOOLCHAIN_NOT_READY") {
        return Some("E_TOOLCHAIN_NOT_READY");
    }
    None
}

fn verify_toolchain_dir(dir: &Path, spec: &PlatformSpec) -> Result<()> {
    let ffmpeg = tool_binary_from_dir(dir, spec.ffmpeg_file);
    let ffprobe = tool_binary_from_dir(dir, spec.ffprobe_file);

    if !ffmpeg.exists() {
        return Err(anyhow!(
            "E_TOOLCHAIN_NOT_READY: missing ffmpeg binary at {}",
            ffmpeg.display()
        ));
    }
    if !ffprobe.exists() {
        return Err(anyhow!(
            "E_TOOLCHAIN_NOT_READY: missing ffprobe binary at {}",
            ffprobe.display()
        ));
    }

    verify_sha256(&ffmpeg, spec.ffmpeg_sha256, "ffmpeg")?;
    verify_sha256(&ffprobe, spec.ffprobe_sha256, "ffprobe")?;

    verify_version(&ffmpeg, spec.version, "ffmpeg")?;
    verify_version(&ffprobe, spec.version, "ffprobe")?;

    Ok(())
}

fn verify_sha256(path: &Path, expected: &str, tool_name: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(anyhow!(
            "E_TOOLCHAIN_CHECKSUM_MISMATCH: {} sha256 mismatch (expected={} actual={} path={})",
            tool_name,
            expected,
            actual,
            path.display()
        ));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read file failed: {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    Ok(hex::encode(out))
}

fn verify_version(bin: &Path, expected_version: &str, tool_name: &str) -> Result<()> {
    let out = Command::new(bin)
        .arg("-version")
        .output()
        .with_context(|| format!("run {} -version failed: {}", tool_name, bin.display()))?;

    if !out.status.success() {
        return Err(anyhow!(
            "E_TOOLCHAIN_VERSION_MISMATCH: {} -version exited with {}",
            tool_name,
            out.status
        ));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let merged = if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        stdout.to_string()
    };
    let first_line = merged.lines().next().unwrap_or("").trim().to_string();

    if !first_line.contains(expected_version) {
        return Err(anyhow!(
            "E_TOOLCHAIN_VERSION_MISMATCH: {} version mismatch (expected contains={} got={})",
            tool_name,
            expected_version,
            first_line
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_tool_binary;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_tool_binary_requires_config() {
        let _g = env_lock().lock().unwrap();
        std::env::remove_var("TYPEVOICE_FFMPEG");
        std::env::remove_var("TYPEVOICE_TOOLCHAIN_DIR");

        let err = resolve_tool_binary("TYPEVOICE_FFMPEG", "ffmpeg").unwrap_err();
        assert!(err.to_string().contains("E_TOOLCHAIN_NOT_READY"));
    }

    #[test]
    fn resolve_tool_binary_prefers_explicit_env_path() {
        let _g = env_lock().lock().unwrap();
        let td = tempfile::tempdir().expect("tempdir");
        let ffmpeg = td.path().join("ffmpeg");
        std::fs::write(&ffmpeg, b"x").expect("write");

        std::env::set_var("TYPEVOICE_FFMPEG", ffmpeg.display().to_string());
        std::env::remove_var("TYPEVOICE_TOOLCHAIN_DIR");
        let got = resolve_tool_binary("TYPEVOICE_FFMPEG", "ffmpeg").expect("resolve");
        assert_eq!(got, ffmpeg);

        std::env::remove_var("TYPEVOICE_FFMPEG");
    }

    #[test]
    fn resolve_tool_binary_uses_toolchain_dir() {
        let _g = env_lock().lock().unwrap();
        let td = tempfile::tempdir().expect("tempdir");
        let ffmpeg = td.path().join("ffmpeg");
        std::fs::write(&ffmpeg, b"x").expect("write");

        std::env::remove_var("TYPEVOICE_FFMPEG");
        std::env::set_var("TYPEVOICE_TOOLCHAIN_DIR", td.path().display().to_string());
        let got = resolve_tool_binary("TYPEVOICE_FFMPEG", "ffmpeg").expect("resolve");
        assert_eq!(got, ffmpeg);

        std::env::remove_var("TYPEVOICE_TOOLCHAIN_DIR");
    }
}
