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
    allow_input_state: bool,
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

fn is_browser_basename(name: &str) -> bool {
    matches!(
        name,
        "chrome.exe"
            | "msedge.exe"
            | "firefox.exe"
            | "brave.exe"
            | "chromium.exe"
            | "vivaldi.exe"
            | "opera.exe"
            | "opera_gx.exe"
            | "arc.exe"
    )
}

pub(crate) fn is_browser_process(process_image: Option<&str>) -> bool {
    process_basename(process_image)
        .map(|name| is_browser_basename(name.as_str()))
        .unwrap_or(false)
}

fn is_browser_window_class(class_name: Option<&str>) -> bool {
    class_name
        .map(|name| {
            let normalized = name.trim().to_ascii_lowercase();
            normalized.starts_with("chrome_widgetwin_")
                || normalized == "mozillawindowclass"
                || normalized == "mozilla_dialog_class"
        })
        .unwrap_or(false)
}

pub(crate) fn is_browser_target(
    process_image: Option<&str>,
    window_class: Option<&str>,
    url: Option<&str>,
) -> bool {
    is_browser_process(process_image)
        || is_browser_window_class(window_class)
        || url.map(|value| !value.trim().is_empty()).unwrap_or(false)
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
    window_class: Option<&str>,
    url: Option<&str>,
) -> PolicyResolution {
    let app_name = process_basename(process_image);
    let host = extract_hostname(url);
    let requires_domain_resolution = is_browser_target(process_image, window_class, url)
        && host.is_none()
        && (!cfg.rules.domain_allowlist.is_empty() || !cfg.rules.domain_denylist.is_empty());
    let app_rule = if let Some(name) = app_name.as_deref() {
        if cfg.rules.app_denylist.iter().any(|v| v == name) {
            Some("deny".to_string())
        } else if cfg.rules.app_allowlist.iter().any(|v| v == name) {
            Some("allow".to_string())
        } else {
            None
        }
    } else {
        None
    };
    let domain_rule = if let Some(name) = host.as_deref() {
        if cfg
            .rules
            .domain_denylist
            .iter()
            .any(|v| name == v || name.ends_with(&format!(".{v}")))
        {
            Some("deny".to_string())
        } else if cfg
            .rules
            .domain_allowlist
            .iter()
            .any(|v| name == v || name.ends_with(&format!(".{v}")))
        {
            Some("allow".to_string())
        } else {
            None
        }
    } else if requires_domain_resolution {
        Some("deny".to_string())
    } else {
        None
    };

    let denied = matches!(app_rule.as_deref(), Some("deny"))
        || matches!(domain_rule.as_deref(), Some("deny"));
    let allowed = matches!(app_rule.as_deref(), Some("allow"))
        || matches!(domain_rule.as_deref(), Some("allow"));
    let (default_allow_input_state, default_allow_related_content, default_allow_visible_text) =
        match cfg.rules.capture_mode.as_str() {
            "minimal" => (false, false, false),
            "full" => (true, true, true),
            _ => (true, true, false),
        };

    let allow_input_state = if denied {
        false
    } else if allowed {
        true
    } else {
        default_allow_input_state
    };
    let allow_related_content = if denied {
        false
    } else if allowed {
        true
    } else {
        default_allow_related_content
    };
    let allow_visible_text = if denied {
        false
    } else if allowed {
        true
    } else {
        default_allow_visible_text
    };

    PolicyResolution {
        app_rule,
        domain_rule,
        allow_input_state,
        allow_related_content,
        allow_visible_text,
    }
}

fn metadata_probe_config(cfg: &ContextConfig) -> ContextConfig {
    let mut probe = cfg.clone();
    probe.include_input_state = false;
    probe.include_related_content = false;
    probe.include_visible_text = false;
    probe
}

fn pre_policy_last_external_config(cfg: &ContextConfig) -> ContextConfig {
    metadata_probe_config(cfg)
}

fn policy_capture_config(cfg: &ContextConfig, policy: &PolicyResolution) -> ContextConfig {
    let mut effective = cfg.clone();
    effective.include_input_state &= policy.allow_input_state;
    effective.include_related_content &= policy.allow_related_content;
    effective.include_visible_text &= policy.allow_visible_text;
    effective
}

fn merge_runtime_snapshot(mut snap: ContextSnapshot, runtime: ContextSnapshot) -> ContextSnapshot {
    snap.focused_app = runtime.focused_app;
    snap.focused_window = runtime.focused_window;
    snap.focused_element = runtime.focused_element;
    snap.input_state = runtime.input_state;
    snap.related_content = runtime.related_content;
    snap.visible_text = runtime.visible_text;
    snap.policy_decision = runtime.policy_decision;
    snap.capture_diag = runtime.capture_diag;
    snap
}

