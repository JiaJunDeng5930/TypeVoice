mod asr_service;
mod context_capture;
#[cfg(windows)]
mod context_capture_windows;
mod context_pack;
mod data_dir;
mod debug_log;
mod history;
mod hotkeys;
mod llm;
mod metrics;
mod model;
mod panic_log;
mod pipeline;
mod python_runtime;
mod safe_print;
mod settings;
mod startup_trace;
mod task_manager;
mod templates;
mod toolchain;
mod trace;

use history::HistoryItem;
use llm::ApiKeyStatus;
use model::ModelStatus;
use settings::Settings;
use settings::SettingsPatch;
use task_manager::TaskManager;
use tauri::Emitter;
use tauri::Manager;
use templates::PromptTemplate;
use trace::Span;

struct RuntimeState {
    toolchain: std::sync::Mutex<toolchain::ToolchainStatus>,
    python: std::sync::Mutex<python_runtime::PythonStatus>,
}

struct ActiveBackendRecording {
    recording_id: String,
    output_path: std::path::PathBuf,
    child: std::process::Child,
    started_at: std::time::Instant,
}

struct RecordedAsset {
    asset_id: String,
    output_path: std::path::PathBuf,
    record_elapsed_ms: u128,
    created_at: std::time::Instant,
}

struct BackendRecordingInner {
    active: Option<ActiveBackendRecording>,
    assets: std::collections::HashMap<String, RecordedAsset>,
}

struct BackendRecordingState {
    inner: std::sync::Mutex<BackendRecordingInner>,
}

impl RuntimeState {
    fn new() -> Self {
        Self {
            toolchain: std::sync::Mutex::new(toolchain::ToolchainStatus::pending()),
            python: std::sync::Mutex::new(python_runtime::PythonStatus::pending()),
        }
    }

    fn set_toolchain(&self, st: toolchain::ToolchainStatus) {
        let mut g = self.toolchain.lock().unwrap();
        *g = st;
    }

    fn get_toolchain(&self) -> toolchain::ToolchainStatus {
        self.toolchain.lock().unwrap().clone()
    }

    fn set_python(&self, st: python_runtime::PythonStatus) {
        let mut g = self.python.lock().unwrap();
        *g = st;
    }

    fn get_python(&self) -> python_runtime::PythonStatus {
        self.python.lock().unwrap().clone()
    }
}

impl BackendRecordingState {
    fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(BackendRecordingInner {
                active: None,
                assets: std::collections::HashMap::new(),
            }),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OverlayState {
    visible: bool,
    status: String,
    detail: Option<String>,
    ts_ms: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartTaskRequest {
    trigger_source: String, // ui|hotkey|fixture
    record_mode: String, // recording_asset|fixture
    recording_asset_id: Option<String>,
    fixture_name: Option<String>,
    recording_session_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StopBackendRecordingResult {
    recording_id: String,
    recording_asset_id: String,
    ext: String,
}

#[tauri::command]
fn overlay_set_state(app: tauri::AppHandle, state: OverlayState) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.overlay_set_state",
        Some(serde_json::json!({
            "visible": state.visible,
            "status": state.status,
            "has_detail": state.detail.as_deref().map(|s| !s.is_empty()).unwrap_or(false),
        })),
    );

    if let Some(w) = app.get_webview_window("overlay") {
        if state.visible {
            let _ = w.show();
        } else {
            let _ = w.hide();
        }
    }

    // Broadcast: the overlay window listens and updates its UI.
    let _ = app.emit("tv_overlay_state", state);
    span.ok(None);
    Ok(())
}

fn cmd_span(
    data_dir: &std::path::Path,
    task_id: Option<&str>,
    step_id: &str,
    ctx: Option<serde_json::Value>,
) -> Span {
    Span::start(data_dir, task_id, "Cmd", step_id, ctx)
}

fn repo_root() -> Result<std::path::PathBuf, String> {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "repo root not found".to_string())
}

fn runtime_not_ready(runtime: &RuntimeState) -> Option<(&'static str, String)> {
    let tc = runtime.get_toolchain();
    if !tc.ready {
        let msg = tc
            .message
            .unwrap_or_else(|| "E_TOOLCHAIN_NOT_READY: toolchain is not ready".to_string());
        return Some(("E_TOOLCHAIN_NOT_READY", msg));
    }

    let py = runtime.get_python();
    if !py.ready {
        let msg = py
            .message
            .unwrap_or_else(|| "E_PYTHON_NOT_READY: python runtime is not ready".to_string());
        return Some(("E_PYTHON_NOT_READY", msg));
    }
    None
}

fn start_opts_from_settings(data_dir: &std::path::Path) -> Result<task_manager::StartOpts, String> {
    let s = settings::load_settings_strict(data_dir).map_err(|e| e.to_string())?;
    let (rewrite_enabled, template_id) =
        settings::resolve_rewrite_start_config(&s).map_err(|e| e.to_string())?;
    let asr_preprocess = resolve_asr_preprocess_config(&s);
    Ok(task_manager::StartOpts {
        rewrite_enabled,
        template_id,
        context_cfg: context_capture::config_from_settings(&s),
        rewrite_glossary: sanitize_rewrite_glossary(s.rewrite_glossary),
        rewrite_include_glossary: s.rewrite_include_glossary.unwrap_or(true),
        asr_preprocess,
        pre_captured_context: None,
        recording_session_id: None,
        record_elapsed_ms: 0,
        record_label: "Record".to_string(),
    })
}

