use std::path::Path;

use crate::context_pack::{ContextBudget, ContextSnapshot, HistorySnippet};
use crate::{history, settings};
use crate::{trace, trace::Span};

#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub include_history: bool,
    pub include_clipboard: bool,
    pub include_prev_window_screenshot: bool,
    pub budget: ContextBudget,
    pub llm_supports_vision: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            include_history: true,
            include_clipboard: true,
            include_prev_window_screenshot: true,
            budget: ContextBudget::default(),
            llm_supports_vision: true,
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn config_from_settings(s: &settings::Settings) -> ContextConfig {
    let mut cfg = ContextConfig::default();

    if let Some(v) = s.context_include_clipboard {
        cfg.include_clipboard = v;
    }
    if let Some(v) = s.context_include_prev_window_screenshot {
        cfg.include_prev_window_screenshot = v;
    }
    if let Some(v) = s.context_include_history {
        cfg.include_history = v;
    }
    if let Some(v) = s.llm_supports_vision {
        cfg.llm_supports_vision = v;
    }

    if let Some(n) = s.context_history_n {
        if n > 0 {
            cfg.budget.max_history_items = n as usize;
        }
    }
    if let Some(ms) = s.context_history_window_ms {
        if ms > 0 {
            cfg.budget.history_window_ms = ms;
        }
    }
    cfg
}

#[derive(Clone)]
pub struct ContextService {
    #[cfg(windows)]
    inner: std::sync::Arc<std::sync::Mutex<Inner>>,
}

#[cfg(windows)]
struct Inner {
    win: crate::context_capture_windows::WindowsContext,
}

