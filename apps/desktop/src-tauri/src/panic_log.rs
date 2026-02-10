use std::{
    fs::OpenOptions,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

// Install a panic hook that avoids stdout/stderr on Windows GUI builds.
//
// In `windows_subsystem = "windows"` builds, printing to stderr can fail, and the
// default panic hook prints to stderr. If stderr printing fails inside the panic
// hook, that can trigger recursive panics and manifest as a stack overflow abort
// (0xc00000fd) with no visible message. We instead log panics to the app data dir
// best-effort, and never panic from the hook itself.
pub fn install_best_effort() {
    std::panic::set_hook(Box::new(|info| {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let bt = std::backtrace::Backtrace::force_capture();

        let msg = format!(
            "ts_ms={ts_ms}\npanic={info}\nbacktrace={bt}\n---\n",
            info = info,
            bt = bt
        );

        // Best-effort write into data dir.
        if let Ok(dir) = crate::data_dir::data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("panic.log");
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
                let _ = f.write_all(msg.as_bytes());
            }
        }
    }));
}
