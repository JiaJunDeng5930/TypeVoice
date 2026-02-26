use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::trace::Span;

pub const DEFAULT_ASR_PROVIDER: &str = "local";
pub const DEFAULT_REMOTE_ASR_URL: &str = "http://api.server/transcribe";
pub const DEFAULT_REMOTE_ASR_CONCURRENCY: usize = 4;
pub const MAX_REMOTE_ASR_CONCURRENCY: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub asr_model: Option<String>,    // local dir or HF repo id
    pub asr_provider: Option<String>, // local|remote
    pub remote_asr_url: Option<String>,
    pub remote_asr_model: Option<String>,
    pub remote_asr_concurrency: Option<u64>,
    pub asr_preprocess_silence_trim_enabled: Option<bool>,
    pub asr_preprocess_silence_threshold_db: Option<f64>,
    pub asr_preprocess_silence_start_ms: Option<u64>,
    pub asr_preprocess_silence_end_ms: Option<u64>,

    // LLM settings (non-sensitive). API key is stored in OS keyring.
    pub llm_base_url: Option<String>, // e.g. https://api.openai.com/v1
    pub llm_model: Option<String>,    // e.g. gpt-4o-mini
    pub llm_reasoning_effort: Option<String>, // e.g. none|minimal|low|medium|high|xhigh

    // UX settings
    pub record_input_spec: Option<String>, // ffmpeg dshow input spec, e.g. audio=default
    pub rewrite_enabled: Option<bool>,
    pub rewrite_template_id: Option<String>,
    pub rewrite_glossary: Option<Vec<String>>,
    pub auto_paste_enabled: Option<bool>,

    // Context settings (for LLM rewrite)
    pub context_include_prev_window_meta: Option<bool>,
    pub context_include_history: Option<bool>,
    pub context_history_n: Option<i64>,
    pub context_history_window_ms: Option<i64>,
    pub context_include_clipboard: Option<bool>,
    pub context_include_prev_window_screenshot: Option<bool>,
    pub rewrite_include_glossary: Option<bool>,
    pub llm_supports_vision: Option<bool>,

    // Hotkeys / overlay (post-MVP)
    pub hotkeys_enabled: Option<bool>,
    pub hotkey_ptt: Option<String>,
    pub hotkey_toggle: Option<String>,
    pub hotkeys_show_overlay: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsPatch {
    // Outer Option: whether to update this field.
    // Inner Option: Some(value)=set, None=clear.
    pub asr_model: Option<Option<String>>,
    pub asr_provider: Option<Option<String>>,
    pub remote_asr_url: Option<Option<String>>,
    pub remote_asr_model: Option<Option<String>>,
    pub remote_asr_concurrency: Option<Option<u64>>,
    pub asr_preprocess_silence_trim_enabled: Option<Option<bool>>,
    pub asr_preprocess_silence_threshold_db: Option<Option<f64>>,
    pub asr_preprocess_silence_start_ms: Option<Option<u64>>,
    pub asr_preprocess_silence_end_ms: Option<Option<u64>>,

    pub llm_base_url: Option<Option<String>>,
    pub llm_model: Option<Option<String>>,
    pub llm_reasoning_effort: Option<Option<String>>,

    pub record_input_spec: Option<Option<String>>,
    pub rewrite_enabled: Option<Option<bool>>,
    pub rewrite_template_id: Option<Option<String>>,
    pub rewrite_glossary: Option<Option<Vec<String>>>,
    pub auto_paste_enabled: Option<Option<bool>>,

    pub context_include_history: Option<Option<bool>>,
    pub context_history_n: Option<Option<i64>>,
    pub context_history_window_ms: Option<Option<i64>>,
    pub context_include_clipboard: Option<Option<bool>>,
    pub context_include_prev_window_screenshot: Option<Option<bool>>,
    pub context_include_prev_window_meta: Option<Option<bool>>,
    pub rewrite_include_glossary: Option<Option<bool>>,
    pub llm_supports_vision: Option<Option<bool>>,

    pub hotkeys_enabled: Option<Option<bool>>,
    pub hotkey_ptt: Option<Option<String>>,
    pub hotkey_toggle: Option<Option<String>>,
    pub hotkeys_show_overlay: Option<Option<bool>>,
}

