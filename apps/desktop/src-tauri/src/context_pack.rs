#[cfg(windows)]
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct HistorySnippet {
    pub created_at_ms: i64,
    pub asr_text: String,
    pub final_text: String,
    pub template_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PrevWindowInfo {
    pub title: Option<String>,
    pub process_image: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScreenshotPng {
    pub png_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sha256_hex: String,
}

#[derive(Debug, Clone, Default)]
pub struct ContextSnapshot {
    pub recent_history: Vec<HistorySnippet>,
    pub clipboard_text: Option<String>,
    pub prev_window: Option<PrevWindowInfo>,
    pub screenshot: Option<ScreenshotPng>,
}

#[derive(Debug, Clone)]
pub struct ContextBudget {
    pub max_history_items: usize,
    pub history_window_ms: i64,
    pub max_chars_per_history_item: usize,
    pub max_chars_clipboard: usize,
    pub max_total_context_chars: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_history_items: 3,
            history_window_ms: 30 * 60 * 1000, // 30min
            max_chars_per_history_item: 600,
            max_chars_clipboard: 800,
            max_total_context_chars: 3000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreparedContext {
    pub user_text: String,
    pub screenshot: Option<ScreenshotPng>,
}

fn clamp_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let t = s.trim();
    if t.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in t.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        if ch == '\u{0}' {
            continue;
        }
        out.push(ch);
    }
    out
}

fn push_with_budget(dst: &mut String, s: &str, remaining: &mut usize) {
    if *remaining == 0 {
        return;
    }
    if s.is_empty() {
        return;
    }
    let mut took = 0usize;
    for ch in s.chars() {
        if took >= *remaining {
            break;
        }
        dst.push(ch);
        took += 1;
    }
    *remaining = remaining.saturating_sub(took);
}

pub fn prepare(asr_text: &str, snap: &ContextSnapshot, budget: &ContextBudget) -> PreparedContext {
    let mut out = String::new();
    let mut context_out = String::new();
    let mut remaining = budget.max_total_context_chars;

    // Always include transcript first; we do not apply context budget to transcript itself.
    out.push_str("### TRANSCRIPT\n");
    out.push_str(asr_text.trim());
    out.push_str("\n\n");

    // Recent history
    if !snap.recent_history.is_empty() && budget.max_history_items > 0 && remaining > 0 {
        context_out.push_str("#### RECENT HISTORY\n");
        let mut used_items = 0usize;
        for h in snap.recent_history.iter().take(budget.max_history_items) {
            if remaining == 0 {
                break;
            }
            used_items += 1;
            let txt = if !h.final_text.trim().is_empty() {
                &h.final_text
            } else {
                &h.asr_text
            };
            let clipped = clamp_chars(txt, budget.max_chars_per_history_item);
            if clipped.is_empty() {
                continue;
            }
            let meta = match &h.template_id {
                Some(tid) => format!("- [t={} template={}] ", h.created_at_ms, tid),
                None => format!("- [t={}] ", h.created_at_ms),
            };
            push_with_budget(&mut context_out, &meta, &mut remaining);
            push_with_budget(&mut context_out, &clipped, &mut remaining);
            push_with_budget(&mut context_out, "\n", &mut remaining);
        }
        if used_items > 0 {
            push_with_budget(&mut context_out, "\n", &mut remaining);
        }
    }

    // Clipboard
    if let Some(cb) = snap.clipboard_text.as_deref() {
        if remaining > 0 {
            let clipped = clamp_chars(cb, budget.max_chars_clipboard);
            if !clipped.is_empty() {
                context_out.push_str("#### CLIPBOARD\n");
                push_with_budget(&mut context_out, &clipped, &mut remaining);
                push_with_budget(&mut context_out, "\n\n", &mut remaining);
            }
        }
    }

    // Previous window meta
    if let Some(w) = &snap.prev_window {
        if remaining > 0 {
            context_out.push_str("#### PREVIOUS WINDOW\n");
            if let Some(t) = w.title.as_deref() {
                let v = clamp_chars(t, 200);
                if !v.is_empty() {
                    push_with_budget(&mut context_out, "title=", &mut remaining);
                    push_with_budget(&mut context_out, &v, &mut remaining);
                    push_with_budget(&mut context_out, "\n", &mut remaining);
                }
            }
            if let Some(p) = w.process_image.as_deref() {
                let v = clamp_chars(p, 260);
                if !v.is_empty() {
                    push_with_budget(&mut context_out, "process=", &mut remaining);
                    push_with_budget(&mut context_out, &v, &mut remaining);
                    push_with_budget(&mut context_out, "\n", &mut remaining);
                }
            }
            push_with_budget(&mut context_out, "\n", &mut remaining);
        }
    }

    if !context_out.trim().is_empty() {
        out.push_str("### CONTEXT\n");
        out.push_str(&context_out);
    }

    PreparedContext {
        user_text: out.trim_end().to_string(),
        screenshot: snap.screenshot.clone(),
    }
}

#[cfg(windows)]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    hex::encode(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_is_deterministic_and_budgets_context_only() {
        let snap = ContextSnapshot {
            recent_history: vec![
                HistorySnippet {
                    created_at_ms: 1,
                    asr_text: "a".to_string(),
                    final_text: "final-1".to_string(),
                    template_id: Some("t".to_string()),
                },
                HistorySnippet {
                    created_at_ms: 2,
                    asr_text: "asr-2".to_string(),
                    final_text: "".to_string(),
                    template_id: None,
                },
            ],
            clipboard_text: Some(" clip ".to_string()),
            prev_window: Some(PrevWindowInfo {
                title: Some("win".to_string()),
                process_image: Some("p.exe".to_string()),
            }),
            screenshot: None,
        };
        let mut budget = ContextBudget::default();
        budget.max_total_context_chars = 50;
        let out = prepare(" TRANSCRIPT ", &snap, &budget);
        assert!(out.user_text.contains("### TRANSCRIPT"));
        assert!(out.user_text.contains("TRANSCRIPT"));
        assert!(out.user_text.contains("RECENT HISTORY"));
        assert!(out.user_text.contains("CLIPBOARD"));
        assert!(out.user_text.contains("PREVIOUS WINDOW"));
    }
}