fn abort_recording_session_if_present(
    state: &tauri::State<'_, TaskManager>,
    recording_session_id: &Option<String>,
) {
    if let Some(id) = recording_session_id.as_deref() {
        state.abort_recording_session(id);
    }
}

fn cleanup_expired_recording_assets(
    recorder: &tauri::State<'_, BackendRecordingState>,
    max_age: std::time::Duration,
) {
    let mut g = recorder.inner.lock().unwrap();
    let expired_ids: Vec<String> = g
        .assets
        .iter()
        .filter_map(|(id, asset)| {
            if asset.created_at.elapsed() > max_age {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect();
    for id in expired_ids {
        if let Some(asset) = g.assets.remove(&id) {
            let _ = std::fs::remove_file(&asset.output_path);
        }
    }
}

fn has_active_recording(recorder: &tauri::State<'_, BackendRecordingState>) -> bool {
    recorder.inner.lock().unwrap().active.is_some()
}

fn first_quoted_token(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let tail = &line[start + 1..];
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

fn read_last_stderr_line(stderr: &mut std::process::ChildStderr) -> Option<String> {
    let mut buf = String::new();
    if std::io::Read::read_to_string(stderr, &mut buf).is_err() {
        return None;
    }
    buf.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn read_last_stderr_line_from_bytes(stderr: &[u8]) -> Option<String> {
    let buf = String::from_utf8_lossy(stderr);
    buf.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn parse_dshow_audio_devices(stderr: &str) -> Vec<(String, Option<String>)> {
    let mut devices: Vec<(String, Option<String>)> = Vec::new();
    let mut pending_idx: Option<usize> = None;
    for line in stderr.lines() {
        let text = line.trim();
        if text.contains("Alternative name") {
            if let (Some(idx), Some(alt)) = (pending_idx, first_quoted_token(text)) {
                if let Some((_, slot)) = devices.get_mut(idx) {
                    *slot = Some(alt);
                }
            }
            continue;
        }
        if !text.contains("(audio)") {
            continue;
        }
        if let Some(name) = first_quoted_token(text) {
            devices.push((name, None));
            pending_idx = Some(devices.len() - 1);
        }
    }
    devices
}

fn score_audio_device_name(name: &str) -> i32 {
    let lower = name.to_lowercase();
    let mut score = 0_i32;
    let positive = ["microphone", "mic", "array", "input", "capture"];
    for kw in positive {
        if lower.contains(kw) {
            score += 10;
        }
    }
    if name.contains("麦克风") || name.contains("阵列") {
        score += 12;
    }
    let negative = [
        "headset",
        "headphone",
        "speaker",
        "output",
        "loopback",
        "stereo mix",
        "broadcast",
        "virtual",
    ];
    for kw in negative {
        if lower.contains(kw) {
            score -= 10;
        }
    }
    if name.contains("耳机") || name.contains("扬声器") {
        score -= 12;
    }
    score
}

fn normalize_record_input_spec(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("audio=") {
        let value = rest.trim();
        if value.len() >= 2 {
            let bytes = value.as_bytes();
            let first = bytes[0] as char;
            let last = bytes[value.len() - 1] as char;
            if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
                return format!("audio={}", &value[1..value.len() - 1]);
            }
        }
    }
    trimmed.to_string()
}

fn probe_record_input_spec(ffmpeg: &std::path::Path, spec: &str) -> Result<(), String> {
    let null_sink = if cfg!(windows) { "NUL" } else { "-" };
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "dshow", "-i"])
        .arg(spec)
        .args(["-t", "0.15", "-f", "null", null_sink])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("probe spawn failed: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let tail = read_last_stderr_line_from_bytes(&output.stderr)
        .unwrap_or_else(|| "probe failed without stderr".to_string());
    Err(format!("{tail} (status={})", output.status))
}

fn auto_resolve_record_input_spec(ffmpeg: &std::path::Path) -> Result<String, String> {
    let list_out = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("E_RECORD_INPUT_DISCOVERY_FAILED: enumerate dshow device failed: {e}"))?;
    let stderr = String::from_utf8_lossy(&list_out.stderr);
    let devices = parse_dshow_audio_devices(&stderr);
    if devices.is_empty() {
        return Err("E_RECORD_INPUT_DISCOVERY_FAILED: no dshow audio device found".to_string());
    }

    #[derive(Debug)]
    struct Candidate {
        spec: String,
        name: String,
        score: i32,
        order: usize,
    }

    let mut candidates: Vec<Candidate> = devices
        .into_iter()
        .enumerate()
        .map(|(i, (name, alt))| {
            let target = alt.unwrap_or_else(|| name.clone());
            Candidate {
                spec: format!("audio={target}"),
                name,
                score: score_audio_device_name(&target),
                order: i,
            }
        })
        .collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.order.cmp(&b.order)));

    let mut failures = Vec::new();
    for cand in candidates {
        match probe_record_input_spec(ffmpeg, &cand.spec) {
            Ok(()) => return Ok(cand.spec),
            Err(e) => failures.push(format!("{} => {}", cand.name, e)),
        }
    }
    let summary = failures
        .into_iter()
        .take(3)
        .collect::<Vec<String>>()
        .join(" | ");
    Err(format!(
        "E_RECORD_INPUT_AUTO_RESOLVE_FAILED: no probeable dshow audio input ({summary})"
    ))
}

fn take_recording_asset(recorder: &tauri::State<'_, BackendRecordingState>, asset_id: &str) -> Option<RecordedAsset> {
    let mut g = recorder.inner.lock().unwrap();
    g.assets.remove(asset_id)
}

fn resolve_asr_preprocess_config(s: &settings::Settings) -> pipeline::PreprocessConfig {
    let mut cfg = pipeline::PreprocessConfig::default();
    if let Some(v) = s.asr_preprocess_silence_trim_enabled {
        cfg.silence_trim_enabled = v;
    }
    if let Some(v) = s.asr_preprocess_silence_threshold_db {
        cfg.silence_threshold_db = v;
    }
    if let Some(v) = s.asr_preprocess_silence_start_ms {
        cfg.silence_trim_start_ms = v;
    }
    if let Some(v) = s.asr_preprocess_silence_end_ms {
        cfg.silence_trim_end_ms = v;
    }
    cfg
}

fn sanitize_rewrite_glossary(glossary: Option<Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for item in glossary.unwrap_or_default() {
        let v = item.trim();
        if !v.is_empty() {
            out.push(v.to_string());
        }
    }
    out
}

fn record_input_spec_from_settings(
    data_dir: &std::path::Path,
    ffmpeg: &str,
) -> Result<String, String> {
    let ffmpeg_path = std::path::Path::new(ffmpeg);
    let mut s = settings::load_settings_strict(data_dir).map_err(|e| e.to_string())?;
    if let Some(configured) = s
        .record_input_spec
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
    {
        let normalized = normalize_record_input_spec(&configured);
        if normalized != configured {
            s.record_input_spec = Some(normalized.clone());
            let _ = settings::save_settings(data_dir, &s);
        }
        return Ok(normalized);
    }

    let auto = auto_resolve_record_input_spec(ffmpeg_path)?;
    s.record_input_spec = Some(auto.clone());
    settings::save_settings(data_dir, &s)
        .map_err(|e| format!("E_RECORD_INPUT_AUTO_SAVE_FAILED: {e}"))?;
    Ok(auto)
}

#[tauri::command]
async fn start_task(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskManager>,
    runtime: tauri::State<'_, RuntimeState>,
    recorder: tauri::State<'_, BackendRecordingState>,
    req: StartTaskRequest,
) -> Result<String, String> {
    let dir = match data_dir::data_dir() {
        Ok(v) => v,
        Err(e) => {
            abort_recording_session_if_present(&state, &req.recording_session_id);
            return Err(e.to_string());
        }
    };
    let mut opts = match start_opts_from_settings(&dir) {
        Ok(v) => v,
        Err(e) => {
            let span = cmd_span(&dir, None, "CMD.start_task.settings", None);
            span.err("config", "E_SETTINGS_INVALID", &e, None);
            abort_recording_session_if_present(&state, &req.recording_session_id);
            return Err(e);
        }
    };
    opts.recording_session_id = req.recording_session_id.clone();

    let span = cmd_span(
        &dir,
        None,
        "CMD.start_task",
        Some(serde_json::json!({
            "trigger_source": req.trigger_source.as_str(),
            "record_mode": req.record_mode.as_str(),
            "has_recording_asset_id": req.recording_asset_id.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false),
            "fixture_name": req.fixture_name.as_deref(),
            "has_recording_session_id": opts.recording_session_id.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false),
            "rewrite_enabled": opts.rewrite_enabled,
            "template_id": opts.template_id.as_deref(),
        })),
    );
    let session_id_for_cleanup = opts.recording_session_id.clone();

    cleanup_expired_recording_assets(&recorder, std::time::Duration::from_secs(120));
    if state.has_active_task() {
        let msg = "E_TASK_ALREADY_ACTIVE: another task is already running";
        span.err("task", "E_TASK_ALREADY_ACTIVE", msg, None);
        abort_recording_session_if_present(&state, &session_id_for_cleanup);
        return Err(msg.to_string());
    }
    if has_active_recording(&recorder) {
        let msg = "E_RECORD_ALREADY_ACTIVE: recording is still active";
        span.err("task", "E_RECORD_ALREADY_ACTIVE", msg, None);
        abort_recording_session_if_present(&state, &session_id_for_cleanup);
        return Err(msg.to_string());
    }

    if let Some((code, msg)) = runtime_not_ready(&runtime) {
        span.err("config", code, &msg, None);
        abort_recording_session_if_present(&state, &session_id_for_cleanup);
        return Err(msg);
    }

    let start_res = match req.record_mode.as_str() {
        "recording_asset" => {
            let recording_asset_id = match req.recording_asset_id {
                Some(v) if !v.trim().is_empty() => v,
                _ => {
                    span.err(
                        "config",
                        "E_START_TASK_ASSET_MISSING",
                        "recording_asset_id is required when record_mode=recording_asset",
                        None,
                    );
                    abort_recording_session_if_present(&state, &session_id_for_cleanup);
                    return Err(
                        "E_START_TASK_ASSET_MISSING: recording_asset_id is required".to_string()
                    );
                }
            };
            let asset = match take_recording_asset(&recorder, recording_asset_id.trim()) {
                Some(v) => v,
                None => {
                    span.err("io", "E_RECORD_ASSET_NOT_FOUND", "recording asset not found", None);
                    abort_recording_session_if_present(&state, &session_id_for_cleanup);
                    return Err(
                        "E_RECORD_ASSET_NOT_FOUND: recording asset not found or expired".to_string()
                    );
                }
            };
            opts.record_elapsed_ms = asset.record_elapsed_ms;
            opts.record_label = "Record (backend)".to_string();
            if !asset.output_path.exists() {
                span.err("io", "E_RECORD_OUTPUT_MISSING", "recorded file missing", None);
                abort_recording_session_if_present(&state, &session_id_for_cleanup);
                return Err("E_RECORD_OUTPUT_MISSING: recorded file missing".to_string());
            }
            state.start_recording_file(app, asset.output_path, opts)
        }
        "fixture" => {
            opts.record_elapsed_ms = 0;
            opts.record_label = "Record (fixture)".to_string();
            let fixture_name = match req.fixture_name {
                Some(v) if !v.trim().is_empty() => v,
                _ => {
                    span.err(
                        "config",
                        "E_START_TASK_FIXTURE_MISSING",
                        "fixture_name is required when record_mode=fixture",
                        None,
                    );
                    abort_recording_session_if_present(&state, &session_id_for_cleanup);
                    return Err(
                        "E_START_TASK_FIXTURE_MISSING: fixture_name is required".to_string()
                    );
                }
            };
            state.start_fixture(app, fixture_name, opts)
        }
        _ => {
                span.err(
                    "config",
                    "E_START_TASK_MODE_INVALID",
                    "record_mode must be recording_asset|fixture",
                    None,
                );
                abort_recording_session_if_present(&state, &session_id_for_cleanup);
                return Err("E_START_TASK_MODE_INVALID: invalid record_mode".to_string());
            }
        };

    match start_res {
        Ok(task_id) => {
            span.ok(Some(serde_json::json!({"task_id": task_id})));
            Ok(task_id)
        }
        Err(e) => {
            span.err_anyhow("task", "E_START_TASK_FAILED", &e, None);
            abort_recording_session_if_present(&state, &session_id_for_cleanup);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn start_backend_recording(
    state: tauri::State<'_, TaskManager>,
    recorder: tauri::State<'_, BackendRecordingState>,
) -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.start_backend_recording", None);
    if !cfg!(windows) {
        let msg = "E_RECORD_UNSUPPORTED: backend recording is only supported on Windows";
        span.err("config", "E_RECORD_UNSUPPORTED", msg, None);
        return Err(msg.to_string());
    }
    if state.has_active_task() {
        let msg = "E_RECORD_TASK_ACTIVE: another task is already running";
        span.err("task", "E_RECORD_TASK_ACTIVE", msg, None);
        return Err(msg.to_string());
    }
    cleanup_expired_recording_assets(&recorder, std::time::Duration::from_secs(120));
    let mut g = recorder.inner.lock().unwrap();
    if g.active.is_some() {
        let msg = "E_RECORD_ALREADY_ACTIVE: recording is already active";
        span.err("task", "E_RECORD_ALREADY_ACTIVE", msg, None);
        return Err(msg.to_string());
    }

    let root = repo_root()?;
    let tmp = root.join("tmp").join("desktop");
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let recording_id = uuid::Uuid::new_v4().to_string();
    let output_path = tmp.join(format!("recording-{recording_id}.wav"));
    let ffmpeg = pipeline::ffmpeg_cmd().map_err(|e| e.to_string())?;
    let input_spec = match record_input_spec_from_settings(&dir, ffmpeg.as_str()) {
        Ok(v) => v,
        Err(e) => {
            span.err("config", "E_SETTINGS_INVALID", &e, None);
            return Err(e);
        }
    };

    let mut child = match std::process::Command::new(&ffmpeg)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "dshow",
            "-i",
            input_spec.as_str(),
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(output_path.as_os_str())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            let msg = format!("E_RECORD_START_FAILED: failed to start ffmpeg recorder: {e}");
            span.err("process", "E_RECORD_START_FAILED", &msg, None);
            return Err(msg);
        }
    };

    std::thread::sleep(std::time::Duration::from_millis(120));
    match child.try_wait() {
        Ok(Some(status)) => {
            let stderr_tail = child.stderr.as_mut().and_then(read_last_stderr_line);
            let mut msg = if status.success() {
                "E_RECORD_START_FAILED: recorder exited unexpectedly right after start".to_string()
            } else {
                format!("E_RECORD_START_FAILED: recorder exited right after start with {status}")
            };
            if let Some(line) = stderr_tail.as_deref() {
                msg.push_str("; stderr=");
                msg.push_str(line);
            }
            span.err("process", "E_RECORD_START_FAILED", &msg, None);
            let _ = std::fs::remove_file(&output_path);
            return Err(msg);
        }
        Ok(None) => {}
        Err(e) => {
            let msg = format!("E_RECORD_START_FAILED: failed to probe recorder process: {e}");
            span.err("process", "E_RECORD_START_FAILED", &msg, None);
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&output_path);
            return Err(msg);
        }
    }

    g.active = Some(ActiveBackendRecording {
        recording_id: recording_id.clone(),
        output_path: output_path.clone(),
        child,
        started_at: std::time::Instant::now(),
    });

    span.ok(Some(serde_json::json!({
        "recording_id": recording_id,
        "output_path": output_path,
    })));
    Ok(recording_id)
}