pub fn apply_patch(mut s: Settings, p: SettingsPatch) -> Settings {
    if let Some(v) = p.asr_model {
        s.asr_model = v;
    }
    if let Some(v) = p.asr_provider {
        s.asr_provider = v;
    }
    if let Some(v) = p.remote_asr_url {
        s.remote_asr_url = v;
    }
    if let Some(v) = p.remote_asr_model {
        s.remote_asr_model = v;
    }
    if let Some(v) = p.remote_asr_concurrency {
        s.remote_asr_concurrency = v;
    }
    if let Some(v) = p.asr_preprocess_silence_trim_enabled {
        s.asr_preprocess_silence_trim_enabled = v;
    }
    if let Some(v) = p.asr_preprocess_silence_threshold_db {
        s.asr_preprocess_silence_threshold_db = v;
    }
    if let Some(v) = p.asr_preprocess_silence_start_ms {
        s.asr_preprocess_silence_start_ms = v;
    }
    if let Some(v) = p.asr_preprocess_silence_end_ms {
        s.asr_preprocess_silence_end_ms = v;
    }
    if let Some(v) = p.llm_base_url {
        s.llm_base_url = v;
    }
    if let Some(v) = p.llm_model {
        s.llm_model = v;
    }
    if let Some(v) = p.llm_reasoning_effort {
        s.llm_reasoning_effort = v;
    }
    if let Some(v) = p.record_input_spec {
        s.record_input_spec = v;
    }
    if let Some(v) = p.rewrite_enabled {
        s.rewrite_enabled = v;
    }
    if let Some(v) = p.rewrite_template_id {
        s.rewrite_template_id = v;
    }
    if let Some(v) = p.rewrite_glossary {
        s.rewrite_glossary = v;
    }
    if let Some(v) = p.auto_paste_enabled {
        s.auto_paste_enabled = v;
    }
    if let Some(v) = p.context_include_history {
        s.context_include_history = v;
    }
    if let Some(v) = p.context_history_n {
        s.context_history_n = v;
    }
    if let Some(v) = p.context_history_window_ms {
        s.context_history_window_ms = v;
    }
    if let Some(v) = p.context_include_clipboard {
        s.context_include_clipboard = v;
    }
    if let Some(v) = p.context_include_prev_window_screenshot {
        s.context_include_prev_window_screenshot = v;
    }
    if let Some(v) = p.context_include_prev_window_meta {
        s.context_include_prev_window_meta = v;
    }
    if let Some(v) = p.rewrite_include_glossary {
        s.rewrite_include_glossary = v;
    }
    if let Some(v) = p.llm_supports_vision {
        s.llm_supports_vision = v;
    }
    if let Some(v) = p.hotkeys_enabled {
        s.hotkeys_enabled = v;
    }
    if let Some(v) = p.hotkey_ptt {
        s.hotkey_ptt = v;
    }
    if let Some(v) = p.hotkey_toggle {
        s.hotkey_toggle = v;
    }
    if let Some(v) = p.hotkeys_show_overlay {
        s.hotkeys_show_overlay = v;
    }
    s
}

pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

pub fn load_settings(data_dir: &Path) -> Result<Settings> {
    let p = settings_path(data_dir);
    if !p.exists() {
        return Ok(Settings::default());
    }
    let s = fs::read_to_string(&p).context("read settings.json failed")?;
    let v: Settings = serde_json::from_str(&s).context("parse settings.json failed")?;
    Ok(v)
}

pub fn load_settings_strict(data_dir: &Path) -> Result<Settings> {
    let p = settings_path(data_dir);
    if !p.exists() {
        return Err(anyhow!(
            "E_SETTINGS_NOT_FOUND: settings.json not found at {}",
            p.display()
        ));
    }
    let s = fs::read_to_string(&p).context("read settings.json failed")?;
    let v: Settings = serde_json::from_str(&s).context("parse settings.json failed")?;
    Ok(v)
}

