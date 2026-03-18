use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::UNIX_EPOCH,
};

use sha2::{Digest, Sha256};

use super::{metrics, schema};
use crate::obs::schema::MetricsRecord;

const DEFAULT_MAX_PAYLOAD_BYTES: usize = 2_000_000; // 2MB
const DEFAULT_MAX_TASKS: usize = 50;

fn write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn env_bool(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            t == "1" || t == "true" || t == "yes" || t == "on"
        }
        Err(_) => false,
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    match std::env::var(key) {
        Ok(v) => v.trim().parse::<usize>().unwrap_or(default),
        Err(_) => default,
    }
}

pub fn verbose_enabled() -> bool {
    env_bool("TYPEVOICE_DEBUG_VERBOSE")
}

pub fn include_llm() -> bool {
    match std::env::var("TYPEVOICE_DEBUG_INCLUDE_LLM") {
        Ok(_) => env_bool("TYPEVOICE_DEBUG_INCLUDE_LLM"),
        Err(_) => true,
    }
}

pub fn include_asr_segments() -> bool {
    match std::env::var("TYPEVOICE_DEBUG_INCLUDE_ASR_SEGMENTS") {
        Ok(_) => env_bool("TYPEVOICE_DEBUG_INCLUDE_ASR_SEGMENTS"),
        Err(_) => true,
    }
}

pub fn max_payload_bytes() -> usize {
    env_usize(
        "TYPEVOICE_DEBUG_MAX_PAYLOAD_BYTES",
        DEFAULT_MAX_PAYLOAD_BYTES,
    )
}

pub fn max_tasks() -> usize {
    env_usize("TYPEVOICE_DEBUG_MAX_TASKS", DEFAULT_MAX_TASKS)
}

pub fn debug_root(data_dir: &Path) -> PathBuf {
    data_dir.join("debug")
}

pub fn debug_task_dir(data_dir: &Path, task_id: &str) -> PathBuf {
    debug_root(data_dir).join(task_id)
}

#[derive(Debug, Clone)]
pub struct PayloadInfo {
    pub path: PathBuf,
    pub bytes_written: usize,
    pub truncated: bool,
    pub sha256: String,
}

fn sha256_hex(b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b);
    format!("{:x}", h.finalize())
}

fn truncate_with_suffix(mut b: Vec<u8>, max_bytes: usize, suffix: &[u8]) -> (Vec<u8>, bool) {
    if b.len() <= max_bytes {
        return (b, false);
    }
    let keep = max_bytes.saturating_sub(suffix.len());
    b.truncate(keep);
    b.extend_from_slice(suffix);
    (b, true)
}

fn to_payload_path(data_dir: &Path, path: &Path) -> String {
    match path.strip_prefix(data_dir) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

pub fn write_payload_best_effort(
    data_dir: &Path,
    task_id: &str,
    filename: &str,
    bytes: Vec<u8>,
) -> Option<PayloadInfo> {
    if !verbose_enabled() {
        return None;
    }

    let _guard = write_lock().lock().unwrap();
    let max_bytes = max_payload_bytes();
    let suffix = b"\n...(truncated)\n";
    let (out, truncated) = truncate_with_suffix(bytes, max_bytes, suffix);
    let sha256 = sha256_hex(&out);

    let dir = debug_task_dir(data_dir, task_id);
    if let Err(e) = fs::create_dir_all(&dir) {
        crate::safe_eprintln!("debug_log: create_dir_all failed: {}: {e}", dir.display());
        return None;
    }
    let path = dir.join(filename);
    if let Err(e) = fs::write(&path, &out) {
        crate::safe_eprintln!("debug_log: write failed: {}: {e}", path.display());
        return None;
    }

    prune_debug_dir_best_effort(data_dir);

    Some(PayloadInfo {
        path,
        bytes_written: out.len(),
        truncated,
        sha256,
    })
}

pub fn emit_debug_event_best_effort(
    data_dir: &Path,
    event_type: &str,
    task_id: &str,
    info: &PayloadInfo,
    note: Option<String>,
) {
    if !verbose_enabled() {
        return;
    }

    let rec = MetricsRecord::DebugArtifact {
        ts_ms: schema::now_ms(),
        task_id: task_id.to_string(),
        artifact_type: event_type.to_string(),
        payload_path: to_payload_path(data_dir, &info.path),
        payload_bytes: info.bytes_written,
        truncated: info.truncated,
        sha256: info.sha256.clone(),
        note,
    };
    if let Err(e) = metrics::emit(data_dir, rec) {
        crate::safe_eprintln!("debug_log: metrics append failed: {e:#}");
    }
}

pub fn prune_debug_dir_best_effort(data_dir: &Path) {
    if !verbose_enabled() {
        return;
    }
    let root = debug_root(data_dir);
    let max_keep = max_tasks();

    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for ent in entries.flatten() {
        let p = ent.path();
        if !p.is_dir() {
            continue;
        }
        let modified = ent
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        dirs.push((modified, p));
    }

    if dirs.len() <= max_keep {
        return;
    }

    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_modified, p) in dirs.into_iter().skip(max_keep) {
        if let Err(e) = fs::remove_dir_all(&p) {
            crate::safe_eprintln!("debug_log: remove_dir_all failed: {}: {e}", p.display());
        }
    }
}
