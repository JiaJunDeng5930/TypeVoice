pub mod debug;
pub mod metrics;
pub mod panic;
pub mod schema;
pub mod startup;
pub mod trace;
mod writer;

pub use trace::{event, event_err, event_err_anyhow, Span};

pub(crate) fn runtime_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("TYPEVOICE_DATA_DIR") {
        return Some(std::path::PathBuf::from(p));
    }
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = dir.ancestors().nth(2)?;
    Some(root.join("tmp").join("typevoice-data"))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn flush(timeout_ms: u64) -> bool {
    writer::flush(timeout_ms)
}
