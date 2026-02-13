use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::debug_log;
use crate::trace::Span;

const MAX_TOOL_STDERR_BYTES: usize = 4096;

fn cmd_hint_for_trace(cmd: &str) -> String {
    let t = cmd.trim();
    if t.is_empty() {
        return "".to_string();
    }
    t.rsplit(['\\', '/']).next().unwrap_or(t).to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct PreprocessConfig {
    pub silence_trim_enabled: bool,
    pub silence_threshold_db: f64,
    pub silence_trim_start_ms: u64,
    pub silence_trim_end_ms: u64,
}

impl Default for PreprocessConfig {
    fn default() -> Self {
        Self {
            silence_trim_enabled: false,
            silence_threshold_db: -50.0,
            silence_trim_start_ms: 300,
            silence_trim_end_ms: 300,
        }
    }
}

fn repo_root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("TYPEVOICE_REPO_ROOT") {
        return Ok(PathBuf::from(p));
    }
    // CARGO_MANIFEST_DIR = .../TypeVoice/apps/desktop/src-tauri
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = dir
        .ancestors()
        .nth(3)
        .ok_or_else(|| anyhow!("failed to locate repo root from CARGO_MANIFEST_DIR"))?;
    Ok(root.to_path_buf())
}

fn resolve_tool_path(env_key: &str, candidate_file: &str) -> Result<String> {
    let p = crate::toolchain::resolve_tool_binary(env_key, candidate_file)?;
    Ok(p.display().to_string())
}

pub fn ffmpeg_cmd() -> Result<String> {
    if cfg!(windows) {
        return resolve_tool_path("TYPEVOICE_FFMPEG", "ffmpeg.exe");
    }
    resolve_tool_path("TYPEVOICE_FFMPEG", "ffmpeg")
}

pub fn ffprobe_cmd() -> Result<String> {
    if cfg!(windows) {
        return resolve_tool_path("TYPEVOICE_FFPROBE", "ffprobe.exe");
    }
    resolve_tool_path("TYPEVOICE_FFPROBE", "ffprobe")
}

fn truncate_stderr_bytes(mut b: Vec<u8>) -> Vec<u8> {
    if b.len() > MAX_TOOL_STDERR_BYTES {
        b.truncate(MAX_TOOL_STDERR_BYTES);
    }
    b
}

fn stderr_excerpt_from_child(mut stderr: Option<std::process::ChildStderr>) -> String {
    let mut buf = Vec::new();
    if let Some(ref mut s) = stderr {
        let _ = s.read_to_end(&mut buf);
    }
    let buf = truncate_stderr_bytes(buf);
    String::from_utf8_lossy(&buf).trim().to_string()
}

fn clamp_preprocess_config(mut cfg: PreprocessConfig) -> PreprocessConfig {
    if !cfg.silence_threshold_db.is_finite() {
        cfg.silence_threshold_db = -50.0;
    }
    if cfg.silence_threshold_db > 0.0 {
        cfg.silence_threshold_db = 0.0;
    }
    if cfg.silence_trim_start_ms > 60_000 {
        cfg.silence_trim_start_ms = 60_000;
    }
    if cfg.silence_trim_end_ms > 60_000 {
        cfg.silence_trim_end_ms = 60_000;
    }
    cfg
}

fn build_ffmpeg_preprocess_args(
    input: &Path,
    output: &Path,
    cfg: &PreprocessConfig,
) -> Result<Vec<String>> {
    let input_s = input
        .to_str()
        .ok_or_else(|| anyhow!("non-utf8 input path"))?
        .to_string();
    let output_s = output
        .to_str()
        .ok_or_else(|| anyhow!("non-utf8 output path"))?
        .to_string();

    let cfg = clamp_preprocess_config(cfg.clone());
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-i".to_string(),
        input_s,
        "-ac".to_string(),
        "1".to_string(),
        "-ar".to_string(),
        "16000".to_string(),
        "-c:a".to_string(),
        "pcm_s16le".to_string(),
    ];

    if cfg.silence_trim_enabled {
        let start = (cfg.silence_trim_start_ms as f64) / 1000.0;
        let end = (cfg.silence_trim_end_ms as f64) / 1000.0;
        let filter = format!(
            "silenceremove=start_periods=1:start_duration={start:.3}:start_threshold={thr}dB:stop_periods=-1:stop_duration={end:.3}:stop_threshold={thr}dB",
            start = start,
            end = end,
            thr = cfg.silence_threshold_db,
        );
        args.push("-af".to_string());
        args.push(filter);
    }

    args.push("-vn".to_string());
    args.push(output_s);
    Ok(args)
}

pub fn fixture_path(name: &str) -> Result<PathBuf> {
    let root = repo_root()?;
    Ok(root.join("fixtures").join(name))
}

pub fn save_bytes_file(task_id: &str, bytes: &[u8], ext: &str) -> Result<PathBuf> {
    let root = repo_root()?;
    let tmp = root.join("tmp").join("desktop");
    std::fs::create_dir_all(&tmp).ok();
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let input = tmp.join(format!("{task_id}.{ext}"));
    std::fs::write(&input, bytes).context("failed to write recording file")?;
    Ok(input)
}