pub fn resolve_rewrite_start_config(s: &Settings) -> Result<(bool, Option<String>)> {
    let rewrite_enabled = s.rewrite_enabled.ok_or_else(|| {
        anyhow!("E_SETTINGS_REWRITE_ENABLED_MISSING: rewrite_enabled is required in settings")
    })?;
    let template_id = s
        .rewrite_template_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    if rewrite_enabled && template_id.is_none() {
        return Err(anyhow!(
            "E_SETTINGS_REWRITE_TEMPLATE_MISSING: rewrite_template_id is required when rewrite_enabled=true"
        ));
    }
    Ok((rewrite_enabled, template_id))
}

pub fn resolve_auto_paste_enabled(s: &Settings) -> bool {
    s.auto_paste_enabled.unwrap_or(true)
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyConfigResolved {
    pub enabled: bool,
    pub ptt: Option<String>,
    pub toggle: Option<String>,
}

pub fn resolve_hotkey_config(s: &Settings) -> Result<HotkeyConfigResolved> {
    let enabled = s.hotkeys_enabled.ok_or_else(|| {
        anyhow!("E_SETTINGS_HOTKEYS_ENABLED_MISSING: hotkeys_enabled is required in settings")
    })?;
    if !enabled {
        return Ok(HotkeyConfigResolved {
            enabled: false,
            ptt: None,
            toggle: None,
        });
    }

    let ptt = s
        .hotkey_ptt
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("E_SETTINGS_HOTKEY_PTT_MISSING: hotkey_ptt is required"))?
        .to_string();

    let toggle = s
        .hotkey_toggle
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("E_SETTINGS_HOTKEY_TOGGLE_MISSING: hotkey_toggle is required"))?
        .to_string();

    if ptt.eq_ignore_ascii_case(&toggle) {
        return Err(anyhow!(
            "E_SETTINGS_HOTKEY_CONFLICT: hotkey_ptt and hotkey_toggle must be different"
        ));
    }

    Ok(HotkeyConfigResolved {
        enabled: true,
        ptt: Some(ptt),
        toggle: Some(toggle),
    })
}

pub fn resolve_record_input_spec(s: &Settings) -> String {
    s.record_input_spec
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("audio=default")
        .to_string()
}

pub fn save_settings(data_dir: &Path, settings: &Settings) -> Result<()> {
    let span = Span::start(data_dir, None, "Settings", "SETTINGS.save", None);
    std::fs::create_dir_all(data_dir).context("create data dir failed")?;
    let p = settings_path(data_dir);
    let s = serde_json::to_string_pretty(settings).context("serialize settings failed")?;
    if let Err(e) = fs::write(&p, s) {
        let ae = anyhow::anyhow!("write settings.json failed: {e}");
        span.err_anyhow("io", "E_SETTINGS_WRITE", &ae, None);
        return Err(ae);
    }
    span.ok(None);
    Ok(())
}

pub fn resolve_asr_provider(s: &Settings) -> String {
    let value = s
        .asr_provider
        .as_deref()
        .map(str::trim)
        .unwrap_or(DEFAULT_ASR_PROVIDER)
        .to_ascii_lowercase();
    if value == "remote" {
        "remote".to_string()
    } else {
        "local".to_string()
    }
}

pub fn resolve_remote_asr_url(s: &Settings) -> String {
    s.remote_asr_url
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_REMOTE_ASR_URL)
        .to_string()
}

