use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::debug_log;
use crate::trace::Span;

const MAX_TOOL_STDERR_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize)]
pub struct TranscribeResult {
    pub task_id: String,
    pub asr_text: String,
    pub rtf: f64,
    pub device_used: String,
    pub preprocess_ms: u128,
    pub asr_ms: u128,
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

fn default_python_path(root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("TYPEVOICE_PYTHON") {
        return PathBuf::from(p);
    }
    // Dev default: repo-local venv.
    if cfg!(windows) {
        root.join(".venv").join("Scripts").join("python.exe")
    } else {
        root.join(".venv").join("bin").join("python")
    }
}

fn resolve_tool_path(env_key: &str, candidate_file: &str, fallback: &str) -> String {
    if let Ok(p) = std::env::var(env_key) {
        let t = p.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }

    // In packaged apps it's common to place helper binaries next to the main executable.
    if cfg!(windows) {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let cand = dir.join(candidate_file);
                if cand.exists() {
                    return cand.display().to_string();
                }
            }
        }
    }

    fallback.to_string()
}

pub fn ffmpeg_cmd() -> String {
    resolve_tool_path("TYPEVOICE_FFMPEG", "ffmpeg.exe", "ffmpeg")
}

pub fn ffprobe_cmd() -> String {
    resolve_tool_path("TYPEVOICE_FFPROBE", "ffprobe.exe", "ffprobe")
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

pub fn fixture_path(name: &str) -> Result<PathBuf> {
    let root = repo_root()?;
    Ok(root.join("fixtures").join(name))
}

pub fn save_base64_file(task_id: &str, b64: &str, ext: &str) -> Result<PathBuf> {
    let root = repo_root()?;
    let tmp = root.join("tmp").join("desktop");
    std::fs::create_dir_all(&tmp).ok();
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let input = tmp.join(format!("{task_id}.{ext}"));

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .context("base64 decode failed")?;
    std::fs::write(&input, bytes).context("failed to write recording file")?;
    Ok(input)
}

pub fn preprocess_ffmpeg(input: &Path, output: &Path) -> Result<u128> {
    let t0 = Instant::now();
    let cmd = ffmpeg_cmd();
    let out = Command::new(&cmd)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            input
                .to_str()
                .ok_or_else(|| anyhow!("non-utf8 input path"))?,
            "-ac",
            "1",
            "-ar",
            "16000",
            "-vn",
            output
                .to_str()
                .ok_or_else(|| anyhow!("non-utf8 output path"))?,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!("E_FFMPEG_NOT_FOUND: ffmpeg not found (cmd={cmd})")
            } else {
                anyhow!("E_FFMPEG_FAILED: failed to start ffmpeg (cmd={cmd}): {e}")
            }
        })?;
    if !out.status.success() {
        let mut stderr = out.stderr;
        stderr = truncate_stderr_bytes(stderr);
        let excerpt = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(anyhow!(
            "E_FFMPEG_FAILED: ffmpeg preprocess failed: exit={} stderr={}",
            out.status,
            excerpt
        ));
    }
    Ok(t0.elapsed().as_millis())
}

pub fn transcribe_with_python_runner(
    audio_wav: &Path,
    model_id: &str,
) -> Result<(String, f64, String, u128)> {
    let root = repo_root()?;
    let py = default_python_path(&root);
    let t0 = Instant::now();
    let mut child = Command::new(py)
        .current_dir(&root)
        .env("PYTHONPATH", &root)
        .env("TYPEVOICE_FFPROBE", ffprobe_cmd())
        .args(["-m", "asr_runner.runner", "--model", model_id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn asr runner")?;

    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("runner stdin missing"))?;
    let req = json!({
        "audio_path": audio_wav,
        "language": "Chinese",
        "device": "cuda",
    });
    stdin
        .write_all(format!("{}\n", req.to_string()).as_bytes())
        .context("failed to write runner request")?;
    stdin.flush().ok();

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("runner stdout missing"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| {
        let _ = child.kill();
        let _ = child.wait();
        anyhow!("failed to read runner output: {e}")
    })?;

    // Try to exit quickly.
    let _ = child.kill();
    let _ = child.wait();

    let v: serde_json::Value =
        serde_json::from_str(line.trim()).context("runner returned invalid json")?;
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        let code = v
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|x| x.as_str())
            .unwrap_or("E_ASR_FAILED");
        return Err(anyhow!("asr failed: {code}"));
    }
    let text = v
        .get("text")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("runner missing text"))?
        .to_string();
    let metrics = v
        .get("metrics")
        .ok_or_else(|| anyhow!("runner missing metrics"))?;
    let rtf = metrics
        .get("rtf")
        .and_then(|x| x.as_f64())
        .ok_or_else(|| anyhow!("runner missing rtf"))?;
    let device_used = metrics
        .get("device_used")
        .and_then(|x| x.as_str())
        .unwrap_or("cuda")
        .to_string();
    Ok((text, rtf, device_used, t0.elapsed().as_millis()))
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