#[tauri::command]
fn stop_backend_recording(
    recorder: tauri::State<'_, BackendRecordingState>,
    recording_id: &str,
) -> Result<StopBackendRecordingResult, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.stop_backend_recording",
        Some(serde_json::json!({
            "has_recording_id": !recording_id.trim().is_empty(),
        })),
    );

    cleanup_expired_recording_assets(&recorder, std::time::Duration::from_secs(120));
    let mut active = {
        let mut g = recorder.inner.lock().unwrap();
        match g.active.take() {
            Some(active) => active,
            None => {
                let msg = "E_RECORD_NOT_ACTIVE: no active recording";
                span.err("task", "E_RECORD_NOT_ACTIVE", msg, None);
                return Err(msg.to_string());
            }
        }
    };

    if !recording_id.trim().is_empty() && active.recording_id != recording_id {
        let mut g = recorder.inner.lock().unwrap();
        g.active = Some(active);
        let msg = "E_RECORD_ID_MISMATCH: recording id mismatch";
        span.err("task", "E_RECORD_ID_MISMATCH", msg, None);
        return Err(msg.to_string());
    }

    if let Some(stdin) = active.child.stdin.as_mut() {
        let _ = std::io::Write::write_all(stdin, b"q\n");
        let _ = std::io::Write::flush(stdin);
    }

    let mut status = None;
    for _ in 0..100 {
        match active.child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    if status.is_none() {
        let _ = active.child.kill();
        status = active.child.wait().ok();
    }
    let status = match status {
        Some(s) => s,
        None => {
            let stderr_tail = active.child.stderr.as_mut().and_then(read_last_stderr_line);
            let mut msg = "E_RECORD_STOP_FAILED: recorder process wait failed".to_string();
            if let Some(line) = stderr_tail.as_deref() {
                msg.push_str("; stderr=");
                msg.push_str(line);
            }
            span.err("process", "E_RECORD_STOP_FAILED", &msg, None);
            return Err(msg);
        }
    };
    let stderr_tail = active.child.stderr.as_mut().and_then(read_last_stderr_line);
    if !status.success() {
        let mut msg = format!("E_RECORD_STOP_FAILED: recorder exited with {status}");
        if let Some(line) = stderr_tail.as_deref() {
            msg.push_str("; stderr=");
            msg.push_str(line);
        }
        span.err("process", "E_RECORD_STOP_FAILED", &msg, None);
        let _ = std::fs::remove_file(&active.output_path);
        return Err(msg);
    }

    if !active.output_path.exists() {
        let msg = "E_RECORD_OUTPUT_MISSING: recorded file missing";
        span.err("io", "E_RECORD_OUTPUT_MISSING", msg, None);
        return Err(msg.to_string());
    }

    let elapsed_ms = active.started_at.elapsed().as_millis();
    let asset_id = uuid::Uuid::new_v4().to_string();
    {
        let mut g = recorder.inner.lock().unwrap();
        g.assets.insert(
            asset_id.clone(),
            RecordedAsset {
                asset_id: asset_id.clone(),
                output_path: active.output_path.clone(),
                record_elapsed_ms: elapsed_ms,
                created_at: std::time::Instant::now(),
            },
        );
    }
    let result = StopBackendRecordingResult {
        recording_id: active.recording_id.clone(),
        recording_asset_id: asset_id.clone(),
        ext: "wav".to_string(),
    };
    span.ok(Some(serde_json::json!({
        "recording_id": result.recording_id.clone(),
        "recording_asset_id": result.recording_asset_id.clone(),
        "ext": result.ext.clone(),
        "record_elapsed_ms": elapsed_ms,
    })));
    Ok(result)
}

