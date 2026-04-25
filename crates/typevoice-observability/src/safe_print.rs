// Best-effort stderr logging that never panics.
//
// On Windows GUI subsystem (no console attached), Rust's `eprintln!` may panic
// on write errors, and repeated panics can lead to stack overflow + abort.
// We avoid that by explicitly ignoring stderr write failures.

#[macro_export]
macro_rules! safe_eprintln {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}
