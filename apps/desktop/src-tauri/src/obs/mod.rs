pub mod debug;
pub mod metrics;
pub mod panic;
pub mod schema;
pub mod startup;
pub mod trace;
mod writer;

pub use trace::{event, event_err, event_err_anyhow, Span};

#[cfg_attr(not(test), allow(dead_code))]
pub fn flush(timeout_ms: u64) -> bool {
    writer::flush(timeout_ms)
}
