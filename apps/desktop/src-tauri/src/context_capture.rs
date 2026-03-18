#[cfg(windows)]
use std::collections::HashMap;
use std::path::Path;

use crate::context_pack::{
    ContextBudget, ContextCaptureDiag, ContextPolicyDecision, ContextSnapshot, FocusedWindowInfo,
    HistorySnippet,
};
use crate::{history, settings};
use crate::{obs, obs::Span};
#[cfg(windows)]
use anyhow::Result;
#[cfg(windows)]
use url::Url;
#[cfg(windows)]
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ContextRules {
    pub capture_mode: String,
    pub app_allowlist: Vec<String>,
    pub app_denylist: Vec<String>,
    pub domain_allowlist: Vec<String>,
    pub domain_denylist: Vec<String>,
}

impl Default for ContextRules {
    fn default() -> Self {
        Self {
            capture_mode: "balanced".to_string(),
            app_allowlist: Vec::new(),
            app_denylist: Vec::new(),
            domain_allowlist: Vec::new(),
            domain_denylist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub include_history: bool,
    pub include_clipboard: bool,
    pub include_prev_window_meta: bool,
    pub include_focused_app_meta: bool,
    pub include_focused_element_meta: bool,
    pub include_input_state: bool,
    pub include_related_content: bool,
    pub include_visible_text: bool,
    pub budget: ContextBudget,
    pub rules: ContextRules,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            include_history: true,
            include_clipboard: true,
            include_prev_window_meta: true,
            include_focused_app_meta: true,
            include_focused_element_meta: true,
            include_input_state: true,
            include_related_content: true,
            include_visible_text: true,
            budget: ContextBudget::default(),
            rules: ContextRules::default(),
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

fn normalize_rules(values: &Option<Vec<String>>) -> Vec<String> {
    values
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .collect()
}

pub fn config_from_settings(s: &settings::Settings) -> ContextConfig {
    let mut cfg = ContextConfig::default();

    if let Some(v) = s.context_include_clipboard {
        cfg.include_clipboard = v;
    }
    if let Some(v) = s.context_include_prev_window_meta {
        cfg.include_prev_window_meta = v;
    }
    if let Some(v) = s.context_include_focused_app_meta {
        cfg.include_focused_app_meta = v;
    }
    if let Some(v) = s.context_include_focused_element_meta {
        cfg.include_focused_element_meta = v;
    }
    if let Some(v) = s.context_include_input_state {
        cfg.include_input_state = v;
    }
    if let Some(v) = s.context_include_related_content {
        cfg.include_related_content = v;
    }
    if let Some(v) = s.context_include_visible_text {
        cfg.include_visible_text = v;
    }
    if let Some(v) = s.context_include_history {
        cfg.include_history = v;
    }
    if let Some(v) = s.context_capture_mode.as_deref() {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            cfg.rules.capture_mode = trimmed.to_ascii_lowercase();
        }
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
    if let Some(v) = s.context_input_max_chars {
        if v > 0 {
            cfg.budget.max_chars_input = v as usize;
        }
    }
    if let Some(v) = s.context_related_before_chars {
        if v > 0 {
            cfg.budget.max_chars_related_before = v as usize;
        }
    }
    if let Some(v) = s.context_related_after_chars {
        if v > 0 {
            cfg.budget.max_chars_related_after = v as usize;
        }
    }
    if let Some(v) = s.context_visible_text_max_chars {
        if v > 0 {
            cfg.budget.max_chars_visible_text = v as usize;
        }
    }

    cfg.rules.app_allowlist = normalize_rules(&s.context_app_allowlist);
    cfg.rules.app_denylist = normalize_rules(&s.context_app_denylist);
    cfg.rules.domain_allowlist = normalize_rules(&s.context_domain_allowlist);
    cfg.rules.domain_denylist = normalize_rules(&s.context_domain_denylist);

    cfg
}

#[derive(Debug, Clone)]
struct PolicyResolution {
    app_rule: Option<String>,
    domain_rule: Option<String>,
    allow_related_content: bool,
    allow_visible_text: bool,
}

fn process_basename(process_image: Option<&str>) -> Option<String> {
    let raw = process_image?.replace('/', "\\");
    raw.rsplit('\\')
        .next()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
}

fn extract_hostname(url: Option<&str>) -> Option<String> {
    let raw = url?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = Url::parse(raw).or_else(|_| {
        if raw.contains("://") {
            Err(url::ParseError::RelativeUrlWithoutBase)
        } else {
            Url::parse(&format!("https://{raw}"))
        }
    });
    parsed
        .ok()
        .and_then(|v| v.host_str().map(|h| h.to_ascii_lowercase()))
}

fn resolve_policy(
    cfg: &ContextConfig,
    process_image: Option<&str>,
    url: Option<&str>,
) -> PolicyResolution {
    let app_name = process_basename(process_image);
    let host = extract_hostname(url);
    let app_rule = if let Some(name) = app_name.as_deref() {
        if cfg.rules.app_allowlist.iter().any(|v| v == name) {
            Some("allow".to_string())
        } else if cfg.rules.app_denylist.iter().any(|v| v == name) {
            Some("deny".to_string())
        } else {
            None
        }
    } else {
        None
    };
    let domain_rule = if let Some(name) = host.as_deref() {
        if cfg
            .rules
            .domain_allowlist
            .iter()
            .any(|v| name == v || name.ends_with(&format!(".{v}")))
        {
            Some("allow".to_string())
        } else if cfg
            .rules
            .domain_denylist
            .iter()
            .any(|v| name == v || name.ends_with(&format!(".{v}")))
        {
            Some("deny".to_string())
        } else {
            None
        }
    } else {
        None
    };

    let default_allow = cfg.rules.capture_mode != "minimal";
    let allow_related_content = if matches!(app_rule.as_deref(), Some("deny"))
        || matches!(domain_rule.as_deref(), Some("deny"))
    {
        false
    } else if matches!(app_rule.as_deref(), Some("allow"))
        || matches!(domain_rule.as_deref(), Some("allow"))
    {
        true
    } else {
        default_allow
    };

    PolicyResolution {
        app_rule,
        domain_rule,
        allow_related_content,
        allow_visible_text: allow_related_content,
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

    fn load_recent_history_best_effort(
        &self,
        data_dir: &Path,
        task_id: Option<&str>,
        cfg: &ContextConfig,
        captured_at_ms: i64,
    ) -> Vec<HistorySnippet> {
        if !cfg.include_history || cfg.budget.max_history_items == 0 {
            return Vec::new();
        }

        let db = data_dir.join("history.sqlite3");
        let span = Span::start(
            data_dir,
            task_id,
            "ContextCapture",
            "CTX.history.list",
            Some(serde_json::json!({
                "limit": (cfg.budget.max_history_items as i64).max(1),
                "before_ms": captured_at_ms,
            })),
        );
        match history::list(
            &db,
            (cfg.budget.max_history_items as i64).max(1),
            Some(captured_at_ms),
        ) {
            Ok(mut rows) => {
                let min_ms = captured_at_ms.saturating_sub(cfg.budget.history_window_ms);
                rows.retain(|h| h.created_at_ms >= min_ms);
                let history: Vec<HistorySnippet> = rows
                    .into_iter()
                    .map(|h| HistorySnippet {
                        created_at_ms: h.created_at_ms,
                        asr_text: h.asr_text,
                        final_text: h.final_text,
                        template_id: h.template_id,
                    })
                    .collect();
                span.ok(Some(serde_json::json!({
                    "items": history.len(),
                    "min_ms": min_ms,
                })));
                history
            }
            Err(e) => {
                span.err("io", "E_HISTORY_LIST", &e.to_string(), None);
                Vec::new()
            }
        }
    }

    #[cfg(windows)]
    fn load_clipboard_text_best_effort(
        &self,
        win: &crate::context_capture_windows::WindowsContext,
        data_dir: &Path,
        task_id: Option<&str>,
        cfg: &ContextConfig,
    ) -> Option<String> {
        if !cfg.include_clipboard {
            return None;
        }

        let span = Span::start(
            data_dir,
            task_id,
            "ContextCapture",
            "CTX.clipboard.read",
            None,
        );
        let clip = win.read_clipboard_text_diag_best_effort();
        match clip.diag.status.as_str() {
            "ok" => span.ok(Some(serde_json::json!({
                "bytes": clip.text.as_deref().map(|s| s.len()).unwrap_or(0)
            }))),
            "skipped" => span.skipped(
                clip.diag.note.as_deref().unwrap_or("skipped"),
                Some(serde_json::json!({
                    "step": clip.diag.step,
                    "last_error": clip.diag.last_error,
                })),
            ),
            _ => span.err(
                "winapi",
                "E_CLIPBOARD",
                clip.diag.note.as_deref().unwrap_or("clipboard read failed"),
                Some(serde_json::json!({
                    "step": clip.diag.step,
                    "last_error": clip.diag.last_error,
                })),
            ),
        }
        clip.text
    }

    #[cfg(windows)]
    pub fn capture_hotkey_context_now(
        &self,
        data_dir: &Path,
        cfg: &ContextConfig,
    ) -> Result<String> {
        let span = Span::start(
            data_dir,
            None,
            "ContextCapture",
            "CTX.hotkey_capture_now",
            Some(serde_json::json!({
                "include_prev_window_meta": cfg.include_prev_window_meta,
                "include_focused_app_meta": cfg.include_focused_app_meta,
                "include_focused_element_meta": cfg.include_focused_element_meta,
                "include_input_state": cfg.include_input_state,
                "include_related_content": cfg.include_related_content,
                "include_visible_text": cfg.include_visible_text,
            })),
        );

        let captured_at_ms = now_ms();
        let mut g = self.inner.lock().unwrap();
        let mut snapshot = self.capture_runtime_snapshot(&g.win, cfg, true);
        snapshot.recent_history =
            self.load_recent_history_best_effort(data_dir, None, cfg, captured_at_ms);
        snapshot.clipboard_text = self.load_clipboard_text_best_effort(&g.win, data_dir, None, cfg);
        let capture_id = Uuid::new_v4().to_string();
        g.hotkey_capture_registry.insert(
            capture_id.clone(),
            StoredHotkeyCapture {
                snapshot: snapshot.clone(),
            },
        );

        span.ok(Some(serde_json::json!({
            "capture_id": capture_id,
            "has_focused_app": snapshot.focused_app.is_some(),
            "has_focused_element": snapshot.focused_element.is_some(),
            "has_input_state": snapshot.input_state.is_some(),
            "has_visible_text": snapshot.visible_text.is_some(),
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
                "include_focused_app_meta": cfg.include_focused_app_meta,
                "include_focused_element_meta": cfg.include_focused_element_meta,
                "include_input_state": cfg.include_input_state,
                "include_related_content": cfg.include_related_content,
                "include_visible_text": cfg.include_visible_text,
                "capture_mode": cfg.rules.capture_mode,
            })),
        );

        let mut snap = ContextSnapshot::default();

        snap.recent_history =
            self.load_recent_history_best_effort(data_dir, Some(task_id), cfg, captured_at_ms);

        #[cfg(windows)]
        {
            let g = self.inner.lock().unwrap();
            snap = self.capture_runtime_snapshot(&g.win, cfg, true);
            snap.clipboard_text =
                self.load_clipboard_text_best_effort(&g.win, data_dir, Some(task_id), cfg);
        }

        obs::event(
            data_dir,
            Some(task_id),
            "ContextCapture",
            "CTX.capture_snapshot.summary",
            "ok",
            Some(serde_json::json!({
                "history_items": snap.recent_history.len(),
                "clipboard_bytes": snap.clipboard_text.as_deref().map(|s| s.len()).unwrap_or(0),
                "has_focused_app": snap.focused_app.is_some(),
                "has_focused_element": snap.focused_element.is_some(),
                "has_input_state": snap.input_state.is_some(),
                "has_visible_text": snap.visible_text.is_some(),
            })),
        );
        _span_all.ok(None);

        snap
    }

    #[cfg(windows)]
    fn capture_runtime_snapshot(
        &self,
        win: &crate::context_capture_windows::WindowsContext,
        cfg: &ContextConfig,
        allow_last_external: bool,
    ) -> ContextSnapshot {
        let mut captured = win.capture_foreground_text_context_best_effort(cfg);
        let self_process = process_basename(
            captured
                .focused_app
                .as_ref()
                .and_then(|v| v.process_image.as_deref()),
        );
        if allow_last_external && matches!(self_process.as_deref(), Some("typevoice-desktop.exe")) {
            if let Some(last_external) =
                win.capture_last_external_text_context_best_effort(cfg, 5_000)
            {
                captured = last_external;
            }
        }

        let policy = resolve_policy(
            cfg,
            captured
                .focused_app
                .as_ref()
                .and_then(|v| v.process_image.as_deref()),
            captured.focused_app.as_ref().and_then(|v| v.url.as_deref()),
        );

        let mut snap = ContextSnapshot {
            recent_history: Vec::new(),
            clipboard_text: None,
            focused_app: if cfg.include_focused_app_meta {
                captured.focused_app
            } else {
                None
            },
            focused_window: if cfg.include_prev_window_meta {
                captured.focused_window
            } else {
                None
            },
            focused_element: if cfg.include_focused_element_meta {
                captured.focused_element
            } else {
                None
            },
            input_state: if cfg.include_input_state {
                captured.input_state
            } else {
                None
            },
            related_content: if cfg.include_related_content && policy.allow_related_content {
                captured.related_content
            } else {
                None
            },
            visible_text: if cfg.include_visible_text && policy.allow_visible_text {
                captured.visible_text
            } else {
                None
            },
            policy_decision: Some(ContextPolicyDecision {
                capture_mode: cfg.rules.capture_mode.clone(),
                app_rule: policy.app_rule.clone(),
                domain_rule: policy.domain_rule.clone(),
                allow_related_content: policy.allow_related_content,
                allow_visible_text: policy.allow_visible_text,
            }),
            capture_diag: captured.capture_diag.or_else(|| {
                Some(ContextCaptureDiag {
                    target_source: Some("foreground".to_string()),
                    target_age_ms: Some(0),
                    focus_stable: false,
                })
            }),
        };

        if snap.focused_window.is_none() {
            snap.focused_window = snap.focused_app.as_ref().map(|app| FocusedWindowInfo {
                title: app.window_title.clone(),
                class_name: None,
            });
        }
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::{config_from_settings, extract_hostname, resolve_policy, ContextConfig};
    use crate::settings::Settings;

    #[test]
    fn config_from_settings_honors_separate_related_limits() {
        let settings = Settings {
            context_related_before_chars: Some(111),
            context_related_after_chars: Some(37),
            ..Default::default()
        };

        let cfg = config_from_settings(&settings);

        assert_eq!(cfg.budget.max_chars_related_before, 111);
        assert_eq!(cfg.budget.max_chars_related_after, 37);
    }

    #[test]
    fn extract_hostname_accepts_scheme_less_browser_values() {
        assert_eq!(
            extract_hostname(Some("mail.google.com/mail/u/0/#inbox")),
            Some("mail.google.com".to_string())
        );
    }

    #[test]
    fn resolve_policy_prioritizes_deny_over_allow() {
        let mut cfg = ContextConfig::default();
        cfg.rules.app_allowlist = vec!["chrome.exe".to_string()];
        cfg.rules.domain_denylist = vec!["mail.google.com".to_string()];

        let policy = resolve_policy(
            &cfg,
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            Some("https://mail.google.com/mail/u/0/#inbox"),
        );

        assert_eq!(policy.app_rule.as_deref(), Some("allow"));
        assert_eq!(policy.domain_rule.as_deref(), Some("deny"));
        assert!(!policy.allow_related_content);
        assert!(!policy.allow_visible_text);
    }
}
