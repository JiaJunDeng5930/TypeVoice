use std::path::PathBuf;

use anyhow::{anyhow, Result};

pub fn data_dir() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("TYPEVOICE_DATA_DIR") {
        return Ok(PathBuf::from(p));
    }
    // Dev default: repo-root/tmp/typevoice-data
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = dir
        .ancestors()
        .nth(3)
        .ok_or_else(|| anyhow!("failed to locate repo root"))?;
    Ok(root.join("tmp").join("typevoice-data"))
}

