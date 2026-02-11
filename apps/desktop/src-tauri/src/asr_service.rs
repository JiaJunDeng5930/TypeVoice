use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::trace::Span;
use crate::{debug_log, pipeline};

fn model_id_hint_for_trace(model_id: &str) -> String {
    let t = model_id.trim();
    if t.is_empty() {
        return "".to_string();
    }
    // If it's a filesystem path, keep only the last component to avoid leaking personal paths.
    if t.contains('\\') || t.contains(':') || t.starts_with('/') {
        return t.rsplit(['\\', '/']).next().unwrap_or(t).to_string();
    }
    // Otherwise assume it's a repo-like id and keep as-is.
    t.to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsrError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsrMetrics {
    pub audio_seconds: f64,
    pub elapsed_ms: i64,
    pub rtf: f64,
    pub device_used: String,
    pub model_id: String,
    pub model_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrSegment {
    pub index: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub duration_sec: f64,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrChunking {
    pub enabled: bool,
    pub chunk_sec: f64,
    pub num_segments: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AsrResponse {
    pub ok: bool,
    pub text: Option<String>,
    pub metrics: Option<AsrMetrics>,
    pub error: Option<AsrError>,
    pub segments: Option<Vec<AsrSegment>>,
    pub chunking: Option<AsrChunking>,
}

#[derive(Debug, Clone, Deserialize)]
struct AsrReady {
    #[allow(dead_code)]
    r#type: String,
    ok: bool,
    model_id: String,
    model_version: Option<String>,
    device_used: String,
    warmup_ms: i64,
}

struct Inner {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<std::process::ChildStdout>>,
    model_id: Option<String>,
    chunk_sec: f64,
    warmup_ms: Option<i64>,
    model_version: Option<String>,
}

#[derive(Clone)]
pub struct AsrService {
    inner: Arc<Mutex<Inner>>,
}

impl AsrService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                child: None,
                stdin: None,
                stdout: None,
                model_id: None,
                chunk_sec: 60.0,
                warmup_ms: None,
                model_version: None,
            })),
        }
    }

    pub fn ensure_started(&self, data_dir: &Path) -> Result<()> {
        let desired_model = pipeline::resolve_asr_model_id(data_dir)?;
        let desired_chunk = 60.0_f64;

        {
            let g = self.inner.lock().unwrap();
            if g.child.is_some()
                && g.model_id.as_deref() == Some(desired_model.as_str())
                && (g.chunk_sec - desired_chunk).abs() < 1e-6
            {
                return Ok(());
            }
        }

        self.restart(data_dir, "ensure_started")?;
        Ok(())
    }

    pub fn restart(&self, data_dir: &Path, reason: &str) -> Result<()> {
        self.kill_best_effort(reason);

        let model_id = pipeline::resolve_asr_model_id(data_dir)?;
        let chunk_sec = 60.0_f64;

        let root = repo_root()?;
        let py = crate::python_runtime::resolve_python_binary(&root)?;

        let span = Span::start(
            data_dir,
            None,
            "ASR",
            "ASR.restart",
            Some(serde_json::json!({
                "reason": reason,
                "model_id_hint": model_id_hint_for_trace(&model_id),
                "chunk_sec": chunk_sec,
            })),
        );

        let t0 = Instant::now();
        let mut child = match Command::new(&py)
            .current_dir(&root)
            .env("PYTHONPATH", &root)
            .env("TYPEVOICE_FFPROBE", pipeline::ffprobe_cmd()?)
            .args([
                "-m",
                "asr_runner.runner",
                "--daemon",
                "--model",
                &model_id,
                "--chunk-sec",
                &format!("{chunk_sec}"),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                span.err(
                    "process",
                    "E_ASR_SPAWN",
                    &format!(
                        "failed to spawn asr runner daemon: {e} (python={} root={})",
                        py.display(),
                        root.display()
                    ),
                    None,
                );
                return Err(anyhow!(
                    "failed to spawn asr runner daemon: {e} (python={} root={})",
                    py.display(),
                    root.display()
                ));
            }
        };

        let pid = child.id();
        // Ensure we never hang forever waiting for a ready line.
        let ready_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ready_flag2 = ready_flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(60));
            if !ready_flag2.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = kill_pid(pid);
            }
        });

        let stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                span.err("logic", "E_ASR_STDIN_MISSING", "runner stdin missing", None);
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("runner stdin missing"));
            }
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                span.err(
                    "logic",
                    "E_ASR_STDOUT_MISSING",
                    "runner stdout missing",
                    None,
                );
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("runner stdout missing"));
            }
        };
        let mut reader = BufReader::new(stdout);

        // Read one ready line.
        let mut line = String::new();
        loop {
            line.clear();
            let n = match reader.read_line(&mut line) {
                Ok(n) => n,
                Err(e) => {
                    span.err(
                        "io",
                        "E_ASR_READY_READ",
                        &format!("failed to read asr_ready line: {e}"),
                        None,
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!("failed to read asr_ready line: {e}"));
                }
            };
            if n == 0 {
                let _ = child.kill();
                let _ = child.wait();
                span.err(
                    "io",
                    "E_ASR_READY_EOF",
                    "asr runner daemon stdout EOF before ready",
                    None,
                );
                return Err(anyhow!("asr runner daemon stdout EOF before ready"));
            }
            let v: serde_json::Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(e) => {
                    span.err(
                        "parse",
                        "E_ASR_READY_PARSE",
                        &format!("invalid json from asr runner during ready: {e}"),
                        Some(serde_json::json!({"line_len": line.len()})),
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!("invalid json from asr runner during ready: {e}"));
                }
            };
            if v.get("type").and_then(|x| x.as_str()) == Some("asr_ready") {
                let ready: AsrReady = match serde_json::from_value(v) {
                    Ok(r) => r,
                    Err(e) => {
                        span.err(
                            "parse",
                            "E_ASR_READY_SCHEMA",
                            &format!("parse asr_ready failed: {e}"),
                            None,
                        );
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(anyhow!("parse asr_ready failed: {e}"));
                    }
                };
                if !ready.ok {
                    let _ = child.kill();
                    let _ = child.wait();
                    span.err(
                        "process",
                        "E_ASR_READY_NOT_OK",
                        "asr runner ready not ok",
                        None,
                    );
                    return Err(anyhow!("asr runner ready not ok"));
                }
                if ready.device_used != "cuda" {
                    let _ = child.kill();
                    let _ = child.wait();
                    span.err(
                        "process",
                        "E_ASR_DEVICE",
                        &format!("asr runner ready not cuda: {}", ready.device_used),
                        None,
                    );
                    return Err(anyhow!("asr runner ready not cuda: {}", ready.device_used));
                }

                let warmup_ms = t0.elapsed().as_millis() as i64;
                ready_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                let mut g = self.inner.lock().unwrap();
                g.model_id = Some(ready.model_id);
                g.chunk_sec = chunk_sec;
                g.warmup_ms = Some(ready.warmup_ms.max(0).max(warmup_ms));
                g.model_version = ready.model_version;
                g.stdin = Some(stdin);
                g.stdout = Some(reader);
                g.child = Some(child);
                span.ok(Some(serde_json::json!({
                    "model_id_hint": g.model_id.as_deref().map(model_id_hint_for_trace),
                    "device_used": "cuda",
                    "warmup_ms": g.warmup_ms,
                })));
                return Ok(());
            }
            // Ignore any other unexpected lines (should not happen).
        }
    }

    pub fn transcribe(
        &self,
        data_dir: &Path,
        task_id: &str,
        audio_path: &Path,
        language: &str,
        token: &CancellationToken,
        pid_slot: &Arc<Mutex<Option<u32>>>,
    ) -> Result<(AsrResponse, u128)> {
        if token.is_cancelled() {
            return Err(anyhow!("cancelled"));
        }

        let span = Span::start(
            data_dir,
            Some(task_id),
            "Transcribe",
            "ASR.transcribe",
            Some(serde_json::json!({
                "language": language,
            })),
        );

        if let Err(e) = self.ensure_started(data_dir) {
            span.err("process", "E_ASR_START", &e.to_string(), None);
            return Err(e);
        }

        let t0 = Instant::now();
        let mut g = self.inner.lock().unwrap();
        let child = match g.child.as_mut() {
            Some(c) => c,
            None => {
                span.err(
                    "process",
                    "E_ASR_NOT_STARTED",
                    "asr runner not started",
                    None,
                );
                return Err(anyhow!("asr runner not started"));
            }
        };
        let pid = child.id();
        *pid_slot.lock().unwrap() = Some(pid);

        let stdin = match g.stdin.as_mut() {
            Some(s) => s,
            None => {
                span.err("logic", "E_ASR_STDIN_MISSING", "runner stdin missing", None);
                return Err(anyhow!("runner stdin missing"));
            }
        };
        let req = serde_json::json!({
            "audio_path": audio_path,
            "language": language,
            "device": "cuda",
        });
        if let Err(e) = stdin.write_all(format!("{}\n", req).as_bytes()) {
            span.err(
                "io",
                "E_ASR_WRITE",
                &format!("failed to write runner request: {e}"),
                None,
            );
            return Err(anyhow!("failed to write runner request: {e}"));
        }
        stdin.flush().ok();

        let stdout = match g.stdout.as_mut() {
            Some(s) => s,
            None => {
                span.err(
                    "logic",
                    "E_ASR_STDOUT_MISSING",
                    "runner stdout missing",
                    None,
                );
                return Err(anyhow!("runner stdout missing"));
            }
        };
        let mut line = String::new();
        let read_res = stdout.read_line(&mut line);
        let wall_ms = t0.elapsed().as_millis();

        // Clear pid slot no matter what; cancellation kills the process itself.
        *pid_slot.lock().unwrap() = None;

        match read_res {
            Ok(0) => {
                drop(g);
                self.kill_best_effort("stdout_eof");
                if token.is_cancelled() {
                    return Err(anyhow!("cancelled"));
                }
                return Err(anyhow!("asr runner stdout EOF"));
            }
            Ok(_) => {
                let resp: AsrResponse = match serde_json::from_str(line.trim()) {
                    Ok(v) => v,
                    Err(e) => {
                        span.err(
                            "parse",
                            "E_ASR_PARSE",
                            &format!("runner returned invalid json: {e}"),
                            Some(serde_json::json!({"line_len": line.len()})),
                        );
                        return Err(anyhow!("runner returned invalid json: {e}"));
                    }
                };

                if debug_log::verbose_enabled() && debug_log::include_asr_segments() {
                    if let Some(segments) = resp.segments.clone() {
                        let payload = serde_json::to_vec_pretty(&serde_json::json!({
                            "task_id": task_id,
                            "chunking": resp.chunking,
                            "segments": segments,
                        }))
                        .unwrap_or_default();
                        if let Some(info) = debug_log::write_payload_best_effort(
                            data_dir,
                            task_id,
                            "asr_segments.json",
                            payload,
                        ) {
                            let note = resp
                                .chunking
                                .as_ref()
                                .map(|c| {
                                    format!(
                                        "chunking_enabled={} chunk_sec={} num_segments={}",
                                        c.enabled, c.chunk_sec, c.num_segments
                                    )
                                })
                                .or_else(|| {
                                    resp.segments.as_ref().map(|s| {
                                        format!("chunking_enabled=false num_segments={}", s.len())
                                    })
                                });
                            debug_log::emit_debug_event_best_effort(
                                data_dir,
                                "debug_asr_segments",
                                task_id,
                                &info,
                                note,
                            );
                        }
                    }
                }

                span.ok(Some(serde_json::json!({
                    "wall_ms": wall_ms,
                    "ok": resp.ok,
                    "has_segments": resp.segments.as_ref().map(|s| s.len()).unwrap_or(0),
                    "has_metrics": resp.metrics.is_some(),
                })));
                Ok((resp, wall_ms))
            }
            Err(e) => {
                drop(g);
                self.kill_best_effort("read_error");
                if token.is_cancelled() {
                    return Err(anyhow!("cancelled"));
                }
                span.err(
                    "io",
                    "E_ASR_READ",
                    &format!("failed to read runner output: {e}"),
                    None,
                );
                Err(anyhow!("failed to read runner output: {e}"))
            }
        }
    }

    pub fn kill_best_effort(&self, reason: &str) {
        let mut g = self.inner.lock().unwrap();
        if let Some(mut child) = g.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        g.stdin = None;
        g.stdout = None;
        g.model_id = None;
        g.warmup_ms = None;
        g.model_version = None;
        crate::safe_eprintln!("asr_service: killed runner ({reason})");
    }

    pub fn warmup_ms(&self) -> Option<i64> {
        let g = self.inner.lock().unwrap();
        g.warmup_ms
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) -> Result<()> {
    let status = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .context("kill failed")?;
    if !status.success() {
        return Err(anyhow!("kill exit={status}"));
    }
    Ok(())
}

#[cfg(windows)]
fn kill_pid(pid: u32) -> Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .context("taskkill failed")?;
    if !status.success() {
        return Err(anyhow!("taskkill exit={status}"));
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("TYPEVOICE_REPO_ROOT") {
        return Ok(PathBuf::from(p));
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = dir
        .ancestors()
        .nth(3)
        .ok_or_else(|| anyhow!("failed to locate repo root from CARGO_MANIFEST_DIR"))?;
    Ok(root.to_path_buf())
}
