#![cfg(windows)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use uiautomation::patterns::{UITextPattern, UIValuePattern};
use uiautomation::types::{ControlType, TextPatternRangeEndpoint, TreeScope};
use uiautomation::{UIAutomation, UIElement};
use url::Url;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HWND};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow,
};

use crate::context_capture::ContextConfig;
use crate::context_pack::{
    ContextCaptureDiag, FocusedAppInfo, FocusedElementInfo, FocusedWindowInfo, InputContext,
    RelatedContent,
};

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub title: Option<String>,
    pub process_image: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClipboardDiag {
    pub status: String, // ok|skipped|err
    pub step: Option<String>,
    pub last_error: Option<u32>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClipboardRead {
    pub text: Option<String>,
    pub diag: ClipboardDiag,
}

#[derive(Debug, Clone, Default)]
pub struct ForegroundTextContext {
    pub focused_app: Option<FocusedAppInfo>,
    pub focused_window: Option<FocusedWindowInfo>,
    pub focused_element: Option<FocusedElementInfo>,
    pub input_state: Option<InputContext>,
    pub related_content: Option<RelatedContent>,
    pub visible_text: Option<String>,
    pub capture_diag: Option<ContextCaptureDiag>,
}

#[derive(Clone)]
pub struct WindowsContext {
    tracker: ForegroundTracker,
}

impl WindowsContext {
    pub fn new() -> Self {
        Self {
            tracker: ForegroundTracker::new(),
        }
    }

    pub fn warmup_best_effort(&self) {
        self.tracker.ensure_started();
    }

    pub fn last_external_window_info_best_effort(&self) -> Option<WindowInfo> {
        self.tracker.ensure_started();
        let snap = self.tracker.last_external_snapshot();
        let hwnd = snap.hwnd? as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            return None;
        }
        Some(WindowInfo {
            title: snap.title.or_else(|| get_window_title_best_effort(hwnd)),
            process_image: snap
                .process_image
                .or_else(|| get_process_image_best_effort(snap.pid)),
        })
    }

    pub fn last_external_hwnd_best_effort(&self) -> Option<isize> {
        self.tracker.ensure_started();
        let snap = self.tracker.last_external_snapshot();
        let hwnd = snap.hwnd?;
        if unsafe { IsWindow(hwnd as HWND) } == 0 {
            return None;
        }
        Some(hwnd)
    }

    pub fn last_external_age_ms_best_effort(&self) -> Option<i64> {
        self.tracker.ensure_started();
        let snap = self.tracker.last_external_snapshot();
        snap.seen_at_ms.map(|seen| now_ms().saturating_sub(seen))
    }

    pub fn foreground_window_info_best_effort(&self) -> Option<WindowInfo> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
            return None;
        }
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 {
            return None;
        }
        Some(WindowInfo {
            title: get_window_title_best_effort(hwnd),
            process_image: get_process_image_best_effort(pid),
        })
    }

    pub fn read_clipboard_text_diag_best_effort(&self) -> ClipboardRead {
        match read_clipboard_text_diagnose() {
            Ok(Some(s)) => ClipboardRead {
                text: Some(s),
                diag: ClipboardDiag {
                    status: "ok".to_string(),
                    step: None,
                    last_error: None,
                    note: None,
                },
            },
            Ok(None) => ClipboardRead {
                text: None,
                diag: ClipboardDiag {
                    status: "skipped".to_string(),
                    step: None,
                    last_error: None,
                    note: Some("empty_or_unavailable".to_string()),
                },
            },
            Err(e) => ClipboardRead {
                text: None,
                diag: ClipboardDiag {
                    status: "err".to_string(),
                    step: Some(e.step),
                    last_error: Some(e.last_error),
                    note: Some(e.note),
                },
            },
        }
    }

    pub fn capture_foreground_text_context_best_effort(
        &self,
        cfg: &ContextConfig,
    ) -> ForegroundTextContext {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
            return ForegroundTextContext {
                capture_diag: Some(ContextCaptureDiag {
                    target_source: Some("foreground".to_string()),
                    target_age_ms: Some(0),
                    focus_stable: false,
                }),
                ..Default::default()
            };
        }

        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        self.capture_window_context_best_effort(hwnd, pid, "foreground", Some(0), true, cfg)
    }

    pub fn capture_last_external_text_context_best_effort(
        &self,
        cfg: &ContextConfig,
        max_age_ms: i64,
    ) -> Option<ForegroundTextContext> {
        self.tracker.ensure_started();
        let snap = self.tracker.last_external_snapshot();
        let hwnd = snap.hwnd? as HWND;
        let age_ms = snap
            .seen_at_ms
            .map(|seen| now_ms().saturating_sub(seen))
            .unwrap_or(i64::MAX);
        if age_ms > max_age_ms || unsafe { IsWindow(hwnd) } == 0 {
            return None;
        }
        Some(self.capture_window_context_best_effort(
            hwnd,
            snap.pid,
            "last_external",
            Some(age_ms),
            false,
            cfg,
        ))
    }

    fn capture_window_context_best_effort(
        &self,
        hwnd: HWND,
        pid: u32,
        target_source: &str,
        target_age_ms: Option<i64>,
        allow_focus_context: bool,
        cfg: &ContextConfig,
    ) -> ForegroundTextContext {
        let title = get_window_title_best_effort(hwnd);
        let window_class = get_window_class_best_effort(hwnd);
        let process_image = if pid != 0 {
            get_process_image_best_effort(pid)
        } else {
            None
        };
        let is_browser = crate::context_capture::is_browser_target(
            process_image.as_deref(),
            window_class.as_deref(),
            None,
        );
        let mut out = ForegroundTextContext {
            focused_app: Some(FocusedAppInfo {
                process_image: process_image.clone(),
                window_title: if cfg.include_focused_app_meta {
                    title.clone()
                } else {
                    None
                },
                url: None,
                is_browser,
                target_source: Some(target_source.to_string()),
            }),
            focused_window: if cfg.include_prev_window_meta {
                Some(FocusedWindowInfo {
                    title: title.clone(),
                    class_name: window_class.clone(),
                })
            } else {
                None
            },
            capture_diag: Some(ContextCaptureDiag {
                target_source: Some(target_source.to_string()),
                target_age_ms,
                focus_stable: false,
            }),
            ..Default::default()
        };

        let automation = match UIAutomation::new() {
            Ok(v) => v,
            Err(_) => return out,
        };
        let root = match automation.element_from_handle((hwnd as isize).into()) {
            Ok(v) => v,
            Err(_) => return out,
        };

        if let Some(url) = find_browser_url(&automation, &root) {
            if let Some(app) = out.focused_app.as_mut() {
                app.url = Some(url);
                app.is_browser = true;
            }
        }

        if cfg.include_visible_text {
            out.visible_text =
                collect_visible_text(&automation, &root, cfg.budget.max_chars_visible_text);
        }

        let (focus_stable, focused) =
            resolve_context_element(&automation, &root, allow_focus_context);
        if let Some(diag) = out.capture_diag.as_mut() {
            diag.focus_stable = focus_stable;
        }
        let Some(element) = focused else {
            return out;
        };
        if !element_belongs_to_window(&automation, &root, &element) {
            return out;
        }

        let element_info = describe_element(&element);
        let editable = element_info.editable;
        let element_info_for_output = element_info.clone();

        if cfg.include_focused_element_meta {
            out.focused_element = Some(element_info_for_output);
        }
        if !editable {
            return out;
        }

        if cfg.include_input_state {
            out.input_state = extract_input_state(&element, cfg.budget.max_chars_input);
        }
        if cfg.include_related_content {
            out.related_content = extract_related_content(
                &automation,
                &root,
                &element,
                cfg.budget.max_chars_related_before,
                cfg.budget.max_chars_related_after,
            );
        }
        out
    }
}

