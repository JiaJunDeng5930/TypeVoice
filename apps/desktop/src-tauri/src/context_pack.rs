#[derive(Debug, Clone)]
pub struct HistorySnippet {
    pub created_at_ms: i64,
    pub asr_text: String,
    pub final_text: String,
    pub template_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FocusedAppInfo {
    pub process_image: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub is_browser: bool,
    pub target_source: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FocusedWindowInfo {
    pub title: Option<String>,
    pub class_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FocusedElementInfo {
    pub role: Option<String>,
    pub name: Option<String>,
    pub class_name: Option<String>,
    pub automation_id: Option<String>,
    pub editable: bool,
    pub has_keyboard_focus: bool,
}

#[derive(Debug, Clone)]
pub struct InputContext {
    pub selection_text: Option<String>,
    pub selection_start: Option<i32>,
    pub selection_end: Option<i32>,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
    pub full_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RelatedContent {
    pub before_text: Option<String>,
    pub after_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextPolicyDecision {
    pub capture_mode: String,
    pub app_rule: Option<String>,
    pub domain_rule: Option<String>,
    pub allow_related_content: bool,
    pub allow_visible_text: bool,
}

#[derive(Debug, Clone)]
pub struct ContextCaptureDiag {
    pub target_source: Option<String>,
    pub target_age_ms: Option<i64>,
    pub focus_stable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ContextSnapshot {
    pub recent_history: Vec<HistorySnippet>,
    pub clipboard_text: Option<String>,
    pub focused_app: Option<FocusedAppInfo>,
    pub focused_window: Option<FocusedWindowInfo>,
    pub focused_element: Option<FocusedElementInfo>,
    pub input_state: Option<InputContext>,
    pub related_content: Option<RelatedContent>,
    pub visible_text: Option<String>,
    pub policy_decision: Option<ContextPolicyDecision>,
    pub capture_diag: Option<ContextCaptureDiag>,
}

#[derive(Debug, Clone)]
pub struct ContextBudget {
    pub max_history_items: usize,
    pub history_window_ms: i64,
    pub max_chars_per_history_item: usize,
    pub max_chars_clipboard: usize,
    pub max_chars_input: usize,
    pub max_chars_related_side: usize,
    pub max_chars_visible_text: usize,
    pub max_total_context_chars: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_history_items: 3,
            history_window_ms: 30 * 60 * 1000, // 30min
            max_chars_per_history_item: 600,
            max_chars_clipboard: 800,
            max_chars_input: 4_096,
            max_chars_related_side: 1_200,
            max_chars_visible_text: 4_000,
            max_total_context_chars: 6_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreparedContext {
    pub user_text: String,
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
    if *remaining == 0 || s.is_empty() {
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

fn push_section(
    context_out: &mut String,
    title: &str,
    body: &str,
    remaining: &mut usize,
    trailing_newline: bool,
) {
    let trimmed = body.trim();
    if trimmed.is_empty() || *remaining == 0 {
        return;
    }
    push_with_budget(context_out, title, remaining);
    push_with_budget(context_out, "\n", remaining);
    push_with_budget(context_out, trimmed, remaining);
    push_with_budget(context_out, "\n", remaining);
    if trailing_newline {
        push_with_budget(context_out, "\n", remaining);
    }
}

pub fn prepare(asr_text: &str, snap: &ContextSnapshot, budget: &ContextBudget) -> PreparedContext {
    let mut out = String::new();
    let mut context_out = String::new();
    let mut remaining = budget.max_total_context_chars;

    out.push_str("### TRANSCRIPT\n");
    out.push_str(asr_text.trim());
    out.push_str("\n\n");

    if let Some(app) = &snap.focused_app {
        let mut body = String::new();
        if let Some(v) = app.window_title.as_deref() {
            body.push_str("title=");
            body.push_str(&clamp_chars(v, 200));
            body.push('\n');
        }
        if let Some(v) = app.process_image.as_deref() {
            body.push_str("process=");
            body.push_str(&clamp_chars(v, 260));
            body.push('\n');
        }
        if let Some(v) = app.url.as_deref() {
            body.push_str("url=");
            body.push_str(&clamp_chars(v, 300));
            body.push('\n');
        }
        body.push_str("is_browser=");
        body.push_str(if app.is_browser { "true" } else { "false" });
        body.push('\n');
        if let Some(v) = app.target_source.as_deref() {
            body.push_str("target_source=");
            body.push_str(&clamp_chars(v, 80));
            body.push('\n');
        }
        push_section(&mut context_out, "#### FOCUSED APP", &body, &mut remaining, true);
    }

    if let Some(window) = &snap.focused_window {
        let mut body = String::new();
        if let Some(v) = window.title.as_deref() {
            body.push_str("title=");
            body.push_str(&clamp_chars(v, 200));
            body.push('\n');
        }
        if let Some(v) = window.class_name.as_deref() {
            body.push_str("class=");
            body.push_str(&clamp_chars(v, 160));
            body.push('\n');
        }
        push_section(&mut context_out, "#### FOCUSED WINDOW", &body, &mut remaining, true);
    }

    if let Some(element) = &snap.focused_element {
        let mut body = String::new();
        if let Some(v) = element.role.as_deref() {
            body.push_str("role=");
            body.push_str(&clamp_chars(v, 80));
            body.push('\n');
        }
        if let Some(v) = element.name.as_deref() {
            body.push_str("name=");
            body.push_str(&clamp_chars(v, 200));
            body.push('\n');
        }
        if let Some(v) = element.class_name.as_deref() {
            body.push_str("class=");
            body.push_str(&clamp_chars(v, 160));
            body.push('\n');
        }
        if let Some(v) = element.automation_id.as_deref() {
            body.push_str("automation_id=");
            body.push_str(&clamp_chars(v, 120));
            body.push('\n');
        }
        body.push_str("editable=");
        body.push_str(if element.editable { "true" } else { "false" });
        body.push('\n');
        body.push_str("has_keyboard_focus=");
        body.push_str(if element.has_keyboard_focus {
            "true"
        } else {
            "false"
        });
        body.push('\n');
        push_section(
            &mut context_out,
            "#### FOCUSED ELEMENT",
            &body,
            &mut remaining,
            true,
        );
    }

    if let Some(input) = &snap.input_state {
        let mut body = String::new();
        if let Some(v) = input.selection_start {
            body.push_str("selection_start=");
            body.push_str(&v.to_string());
            body.push('\n');
        }
        if let Some(v) = input.selection_end {
            body.push_str("selection_end=");
            body.push_str(&v.to_string());
            body.push('\n');
        }
        if let Some(v) = input.selection_text.as_deref() {
            body.push_str("selection_text=\n");
            body.push_str(&clamp_chars(v, budget.max_chars_input));
            body.push('\n');
        }
        if let Some(v) = input.before_text.as_deref() {
            body.push_str("before_text=\n");
            body.push_str(&clamp_chars(v, budget.max_chars_input));
            body.push('\n');
        }
        if let Some(v) = input.after_text.as_deref() {
            body.push_str("after_text=\n");
            body.push_str(&clamp_chars(v, budget.max_chars_input));
            body.push('\n');
        }
        if let Some(v) = input.full_text.as_deref() {
            body.push_str("full_text=\n");
            body.push_str(&clamp_chars(v, budget.max_chars_input));
            body.push('\n');
        }
        push_section(&mut context_out, "#### INPUT STATE", &body, &mut remaining, true);
    }

    if let Some(related) = &snap.related_content {
        let mut body = String::new();
        if let Some(v) = related.before_text.as_deref() {
            body.push_str("before=\n");
            body.push_str(&clamp_chars(v, budget.max_chars_related_side));
            body.push('\n');
        }
        if let Some(v) = related.after_text.as_deref() {
            body.push_str("after=\n");
            body.push_str(&clamp_chars(v, budget.max_chars_related_side));
            body.push('\n');
        }
        push_section(
            &mut context_out,
            "#### RELATED CONTENT",
            &body,
            &mut remaining,
            true,
        );
    }

    if let Some(v) = snap.visible_text.as_deref() {
        let body = clamp_chars(v, budget.max_chars_visible_text);
        push_section(
            &mut context_out,
            "#### VISIBLE WINDOW TEXT",
            &body,
            &mut remaining,
            true,
        );
    }

    if !snap.recent_history.is_empty() && budget.max_history_items > 0 && remaining > 0 {
        let mut body = String::new();
        for h in snap.recent_history.iter().take(budget.max_history_items) {
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
            body.push_str(&meta);
            body.push_str(&clipped);
            body.push('\n');
        }
        push_section(
            &mut context_out,
            "#### RECENT HISTORY",
            &body,
            &mut remaining,
            true,
        );
    }

    if let Some(cb) = snap.clipboard_text.as_deref() {
        let body = clamp_chars(cb, budget.max_chars_clipboard);
        push_section(&mut context_out, "#### CLIPBOARD", &body, &mut remaining, true);
    }

    if let Some(policy) = &snap.policy_decision {
        let mut body = String::new();
        body.push_str("capture_mode=");
        body.push_str(&clamp_chars(&policy.capture_mode, 32));
        body.push('\n');
        if let Some(v) = policy.app_rule.as_deref() {
            body.push_str("app_rule=");
            body.push_str(&clamp_chars(v, 32));
            body.push('\n');
        }
        if let Some(v) = policy.domain_rule.as_deref() {
            body.push_str("domain_rule=");
            body.push_str(&clamp_chars(v, 64));
            body.push('\n');
        }
        body.push_str("allow_related_content=");
        body.push_str(if policy.allow_related_content {
            "true"
        } else {
            "false"
        });
        body.push('\n');
        body.push_str("allow_visible_text=");
        body.push_str(if policy.allow_visible_text {
            "true"
        } else {
            "false"
        });
        body.push('\n');
        push_section(&mut context_out, "#### POLICY", &body, &mut remaining, true);
    }

    if let Some(diag) = &snap.capture_diag {
        let mut body = String::new();
        if let Some(v) = diag.target_source.as_deref() {
            body.push_str("target_source=");
            body.push_str(&clamp_chars(v, 80));
            body.push('\n');
        }
        if let Some(v) = diag.target_age_ms {
            body.push_str("target_age_ms=");
            body.push_str(&v.to_string());
            body.push('\n');
        }
        body.push_str("focus_stable=");
        body.push_str(if diag.focus_stable { "true" } else { "false" });
        body.push('\n');
        push_section(
            &mut context_out,
            "#### CAPTURE DIAG",
            &body,
            &mut remaining,
            false,
        );
    }

    if !context_out.trim().is_empty() {
        out.push_str("### CONTEXT\n");
        out.push_str(context_out.trim_end());
    }

    PreparedContext {
        user_text: out.trim_end().to_string(),
    }
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
            focused_app: Some(FocusedAppInfo {
                process_image: Some("p.exe".to_string()),
                window_title: Some("win".to_string()),
                url: Some("https://example.com".to_string()),
                is_browser: true,
                target_source: Some("foreground".to_string()),
            }),
            focused_window: Some(FocusedWindowInfo {
                title: Some("win".to_string()),
                class_name: Some("Chrome_WidgetWin_1".to_string()),
            }),
            focused_element: Some(FocusedElementInfo {
                role: Some("Edit".to_string()),
                name: Some("message".to_string()),
                class_name: Some("Chrome_RenderWidgetHostHWND".to_string()),
                automation_id: Some("compose".to_string()),
                editable: true,
                has_keyboard_focus: true,
            }),
            input_state: Some(InputContext {
                selection_text: Some("clip".to_string()),
                selection_start: Some(1),
                selection_end: Some(5),
                before_text: Some("before".to_string()),
                after_text: Some("after".to_string()),
                full_text: Some("before clip after".to_string()),
            }),
            related_content: Some(RelatedContent {
                before_text: Some("related before".to_string()),
                after_text: Some("related after".to_string()),
            }),
            visible_text: Some("visible text".to_string()),
            policy_decision: Some(ContextPolicyDecision {
                capture_mode: "balanced".to_string(),
                app_rule: Some("allow".to_string()),
                domain_rule: None,
                allow_related_content: true,
                allow_visible_text: true,
            }),
            capture_diag: Some(ContextCaptureDiag {
                target_source: Some("foreground".to_string()),
                target_age_ms: Some(12),
                focus_stable: true,
            }),
        };
        let mut budget = ContextBudget::default();
        budget.max_total_context_chars = 1_200;
        let out = prepare(" TRANSCRIPT ", &snap, &budget);
        assert!(out.user_text.contains("### TRANSCRIPT"));
        assert!(out.user_text.contains("TRANSCRIPT"));
        assert!(out.user_text.contains("FOCUSED APP"));
        assert!(out.user_text.contains("FOCUSED ELEMENT"));
        assert!(out.user_text.contains("INPUT STATE"));
        assert!(out.user_text.contains("RELATED CONTENT"));
        assert!(out.user_text.contains("VISIBLE WINDOW TEXT"));
        assert!(out.user_text.contains("RECENT HISTORY"));
        assert!(out.user_text.contains("CLIPBOARD"));
        assert!(out.user_text.contains("POLICY"));
    }
}
