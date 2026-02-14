use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Error as AnyhowError;
use serde::Serialize;
use serde_json::Value;

const DEFAULT_TRACE_MAX_BYTES: u64 = 10_000_000; // 10MB
const DEFAULT_TRACE_MAX_FILES: usize = 5;
const DEFAULT_BACKTRACE_MAX_CHARS: usize = 12_000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn env_bool_default_true(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !(t == "0" || t == "false" || t == "no" || t == "off")
        }
        Err(_) => true,
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    match std::env::var(key) {
        Ok(v) => v.trim().parse::<u64>().unwrap_or(default),
        Err(_) => default,
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    match std::env::var(key) {
        Ok(v) => v.trim().parse::<usize>().unwrap_or(default),
        Err(_) => default,
    }
}

pub fn enabled() -> bool {
    // Default: enabled. Users can set TYPEVOICE_TRACE_ENABLED=0 to disable.
    env_bool_default_true("TYPEVOICE_TRACE_ENABLED")
}

fn backtrace_enabled() -> bool {
    // Default: enabled. Users can set TYPEVOICE_TRACE_BACKTRACE=0 to disable.
    env_bool_default_true("TYPEVOICE_TRACE_BACKTRACE")
}

fn max_bytes() -> u64 {
    env_u64("TYPEVOICE_TRACE_MAX_BYTES", DEFAULT_TRACE_MAX_BYTES)
}

fn max_files() -> usize {
    env_usize("TYPEVOICE_TRACE_MAX_FILES", DEFAULT_TRACE_MAX_FILES)
}

pub fn trace_path(data_dir: &Path) -> PathBuf {
    data_dir.join("trace.jsonl")
}

fn rotate_if_needed_best_effort(data_dir: &Path) {
    if !enabled() {
        return;
    }

    let p = trace_path(data_dir);
    let max_b = max_bytes();
    let max_f = max_files();
    if max_f == 0 {
        return;
    }

    let len = match std::fs::metadata(&p) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if len <= max_b {
        return;
    }

    // Remove the oldest first so Windows renames won't fail due to existing dest files.
    let oldest = data_dir.join(format!("trace.jsonl.{max_f}"));
    if oldest.exists() {
        let _ = std::fs::remove_file(&oldest);
    }

    // Shift: trace.jsonl.(n-1) -> trace.jsonl.n, then trace.jsonl -> trace.jsonl.1
    for i in (1..max_f).rev() {
        let src = data_dir.join(format!("trace.jsonl.{i}"));
        let dst = data_dir.join(format!("trace.jsonl.{}", i + 1));
        if src.exists() {
            let _ = std::fs::rename(&src, &dst);
        }
    }
    let first = data_dir.join("trace.jsonl.1");
    let _ = std::fs::rename(&p, &first);
}

fn trace_write_lock() -> &'static Mutex<()> {
    static TRACE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    TRACE_WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

pub fn emit_best_effort(data_dir: &Path, ev: &TraceEvent) {
    if !enabled() {
        return;
    }
    let _guard = trace_write_lock().lock().unwrap();
    let _ = std::fs::create_dir_all(data_dir);
    rotate_if_needed_best_effort(data_dir);

    let p = trace_path(data_dir);
    let mut f = match OpenOptions::new().create(true).append(true).open(&p) {
        Ok(f) => f,
        Err(e) => {
            crate::safe_eprintln!("trace: open failed: {}: {e}", p.display());
            return;
        }
    };
    let mut line = match serde_json::to_string(ev) {
        Ok(s) => s,
        Err(e) => {
            crate::safe_eprintln!("trace: serialize failed: {e}");
            return;
        }
    };
    line.push('\n');
    if let Err(e) = f.write_all(line.as_bytes()) {
        crate::safe_eprintln!("trace: write failed: {e}");
        return;
    }
}

fn clamp_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(std::cmp::min(s.len(), max_chars));
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        if ch == '\0' {
            continue;
        }
        out.push(ch);
    }
    out
}

