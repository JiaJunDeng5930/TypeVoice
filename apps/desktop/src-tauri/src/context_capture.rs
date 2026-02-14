#[cfg(windows)]
use std::collections::HashMap;
use std::path::Path;

use crate::context_pack::{ContextBudget, ContextSnapshot, HistorySnippet};
use crate::{history, settings};
use crate::{trace, trace::Span};
#[cfg(windows)]
use anyhow::{anyhow, Result};
#[cfg(windows)]
use uuid::Uuid;

#[cfg(windows)]
use crate::debug_log;

#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub include_history: bool,
    pub include_clipboard: bool,
    pub include_prev_window_meta: bool,
    pub include_prev_window_screenshot: bool,
    pub budget: ContextBudget,
    pub llm_supports_vision: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            include_history: true,
            include_clipboard: true,
            include_prev_window_meta: true,
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

pub fn config_from_settings(s: &settings::Settings) -> ContextConfig {
    let mut cfg = ContextConfig::default();

    if let Some(v) = s.context_include_clipboard {
        cfg.include_clipboard = v;
    }
    if let Some(v) = s.context_include_prev_window_screenshot {
        cfg.include_prev_window_screenshot = v;
    }
    if let Some(v) = s.context_include_prev_window_meta {
        cfg.include_prev_window_meta = v;
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

#[cfg(windows)]
fn env_u32(key: &str, default: u32) -> u32 {
    match std::env::var(key) {
        Ok(v) => v
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or(default),
        Err(_) => default,
    }
}

#[derive(Clone)]
pub struct ContextService {
    #[cfg(windows)]
    inner: std::sync::Arc<std::sync::Mutex<Inner>>,
}

#[cfg(windows)]
struct Inner {
    win: crate::context_capture_windows::WindowsContext,
    hotkey_capture_registry: HashMap<String, StoredHotkeyCapture>,
}

#[cfg(windows)]
#[derive(Clone)]
struct StoredHotkeyCapture {
    snapshot: ContextSnapshot,
}

impl ContextService {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let inner = Inner {
                win: crate::context_capture_windows::WindowsContext::new(),
                hotkey_capture_registry: HashMap::new(),
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

    #[cfg(windows)]
    pub fn capture_hotkey_context_now(
        &self,
        data_dir: &Path,
        cfg: &ContextConfig,
    ) -> Result<String> {
        let max_side = env_u32("TYPEVOICE_CONTEXT_SCREENSHOT_MAX_SIDE", 1600);
        let span = Span::start(
            data_dir,
            None,
            "ContextCapture",
            "CTX.hotkey_capture_now",
            Some(serde_json::json!({
                "max_side": max_side,
                "include_prev_window_meta": cfg.include_prev_window_meta,
                "include_prev_window_screenshot": cfg.include_prev_window_screenshot,
            })),
        );

        if !cfg.include_prev_window_screenshot {
            let mut g = self.inner.lock().unwrap();
            let mut snapshot = ContextSnapshot {
                recent_history: vec![],
                clipboard_text: None,
                prev_window: None,
                screenshot: None,
            };
            if cfg.include_prev_window_meta {
                if let Some(info) = g.win.foreground_window_info_best_effort() {
                    snapshot.prev_window = Some(crate::context_pack::PrevWindowInfo {
                        title: info.title,
                        process_image: info.process_image,
                    });
                }
            }
            let capture_id = Uuid::new_v4().to_string();
            g.hotkey_capture_registry
                .insert(capture_id.clone(), StoredHotkeyCapture { snapshot });

            span.ok(Some(serde_json::json!({
                "capture_id": capture_id,
                "has_title": g.hotkey_capture_registry.get(&capture_id).and_then(|v| v.snapshot.prev_window.as_ref()).and_then(|w| w.title.as_ref()).is_some(),
                "has_process": g.hotkey_capture_registry.get(&capture_id).and_then(|v| v.snapshot.prev_window.as_ref()).and_then(|w| w.process_image.as_ref()).is_some(),
                "screenshot_disabled": true,
            })));
            return Ok(capture_id);
        }

        let mut g = self.inner.lock().unwrap();
        let cap = g
            .win
            .capture_foreground_window_now_diag_best_effort(max_side);
        let cap = match cap.capture {
            Some(v) => v,
            None => {
                let err =
                    cap.error
                        .unwrap_or(crate::context_capture_windows::ScreenshotDiagError {
                            step: "unknown".to_string(),
                            api: "unknown".to_string(),
                            api_ret: "none".to_string(),
                            last_error: 0,
                            note: Some("unknown capture failure".to_string()),
                            window_w: 0,
                            window_h: 0,
                            max_side,
                        });
                span.err(
                    "winapi",
                    "E_HOTKEY_CAPTURE",
                    err.note.as_deref().unwrap_or("hotkey capture failed"),
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
                return Err(anyhow!(
                    "E_HOTKEY_CAPTURE: {}",
                    err.note.unwrap_or_else(|| "capture failed".to_string())
                ));
            }
        };

        let sha = crate::context_pack::sha256_hex(&cap.screenshot.png_bytes);
        let snapshot = ContextSnapshot {
            recent_history: vec![],
            clipboard_text: None,
            prev_window: if cfg.include_prev_window_meta {
                Some(crate::context_pack::PrevWindowInfo {
                    title: cap.window.title,
                    process_image: cap.window.process_image,
                })
            } else {
                None
            },
            screenshot: Some(crate::context_pack::ScreenshotPng {
                png_bytes: cap.screenshot.png_bytes,
                width: cap.screenshot.width,
                height: cap.screenshot.height,
                sha256_hex: sha,
            }),
        };
        let capture_id = Uuid::new_v4().to_string();
        g.hotkey_capture_registry
            .insert(capture_id.clone(), StoredHotkeyCapture { snapshot });

        span.ok(Some(serde_json::json!({
            "capture_id": capture_id,
            "hwnd": cap.hwnd,
            "pid": cap.pid,
            "has_title": g.hotkey_capture_registry.get(&capture_id).and_then(|v| v.snapshot.prev_window.as_ref()).and_then(|w| w.title.as_ref()).is_some(),
            "has_process": g.hotkey_capture_registry.get(&capture_id).and_then(|v| v.snapshot.prev_window.as_ref()).and_then(|w| w.process_image.as_ref()).is_some(),
            "w": g.hotkey_capture_registry.get(&capture_id).and_then(|v| v.snapshot.screenshot.as_ref()).map(|s| s.width).unwrap_or(0),
            "h": g.hotkey_capture_registry.get(&capture_id).and_then(|v| v.snapshot.screenshot.as_ref()).map(|s| s.height).unwrap_or(0),
        })));
        Ok(capture_id)
    }

    #[cfg(windows)]
    pub fn take_hotkey_context_once(&self, capture_id: &str) -> Option<ContextSnapshot> {
        let mut g = self.inner.lock().unwrap();
        g.hotkey_capture_registry
            .remove(capture_id)
            .map(|v| v.snapshot)
    }

    #[cfg(windows)]
    pub fn last_external_hwnd_best_effort(&self) -> Option<isize> {
        let g = self.inner.lock().unwrap();
        g.win.last_external_hwnd_best_effort()
    }

    #[cfg(not(windows))]
    pub fn capture_hotkey_context_now(
        &self,
        _data_dir: &Path,
        _cfg: &ContextConfig,
    ) -> anyhow::Result<String> {
        Err(anyhow::anyhow!(
            "E_HOTKEY_CAPTURE_UNSUPPORTED: hotkey capture is only supported on Windows"
        ))
    }

    #[cfg(not(windows))]
    pub fn take_hotkey_context_once(&self, _capture_id: &str) -> Option<ContextSnapshot> {
        None
    }

    #[cfg(not(windows))]
    pub fn last_external_hwnd_best_effort(&self) -> Option<isize> {
        None
    }

    pub fn capture_snapshot_best_effort_with_config(
        &self,
        data_dir: &Path,
        task_id: &str,
        cfg: &ContextConfig,
    ) -> ContextSnapshot {
        let captured_at_ms = now_ms();

        let _span_all = Span::start(
            data_dir,
            Some(task_id),
            "ContextCapture",
            "CTX.capture_snapshot",
            Some(serde_json::json!({
                "include_history": cfg.include_history,
                "include_clipboard": cfg.include_clipboard,
                "include_prev_window_meta": cfg.include_prev_window_meta,
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

        if cfg.include_prev_window_meta {
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
                if let Some(info) = g.win.foreground_window_info_best_effort() {
                    snap.prev_window = Some(crate::context_pack::PrevWindowInfo {
                        title: info.title,
                        process_image: info.process_image,
                    });
                    info_span.ok(Some(serde_json::json!({
                        "has_title": snap.prev_window.as_ref().and_then(|w| w.title.as_ref()).is_some(),
                        "has_process": snap.prev_window.as_ref().and_then(|w| w.process_image.as_ref()).is_some(),
                    })));
                } else {
                    info_span.skipped("no_last_external_window", None);
                }
            }
        }

        if cfg.include_prev_window_screenshot {
            #[cfg(windows)]
            {
                let g = self.inner.lock().unwrap();
                let shot_span = Span::start(
                    data_dir,
                    Some(task_id),
                    "ContextCapture",
                    "CTX.prev_window.screenshot",
                    {
                        let max_side = env_u32("TYPEVOICE_CONTEXT_SCREENSHOT_MAX_SIDE", 1600);
                        Some(serde_json::json!({"max_side": max_side}))
                    },
                );
                let max_side = env_u32("TYPEVOICE_CONTEXT_SCREENSHOT_MAX_SIDE", 1600);
                let sc = g
                    .win
                    .capture_foreground_window_now_diag_best_effort(max_side);
                let capture = sc.capture;
                let error = sc.error;
                if let Some(raw_capture) = capture {
                    let sha = crate::context_pack::sha256_hex(&raw_capture.screenshot.png_bytes);
                    snap.screenshot = Some(crate::context_pack::ScreenshotPng {
                        width: raw_capture.screenshot.width,
                        height: raw_capture.screenshot.height,
                        sha256_hex: sha,
                        png_bytes: raw_capture.screenshot.png_bytes,
                    });
                    if cfg.include_prev_window_meta {
                        snap.prev_window = Some(crate::context_pack::PrevWindowInfo {
                            title: raw_capture.window.title,
                            process_image: raw_capture.window.process_image,
                        });
                    }
                    shot_span.ok(Some(serde_json::json!({
                        "w": snap.screenshot.as_ref().unwrap().width,
                        "h": snap.screenshot.as_ref().unwrap().height,
                        "bytes": snap.screenshot.as_ref().unwrap().png_bytes.len(),
                        "sha256": snap.screenshot.as_ref().unwrap().sha256_hex,
                        "max_side": max_side,
                    })));

                    // Optional debug artifact: persist the screenshot PNG for manual inspection.
                    // This is OFF by default because screenshots are sensitive.
                    if debug_log::verbose_enabled() && debug_log::include_screenshots() {
                        if let Some(sc) = snap.screenshot.as_ref() {
                            if let Some(info) =
                                debug_log::write_payload_binary_no_truncate_best_effort(
                                    data_dir,
                                    task_id,
                                    "prev_window.png",
                                    sc.png_bytes.clone(),
                                )
                            {
                                debug_log::emit_debug_event_best_effort(
                                    data_dir,
                                    "debug_prev_window_png",
                                    task_id,
                                    &info,
                                    Some(format!(
                                        "w={} h={} bytes={} sha256={}",
                                        sc.width,
                                        sc.height,
                                        sc.png_bytes.len(),
                                        sc.sha256_hex
                                    )),
                                );
                            }
                        }
                    }
                } else if let Some(err) = error {
                    shot_span.err(
                        "winapi",
                        "E_SCREENSHOT",
                        &err.note
                            .clone()
                            .unwrap_or_else(|| "screenshot failed".to_string()),
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

        snap
    }
}
