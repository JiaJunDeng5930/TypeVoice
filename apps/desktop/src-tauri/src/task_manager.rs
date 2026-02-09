use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{data_dir, history, llm, metrics, pipeline, templates};

#[derive(Debug, Clone, Serialize)]
pub struct TaskEvent {
    pub task_id: String,
    pub stage: String,
    pub status: String, // started|completed|failed|cancelled
    pub message: String,
    pub elapsed_ms: Option<u128>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskDone {
    pub task_id: String,
    pub asr_text: String,
    pub final_text: String,
    pub rtf: f64,
    pub device_used: String,
    pub preprocess_ms: u128,
    pub asr_ms: u128,
    pub rewrite_ms: Option<u128>,
    pub rewrite_enabled: bool,
    pub template_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StartOpts {
    pub rewrite_enabled: bool,
    pub template_id: Option<String>,
}

#[derive(Clone)]
pub struct TaskManager {
    inner: Arc<Mutex<Option<ActiveTask>>>,
}

struct ActiveTask {
    task_id: String,
    token: CancellationToken,
    ffmpeg_pid: Arc<Mutex<Option<u32>>>,
    asr_pid: Arc<Mutex<Option<u32>>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start_fixture(
        &self,
        app: AppHandle,
        fixture_name: String,
        opts: StartOpts,
    ) -> Result<String> {
        let input = pipeline::fixture_path(&fixture_name)?;
        self.start_audio(app, input, opts, "Record (fixture)")
    }

    pub fn start_recording_base64(
        &self,
        app: AppHandle,
        b64: String,
        ext: String,
        opts: StartOpts,
    ) -> Result<String> {
        let task_id = Uuid::new_v4().to_string();
        let input = pipeline::save_base64_file(&task_id, &b64, &ext)?;
        self.start_audio_with_task_id(app, task_id, input, opts, "Record (saved)")
    }

    fn start_audio(
        &self,
        app: AppHandle,
        input: PathBuf,
        opts: StartOpts,
        record_msg: &str,
    ) -> Result<String> {
        let task_id = Uuid::new_v4().to_string();
        self.start_audio_with_task_id(app, task_id, input, opts, record_msg)
    }

    fn start_audio_with_task_id(
        &self,
        app: AppHandle,
        task_id: String,
        input: PathBuf,
        opts: StartOpts,
        record_msg: &str,
    ) -> Result<String> {
        {
            let mut g = self.inner.lock().unwrap();
            if g.is_some() {
                return Err(anyhow!("another task is already running"));
            }
            *g = Some(ActiveTask {
                task_id: task_id.clone(),
                token: CancellationToken::new(),
                ffmpeg_pid: Arc::new(Mutex::new(None)),
                asr_pid: Arc::new(Mutex::new(None)),
            });
        }

        let active = {
            let g = self.inner.lock().unwrap();
            g.as_ref().unwrap().task_id.clone()
        };
        let this = self.clone();
        let record_msg = record_msg.to_string();

        // The invoke handler may execute on a thread without an active Tokio
        // runtime/reactor. We detach into an OS thread and drive the async
        // pipeline using a dedicated Tokio runtime to avoid "no reactor
        // running" panics (panicking here aborts the process on Windows).
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match rt {
                Ok(rt) => {
                    rt.block_on(async move {
                        let res = this
                            .run_pipeline(app.clone(), task_id.clone(), input, opts, &record_msg)
                            .await;
                        if let Err(e) = res {
                            // Fail-safe: ensure the UI always gets a terminal event.
                            let maybe_dir = data_dir::data_dir().ok();
                            if let Some(dir) = maybe_dir {
                                emit_failed(
                                    &app,
                                    &dir,
                                    &task_id,
                                    "Internal",
                                    None,
                                    "E_INTERNAL",
                                    &e.to_string(),
                                );
                            } else {
                                let _ = app.emit(
                                    "task_event",
                                    TaskEvent {
                                        task_id: task_id.clone(),
                                        stage: "Internal".to_string(),
                                        status: "failed".to_string(),
                                        message: e.to_string(),
                                        elapsed_ms: None,
                                        error_code: Some("E_INTERNAL".to_string()),
                                    },
                                );
                            }
                        }
                        let mut g = this.inner.lock().unwrap();
                        *g = None;
                    });
                }
                Err(e) => {
                    // Best-effort cleanup; we might not have a data_dir to emit metrics.
                    eprintln!("failed to create tokio runtime for task {task_id}: {e}");
                    let mut g = this.inner.lock().unwrap();
                    *g = None;
                }
            }
        });

        Ok(active)
    }

