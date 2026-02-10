use std::path::Path;

use crate::context_pack::{ContextBudget, ContextSnapshot, HistorySnippet};
use crate::{history, settings};

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
    ) -> (ContextConfig, ContextSnapshot) {
        let captured_at_ms = now_ms();
        let s = settings::load_settings_or_recover(data_dir);
        let cfg = config_from_settings(&s);

        let mut snap = ContextSnapshot::default();

        if cfg.include_history && cfg.budget.max_history_items > 0 {
            let db = data_dir.join("history.sqlite3");
            let before = Some(captured_at_ms);
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
                }
                Err(_) => {
                    // best-effort: ignore history failures
                }
            }
        }

        if cfg.include_clipboard {
            #[cfg(windows)]
            {
                let g = self.inner.lock().unwrap();
                snap.clipboard_text = g.win.read_clipboard_text_best_effort();
            }
        }

        if cfg.include_prev_window_screenshot {
            #[cfg(windows)]
            {
                let g = self.inner.lock().unwrap();
                if let Some(info) = g.win.last_external_window_info_best_effort() {
                    snap.prev_window = Some(crate::context_pack::PrevWindowInfo {
                        title: info.title,
                        process_image: info.process_image,
                    });
                    if let Some(sc) = g.win.capture_last_external_window_png_best_effort(1024) {
                        snap.screenshot = Some(crate::context_pack::ScreenshotPng {
                            width: sc.width,
                            height: sc.height,
                            sha256_hex: crate::context_pack::sha256_hex(&sc.png_bytes),
                            png_bytes: sc.png_bytes,
                        });
                    }
                }
            }
        }

        (cfg, snap)
    }
}
