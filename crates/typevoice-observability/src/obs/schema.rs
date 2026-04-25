use serde::Serialize;
use serde_json::Value;

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MetricsRecord {
    TaskEvent {
        ts_ms: i64,
        task_id: String,
        stage: String,
        status: String,
        elapsed_ms: Option<u128>,
        error_code: Option<String>,
        message: String,
    },
    TaskPerf {
        ts_ms: i64,
        task_id: String,
        asr_provider: String,
        audio_seconds: f64,
        preprocess_ms: u128,
        asr_roundtrip_ms: u128,
        asr_provider_elapsed_ms: i64,
        asr_transport_overhead_ms: u64,
        rtf: f64,
        rewrite_ms: Option<u128>,
        device_used: String,
        asr_model_id: String,
        asr_model_version: Option<String>,
        remote_asr_slice_count: Option<usize>,
        remote_asr_concurrency_used: Option<usize>,
        asr_preprocess_silence_trim_enabled: bool,
        asr_preprocess_threshold_db: f64,
        asr_preprocess_trim_start_ms: u64,
        asr_preprocess_trim_end_ms: u64,
    },
    TaskDone {
        ts_ms: i64,
        task_id: String,
        rtf: f64,
        device: String,
    },
    DebugArtifact {
        ts_ms: i64,
        task_id: String,
        artifact_type: String,
        payload_path: String,
        payload_bytes: usize,
        truncated: bool,
        sha256: String,
        note: Option<String>,
    },
    LoggerDropped {
        ts_ms: i64,
        stream: String,
        count: u64,
        queue_capacity: usize,
    },
}