#[allow(dead_code)]
pub fn transcribe_with_python_runner_cancellable(
    audio_wav: &Path,
    model_id: &str,
    token: &tokio_util::sync::CancellationToken,
    pid_slot: &std::sync::Arc<std::sync::Mutex<Option<u32>>>,
) -> Result<(String, f64, String, u128)> {
    let root = repo_root()?;
    let py = default_python_path(&root);
    let t0 = Instant::now();
    let mut child = Command::new(py)
        .current_dir(&root)
        .env("PYTHONPATH", &root)
        // If the app bundles ffprobe, provide its location to the runner.
        .env("TYPEVOICE_FFPROBE", ffprobe_cmd())
        .args(["-m", "asr_runner.runner", "--model", model_id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn asr runner")?;

    let pid = child.id();
    *pid_slot.lock().unwrap() = Some(pid);

    if token.is_cancelled() {
        let _ = child.kill();
        let _ = child.wait();
        *pid_slot.lock().unwrap() = None;
        return Err(anyhow!("cancelled"));
    }

    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("runner stdin missing"))?;
    let req = json!({
        "audio_path": audio_wav,
        "language": "Chinese",
        "device": "cuda",
    });
    stdin
        .write_all(format!("{}\n", req.to_string()).as_bytes())
        .context("failed to write runner request")?;
    stdin.flush().ok();

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("runner stdout missing"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();

    // Poll cancellation while waiting for output.
    loop {
        if token.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            *pid_slot.lock().unwrap() = None;
            return Err(anyhow!("cancelled"));
        }
        // read_line blocks; so we use try_wait on process + small sleep? Keep simple:
        // attempt read_line once (will block) is not cancellable. To keep cancel <=300ms
        // we rely on external kill by pid_slot in TaskManager.cancel().
        break;
    }

    reader.read_line(&mut line).map_err(|e| {
        let _ = child.kill();
        let _ = child.wait();
        *pid_slot.lock().unwrap() = None;
        anyhow!("failed to read runner output: {e}")
    })?;

    // Ensure process stops.
    let _ = child.kill();
    let _ = child.wait();
    *pid_slot.lock().unwrap() = None;

    let v: serde_json::Value =
        serde_json::from_str(line.trim()).context("runner returned invalid json")?;
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        let code = v
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|x| x.as_str())
            .unwrap_or("E_ASR_FAILED");
        return Err(anyhow!("asr failed: {code}"));
    }
    let text = v
        .get("text")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("runner missing text"))?
        .to_string();
    let metrics = v
        .get("metrics")
        .ok_or_else(|| anyhow!("runner missing metrics"))?;
    let rtf = metrics
        .get("rtf")
        .and_then(|x| x.as_f64())
        .ok_or_else(|| anyhow!("runner missing rtf"))?;
    let device_used = metrics
        .get("device_used")
        .and_then(|x| x.as_str())
        .unwrap_or("cuda")
        .to_string();
    Ok((text, rtf, device_used, t0.elapsed().as_millis()))
}

pub fn preprocess_ffmpeg_cancellable(
    data_dir: &Path,
    task_id: &str,
    input: &Path,
    output: &Path,
    token: &tokio_util::sync::CancellationToken,
    pid_slot: &std::sync::Arc<std::sync::Mutex<Option<u32>>>,
) -> Result<u128> {
    let span = Span::start(
        data_dir,
        Some(task_id),
        "Preprocess",
        "FFMPEG.preprocess",
        Some(serde_json::json!({
            "cmd": ffmpeg_cmd(),
        })),
    );

    let t0 = Instant::now();
    let cmd = ffmpeg_cmd();
    let input_s = match input.to_str() {
        Some(s) => s,
        None => {
            span.err("io", "E_PATH_UTF8", "non-utf8 input path", None);
            return Err(anyhow!("non-utf8 input path"));
        }
    };
    let output_s = match output.to_str() {
        Some(s) => s,
        None => {
            span.err("io", "E_PATH_UTF8", "non-utf8 output path", None);
            return Err(anyhow!("non-utf8 output path"));
        }
    };

    let mut child = match Command::new(&cmd)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            input_s,
            "-ac",
            "1",
            "-ar",
            "16000",
            "-vn",
            output_s,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                span.err("process", "E_FFMPEG_NOT_FOUND", &format!("ffmpeg not found (cmd={cmd})"), None);
                return Err(anyhow!("E_FFMPEG_NOT_FOUND: ffmpeg not found (cmd={cmd})"));
            }
            span.err(
                "process",
                "E_FFMPEG_FAILED",
                &format!("failed to start ffmpeg (cmd={cmd}): {e}"),
                None,
            );
            return Err(anyhow!("E_FFMPEG_FAILED: failed to start ffmpeg (cmd={cmd}): {e}"));
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

pub fn run_audio_pipeline_with_task_id(
    task_id: String,
    input_audio: &Path,
    model_id: &str,
) -> Result<TranscribeResult> {
    let root = repo_root()?;
    if !input_audio.exists() {
        return Err(anyhow!("input audio not found: {}", input_audio.display()));
    }
    let tmp = root.join("tmp").join("desktop");
    std::fs::create_dir_all(&tmp).ok();
    let wav = tmp.join(format!("{task_id}.wav"));

    let preprocess_ms = preprocess_ffmpeg(input_audio, &wav)?;
    let (text, rtf, device_used, asr_ms) = transcribe_with_python_runner(&wav, model_id)?;

    let _ = cleanup_audio_artifacts(input_audio, &wav);

    Ok(TranscribeResult {
        task_id,
        asr_text: text,
        rtf,
        device_used,
        preprocess_ms,
        asr_ms,
    })
}

pub fn run_fixture_pipeline(fixture_name: &str) -> Result<TranscribeResult> {
    let input = fixture_path(fixture_name)?;
    run_audio_pipeline_with_task_id(Uuid::new_v4().to_string(), &input, "Qwen/Qwen3-ASR-0.6B")
}

// Intentionally no generic "run_audio_pipeline" helper to keep call sites explicit.