#[derive(Debug, Clone)]
struct ExternalSnapshot {
    hwnd: Option<isize>,
    pid: u32,
    process_image: Option<String>,
    title: Option<String>,
    seen_at_ms: Option<i64>,
}

#[derive(Clone)]
struct ForegroundTracker {
    started: Arc<AtomicBool>,
    last_external: Arc<Mutex<ExternalSnapshot>>,
}

impl ForegroundTracker {
    fn new() -> Self {
        Self {
            started: Arc::new(AtomicBool::new(false)),
            last_external: Arc::new(Mutex::new(ExternalSnapshot {
                hwnd: None,
                pid: 0,
                process_image: None,
                title: None,
                seen_at_ms: None,
            })),
        }
    }

    fn ensure_started(&self) {
        if self.started.load(Ordering::SeqCst) {
            return;
        }
        if self
            .started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let last_external = self.last_external.clone();
        let this_pid = std::process::id();
        std::thread::Builder::new()
            .name("foreground_tracker".to_string())
            .spawn(move || loop {
                let hwnd = unsafe { GetForegroundWindow() };
                if !hwnd.is_null() {
                    let mut pid: u32 = 0;
                    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
                    if pid != 0 && pid != this_pid {
                        let mut g = last_external.lock().unwrap();
                        g.hwnd = Some(hwnd as isize);
                        g.pid = pid;
                        g.process_image = get_process_image_best_effort(pid);
                        g.title = get_window_title_best_effort(hwnd);
                        g.seen_at_ms = Some(now_ms());
                    }
                }
                std::thread::sleep(Duration::from_millis(80));
            })
            .ok();
    }