pub fn resolve_remote_asr_model(s: &Settings) -> Option<String> {
    s.remote_asr_model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

pub fn resolve_remote_asr_concurrency(s: &Settings) -> usize {
    let raw = s
        .remote_asr_concurrency
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_REMOTE_ASR_CONCURRENCY);
    raw.clamp(1, MAX_REMOTE_ASR_CONCURRENCY)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_patch, resolve_asr_provider, resolve_remote_asr_concurrency,
        resolve_remote_asr_model, resolve_remote_asr_url, Settings, SettingsPatch,
        DEFAULT_REMOTE_ASR_URL,
    };

    #[test]
    fn apply_patch_is_partial_and_can_clear() {
        let base = Settings {
            asr_model: Some("asr".to_string()),
            asr_provider: Some("local".to_string()),
            remote_asr_url: None,
            remote_asr_model: None,
            remote_asr_concurrency: None,
            llm_base_url: Some("https://x/v1".to_string()),
            llm_model: Some("m1".to_string()),
            llm_reasoning_effort: Some("low".to_string()),
            record_input_spec: None,
            rewrite_enabled: Some(false),
            rewrite_template_id: Some("t1".to_string()),
            rewrite_glossary: None,
            auto_paste_enabled: Some(true),
            context_include_history: None,
            context_history_n: None,
            context_history_window_ms: None,
            context_include_prev_window_meta: None,
            context_include_clipboard: None,
            context_include_prev_window_screenshot: None,
            rewrite_include_glossary: None,
            llm_supports_vision: None,
            hotkeys_enabled: None,
            hotkey_ptt: None,
            hotkey_toggle: None,
            hotkeys_show_overlay: None,
            ..Default::default()
        };

        let p = SettingsPatch {
            asr_provider: Some(Some("remote".to_string())),
            remote_asr_url: Some(Some("http://127.0.0.1:8317/transcribe".to_string())),
            remote_asr_model: Some(Some("whisper-1".to_string())),
            remote_asr_concurrency: Some(Some(6)),
            llm_model: Some(Some("m2".to_string())),
            llm_reasoning_effort: Some(None),
            rewrite_enabled: Some(Some(true)),
            rewrite_template_id: Some(None),
            context_history_n: Some(Some(5)),
            context_include_prev_window_meta: Some(Some(true)),
            rewrite_include_glossary: Some(Some(false)),
            auto_paste_enabled: Some(Some(false)),
            ..Default::default()
        };

        let next = apply_patch(base, p);
        assert_eq!(next.asr_model.as_deref(), Some("asr"));
        assert_eq!(next.asr_provider.as_deref(), Some("remote"));
        assert_eq!(
            next.remote_asr_url.as_deref(),
            Some("http://127.0.0.1:8317/transcribe")
        );
        assert_eq!(next.remote_asr_model.as_deref(), Some("whisper-1"));
        assert_eq!(next.remote_asr_concurrency, Some(6));
        assert_eq!(next.llm_base_url.as_deref(), Some("https://x/v1"));
        assert_eq!(next.llm_model.as_deref(), Some("m2"));
        assert_eq!(next.llm_reasoning_effort, None);
        assert_eq!(next.rewrite_enabled, Some(true));
        assert_eq!(next.rewrite_template_id, None);
        assert_eq!(next.rewrite_glossary.as_deref(), None);
        assert_eq!(next.auto_paste_enabled, Some(false));
        assert_eq!(next.context_history_n, Some(5));
        assert_eq!(next.context_include_prev_window_meta, Some(true));
        assert_eq!(next.rewrite_include_glossary, Some(false));
    }

    #[test]
    fn resolve_remote_asr_fields_apply_defaults_and_clamp() {
        let s = Settings::default();
        assert_eq!(resolve_asr_provider(&s), "local");
        assert_eq!(resolve_remote_asr_url(&s), DEFAULT_REMOTE_ASR_URL);
        assert_eq!(resolve_remote_asr_model(&s), None);
        assert_eq!(resolve_remote_asr_concurrency(&s), 4);

        let s = Settings {
            asr_provider: Some("REMOTE".to_string()),
            remote_asr_url: Some(" http://localhost/transcribe ".to_string()),
            remote_asr_model: Some(" whisper-1 ".to_string()),
            remote_asr_concurrency: Some(100),
            ..Default::default()
        };
        assert_eq!(resolve_asr_provider(&s), "remote");
        assert_eq!(resolve_remote_asr_url(&s), "http://localhost/transcribe");
        assert_eq!(resolve_remote_asr_model(&s).as_deref(), Some("whisper-1"));
        assert_eq!(resolve_remote_asr_concurrency(&s), 16);
    }
}
