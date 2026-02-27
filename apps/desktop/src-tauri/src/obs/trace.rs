use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Error as AnyhowError;
use serde_json::Value;

use super::schema::{now_ms, TraceError, TraceEvent};
use super::writer;

const DEFAULT_BACKTRACE_MAX_CHARS: usize = 12_000;

fn env_bool_default_true(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !(t == "0" || t == "false" || t == "no" || t == "off")
        }
        Err(_) => true,
    }
}

pub fn enabled() -> bool {
    env_bool_default_true("TYPEVOICE_TRACE_ENABLED")
}

fn backtrace_enabled() -> bool {
    env_bool_default_true("TYPEVOICE_TRACE_BACKTRACE")
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn trace_path(data_dir: &Path) -> PathBuf {
    data_dir.join("trace.jsonl")
}

fn clamp_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(std::cmp::min(s.len(), max_chars));
    for (idx, ch) in s.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        if ch == '\0' {
            continue;
        }
        out.push(ch);
    }
    out
}

pub(crate) fn redact_user_paths(s: &str) -> String {
    fn scrub_after(hay: &str, marker: &str, sep: char) -> String {
        let mut out = String::with_capacity(hay.len());
        let mut idx = 0usize;
        while let Some(pos) = hay[idx..].find(marker) {
            let abs = idx + pos;
            out.push_str(&hay[idx..abs]);
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
            idx = name_end;
        }
        out.push_str(&hay[idx..]);
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
        Some(Value::Object(map)) => {
            let mut out = base;
            for (k, v) in map {
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
    let mut map = serde_json::Map::new();
    map.insert(
        "err_chain".to_string(),
        serde_json::json!(anyhow_chain(err)),
    );
    if let Some(bt) = maybe_backtrace_string() {
        map.insert("backtrace".to_string(), serde_json::json!(bt));
    }
    merge_ctx(map, extra)
}

fn ctx_with_backtrace(extra: Option<Value>) -> Option<Value> {
    if !backtrace_enabled() {
        return extra;
    }
    let mut map = serde_json::Map::new();
    if let Some(bt) = maybe_backtrace_string() {
        map.insert("backtrace".to_string(), serde_json::json!(bt));
    }
    Some(merge_ctx(map, extra))
}

fn emit_event(data_dir: &Path, ev: &TraceEvent) {
    if !enabled() {
        return;
    }
    if let Err(e) = writer::emit_trace_event(data_dir, ev) {
        crate::safe_eprintln!("trace: emit failed: {e:#}");
    }
}

pub fn event(
    data_dir: &Path,
    task_id: Option<&str>,
    stage: &str,
    step_id: &str,
    status: &str,
    ctx: Option<Value>,
) {
    emit_event(
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
        emit_event(
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
        emit_event(
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
        emit_event(
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
        emit_event(
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
        emit_event(
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
        emit_event(
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
                ctx: ctx_with_backtrace(None),
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
        for idx in 0..threads {
            let dir2 = dir.clone();
            joins.push(thread::spawn(move || {
                for j in 0..per_thread {
                    event(
                        &dir2,
                        Some("task-concurrent"),
                        "TraceTest",
                        "TRACE.concurrent_emit",
                        "ok",
                        Some(serde_json::json!({"i": idx, "j": j})),
                    );
                }
            }));
        }
        for j in joins {
            j.join().expect("join");
        }

        assert!(writer::flush(2_000), "trace writer flush timeout");
        let raw = fs::read_to_string(trace_path(&dir)).expect("read trace");
        assert!(!raw.is_empty());

        let mut lines = 0usize;
        for line in raw.lines() {
            lines += 1;
            let v: serde_json::Value = serde_json::from_str(line).expect("valid json line");
            assert!(v.get("step_id").is_some());
            assert!(v.get("status").is_some());
        }
        assert!(lines > 0, "trace should contain at least one line");
        assert!(
            lines <= threads * per_thread,
            "trace lines should not exceed emitted count"
        );
    }
}
