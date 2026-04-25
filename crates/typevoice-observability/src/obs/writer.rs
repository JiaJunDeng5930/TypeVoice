use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Mutex, OnceLock,
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};

use super::schema::{now_ms, MetricsRecord, TraceEvent};

const DEFAULT_QUEUE_CAPACITY: usize = 8192;
const DEFAULT_TRACE_MAX_BYTES: u64 = 10_000_000;
const DEFAULT_TRACE_MAX_FILES: usize = 5;
const DEFAULT_METRICS_MAX_BYTES: u64 = 10_000_000;
const DEFAULT_METRICS_MAX_FILES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StreamKind {
    Trace,
    Metrics,
}

impl StreamKind {
    fn file_name(&self) -> &'static str {
        match self {
            Self::Trace => "trace.jsonl",
            Self::Metrics => "metrics.jsonl",
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Metrics => "metrics",
        }
    }
}

#[derive(Debug, Clone)]
struct RecordMsg {
    data_dir: PathBuf,
    stream: StreamKind,
    line: String,
}

#[cfg_attr(not(test), allow(dead_code))]
enum Msg {
    Record(RecordMsg),
    Flush(mpsc::Sender<()>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DropKey {
    data_dir: PathBuf,
    stream: StreamKind,
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

fn queue_capacity() -> usize {
    let v = env_usize("TYPEVOICE_OBS_QUEUE_CAPACITY", DEFAULT_QUEUE_CAPACITY);
    std::cmp::max(1, v)
}

fn trace_max_bytes() -> u64 {
    env_u64("TYPEVOICE_TRACE_MAX_BYTES", DEFAULT_TRACE_MAX_BYTES)
}

fn trace_max_files() -> usize {
    env_usize("TYPEVOICE_TRACE_MAX_FILES", DEFAULT_TRACE_MAX_FILES)
}

fn metrics_max_bytes() -> u64 {
    env_u64("TYPEVOICE_METRICS_MAX_BYTES", DEFAULT_METRICS_MAX_BYTES)
}

fn metrics_max_files() -> usize {
    env_usize("TYPEVOICE_METRICS_MAX_FILES", DEFAULT_METRICS_MAX_FILES)
}

fn writer_tx() -> &'static SyncSender<Msg> {
    static TX: OnceLock<SyncSender<Msg>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel(queue_capacity());
        std::thread::Builder::new()
            .name("typevoice-obs-writer".to_string())
            .spawn(move || writer_loop(rx))
            .expect("failed to start obs writer thread");
        tx
    })
}

fn dropped_counts() -> &'static Mutex<HashMap<DropKey, u64>> {
    static DROPPED: OnceLock<Mutex<HashMap<DropKey, u64>>> = OnceLock::new();
    DROPPED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn note_dropped(data_dir: &Path, stream: StreamKind) {
    let mut g = dropped_counts().lock().unwrap();
    let key = DropKey {
        data_dir: data_dir.to_path_buf(),
        stream,
    };
    *g.entry(key).or_insert(0) += 1;
}

fn take_dropped_counts() -> HashMap<DropKey, u64> {
    let mut g = dropped_counts().lock().unwrap();
    std::mem::take(&mut *g)
}

fn rotation_for(stream: StreamKind) -> (u64, usize) {
    match stream {
        StreamKind::Trace => (trace_max_bytes(), trace_max_files()),
        StreamKind::Metrics => (metrics_max_bytes(), metrics_max_files()),
    }
}

fn rotate_if_needed_best_effort(
    data_dir: &Path,
    file_name: &str,
    max_bytes: u64,
    max_files: usize,
) {
    if max_files == 0 {
        return;
    }
    let p = data_dir.join(file_name);
    let len = match std::fs::metadata(&p) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if len <= max_bytes {
        return;
    }

    let oldest = data_dir.join(format!("{file_name}.{max_files}"));
    if oldest.exists() {
        let _ = std::fs::remove_file(&oldest);
    }
    for idx in (1..max_files).rev() {
        let src = data_dir.join(format!("{file_name}.{idx}"));
        let dst = data_dir.join(format!("{file_name}.{}", idx + 1));
        if src.exists() {
            let _ = std::fs::rename(&src, &dst);
        }
    }
    let first = data_dir.join(format!("{file_name}.1"));
    let _ = std::fs::rename(&p, &first);
}

fn append_line(data_dir: &Path, stream: StreamKind, line: &str) -> Result<()> {
    std::fs::create_dir_all(data_dir).context("create data dir failed")?;
    let (max_bytes, max_files) = rotation_for(stream);
    rotate_if_needed_best_effort(data_dir, stream.file_name(), max_bytes, max_files);
    let path = data_dir.join(stream.file_name());
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {} failed: {}", stream.file_name(), path.display()))?;
    f.write_all(line.as_bytes())
        .with_context(|| format!("write {} line failed", stream.file_name()))?;
    f.write_all(b"\n")
        .with_context(|| format!("write {} newline failed", stream.file_name()))?;
    Ok(())
}

fn emit_logger_dropped_direct(data_dir: &Path, stream: StreamKind, count: u64) {
    let record = MetricsRecord::LoggerDropped {
        ts_ms: now_ms(),
        stream: stream.as_str().to_string(),
        count,
        queue_capacity: queue_capacity(),
    };
    let line = match serde_json::to_string(&record) {
        Ok(v) => v,
        Err(e) => {
            crate::safe_eprintln!("obs writer: serialize logger_dropped failed: {e}");
            return;
        }
    };
    if let Err(e) = append_line(data_dir, StreamKind::Metrics, &line) {
        crate::safe_eprintln!("obs writer: write logger_dropped failed: {e:#}");
    }
}