#[tauri::command]
fn abort_backend_recording(
    recorder: tauri::State<'_, BackendRecordingState>,
    recording_id: Option<String>,
) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.abort_backend_recording",
        Some(serde_json::json!({
            "has_recording_id": recording_id.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false),
        })),
    );
    let mut active = {
        let mut g = recorder.inner.lock().unwrap();
        match g.active.take() {
            Some(v) => v,
            None => {
                span.ok(Some(serde_json::json!({"aborted": false})));
                return Ok(());
            }
        }
    };
    if let Some(expected) = recording_id {
        if !expected.trim().is_empty() && active.recording_id != expected {
            let mut g = recorder.inner.lock().unwrap();
            g.active = Some(active);
            let msg = "E_RECORD_ID_MISMATCH: recording id mismatch";
            span.err("task", "E_RECORD_ID_MISMATCH", msg, None);
            return Err(msg.to_string());
        }
    }
    if let Some(stdin) = active.child.stdin.as_mut() {
        let _ = std::io::Write::write_all(stdin, b"q\n");
        let _ = std::io::Write::flush(stdin);
    }
    let _ = active.child.kill();
    let _ = active.child.wait();
    let _ = std::fs::remove_file(&active.output_path);
    span.ok(Some(serde_json::json!({"aborted": true})));
    Ok(())
}