    pub fn cancel(&self, task_id: &str) -> Result<()> {
        let g = self.inner.lock().unwrap();
        let active = g.as_ref().ok_or_else(|| anyhow!("no active task"))?;
        if active.task_id != task_id {
            return Err(anyhow!("task_id mismatch"));
        }
        active.token.cancel();
        // Best-effort kill for external processes.
        if let Some(pid) = *active.ffmpeg_pid.lock().unwrap() {
            let _ = kill_pid(pid);
        }
        if let Some(pid) = *active.asr_pid.lock().unwrap() {
            let _ = kill_pid(pid);
        }
        Ok(())
    }

    async fn run_pipeline(
        &self,
        app: AppHandle,
        task_id: String,
        input: PathBuf,
        opts: StartOpts,
        record_msg: &str,
    ) -> Result<()> {
        let data_dir = data_dir::data_dir()?;

        emit_event(
            &app,
            &data_dir,
            TaskEvent {
                task_id: task_id.clone(),
                stage: "Record".to_string(),
                status: "completed".to_string(),
                message: record_msg.to_string(),
                elapsed_ms: Some(0),
                error_code: None,
            },
        );

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Record");
            return Ok(());
        }

        // Preprocess
        emit_started(&app, &data_dir, &task_id, "Preprocess", "ffmpeg");
        let wav_path = pipeline::preprocess_to_temp_wav(&task_id, &input)?;
        let preprocess_ms = {
            let inner = self.inner.clone();
            let input2 = input.clone();
            let wav2 = wav_path.clone();
            let join = tokio::task::spawn_blocking(move || {
                let active = inner.lock().unwrap();
                let a = active.as_ref().ok_or_else(|| anyhow!("task missing"))?;
                // launch ffmpeg inside helper so we can store pid
                let ms = pipeline::preprocess_ffmpeg_cancellable(
                    &input2,
                    &wav2,
                    &a.token,
                    &a.ffmpeg_pid,
                )?;
                Ok::<_, anyhow::Error>(ms)
            })
            .await;
            match join {
                Ok(Ok(ms)) => ms,
                Ok(Err(e)) => {
                    if is_cancelled_err(&e) || is_cancelled(&self.inner, &task_id) {
                        emit_cancelled(&app, &data_dir, &task_id, "Preprocess");
                        let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
                        return Ok(());
                    }
                    let msg = e.to_string();
                    let code = if msg.contains("E_FFMPEG_NOT_FOUND") {
                        "E_FFMPEG_NOT_FOUND"
                    } else if msg.contains("E_FFMPEG_FAILED") {
                        "E_FFMPEG_FAILED"
                    } else {
                        "E_PREPROCESS_FAILED"
                    };
                    emit_failed(
                        &app,
                        &data_dir,
                        &task_id,
                        "Preprocess",
                        None,
                        code,
                        &msg,
                    );
                    let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
                    return Ok(());
                }
                Err(e) => {
                    emit_failed(
                        &app,
                        &data_dir,
                        &task_id,
                        "Preprocess",
                        None,
                        "E_INTERNAL",
                        &format!("preprocess_join_failed:{e}"),
                    );
                    let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
                    return Ok(());
                }
            }
        };
        emit_completed(&app, &data_dir, &task_id, "Preprocess", preprocess_ms, "ok");

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Preprocess");
            let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
            return Ok(());
        }

        // ASR
        emit_started(&app, &data_dir, &task_id, "Transcribe", "asr");
        let (asr_text, rtf, device_used, asr_ms) = {
            let inner = self.inner.clone();
            let wav_path2 = wav_path.clone();
            let data_dir2 = data_dir.clone();
            let join = tokio::task::spawn_blocking(move || {
                let active = inner.lock().unwrap();
                let a = active.as_ref().ok_or_else(|| anyhow!("task missing"))?;
                let model_id = pipeline::resolve_asr_model_id(&data_dir2)?;
                let (text, rtf, device, ms) = pipeline::transcribe_with_python_runner_cancellable(
                    &wav_path2, &model_id, &a.token, &a.asr_pid,
                )?;
                if device != "cuda" {
                    return Err(anyhow!("device_not_cuda:{device}"));
                }
                Ok::<_, anyhow::Error>((text, rtf, device, ms))
            })
            .await;
            match join {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => {
                    if is_cancelled_err(&e) || is_cancelled(&self.inner, &task_id) {
                        emit_cancelled(&app, &data_dir, &task_id, "Transcribe");
                        let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
                        return Ok(());
                    }
                    emit_failed(
                        &app,
                        &data_dir,
                        &task_id,
                        "Transcribe",
                        None,
                        "E_ASR_FAILED",
                        &e.to_string(),
                    );
                    let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
                    return Ok(());
                }
                Err(e) => {
                    emit_failed(
                        &app,
                        &data_dir,
                        &task_id,
                        "Transcribe",
                        None,
                        "E_INTERNAL",
                        &format!("transcribe_join_failed:{e}"),
                    );
                    let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
                    return Ok(());
                }
            }
        };
        emit_completed(
            &app,
            &data_dir,
            &task_id,
            "Transcribe",
            asr_ms,
            format!("rtf={rtf:.3}"),
        );

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Transcribe");
            let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);
            return Ok(());
        }

        // We no longer need audio artifacts after ASR; cleanup early.
        let _ = pipeline::cleanup_audio_artifacts(&input, &wav_path);

        // Rewrite (optional)
        let mut final_text = asr_text.clone();
        let mut rewrite_ms = None;
        let mut template_id = None;
        if opts.rewrite_enabled {
            if let Some(tid) = opts.template_id.clone() {
                template_id = Some(tid.clone());
                emit_started(&app, &data_dir, &task_id, "Rewrite", "llm");
                let t0 = Instant::now();
                let tpl = match templates::get_template(&data_dir, &tid) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        rewrite_ms = Some(t0.elapsed().as_millis());
                        emit_failed(
                            &app,
                            &data_dir,
                            &task_id,
                            "Rewrite",
                            rewrite_ms,
                            "E_TEMPLATE_NOT_FOUND",
                            &e.to_string(),
                        );
                        None
                    }
                };
                if let Some(tpl) = tpl {
                    let token = {
                        let g = self.inner.lock().unwrap();
                        g.as_ref().unwrap().token.clone()
                    };
                    let rewrite_res = tokio::select! {
                        _ = token.cancelled() => Err(anyhow!("cancelled")),
                        r = llm::rewrite(&data_dir, &tpl.system_prompt, &asr_text) => r,
                    };
                    match rewrite_res {
                        Ok(txt) => {
                            final_text = txt;
                            rewrite_ms = Some(t0.elapsed().as_millis());
                            emit_completed(
                                &app,
                                &data_dir,
                                &task_id,
                                "Rewrite",
                                rewrite_ms.unwrap(),
                                "ok",
                            );
                        }
                        Err(e) => {
                            if is_cancelled_err(&e) || is_cancelled(&self.inner, &task_id) {
                                emit_cancelled(&app, &data_dir, &task_id, "Rewrite");
                                return Ok(());
                            }
                            // fallback to asr_text
                            rewrite_ms = Some(t0.elapsed().as_millis());
                            emit_failed(
                                &app,
                                &data_dir,
                                &task_id,
                                "Rewrite",
                                rewrite_ms,
                                "E_LLM_FAILED",
                                &e.to_string(),
                            );
                        }
                    }
                }
            }
        }

        if is_cancelled(&self.inner, &task_id) {
            emit_cancelled(&app, &data_dir, &task_id, "Rewrite");
            return Ok(());
        }

        // Persist history
        emit_started(&app, &data_dir, &task_id, "Persist", "sqlite");
        let created_at_ms = chrono_now_ms();
        let item = history::HistoryItem {
            task_id: task_id.clone(),
            created_at_ms,
            asr_text: asr_text.clone(),
            final_text: final_text.clone(),
            template_id: template_id.clone(),
            rtf,
            device_used: device_used.clone(),
            preprocess_ms: preprocess_ms as i64,
            asr_ms: asr_ms as i64,
        };
        let db = data_dir.join("history.sqlite3");
        if let Err(e) = history::append(&db, &item) {
            emit_failed(
                &app,
                &data_dir,
                &task_id,
                "Persist",
                None,
                "E_PERSIST_FAILED",
                &e.to_string(),
            );
            return Ok(());
        }
        emit_completed(&app, &data_dir, &task_id, "Persist", 0, "ok");

        // Export stage is UI-driven (copy). We still emit completed to align spec.
        emit_event(
            &app,
            &data_dir,
            TaskEvent {
                task_id: task_id.clone(),
                stage: "Export".to_string(),
                status: "completed".to_string(),
                message: "copy in UI".to_string(),
                elapsed_ms: Some(0),
                error_code: None,
            },
        );

        // Done event
        let done = TaskDone {
            task_id: task_id.clone(),
            asr_text,
            final_text,
            rtf,
            device_used,
            preprocess_ms,
            asr_ms,
            rewrite_ms,
            rewrite_enabled: opts.rewrite_enabled,
            template_id,
        };
        let _ = app.emit("task_done", done.clone());
        if let Err(e) = metrics::append_jsonl(
            &data_dir,
            &json!({"type":"task_done","task_id":task_id,"rtf":done.rtf,"device":done.device_used}),
        ) {
            eprintln!("metrics append failed (task_done): {e:#}");
        }
        Ok(())
    }
}