fn redact_user_paths(s: &str) -> String {
    // Goal: avoid leaking personal absolute paths in trace logs while keeping backtraces usable.
    // We do NOT try to perfectly sanitize everything; we just scrub common "home dir" patterns.
    fn scrub_after(hay: &str, marker: &str, sep: char) -> String {
        let mut out = String::with_capacity(hay.len());
        let mut i = 0;
        while let Some(pos) = hay[i..].find(marker) {
            let abs = i + pos;
            out.push_str(&hay[i..abs]);
            out.push_str(marker);
            let name_start = abs + marker.len();
            let rest = &hay[name_start..];
            let mut name_end = name_start;
            for ch in rest.chars() {
                if ch == sep {
                    break;
                }
                name_end += ch.len_utf8();
            }
            out.push_str("<redacted>");
            i = name_end;
        }
        out.push_str(&hay[i..]);
        out
    }

    let mut t = s.to_string();
    t = scrub_after(&t, "\\Users\\", '\\');
    t = scrub_after(&t, "/Users/", '/');
    t = scrub_after(&t, "/home/", '/');
    t
}

fn anyhow_chain(err: &AnyhowError) -> Vec<String> {
    err.chain().map(|e| e.to_string()).collect()
}

fn maybe_backtrace_string() -> Option<String> {
    if !backtrace_enabled() {
        return None;
    }
    let bt = std::backtrace::Backtrace::force_capture();
    let s = format!("{bt:?}");
    Some(clamp_chars(
        &redact_user_paths(&s),
        DEFAULT_BACKTRACE_MAX_CHARS,
    ))
}