#[tauri::command]
fn runtime_toolchain_status(
    runtime: tauri::State<'_, RuntimeState>,
) -> Result<toolchain::ToolchainStatus, String> {
    Ok(runtime.get_toolchain())
}

#[tauri::command]
fn runtime_python_status(
    runtime: tauri::State<'_, RuntimeState>,
) -> Result<python_runtime::PythonStatus, String> {
    Ok(runtime.get_python())
}

#[tauri::command]
fn cancel_task(state: tauri::State<TaskManager>, task_id: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        Some(task_id),
        "CMD.cancel_task",
        Some(serde_json::json!({"task_id": task_id})),
    );
    match state.cancel(task_id) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("task", "E_CMD_CANCEL", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn abort_recording_session(
    state: tauri::State<TaskManager>,
    recording_session_id: &str,
) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.abort_recording_session",
        Some(serde_json::json!({"has_recording_session_id": !recording_session_id.trim().is_empty()})),
    );
    if recording_session_id.trim().is_empty() {
        span.ok(Some(serde_json::json!({"removed": false})));
        return Ok(());
    }
    let removed = state.abort_recording_session(recording_session_id);
    span.ok(Some(serde_json::json!({"removed": removed})));
    Ok(())
}

#[tauri::command]
fn list_templates() -> Result<Vec<PromptTemplate>, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.list_templates", None);
    match templates::load_templates(&dir) {
        Ok(v) => {
            span.ok(Some(serde_json::json!({"count": v.len()})));
            Ok(v)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_LIST", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn upsert_template(tpl: PromptTemplate) -> Result<PromptTemplate, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let tpl_id = tpl.id.clone();
    let has_id = !tpl_id.trim().is_empty();
    let name_chars = tpl.name.len();
    let prompt_chars = tpl.system_prompt.len();
    let span = cmd_span(
        &dir,
        None,
        "CMD.upsert_template",
        Some(
            serde_json::json!({"has_id": has_id, "id": tpl_id, "name_chars": name_chars, "prompt_chars": prompt_chars}),
        ),
    );
    match templates::upsert_template(&dir, tpl) {
        Ok(v) => {
            span.ok(Some(serde_json::json!({"id": v.id})));
            Ok(v)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_UPSERT", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn delete_template(id: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.delete_template",
        Some(serde_json::json!({"id": id})),
    );
    match templates::delete_template(&dir, id) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_DELETE", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn templates_export_json() -> Result<String, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.templates_export_json", None);
    match templates::export_templates_json(&dir) {
        Ok(s) => {
            span.ok(Some(serde_json::json!({"bytes": s.len()})));
            Ok(s)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_EXPORT", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn templates_import_json(json: &str, mode: &str) -> Result<usize, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.templates_import_json",
        Some(serde_json::json!({"mode": mode, "json_chars": json.len()})),
    );
    match templates::import_templates_json(&dir, json, mode) {
        Ok(n) => {
            span.ok(Some(serde_json::json!({"count": n})));
            Ok(n)
        }
        Err(e) => {
            span.err_anyhow("templates", "E_CMD_TPL_IMPORT", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn set_llm_api_key(api_key: &str) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.set_llm_api_key",
        Some(serde_json::json!({"api_key_chars": api_key.len()})),
    );
    match llm::set_api_key(api_key) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_SET_KEY", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn clear_llm_api_key() -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.clear_llm_api_key", None);
    match llm::clear_api_key() {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("auth", "E_CMD_CLEAR_KEY", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn llm_api_key_status() -> Result<ApiKeyStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.llm_api_key_status", None);
    let st = llm::api_key_status();
    span.ok(Some(
        serde_json::json!({"configured": st.configured, "source": st.source, "reason": st.reason}),
    ));
    Ok(st)
}

fn history_db_path() -> Result<std::path::PathBuf, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("history.sqlite3"))
}

#[tauri::command]
fn history_append(item: HistoryItem) -> Result<(), String> {
    let db = history_db_path()?;
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        Some(item.task_id.as_str()),
        "CMD.history_append",
        None,
    );
    match history::append(&db, &item) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("history", "E_CMD_HISTORY_APPEND", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn history_list(limit: i64, before_ms: Option<i64>) -> Result<Vec<HistoryItem>, String> {
    let db = history_db_path()?;
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(
        &dir,
        None,
        "CMD.history_list",
        Some(serde_json::json!({"limit": limit, "before_ms": before_ms})),
    );
    match history::list(&db, limit, before_ms) {
        Ok(v) => {
            span.ok(Some(serde_json::json!({"count": v.len()})));
            Ok(v)
        }
        Err(e) => {
            span.err_anyhow("history", "E_CMD_HISTORY_LIST", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn history_clear() -> Result<(), String> {
    let db = history_db_path()?;
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.history_clear", None);
    match history::clear(&db) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("history", "E_CMD_HISTORY_CLEAR", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn get_settings() -> Result<Settings, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.get_settings", None);
    match settings::load_settings_strict(&dir) {
        Ok(s) => {
            span.ok(Some(
                serde_json::json!({"rewrite_enabled": s.rewrite_enabled, "template_id": s.rewrite_template_id}),
            ));
            Ok(s)
        }
        Err(e) => {
            span.err_anyhow("settings", "E_CMD_GET_SETTINGS", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn set_settings(s: Settings) -> Result<(), String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.set_settings", None);
    match settings::save_settings(&dir, &s) {
        Ok(()) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err_anyhow("settings", "E_CMD_SET_SETTINGS", &e, None);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
fn update_settings(
    app: tauri::AppHandle,
    state: tauri::State<TaskManager>,
    hotkeys: tauri::State<hotkeys::HotkeyManager>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let patch_summary = serde_json::json!({
        "asr_model": patch.asr_model.is_some(),
        "llm_base_url": patch.llm_base_url.is_some(),
        "llm_model": patch.llm_model.is_some(),
        "llm_reasoning_effort": patch.llm_reasoning_effort.is_some(),
        "rewrite_enabled": patch.rewrite_enabled.is_some(),
        "rewrite_template_id": patch.rewrite_template_id.is_some(),
        "rewrite_glossary": patch.rewrite_glossary.is_some(),
        "rewrite_include_glossary": patch.rewrite_include_glossary.is_some(),
        "context_include_history": patch.context_include_history.is_some(),
        "context_history_n": patch.context_history_n.is_some(),
        "context_history_window_ms": patch.context_history_window_ms.is_some(),
        "context_include_clipboard": patch.context_include_clipboard.is_some(),
        "context_include_prev_window_meta": patch.context_include_prev_window_meta.is_some(),
        "context_include_prev_window_screenshot": patch.context_include_prev_window_screenshot.is_some(),
        "llm_supports_vision": patch.llm_supports_vision.is_some(),
        "hotkeys_enabled": patch.hotkeys_enabled.is_some(),
        "hotkey_ptt": patch.hotkey_ptt.is_some(),
        "hotkey_toggle": patch.hotkey_toggle.is_some(),
        "hotkeys_show_overlay": patch.hotkeys_show_overlay.is_some(),
        "asr_preprocess_silence_trim_enabled": patch.asr_preprocess_silence_trim_enabled.is_some(),
        "asr_preprocess_silence_threshold_db": patch
            .asr_preprocess_silence_threshold_db
            .is_some(),
        "asr_preprocess_silence_start_ms": patch.asr_preprocess_silence_start_ms.is_some(),
        "asr_preprocess_silence_end_ms": patch.asr_preprocess_silence_end_ms.is_some(),
    });
    let span = cmd_span(&dir, None, "CMD.update_settings", Some(patch_summary));
    let cur = match settings::load_settings_strict(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("settings", "E_CMD_UPDATE_SETTINGS_LOAD", &e, None);
            return Err(e.to_string());
        }
    };
    let asr_model_changed = patch.asr_model.is_some();
    let next = settings::apply_patch(cur, patch);
    if let Err(e) = settings::save_settings(&dir, &next) {
        span.err_anyhow("settings", "E_CMD_UPDATE_SETTINGS", &e, None);
        return Err(e.to_string());
    }
    // If ASR model changed, restart the resident ASR runner.
    // We do this best-effort; errors are surfaced later via task events.
    if asr_model_changed {
        state.restart_asr_best_effort("settings_changed");
    }

    // Hotkeys are also best-effort; failures are traced and should not break settings.
    hotkeys.apply_from_settings_best_effort(&app, &dir, &next);

    span.ok(None);
    Ok(next)
}

#[tauri::command]
fn asr_model_status() -> Result<ModelStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.asr_model_status", None);
    let model_id = match pipeline::resolve_asr_model_id(&dir) {
        Ok(v) => v,
        Err(e) => {
            span.err_anyhow("model", "E_CMD_MODEL_ID", &e, None);
            return Err(e.to_string());
        }
    };

    let st = if std::path::Path::new(&model_id).exists() {
        match model::verify_model_dir(std::path::Path::new(&model_id)) {
            Ok(st) => st,
            Err(e) => {
                span.err_anyhow("model", "E_CMD_MODEL_STATUS", &e, None);
                return Err(e.to_string());
            }
        }
    } else {
        ModelStatus {
            model_dir: model_id,
            ok: true,
            reason: Some("remote_model_not_locally_verified".to_string()),
            model_version: None,
        }
    };
    let _ok = st.ok;
    span.ok(Some(
        serde_json::json!({"ok": st.ok, "reason": st.reason, "model_version": st.model_version}),
    ));
    Ok(st)
}

#[tauri::command]
async fn download_asr_model() -> Result<ModelStatus, String> {
    let dir = data_dir::data_dir().map_err(|e| e.to_string())?;
    let span = cmd_span(&dir, None, "CMD.download_asr_model", None);
    let root = repo_root()?;
    let model_dir = model::default_model_dir(&root);
    let py = match python_runtime::resolve_python_binary(&root) {
        Ok(p) => p,
        Err(e) => {
            span.err_anyhow("config", "E_PYTHON_NOT_READY", &e, None);
            return Err(e.to_string());
        }
    };
    let root2 = root.clone();
    let py2 = py.clone();
    let model_dir2 = model_dir.clone();
    let st_res = tauri::async_runtime::spawn_blocking(move || {
        model::download_model(&root2, &py2, &model_dir2)
    })
    .await;
    let st = match st_res {
        Ok(Ok(st)) => st,
        Ok(Err(e)) => {
            span.err_anyhow("model", "E_CMD_MODEL_DOWNLOAD", &e, None);
            return Err(e.to_string());
        }
        Err(e) => {
            let ae = anyhow::anyhow!("spawn_blocking failed: {e}");
            span.err_anyhow("runtime", "E_CMD_JOIN", &ae, None);
            return Err(ae.to_string());
        }
    };
    // Set settings.asr_model to local dir if ok.
    if st.ok {
        let mut s = match settings::load_settings_strict(&dir) {
            Ok(v) => v,
            Err(e) => {
                span.err_anyhow("settings", "E_CMD_MODEL_DOWNLOAD_SETTINGS", &e, None);
                return Err(e.to_string());
            }
        };
        s.asr_model = Some(model_dir.display().to_string());
        if let Err(e) = settings::save_settings(&dir, &s) {
            span.err_anyhow("settings", "E_CMD_MODEL_DOWNLOAD_SAVE", &e, None);
            return Err(e.to_string());
        }
    }
    span.ok(Some(
        serde_json::json!({"ok": st.ok, "reason": st.reason, "model_version": st.model_version}),
    ));
    Ok(st)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    startup_trace::mark_best_effort("run_enter");
    panic_log::install_best_effort();
    startup_trace::mark_best_effort("panic_hook_installed");
    let ctx = tauri::generate_context!();
    startup_trace::mark_best_effort("context_generated");
    tauri::Builder::default()
        .manage(TaskManager::new())
        .manage(RuntimeState::new())
        .manage(BackendRecordingState::new())
        .manage(hotkeys::HotkeyManager::new())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            #[derive(Clone, serde::Serialize)]
            struct Payload {
                args: Vec<String>,
                cwd: String,
            }

            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            let _ = app.emit("tv_single_instance", Payload { args: argv, cwd });

            if let Ok(dir) = data_dir::data_dir() {
                trace::event(
                    &dir,
                    None,
                    "App",
                    "APP.single_instance",
                    "ok",
                    Some(serde_json::json!({"note": "second_instance_redirected"})),
                );
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            startup_trace::mark_best_effort("setup_enter");

            // Small always-on-top overlay window for hotkey-driven UX.
            // Keep it hidden by default; the frontend will invoke overlay_set_state to show/hide.
            let _overlay = tauri::WebviewWindowBuilder::new(
                app,
                "overlay",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("TypeVoice Overlay")
            .inner_size(240.0, 64.0)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .visible(false)
            .skip_taskbar(true)
            .focused(false)
            .build();

            let mut toolchain_ready = false;
            let mut python_ready = false;
            if let Ok(dir) = data_dir::data_dir() {
                let runtime = app.state::<RuntimeState>();
                let st = toolchain::initialize_and_verify(&app.handle(), &dir);
                toolchain_ready = st.ready;
                runtime.set_toolchain(st);
                if let Ok(root) = repo_root() {
                    let py = python_runtime::initialize_and_verify(&dir, &root);
                    python_ready = py.ready;
                    runtime.set_python(py);
                } else {
                    let py = python_runtime::PythonStatus {
                        ready: false,
                        code: Some("E_PYTHON_NOT_READY".to_string()),
                        message: Some("E_PYTHON_NOT_READY: repo root not found".to_string()),
                        python_path: None,
                        python_version: None,
                    };
                    runtime.set_python(py);
                }
            }

            // Warm up the ASR runner in background so first transcription is fast.
            // If runtime preflight failed, skip warmup to avoid noisy startup failures.
            if toolchain_ready && python_ready {
                let state = app.state::<TaskManager>();
                state.warmup_asr_best_effort();
                state.warmup_context_best_effort();
            }

            // Apply hotkeys from persisted settings.
            if let Ok(dir) = data_dir::data_dir() {
                match settings::load_settings_strict(&dir) {
                    Ok(s) => {
                        let hk = app.state::<hotkeys::HotkeyManager>();
                        hk.apply_from_settings_best_effort(&app.handle(), &dir, &s);
                    }
                    Err(e) => {
                        trace::event(
                            &dir,
                            None,
                            "App",
                            "APP.hotkeys_init",
                            "err",
                            Some(serde_json::json!({
                                "code": "E_SETTINGS_INVALID",
                                "error": e.to_string()
                            })),
                        );
                    }
                }
            }

            startup_trace::mark_best_effort("setup_exit");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_task,
            start_backend_recording,
            stop_backend_recording,
            abort_backend_recording,
            cancel_task,
            abort_recording_session,
            list_templates,
            upsert_template,
            delete_template,
            templates_export_json,
            templates_import_json,
            set_llm_api_key,
            clear_llm_api_key,
            llm_api_key_status,
            history_append,
            history_list,
            history_clear,
            get_settings,
            set_settings,
            update_settings,
            hotkeys::check_hotkey_available,
            runtime_toolchain_status,
            runtime_python_status,
            overlay_set_state,
            asr_model_status,
            download_asr_model
        ])
        .run(ctx)
        .expect("error while running tauri application");
}