impl ContextService {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let inner = Inner {
                win: crate::context_capture_windows::WindowsContext::new(),
            };
            return Self {
                inner: std::sync::Arc::new(std::sync::Mutex::new(inner)),
            };
        }
        #[cfg(not(windows))]
        {
            Self {}
        }
    }

    pub fn warmup_best_effort(&self) {
        #[cfg(windows)]
        {
            let g = self.inner.lock().unwrap();
            g.win.warmup_best_effort();
        }
    }

    pub fn capture_snapshot_best_effort(
        &self,
        data_dir: &Path,
        task_id: &str,
    ) -> (ContextConfig, ContextSnapshot) {
        let captured_at_ms = now_ms();
        let s = settings::load_settings_or_recover(data_dir);
        let cfg = config_from_settings(&s);

        let _span_all = Span::start(
            data_dir,
            Some(task_id),
            "ContextCapture",
            "CTX.capture_snapshot",
            Some(serde_json::json!({
                "include_history": cfg.include_history,
                "include_clipboard": cfg.include_clipboard,
                "include_prev_window_screenshot": cfg.include_prev_window_screenshot,
                "max_history_items": cfg.budget.max_history_items,
                "history_window_ms": cfg.budget.history_window_ms,
                "llm_supports_vision": cfg.llm_supports_vision,
            })),
        );

        let mut snap = ContextSnapshot::default();

        if cfg.include_history && cfg.budget.max_history_items > 0 {
            let db = data_dir.join("history.sqlite3");
            let before = Some(captured_at_ms);
            let span = Span::start(
                data_dir,
                Some(task_id),
                "ContextCapture",
                "CTX.history.list",
                Some(serde_json::json!({
                    "limit": (cfg.budget.max_history_items as i64).max(1),
                    "before_ms": before,
                })),
            );
            match history::list(&db, (cfg.budget.max_history_items as i64).max(1), before) {
                Ok(mut rows) => {
                    let min_ms = captured_at_ms.saturating_sub(cfg.budget.history_window_ms);
                    rows.retain(|h| h.created_at_ms >= min_ms);
                    snap.recent_history = rows
                        .into_iter()
                        .map(|h| HistorySnippet {
                            created_at_ms: h.created_at_ms,
                            asr_text: h.asr_text,
                            final_text: h.final_text,
                            template_id: h.template_id,
                        })
                        .collect();
                    span.ok(Some(serde_json::json!({
                        "items": snap.recent_history.len(),
                        "min_ms": min_ms,
                    })));
                }
                Err(e) => {
                    span.err(
                        "io",
                        "E_HISTORY_LIST",
                        &e.to_string(),
                        Some(serde_json::json!({
                            "db": "history.sqlite3",
                        })),
                    );
                    // best-effort: ignore history failures.
                }
            }
        }

        if cfg.include_clipboard {
            #[cfg(windows)]
            {
                let g = self.inner.lock().unwrap();
                let span = Span::start(
                    data_dir,
                    Some(task_id),
                    "ContextCapture",
                    "CTX.clipboard.read",
                    None,
                );
                let r = g.win.read_clipboard_text_diag_best_effort();
                snap.clipboard_text = r.text;
                match r.diag.status.as_str() {
                    "ok" => span.ok(Some(serde_json::json!({"bytes": snap.clipboard_text.as_deref().map(|s| s.len()).unwrap_or(0)}))),
                    "skipped" => span.skipped(
                        r.diag.note.as_deref().unwrap_or("skipped"),
                        Some(serde_json::json!({"step": r.diag.step, "last_error": r.diag.last_error})),
                    ),
                    _ => span.err(
                        "winapi",
                        "E_CLIPBOARD",
                        r.diag.note.as_deref().unwrap_or("clipboard read failed"),
                        Some(serde_json::json!({"step": r.diag.step, "last_error": r.diag.last_error})),
                    ),
                }
            }
        }

        if cfg.include_prev_window_screenshot {
            #[cfg(windows)]
            {
                let g = self.inner.lock().unwrap();
                let info_span = Span::start(
                    data_dir,
                    Some(task_id),
                    "ContextCapture",
                    "CTX.prev_window.info",
                    None,
                );
                if let Some(info) = g.win.last_external_window_info_best_effort() {
                    snap.prev_window = Some(crate::context_pack::PrevWindowInfo {
                        title: info.title,
                        process_image: info.process_image,
                    });
                    info_span.ok(Some(serde_json::json!({
                        "has_title": snap.prev_window.as_ref().and_then(|w| w.title.as_ref()).is_some(),
                        "has_process": snap.prev_window.as_ref().and_then(|w| w.process_image.as_ref()).is_some(),
                    })));
                }
                else {
                    info_span.skipped("no_last_external_window", None);
                }

                let shot_span = Span::start(
                    data_dir,
                    Some(task_id),
                    "ContextCapture",
                    "CTX.prev_window.screenshot",
                    Some(serde_json::json!({"max_side": 1024})),
                );
                let sc = g.win.capture_last_external_window_png_diag_best_effort(1024);
                if let Some(raw) = sc.raw {
                    let sha = crate::context_pack::sha256_hex(&raw.png_bytes);
                    snap.screenshot = Some(crate::context_pack::ScreenshotPng {
                        width: raw.width,
                        height: raw.height,
                        sha256_hex: sha,
                        png_bytes: raw.png_bytes,
                    });
                    shot_span.ok(Some(serde_json::json!({
                        "w": snap.screenshot.as_ref().unwrap().width,
                        "h": snap.screenshot.as_ref().unwrap().height,
                        "bytes": snap.screenshot.as_ref().unwrap().png_bytes.len(),
                        "sha256": snap.screenshot.as_ref().unwrap().sha256_hex,
                    })));
                } else if let Some(err) = sc.error {
                    shot_span.err(
                        "winapi",
                        "E_SCREENSHOT",
                        &err.note.clone().unwrap_or_else(|| "screenshot failed".to_string()),
                        Some(serde_json::json!({
                            "step": err.step,
                            "api": err.api,
                            "api_ret": err.api_ret,
                            "last_error": err.last_error,
                            "window_w": err.window_w,
                            "window_h": err.window_h,
                            "max_side": err.max_side,
                        })),
                    );
                } else {
                    shot_span.skipped("no_window_or_invalid", None);
                }
            }
        }

        // Mark the overall span as ok (it may contain inner errs/skips).
        // Note: we intentionally do not fail the pipeline based on context capture.
        trace::event(
            data_dir,
            Some(task_id),
            "ContextCapture",
            "CTX.capture_snapshot.summary",
            "ok",
            Some(serde_json::json!({
                "history_items": snap.recent_history.len(),
                "clipboard_bytes": snap.clipboard_text.as_deref().map(|s| s.len()).unwrap_or(0),
                "has_prev_window": snap.prev_window.is_some(),
                "has_screenshot": snap.screenshot.is_some(),
            })),
        );
        _span_all.ok(None);

        (cfg, snap)
    }
}