fn chrono_now_ms() -> i64 {
    // avoid chrono dependency; use std time
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn emit_started(app: &AppHandle, data_dir: &Path, task_id: &str, stage: &str, msg: &str) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "started".to_string(),
            message: msg.to_string(),
            elapsed_ms: None,
            error_code: None,
        },
    );
}

fn emit_completed(
    app: &AppHandle,
    data_dir: &Path,
    task_id: &str,
    stage: &str,
    elapsed_ms: u128,
    msg: impl Into<String>,
) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "completed".to_string(),
            message: msg.into(),
            elapsed_ms: Some(elapsed_ms),
            error_code: None,
        },
    );
}

fn emit_failed(
    app: &AppHandle,
    data_dir: &Path,
    task_id: &str,
    stage: &str,
    elapsed_ms: Option<u128>,
    code: &str,
    msg: &str,
) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "failed".to_string(),
            message: msg.to_string(),
            elapsed_ms,
            error_code: Some(code.to_string()),
        },
    );
}

fn emit_cancelled(app: &AppHandle, data_dir: &Path, task_id: &str, stage: &str) {
    emit_event(
        app,
        data_dir,
        TaskEvent {
            task_id: task_id.to_string(),
            stage: stage.to_string(),
            status: "cancelled".to_string(),
            message: "cancelled".to_string(),
            elapsed_ms: None,
            error_code: Some("E_CANCELLED".to_string()),
        },
    );
}

fn emit_event(app: &AppHandle, data_dir: &Path, ev: TaskEvent) {
    let _ = app.emit("task_event", ev.clone());
    if let Err(e) = metrics::append_jsonl(
        data_dir,
        &json!({"type":"task_event", "task_id":ev.task_id, "stage":ev.stage, "status":ev.status, "elapsed_ms":ev.elapsed_ms, "error_code":ev.error_code, "message":ev.message}),
    ) {
        eprintln!("metrics append failed (task_event): {e:#}");
    }
}

fn is_cancelled(inner: &Arc<Mutex<Option<ActiveTask>>>, task_id: &str) -> bool {
    let g = inner.lock().unwrap();
    if let Some(a) = g.as_ref() {
        if a.task_id == task_id {
            return a.token.is_cancelled();
        }
    }
    false
}

fn is_cancelled_err(e: &anyhow::Error) -> bool {
    let s = e.to_string();
    s == "cancelled" || s.contains("cancelled")
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
