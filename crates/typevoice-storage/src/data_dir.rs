use std::path::PathBuf;

use anyhow::{anyhow, Result};

pub fn data_dir() -> Result<PathBuf> {
    crate::obs::runtime_data_dir().ok_or_else(|| anyhow!("unsupported platform data directory"))
}