    fn last_external_snapshot(&self) -> ExternalSnapshot {
        self.last_external.lock().unwrap().clone()
    }
}

fn stable_focused_element(automation: &UIAutomation) -> (bool, Option<UIElement>) {
    let mut last_key: Option<String> = None;
    let mut last_element: Option<UIElement> = None;
    for _ in 0..4 {
        let element = match automation.get_focused_element() {
            Ok(v) => v,
            Err(_) => return (false, last_element),
        };
        let runtime_id = element
            .get_runtime_id()
            .map(|v| {
                v.into_iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let control = element
            .get_control_type()
            .map(|v| format!("{v:?}"))
            .unwrap_or_default();
        let bounds = element
            .get_bounding_rectangle()
            .map(|v| format!("{v:?}"))
            .unwrap_or_default();
        let key = format!("{runtime_id}|{control}|{bounds}");
        if last_key.as_deref() == Some(key.as_str()) {
            return (true, Some(element));
        }
        last_key = Some(key);
        last_element = Some(element);
        std::thread::sleep(Duration::from_millis(30));
    }
    (false, last_element)
}

fn resolve_context_element(
    automation: &UIAutomation,
    root: &UIElement,
    allow_focus_context: bool,
) -> (bool, Option<UIElement>) {
    if allow_focus_context {
        let (focus_stable, focused) = stable_focused_element(automation);
        if let Some(element) = focused {
            if element_belongs_to_window(automation, root, &element) {
                return (focus_stable, Some(element));
            }
        }
    }

    (false, best_editable_descendant(automation, root))
}

fn best_editable_descendant(automation: &UIAutomation, root: &UIElement) -> Option<UIElement> {
    let condition = automation.create_true_condition().ok()?;
    let descendants = root.find_all(TreeScope::Descendants, &condition).ok()?;
    let mut best: Option<(i32, UIElement)> = None;
    for element in descendants {
        let element_info = describe_element(&element);
        if !element_info.editable {
            continue;
        }
        let score = context_element_candidate_score(
            element_info.role.as_deref(),
            element.has_keyboard_focus().unwrap_or(false),
            element.is_offscreen().unwrap_or(false),
            element_has_text(&element),
        );
        match &best {
            Some((best_score, _)) if *best_score >= score => {}
            _ => best = Some((score, element)),
        }
    }
    best.map(|(_, element)| element)
}

fn context_element_candidate_score(
    role: Option<&str>,
    has_keyboard_focus: bool,
    is_offscreen: bool,
    has_text: bool,
) -> i32 {
    let mut score = 0;
    if has_keyboard_focus {
        score += 100;
    }
    if !is_offscreen {
        score += 20;
    }
    if has_text {
        score += 15;
    }
    match role {
        Some("Document") => score += 12,
        Some("Edit") => score += 10,
        Some(_) | None => {}
    }
    score
}

fn element_has_text(element: &UIElement) -> bool {
    if let Ok(name) = element.get_name() {
        if !name.trim().is_empty() {
            return true;
        }
    }
    if let Ok(value_pattern) = element.get_pattern::<UIValuePattern>() {
        if let Ok(value) = value_pattern.get_value() {
            if !value.trim().is_empty() {
                return true;
            }
        }
    }
    false
}

fn element_belongs_to_window(
    automation: &UIAutomation,
    root: &UIElement,
    element: &UIElement,
) -> bool {
    if automation.compare_elements(root, element).unwrap_or(false) {
        return true;
    }
    let walker = match automation.create_tree_walker() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut current = element.clone();
    for _ in 0..64 {
        let parent = match walker.get_parent(&current) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if automation.compare_elements(&parent, root).unwrap_or(false) {
            return true;
        }
        current = parent;
    }
    false
}

fn describe_element(element: &UIElement) -> FocusedElementInfo {
    let role = element.get_control_type().ok().map(|v| format!("{v:?}"));
    let editable = role
        .as_deref()
        .map(|v| v == "Edit" || v == "Document")
        .unwrap_or(false)
        || element.get_pattern::<UITextPattern>().is_ok()
        || element
            .get_pattern::<UIValuePattern>()
            .ok()
            .and_then(|p| p.is_readonly().ok())
            .map(|v| !v)
            .unwrap_or(false);

    FocusedElementInfo {
        role,
        name: element.get_name().ok().filter(|v| !v.trim().is_empty()),
        class_name: element
            .get_classname()
            .ok()
            .filter(|v| !v.trim().is_empty()),
        automation_id: element
            .get_automation_id()
            .ok()
            .filter(|v| !v.trim().is_empty()),
        editable,
        has_keyboard_focus: element.has_keyboard_focus().unwrap_or(false),
    }
}

fn extract_input_state(element: &UIElement, max_chars: usize) -> Option<InputContext> {
    let text_pattern = match element.get_pattern::<UITextPattern>() {
        Ok(v) => v,
        Err(_) => {
            let value = element
                .get_pattern::<UIValuePattern>()
                .ok()
                .and_then(|p| p.get_value().ok())?;
            return input_context_from_value(value, max_chars);
        }
    };
    let document = text_pattern.get_document_range().ok()?;
    let full_text = document
        .get_text(max_chars as i32)
        .ok()
        .filter(|v| !v.trim().is_empty());

    let mut selection_text = None;
    let mut selection_start = None;
    let mut selection_end = None;
    if let Ok(mut selections) = text_pattern.get_selection() {
        if let Some(selection) = selections.pop() {
            selection_text = selection
                .get_text(max_chars as i32)
                .ok()
                .filter(|v| !v.trim().is_empty());
            selection_start = document
                .compare_endpoints(
                    TextPatternRangeEndpoint::Start,
                    &selection,
                    TextPatternRangeEndpoint::Start,
                )
                .ok();
            selection_end = document
                .compare_endpoints(
                    TextPatternRangeEndpoint::Start,
                    &selection,
                    TextPatternRangeEndpoint::End,
                )
                .ok();
        }
    }

    let (before_text, after_text) = if let Ok((_, caret)) = text_pattern.get_caret_range() {
        let before = document.clone();
        let after = document.clone();
        let _ = before.move_endpoint_by_range(
            TextPatternRangeEndpoint::End,
            &caret,
            TextPatternRangeEndpoint::Start,
        );
        let _ = after.move_endpoint_by_range(
            TextPatternRangeEndpoint::Start,
            &caret,
            TextPatternRangeEndpoint::End,
        );
        (
            before
                .get_text(max_chars as i32)
                .ok()
                .filter(|v| !v.trim().is_empty()),
            after
                .get_text(max_chars as i32)
                .ok()
                .filter(|v| !v.trim().is_empty()),
        )
    } else {
        (None, None)
    };

    Some(InputContext {
        selection_text,
        selection_start,
        selection_end,
        before_text,
        after_text,
        full_text,
    })
}

fn input_context_from_value(value: String, max_chars: usize) -> Option<InputContext> {
    let full_text = non_empty_trimmed(truncate_chars(value.trim(), max_chars));
    full_text.as_ref()?;
    Some(InputContext {
        selection_text: None,
        selection_start: None,
        selection_end: None,
        before_text: None,
        after_text: None,
        full_text,
    })
}

fn extract_related_content(
    automation: &UIAutomation,
    _window_root: &UIElement,
    focused: &UIElement,
    max_chars_before: usize,
    max_chars_after: usize,
) -> Option<RelatedContent> {
    let walker = automation.create_tree_walker().ok()?;
    let parent = walker.get_parent(focused).ok()?;
    let siblings = walker.get_children(&parent)?;
    let mut before = String::new();
    let mut after = String::new();
    let mut seen_focus = false;
    for sibling in siblings {
        let is_focus = automation
            .compare_elements(&sibling, focused)
            .unwrap_or(false);
        if is_focus {
            seen_focus = true;
            continue;
        }
        if seen_focus {
            let text = collect_element_text(&sibling, max_chars_after);
            if text.is_empty() {
                continue;
            }
            if !after.is_empty() {
                after.push('\n');
            }
            after.push_str(&text);
        } else {
            let text = collect_element_text(&sibling, max_chars_before);
            if text.is_empty() {
                continue;
            }
            if !before.is_empty() {
                before.push('\n');
            }
            before.push_str(&text);
        }
    }
    if before.trim().is_empty() && after.trim().is_empty() {
        return None;
    }
    Some(RelatedContent {
        before_text: non_empty_trimmed(before),
        after_text: non_empty_trimmed(after),
    })
}

fn collect_visible_text(
    automation: &UIAutomation,
    root: &UIElement,
    max_chars: usize,
) -> Option<String> {
    let condition = automation.create_true_condition().ok()?;
    let descendants = root.find_all(TreeScope::Descendants, &condition).ok()?;
    let mut out = String::new();
    for element in descendants {
        if out.chars().count() >= max_chars {
            break;
        }
        if element.is_offscreen().unwrap_or(false) {
            continue;
        }
        let text = collect_element_text(&element, 240);
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&text);
    }
    non_empty_trimmed(truncate_chars(&out, max_chars))
}

fn collect_element_text(element: &UIElement, max_chars: usize) -> String {
    let mut candidates = Vec::new();
    if let Ok(name) = element.get_name() {
        if !name.trim().is_empty() {
            candidates.push(name);
        }
    }
    if let Ok(value_pattern) = element.get_pattern::<UIValuePattern>() {
        if let Ok(value) = value_pattern.get_value() {
            if !value.trim().is_empty() {
                candidates.push(value);
            }
        }
    }
    for item in candidates {
        let trimmed = truncate_chars(item.trim(), max_chars);
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    String::new()
}

fn find_browser_url(automation: &UIAutomation, root: &UIElement) -> Option<String> {
    let condition = automation.create_true_condition().ok()?;
    let descendants = root.find_all(TreeScope::Descendants, &condition).ok()?;
    let mut best_match: Option<(usize, String)> = None;
    for element in descendants {
        let role = match element.get_control_type() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if role != ControlType::Edit {
            continue;
        }
        let value = element
            .get_pattern::<UIValuePattern>()
            .ok()
            .and_then(|p| p.get_value().ok())
            .unwrap_or_default();
        let Some(normalized) = normalize_browser_url_candidate(&value) else {
            continue;
        };
        let name = element.get_name().unwrap_or_default();
        let automation_id = element.get_automation_id().unwrap_or_default();
        let score = browser_url_field_score(&name, &automation_id);
        match &best_match {
            Some((best_score, _)) if *best_score >= score => {}
            _ => best_match = Some((score, normalized)),
        }
    }
    best_match.map(|(_, url)| url)
}

fn browser_url_field_score(name: &str, automation_id: &str) -> usize {
    let normalized_name = name.to_lowercase();
    let normalized_automation_id = automation_id.to_lowercase();
    let mut score = 1usize;
    for token in [
        "address",
        "search",
        "url",
        "\u{5730}\u{5740}",
        "\u{7f51}\u{5740}",
        "\u{641c}\u{7d22}",
        "\u{4f4f}\u{6240}",
        "\u{ad6c}\u{c18c}",
        "\u{30a2}\u{30c9}\u{30ec}\u{30b9}",
    ] {
        if normalized_name.contains(token) || normalized_automation_id.contains(token) {
            score += 4;
        }
    }
    if normalized_automation_id.contains("address")
        || normalized_automation_id.contains("search")
        || normalized_automation_id.contains("omnibox")
    {
        score += 6;
    }
    score
}

fn normalize_browser_url_candidate(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if Url::parse(trimmed)
        .ok()
        .and_then(|url| url.host_str().map(|_| ()))
        .is_some()
    {
        return Some(trimmed.to_string());
    }
    if trimmed.contains("://") || trimmed.contains(char::is_whitespace) {
        return None;
    }
    let normalized = format!("https://{trimmed}");
    Url::parse(&normalized)
        .ok()
        .and_then(|url| url.host_str().map(|_| normalized))
}

fn non_empty_trimmed(s: String) -> Option<String> {
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    s.chars().take(max_chars).collect()
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        browser_url_field_score, context_element_candidate_score, input_context_from_value,
        normalize_browser_url_candidate,
    };

    #[test]
    fn normalize_browser_url_candidate_accepts_scheme_less_host() {
        assert_eq!(
            normalize_browser_url_candidate("mail.google.com/mail/u/0/#inbox"),
            Some("https://mail.google.com/mail/u/0/#inbox".to_string())
        );
    }

    #[test]
    fn normalize_browser_url_candidate_rejects_search_text() {
        assert_eq!(normalize_browser_url_candidate("search query"), None);
    }

    #[test]
    fn browser_url_field_score_recognizes_localized_address_bar_names() {
        assert!(
            browser_url_field_score("\u{5730}\u{5740}\u{548c}\u{641c}\u{7d22}\u{680f}", "")
                > browser_url_field_score("message search", "")
        );
    }

    #[test]
    fn input_context_from_value_keeps_full_text_for_value_only_controls() {
        let ctx = input_context_from_value(" hello world ".to_string(), 5).unwrap();
        assert_eq!(ctx.full_text.as_deref(), Some("hello"));
        assert_eq!(ctx.selection_text, None);
        assert_eq!(ctx.before_text, None);
        assert_eq!(ctx.after_text, None);
    }

    #[test]
    fn context_element_candidate_score_prefers_visible_focused_editable_text() {
        let focused_score = context_element_candidate_score(Some("Edit"), true, false, true);
        let unfocused_score = context_element_candidate_score(Some("Edit"), false, false, true);
        let offscreen_score = context_element_candidate_score(Some("Edit"), false, true, true);

        assert!(focused_score > unfocused_score);
        assert!(unfocused_score > offscreen_score);
    }
}

fn get_window_title_best_effort(hwnd: HWND) -> Option<String> {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(String::from_utf16_lossy(&buf).trim().to_string())
}

fn get_window_class_best_effort(hwnd: HWND) -> Option<String> {
    let mut buf = vec![0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(String::from_utf16_lossy(&buf).trim().to_string())
}

fn get_process_image_best_effort(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 260];
        let mut size: u32 = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size);
        let _ = CloseHandle(h);
        if ok == 0 || size == 0 {
            return None;
        }
        buf.truncate(size as usize);
        Some(String::from_utf16_lossy(&buf).trim().to_string())
    }
}

#[derive(Debug, Clone)]
struct ClipboardDiagError {
    step: String,
    last_error: u32,
    note: String,
}

fn read_clipboard_text_diagnose() -> Result<Option<String>, ClipboardDiagError> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) == 0 {
            return Ok(None);
        }
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(ClipboardDiagError {
                step: "open_clipboard".to_string(),
                last_error: GetLastError(),
                note: "OpenClipboard failed".to_string(),
            });
        }
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle.is_null() {
            let _ = CloseClipboard();
            return Err(ClipboardDiagError {
                step: "get_clipboard_data".to_string(),
                last_error: GetLastError(),
                note: "GetClipboardData returned NULL".to_string(),
            });
        }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return Err(ClipboardDiagError {
                step: "global_lock".to_string(),
                last_error: GetLastError(),
                note: "GlobalLock returned NULL".to_string(),
            });
        }

        let mut len = 0usize;
        loop {
            let v = *ptr.add(len);
            if v == 0 {
                break;
            }
            len += 1;
            if len > 200_000 {
                break;
            }
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let s = String::from_utf16_lossy(slice).trim().to_string();
        let _ = GlobalUnlock(handle);
        let _ = CloseClipboard();
        if s.is_empty() {
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }
}
