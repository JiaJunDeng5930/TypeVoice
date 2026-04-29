use std::path::{Path, PathBuf};

use anyhow::Result;

use super::schema::MetricsRecord;
use super::writer;

#[cfg_attr(not(test), allow(dead_code))]
pub fn metrics_path(data_dir: &Path) -> PathBuf {
    data_dir.join("metrics.jsonl")
}

pub fn emit(data_dir: &Path, record: MetricsRecord) -> Result<()> {
    writer::emit_metrics_record(data_dir, &record)
}