fn apply_focused_window_fallback(cfg: &ContextConfig, snap: &mut ContextSnapshot) {
    if !cfg.include_prev_window_meta || snap.focused_window.is_some() {
        return;
    }
    snap.focused_window = snap.focused_app.as_ref().map(|app| FocusedWindowInfo {
        title: app.window_title.clone(),
        class_name: None,
    });
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
            snap = merge_runtime_snapshot(snap, self.capture_runtime_snapshot(&g.win, cfg, true));
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
        let probe_cfg = metadata_probe_config(cfg);
        let mut captured = win.capture_foreground_text_context_best_effort(&probe_cfg);
        let self_process = process_basename(
            captured
                .focused_app
                .as_ref()
                .and_then(|v| v.process_image.as_deref()),
        );
        if allow_last_external && matches!(self_process.as_deref(), Some("typevoice-desktop.exe")) {
            if let Some(last_external) = win.capture_last_external_text_context_best_effort(
                &pre_policy_last_external_config(cfg),
                5_000,
            ) {
                captured = last_external;
            }
        }

        let policy = resolve_policy(
            cfg,
            captured
                .focused_app
                .as_ref()
                .and_then(|v| v.process_image.as_deref()),
            captured
                .focused_window
                .as_ref()
                .and_then(|v| v.class_name.as_deref()),
            captured.focused_app.as_ref().and_then(|v| v.url.as_deref()),
        );
        let effective_cfg = policy_capture_config(cfg, &policy);
        if effective_cfg.include_input_state
            || effective_cfg.include_related_content
            || effective_cfg.include_visible_text
        {
            let target_source = captured
                .capture_diag
                .as_ref()
                .and_then(|diag| diag.target_source.as_deref());
            let recaptured = match target_source {
                Some("last_external") => {
                    win.capture_last_external_text_context_best_effort(&effective_cfg, 5_000)
                }
                _ => Some(win.capture_foreground_text_context_best_effort(&effective_cfg)),
            };
            if let Some(with_text) = recaptured {
                captured = with_text;
            }
        }

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
            input_state: if cfg.include_input_state && policy.allow_input_state {
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
                allow_input_state: policy.allow_input_state,
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

        apply_focused_window_fallback(cfg, &mut snap);
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_focused_window_fallback, config_from_settings, extract_hostname,
        merge_runtime_snapshot, metadata_probe_config, policy_capture_config,
        pre_policy_last_external_config, resolve_policy, ContextConfig,
    };
    use crate::context_pack::{
        ContextCaptureDiag, ContextPolicyDecision, ContextSnapshot, FocusedAppInfo, HistorySnippet,
    };
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
    fn pre_policy_last_external_config_uses_metadata_only() {
        let cfg = ContextConfig::default();

        let probe_cfg = pre_policy_last_external_config(&cfg);

        assert!(!probe_cfg.include_input_state);
        assert!(!probe_cfg.include_related_content);
        assert!(!probe_cfg.include_visible_text);
        assert!(probe_cfg.include_focused_app_meta);
        assert!(probe_cfg.include_prev_window_meta);
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
            None,
            Some("https://mail.google.com/mail/u/0/#inbox"),
        );

        assert_eq!(policy.app_rule.as_deref(), Some("allow"));
        assert_eq!(policy.domain_rule.as_deref(), Some("deny"));
        assert!(!policy.allow_input_state);
        assert!(!policy.allow_related_content);
        assert!(!policy.allow_visible_text);
    }

    #[test]
    fn resolve_policy_denies_browser_capture_when_domain_rules_need_a_host() {
        let mut cfg = ContextConfig::default();
        cfg.rules.domain_denylist = vec!["mail.google.com".to_string()];

        let policy = resolve_policy(
            &cfg,
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            None,
            None,
        );

        assert_eq!(policy.domain_rule.as_deref(), Some("deny"));
        assert!(!policy.allow_input_state);
        assert!(!policy.allow_related_content);
        assert!(!policy.allow_visible_text);
    }

    #[test]
    fn resolve_policy_treats_brave_as_browser_for_domain_rules() {
        let mut cfg = ContextConfig::default();
        cfg.rules.domain_denylist = vec!["mail.google.com".to_string()];

        let policy = resolve_policy(
            &cfg,
            Some(r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"),
            None,
            None,
        );

        assert_eq!(policy.domain_rule.as_deref(), Some("deny"));
        assert!(!policy.allow_input_state);
        assert!(!policy.allow_related_content);
        assert!(!policy.allow_visible_text);
    }

    #[test]
    fn resolve_policy_prioritizes_same_dimension_deny_over_allow() {
        let mut cfg = ContextConfig::default();
        cfg.rules.app_allowlist = vec!["chrome.exe".to_string()];
        cfg.rules.app_denylist = vec!["chrome.exe".to_string()];
        cfg.rules.domain_allowlist = vec!["example.com".to_string()];
        cfg.rules.domain_denylist = vec!["example.com".to_string()];

        let policy = resolve_policy(
            &cfg,
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            None,
            Some("https://example.com/path"),
        );

        assert_eq!(policy.app_rule.as_deref(), Some("deny"));
        assert_eq!(policy.domain_rule.as_deref(), Some("deny"));
        assert!(!policy.allow_input_state);
        assert!(!policy.allow_related_content);
        assert!(!policy.allow_visible_text);
    }

    #[test]
    fn resolve_policy_distinguishes_minimal_balanced_and_full() {
        let mut minimal = ContextConfig::default();
        minimal.rules.capture_mode = "minimal".to_string();
        let minimal_policy = resolve_policy(&minimal, Some("notepad.exe"), None, None);
        assert!(!minimal_policy.allow_input_state);
        assert!(!minimal_policy.allow_related_content);
        assert!(!minimal_policy.allow_visible_text);

        let mut balanced = ContextConfig::default();
        balanced.rules.capture_mode = "balanced".to_string();
        let balanced_policy = resolve_policy(&balanced, Some("notepad.exe"), None, None);
        assert!(balanced_policy.allow_input_state);
        assert!(balanced_policy.allow_related_content);
        assert!(!balanced_policy.allow_visible_text);

        let mut full = ContextConfig::default();
        full.rules.capture_mode = "full".to_string();
        let full_policy = resolve_policy(&full, Some("notepad.exe"), None, None);
        assert!(full_policy.allow_input_state);
        assert!(full_policy.allow_related_content);
        assert!(full_policy.allow_visible_text);
    }

    #[test]
    fn metadata_probe_config_strips_text_bodies_but_keeps_metadata_flags() {
        let cfg = ContextConfig::default();
        let probe = metadata_probe_config(&cfg);

        assert!(probe.include_focused_app_meta);
        assert!(probe.include_prev_window_meta);
        assert!(probe.include_focused_element_meta);
        assert!(!probe.include_input_state);
        assert!(!probe.include_related_content);
        assert!(!probe.include_visible_text);
    }

    #[test]
    fn policy_capture_config_turns_off_denied_text_flags() {
        let cfg = ContextConfig::default();
        let policy = resolve_policy(
            &cfg,
            Some(r"C:\Program Files\Notepad++\notepad++.exe"),
            None,
            Some("https://example.com"),
        );
        let effective = policy_capture_config(&cfg, &policy);

        assert!(effective.include_input_state);
        assert!(effective.include_related_content);
        assert!(!effective.include_visible_text);

        let mut denied_cfg = ContextConfig::default();
        denied_cfg.rules.domain_denylist = vec!["example.com".to_string()];
        let denied_policy = resolve_policy(
            &denied_cfg,
            Some(r"C:\Program Files\Notepad++\notepad++.exe"),
            None,
            Some("https://example.com"),
        );
        let denied_effective = policy_capture_config(&denied_cfg, &denied_policy);

        assert!(!denied_effective.include_input_state);
        assert!(!denied_effective.include_related_content);
        assert!(!denied_effective.include_visible_text);
    }

    #[test]
    fn resolve_policy_denies_browser_capture_when_window_class_identifies_browser() {
        let mut cfg = ContextConfig::default();
        cfg.rules.domain_denylist = vec!["mail.google.com".to_string()];

        let policy = resolve_policy(&cfg, None, Some("Chrome_WidgetWin_1"), None);

        assert_eq!(policy.domain_rule.as_deref(), Some("deny"));
        assert!(!policy.allow_input_state);
        assert!(!policy.allow_related_content);
        assert!(!policy.allow_visible_text);
    }

    #[test]
    fn merge_runtime_snapshot_preserves_preloaded_history() {
        let snap = ContextSnapshot {
            recent_history: vec![HistorySnippet {
                created_at_ms: 1,
                asr_text: "hello".to_string(),
                final_text: "world".to_string(),
                template_id: None,
            }],
            ..Default::default()
        };
        let runtime = ContextSnapshot {
            focused_app: Some(FocusedAppInfo {
                process_image: Some("notepad.exe".to_string()),
                window_title: Some("note".to_string()),
                url: None,
                is_browser: false,
                target_source: Some("foreground".to_string()),
            }),
            policy_decision: Some(ContextPolicyDecision {
                capture_mode: "balanced".to_string(),
                app_rule: None,
                domain_rule: None,
                allow_input_state: true,
                allow_related_content: true,
                allow_visible_text: true,
            }),
            capture_diag: Some(ContextCaptureDiag {
                target_source: Some("foreground".to_string()),
                target_age_ms: Some(0),
                focus_stable: true,
            }),
            ..Default::default()
        };

        let merged = merge_runtime_snapshot(snap, runtime);

        assert_eq!(merged.recent_history.len(), 1);
        assert_eq!(
            merged
                .focused_app
                .as_ref()
                .and_then(|app| app.process_image.as_deref()),
            Some("notepad.exe")
        );
    }

    #[test]
    fn focused_window_fallback_respects_window_meta_toggle() {
        let cfg = ContextConfig {
            include_prev_window_meta: false,
            include_focused_app_meta: true,
            ..ContextConfig::default()
        };
        let mut snap = ContextSnapshot {
            focused_app: Some(FocusedAppInfo {
                process_image: Some("notepad.exe".to_string()),
                window_title: Some("note".to_string()),
                url: None,
                is_browser: false,
                target_source: Some("foreground".to_string()),
            }),
            focused_window: None,
            ..Default::default()
        };

        apply_focused_window_fallback(&cfg, &mut snap);

        assert!(cfg.include_focused_app_meta);
        assert!(!cfg.include_prev_window_meta);
        assert!(snap.focused_window.is_none());
    }
}
