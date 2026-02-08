use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct TaskEvent {
    pub task_id: String,
    pub stage: String,
    pub status: String, // started|completed|failed
    pub message: String,
    pub elapsed_ms: Option<u128>,
}

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
    let status = Command::new("ffmpeg")
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
        .status()
        .context("ffmpeg preprocess failed to start")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg preprocess failed: exit={status}"));
    }
    Ok(t0.elapsed().as_millis())
}

pub fn transcribe_with_python_runner(audio_wav: &Path, model_id: &str) -> Result<(String, f64, String, u128)> {
    let root = repo_root()?;
    let py = default_python_path(&root);
    let t0 = Instant::now();
    let mut child = Command::new(py)
        .current_dir(&root)
        .env("PYTHONPATH", &root)
        .args(["-m", "asr_runner.runner", "--model", model_id])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn asr runner")?;

    let stdin = child.stdin.as_mut().ok_or_else(|| anyhow!("runner stdin missing"))?;
    let req = json!({
        "audio_path": audio_wav,
        "language": "Chinese",
        "device": "cuda",
    });
    stdin
        .write_all(format!("{}\n", req.to_string()).as_bytes())
        .context("failed to write runner request")?;
    stdin.flush().ok();

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("runner stdout missing"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).context("failed to read runner output")?;

    // Try to exit quickly.
    let _ = child.kill();
    let _ = child.wait();

    let v: serde_json::Value = serde_json::from_str(line.trim()).context("runner returned invalid json")?;
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
    let metrics = v.get("metrics").ok_or_else(|| anyhow!("runner missing metrics"))?;
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

pub fn run_audio_pipeline_with_task_id(task_id: String, input_audio: &Path, model_id: &str) -> Result<TranscribeResult> {
    let root = repo_root()?;
    if !input_audio.exists() {
        return Err(anyhow!("input audio not found: {}", input_audio.display()));
    }
    let tmp = root.join("tmp").join("desktop");
    std::fs::create_dir_all(&tmp).ok();
    let wav = tmp.join(format!("{task_id}.wav"));

    let preprocess_ms = preprocess_ffmpeg(input_audio, &wav)?;
    let (text, rtf, device_used, asr_ms) = transcribe_with_python_runner(&wav, model_id)?;

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

pub fn run_audio_pipeline(input_audio: &Path, model_id: &str) -> Result<TranscribeResult> {
    run_audio_pipeline_with_task_id(Uuid::new_v4().to_string(), input_audio, model_id)
}