fn flush_dropped_counts() {
    let counts = take_dropped_counts();
    for (key, count) in counts {
        emit_logger_dropped_direct(&key.data_dir, key.stream, count);
    }
}

fn writer_loop(rx: Receiver<Msg>) {
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Msg::Record(msg)) => {
                if let Err(e) = append_line(&msg.data_dir, msg.stream, &msg.line) {
                    crate::safe_eprintln!("obs writer: append failed: {e:#}");
                }
                flush_dropped_counts();
            }
            Ok(Msg::Flush(ack)) => {
                flush_dropped_counts();
                let _ = ack.send(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                flush_dropped_counts();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                flush_dropped_counts();
                break;
            }
        }
    }
}

fn emit_record_line(data_dir: &Path, stream: StreamKind, line: String) -> Result<()> {
    let tx = writer_tx();
    let msg = Msg::Record(RecordMsg {
        data_dir: data_dir.to_path_buf(),
        stream,
        line,
    });
    match tx.try_send(msg) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            note_dropped(data_dir, stream);
            Err(anyhow!("obs writer queue is full"))
        }
        Err(TrySendError::Disconnected(_)) => Err(anyhow!("obs writer is disconnected")),
    }
}

pub fn emit_trace_event(data_dir: &Path, ev: &TraceEvent) -> Result<()> {
    let line = serde_json::to_string(ev).context("serialize trace event failed")?;
    emit_record_line(data_dir, StreamKind::Trace, line)
}

pub fn emit_metrics_record(data_dir: &Path, rec: &MetricsRecord) -> Result<()> {
    let line = serde_json::to_string(rec).context("serialize metrics record failed")?;
    emit_record_line(data_dir, StreamKind::Metrics, line)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn flush(timeout_ms: u64) -> bool {
    let tx = writer_tx();
    let (ack_tx, ack_rx) = mpsc::channel();
    if tx.send(Msg::Flush(ack_tx)).is_err() {
        return false;
    }
    ack_rx
        .recv_timeout(Duration::from_millis(timeout_ms))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::schema::{now_ms, MetricsRecord, TraceEvent};
    use std::{
        fs,
        sync::{Mutex, OnceLock},
        thread,
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn concurrent_metrics_emit_keeps_jsonl_lines_parseable() {
        let td = tempfile::tempdir().expect("tempdir");
        let data_dir = td.path().to_path_buf();
        let threads = 8;
        let per_thread = 100;

        let mut joins = Vec::new();
        for idx in 0..threads {
            let data_dir2 = data_dir.clone();
            joins.push(thread::spawn(move || {
                for j in 0..per_thread {
                    let rec = MetricsRecord::TaskEvent {
                        ts_ms: now_ms(),
                        task_id: "task-metrics-concurrent".to_string(),
                        stage: "TraceTest".to_string(),
                        status: "ok".to_string(),
                        elapsed_ms: Some(1),
                        error_code: None,
                        message: format!("i={idx} j={j}"),
                    };
                    let _ = emit_metrics_record(&data_dir2, &rec);
                }
            }));
        }
        for j in joins {
            j.join().expect("join");
        }
        assert!(flush(2_000), "metrics writer flush timeout");

        let raw = fs::read_to_string(data_dir.join("metrics.jsonl")).expect("read metrics");
        let mut lines = 0usize;
        for line in raw.lines() {
            lines += 1;
            let v: serde_json::Value = serde_json::from_str(line).expect("valid json line");
            assert!(v.get("type").is_some());
            assert!(v.get("ts_ms").is_some());
        }
        assert!(lines > 0, "metrics should contain at least one line");
        assert!(
            lines <= threads * per_thread,
            "metrics lines should not exceed emitted count"
        );
    }

    #[test]
    fn trace_rotation_creates_suffix_file() {
        let _env_guard = env_lock().lock().unwrap();
        std::env::set_var("TYPEVOICE_TRACE_MAX_BYTES", "1800");
        std::env::set_var("TYPEVOICE_TRACE_MAX_FILES", "2");

        let td = tempfile::tempdir().expect("tempdir");
        let data_dir = td.path().to_path_buf();
        for idx in 0..160 {
            let ev = TraceEvent {
                ts_ms: now_ms(),
                task_id: Some("task-rotate".to_string()),
                stage: "TraceTest".to_string(),
                step_id: "TRACE.rotate".to_string(),
                op: "event".to_string(),
                status: "ok".to_string(),
                duration_ms: None,
                error: None,
                ctx: Some(serde_json::json!({
                    "i": idx,
                    "payload": "0123456789abcdef0123456789abcdef0123456789abcdef",
                })),
            };
            let _ = emit_trace_event(&data_dir, &ev);
        }
        assert!(flush(2_000), "trace writer flush timeout");

        let current = data_dir.join("trace.jsonl");
        let rotated = data_dir.join("trace.jsonl.1");
        assert!(current.exists(), "trace.jsonl should exist");
        assert!(
            rotated.exists(),
            "trace.jsonl.1 should exist after rotation"
        );

        for path in [current, rotated] {
            let raw = fs::read_to_string(path).expect("read trace file");
            for line in raw.lines() {
                let _: serde_json::Value = serde_json::from_str(line).expect("valid json line");
            }
        }

        std::env::remove_var("TYPEVOICE_TRACE_MAX_BYTES");
        std::env::remove_var("TYPEVOICE_TRACE_MAX_FILES");
    }
}