fn merge_ctx(base: serde_json::Map<String, Value>, extra: Option<Value>) -> Value {
    match extra {
        None => Value::Object(base),
        Some(Value::Object(m)) => {
            let mut out = base;
            for (k, v) in m.into_iter() {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Some(v) => {
            let mut out = base;
            out.insert("extra".to_string(), v);
            Value::Object(out)
        }
    }
}

fn ctx_for_anyhow_error(err: &AnyhowError, extra: Option<Value>) -> Value {
    let mut m = serde_json::Map::new();
    m.insert(
        "err_chain".to_string(),
        serde_json::json!(anyhow_chain(err)),
    );
    if let Some(bt) = maybe_backtrace_string() {
        m.insert("backtrace".to_string(), serde_json::json!(bt));
    }
    merge_ctx(m, extra)
}

fn ctx_with_backtrace(extra: Option<Value>) -> Option<Value> {
    if !backtrace_enabled() {
        return extra;
    }
    let mut m = serde_json::Map::new();
    if let Some(bt) = maybe_backtrace_string() {
        m.insert("backtrace".to_string(), serde_json::json!(bt));
    }
    Some(merge_ctx(m, extra))
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceError {
    pub kind: String,    // winapi|http|io|process|logic|parse|unknown
    pub code: String,    // E_* | HTTP_401 | WIN_LAST_ERROR_...
    pub message: String, // short
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceEvent {
    pub ts_ms: i64,
    pub task_id: Option<String>,
    pub stage: String,
    pub step_id: String,
    pub op: String,     // start|end|event
    pub status: String, // ok|err|skipped|aborted
    pub duration_ms: Option<u128>,
    pub error: Option<TraceError>,
    pub ctx: Option<Value>,
}

pub fn event(
    data_dir: &Path,
    task_id: Option<&str>,
    stage: &str,
    step_id: &str,
    status: &str,
    ctx: Option<Value>,
) {
    emit_best_effort(
        data_dir,
        &TraceEvent {
            ts_ms: now_ms(),
            task_id: task_id.map(|s| s.to_string()),
            stage: stage.to_string(),
            step_id: step_id.to_string(),
            op: "event".to_string(),
            status: status.to_string(),
            duration_ms: None,
            error: None,
            ctx,
        },
    );
}

pub struct Span {
    data_dir: PathBuf,
    task_id: Option<String>,
    stage: String,
    step_id: String,
    t0: Instant,
    finished: bool,
}

impl Span {
    pub fn start(
        data_dir: &Path,
        task_id: Option<&str>,
        stage: &str,
        step_id: &str,
        ctx: Option<Value>,
    ) -> Self {
        emit_best_effort(
            data_dir,
            &TraceEvent {
                ts_ms: now_ms(),
                task_id: task_id.map(|s| s.to_string()),
                stage: stage.to_string(),
                step_id: step_id.to_string(),
                op: "start".to_string(),
                status: "ok".to_string(),
                duration_ms: None,
                error: None,
                ctx,
            },
        );
        Self {
            data_dir: data_dir.to_path_buf(),
            task_id: task_id.map(|s| s.to_string()),
            stage: stage.to_string(),
            step_id: step_id.to_string(),
            t0: Instant::now(),
            finished: false,
        }
    }

    pub fn ok(mut self, ctx: Option<Value>) {
        self.finished = true;
        emit_best_effort(
            &self.data_dir,
            &TraceEvent {
                ts_ms: now_ms(),
                task_id: self.task_id.clone(),
                stage: self.stage.clone(),
                step_id: self.step_id.clone(),
                op: "end".to_string(),
                status: "ok".to_string(),
                duration_ms: Some(self.t0.elapsed().as_millis()),
                error: None,
                ctx,
            },
        );
    }

    #[allow(dead_code)]
    pub fn skipped(mut self, reason: &str, ctx: Option<Value>) {
        self.finished = true;
        emit_best_effort(
            &self.data_dir,
            &TraceEvent {
                ts_ms: now_ms(),
                task_id: self.task_id.clone(),
                stage: self.stage.clone(),
                step_id: self.step_id.clone(),
                op: "end".to_string(),
                status: "skipped".to_string(),
                duration_ms: Some(self.t0.elapsed().as_millis()),
                error: Some(TraceError {
                    kind: "logic".to_string(),
                    code: "SKIPPED".to_string(),
                    message: reason.to_string(),
                }),
                ctx,
            },
        );
    }

    pub fn err(mut self, kind: &str, code: &str, message: &str, ctx: Option<Value>) {
        self.finished = true;
        emit_best_effort(
            &self.data_dir,
            &TraceEvent {
                ts_ms: now_ms(),
                task_id: self.task_id.clone(),
                stage: self.stage.clone(),
                step_id: self.step_id.clone(),
                op: "end".to_string(),
                status: "err".to_string(),
                duration_ms: Some(self.t0.elapsed().as_millis()),
                error: Some(TraceError {
                    kind: kind.to_string(),
                    code: code.to_string(),
                    message: message.to_string(),
                }),
                ctx: ctx_with_backtrace(ctx),
            },
        );
    }

    pub fn err_anyhow(mut self, kind: &str, code: &str, err: &AnyhowError, ctx: Option<Value>) {
        self.finished = true;
        emit_best_effort(
            &self.data_dir,
            &TraceEvent {
                ts_ms: now_ms(),
                task_id: self.task_id.clone(),
                stage: self.stage.clone(),
                step_id: self.step_id.clone(),
                op: "end".to_string(),
                status: "err".to_string(),
                duration_ms: Some(self.t0.elapsed().as_millis()),
                error: Some(TraceError {
                    kind: kind.to_string(),
                    code: code.to_string(),
                    message: err.to_string(),
                }),
                ctx: Some(ctx_for_anyhow_error(err, ctx)),
            },
        );
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let ctx = ctx_with_backtrace(None);
        emit_best_effort(
            &self.data_dir,
            &TraceEvent {
                ts_ms: now_ms(),
                task_id: self.task_id.clone(),
                stage: self.stage.clone(),
                step_id: self.step_id.clone(),
                op: "end".to_string(),
                status: "aborted".to_string(),
                duration_ms: Some(self.t0.elapsed().as_millis()),
                error: Some(TraceError {
                    kind: "logic".to_string(),
                    code: "ABORTED".to_string(),
                    message: "span dropped without explicit ok/err".to_string(),
                }),
                ctx,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, thread};

    #[test]
    fn concurrent_emit_keeps_jsonl_lines_parseable() {
        let td = tempfile::tempdir().expect("tempdir");
        let dir = td.path().to_path_buf();
        let threads = 8;
        let per_thread = 120;

        let mut joins = Vec::new();
        for i in 0..threads {
            let dir2 = dir.clone();
            joins.push(thread::spawn(move || {
                for j in 0..per_thread {
                    event(
                        &dir2,
                        Some("task-concurrent"),
                        "TraceTest",
                        "TRACE.concurrent_emit",
                        "ok",
                        Some(serde_json::json!({"i": i, "j": j})),
                    );
                }
            }));
        }

        for j in joins {
            j.join().expect("join");
        }

        let raw = fs::read_to_string(trace_path(&dir)).expect("read trace");
        assert!(!raw.is_empty());

        let mut lines = 0usize;
        for line in raw.lines() {
            lines += 1;
            let v: serde_json::Value = serde_json::from_str(line).expect("valid json line");
            assert!(v.get("step_id").is_some());
            assert!(v.get("status").is_some());
        }
        assert_eq!(lines, threads * per_thread);
    }
}
