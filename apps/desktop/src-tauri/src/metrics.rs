use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Serialize;

pub fn metrics_path(data_dir: &Path) -> PathBuf {
    data_dir.join("metrics.jsonl")
}

pub fn append_jsonl<T: Serialize>(data_dir: &Path, obj: &T) -> Result<()> {
    std::fs::create_dir_all(data_dir).context("create data dir failed")?;
    let p = metrics_path(data_dir);
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .with_context(|| format!("open metrics jsonl failed: {}", p.display()))?;
    let line = serde_json::to_string(obj).context("serialize metrics json failed")?;
    f.write_all(line.as_bytes())
        .context("write metrics line failed")?;
    f.write_all(b"\n").context("write metrics newline failed")?;
    Ok(())
}