pub fn preprocess_to_temp_wav(task_id: &str, _input_audio: &Path) -> Result<PathBuf> {
    let root = repo_root()?;
    let tmp = root.join("tmp").join("desktop");
    std::fs::create_dir_all(&tmp).ok();
    Ok(tmp.join(format!("{task_id}.wav")))
}

pub fn cleanup_audio_artifacts(input_audio: &Path, wav_path: &Path) -> Result<()> {
    // Default: do not persist audio artifacts.
    let keep_audio = std::env::var("TYPEVOICE_KEEP_AUDIO").ok().as_deref() == Some("1");
    if keep_audio {
        return Ok(());
    }

    let root = repo_root()?;
    let tmp = root.join("tmp").join("desktop");

    let _ = std::fs::remove_file(wav_path);
    // Only delete the original input if it's inside our temp dir.
    if input_audio.starts_with(&tmp) {
        let _ = std::fs::remove_file(input_audio);
    }
    Ok(())
}

pub fn resolve_asr_model_id(data_dir: &Path) -> Result<String> {
    // Priority:
    // 1) Settings in data dir
    // 2) Local repo models/Qwen3-ASR-0.6B
    // 3) Env override TYPEVOICE_ASR_MODEL
    // 4) Default HF repo id
    if let Ok(s) = crate::settings::load_settings(data_dir) {
        if let Some(m) = s.asr_model {
            if !m.trim().is_empty() {
                return Ok(m);
            }
        }
    }

    let root = repo_root()?;
    let local = root.join("models").join("Qwen3-ASR-0.6B");
    if local.exists() {
        return Ok(local.display().to_string());
    }

    if let Ok(m) = std::env::var("TYPEVOICE_ASR_MODEL") {
        if !m.trim().is_empty() {
            return Ok(m);
        }
    }

    Ok("Qwen/Qwen3-ASR-0.6B".to_string())
}

pub fn preprocess_ffmpeg_cancellable(
    data_dir: &Path,
    task_id: &str,
    input: &Path,
    output: &Path,
    token: &tokio_util::sync::CancellationToken,
    pid_slot: &std::sync::Arc<std::sync::Mutex<Option<u32>>>,
    cfg: &PreprocessConfig,
) -> Result<u128> {
    let cmd = ffmpeg_cmd()?;
    let span = Span::start(
        data_dir,
        Some(task_id),
        "Preprocess",
        "FFMPEG.preprocess",
        Some(serde_json::json!({
            "cmd_hint": cmd_hint_for_trace(&cmd),
        })),
    );

    let t0 = Instant::now();
    let args = match build_ffmpeg_preprocess_args(input, output, cfg) {
        Ok(v) => v,
        Err(e) => {
            span.err("io", "E_PATH_UTF8", &e.to_string(), None);
            return Err(e);
        }
    };

    let mut child = match Command::new(&cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                span.err(
                    "process",
                    "E_FFMPEG_NOT_FOUND",
                    &format!("ffmpeg not found (cmd={cmd})"),
                    None,
                );
                return Err(anyhow!("E_FFMPEG_NOT_FOUND: ffmpeg not found (cmd={cmd})"));
            }
            span.err(
                "process",
                "E_FFMPEG_FAILED",
                &format!("failed to start ffmpeg (cmd={cmd}): {e}"),
                None,
            );
            return Err(anyhow!(
                "E_FFMPEG_FAILED: failed to start ffmpeg (cmd={cmd}): {e}"
            ));
        }
    };

    *pid_slot.lock().unwrap() = Some(child.id());

    loop {
        if token.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            *pid_slot.lock().unwrap() = None;
            span.err("logic", "E_CANCELLED", "cancelled", None);
            return Err(anyhow!("cancelled"));
        }
        let status_opt = match child.try_wait() {
            Ok(s) => s,
            Err(e) => {
                span.err(
                    "io",
                    "E_FFMPEG_TRYWAIT",
                    &format!("ffmpeg try_wait failed: {e}"),
                    None,
                );
                *pid_slot.lock().unwrap() = None;
                return Err(anyhow!("ffmpeg try_wait failed: {e}"));
            }
        };
        if let Some(status) = status_opt {
            if !status.success() {
                let excerpt = stderr_excerpt_from_child(child.stderr.take());
                *pid_slot.lock().unwrap() = None;
                if debug_log::verbose_enabled() {
                    let _ = debug_log::write_payload_best_effort(
                        data_dir,
                        task_id,
                        "ffmpeg_stderr.txt",
                        excerpt.as_bytes().to_vec(),
                    );
                }
                span.err(
                    "process",
                    "E_FFMPEG_FAILED",
                    &format!("ffmpeg preprocess failed: exit={status}"),
                    Some(serde_json::json!({
                        "exit": status.to_string(),
                        "stderr_chars": excerpt.len(),
                    })),
                );
                return Err(anyhow!(
                    "E_FFMPEG_FAILED: ffmpeg preprocess failed: exit={} stderr={}",
                    status,
                    excerpt
                ));
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Drain stderr on success too, to avoid holding OS pipes unnecessarily.
    let _ = stderr_excerpt_from_child(child.stderr.take());
    *pid_slot.lock().unwrap() = None;
    let ms = t0.elapsed().as_millis();
    span.ok(Some(serde_json::json!({ "elapsed_ms": ms })));
    Ok(ms)
}

// Intentionally no generic "run_audio_pipeline" helper to keep call sites explicit.
