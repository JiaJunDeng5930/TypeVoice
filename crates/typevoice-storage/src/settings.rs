use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::obs::Span;

pub const DEFAULT_ASR_PROVIDER: &str = "doubao";
pub const DEFAULT_REMOTE_ASR_URL: &str = "https://api.server/transcribe";
pub const DEFAULT_REMOTE_ASR_CONCURRENCY: usize = 4;
pub const MAX_REMOTE_ASR_CONCURRENCY: usize = 16;
pub const DEFAULT_OVERLAY_BACKGROUND_OPACITY: f64 = 0.78;
pub const DEFAULT_OVERLAY_FONT_SIZE_PX: u64 = 32;
pub const DEFAULT_OVERLAY_WIDTH_PX: u64 = 960;
pub const DEFAULT_OVERLAY_HEIGHT_PX: u64 = 160;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub asr_provider: Option<String>, // doubao|remote
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
    pub llm_prompt: Option<String>,

    // UX settings
    pub record_input_spec: Option<String>, // ffmpeg dshow input spec, e.g. audio=default
    pub record_input_strategy: Option<String>, // follow_default|fixed_device|auto_select
    pub record_follow_default_role: Option<String>, // communications|console
    pub record_fixed_endpoint_id: Option<String>,
    pub record_fixed_friendly_name: Option<String>,
    pub record_last_working_endpoint_id: Option<String>,
    pub record_last_working_friendly_name: Option<String>,
    pub record_last_working_dshow_spec: Option<String>,
    pub record_last_working_ts_ms: Option<i64>,
    pub rewrite_enabled: Option<bool>,
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
    pub hotkey_primary: Option<String>,
    pub hotkeys_show_overlay: Option<bool>,
    pub overlay_background_opacity: Option<f64>,
    pub overlay_font_size_px: Option<u64>,
    pub overlay_width_px: Option<u64>,
    pub overlay_height_px: Option<u64>,
    pub overlay_position_x: Option<i64>,
    pub overlay_position_y: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsPatch {
    // Outer Option: whether to update this field.
    // Inner Option: Some(value)=set, None=clear.
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
    pub llm_prompt: Option<Option<String>>,

    pub record_input_spec: Option<Option<String>>,
    pub record_input_strategy: Option<Option<String>>,
    pub record_follow_default_role: Option<Option<String>>,
    pub record_fixed_endpoint_id: Option<Option<String>>,
    pub record_fixed_friendly_name: Option<Option<String>>,
    pub rewrite_enabled: Option<Option<bool>>,
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
    pub hotkey_primary: Option<Option<String>>,
    pub hotkeys_show_overlay: Option<Option<bool>>,
    pub overlay_background_opacity: Option<Option<f64>>,
    pub overlay_font_size_px: Option<Option<u64>>,
    pub overlay_width_px: Option<Option<u64>>,
    pub overlay_height_px: Option<Option<u64>>,
    pub overlay_position_x: Option<Option<i64>>,
    pub overlay_position_y: Option<Option<i64>>,
}

pub fn apply_patch(mut s: Settings, p: SettingsPatch) -> Settings {
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
    if let Some(v) = p.llm_prompt {
        s.llm_prompt = v;
    }
    if let Some(v) = p.record_input_spec {
        s.record_input_spec = v;
    }
    if let Some(v) = p.record_input_strategy {
        s.record_input_strategy = v;
    }
    if let Some(v) = p.record_follow_default_role {
        s.record_follow_default_role = v;
    }
    if let Some(v) = p.record_fixed_endpoint_id {
        s.record_fixed_endpoint_id = v;
    }
    if let Some(v) = p.record_fixed_friendly_name {
        s.record_fixed_friendly_name = v;
    }
    if let Some(v) = p.rewrite_enabled {
        s.rewrite_enabled = v;
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
    if let Some(v) = p.hotkey_primary {
        s.hotkey_primary = v;
    }
    if let Some(v) = p.hotkeys_show_overlay {
        s.hotkeys_show_overlay = v;
    }
    if let Some(v) = p.overlay_background_opacity {
        s.overlay_background_opacity = v;
    }
    if let Some(v) = p.overlay_font_size_px {
        s.overlay_font_size_px = v;
    }
    if let Some(v) = p.overlay_width_px {
        s.overlay_width_px = v;
    }
    if let Some(v) = p.overlay_height_px {
        s.overlay_height_px = v;
    }
    if let Some(v) = p.overlay_position_x {
        s.overlay_position_x = v;
    }
    if let Some(v) = p.overlay_position_y {
        s.overlay_position_y = v;
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

pub fn resolve_auto_paste_enabled(s: &Settings) -> bool {
    s.auto_paste_enabled.unwrap_or(true)
}

#[derive(Debug, Clone, Serialize)]
pub struct HotkeyConfigResolved {
    pub enabled: bool,
    pub primary: String,
}

pub fn resolve_hotkey_config(s: &Settings) -> Result<HotkeyConfigResolved> {
    let enabled = s.hotkeys_enabled.ok_or_else(|| {
        anyhow!("E_SETTINGS_HOTKEYS_ENABLED_MISSING: hotkeys_enabled is required in settings")
    })?;
    if !enabled {
        return Ok(HotkeyConfigResolved {
            enabled: false,
            primary: "Alt".to_string(),
        });
    }

    Ok(HotkeyConfigResolved {
        enabled: true,
        primary: normalize_hotkey_primary(s.hotkey_primary.as_deref())?,
    })
}

pub fn normalize_hotkey_primary(raw: Option<&str>) -> Result<String> {
    let value = raw
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("Alt");
    let upper = value.to_ascii_uppercase();
    match upper.as_str() {
        "ALT" => Ok("Alt".to_string()),
        "CTRL" | "CONTROL" => Ok("Ctrl".to_string()),
        "SHIFT" => Ok("Shift".to_string()),
        "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12" => {
            Ok(upper)
        }
        _ => Err(anyhow!(
            "E_SETTINGS_HOTKEY_PRIMARY_INVALID: unsupported primary hotkey '{value}'"
        )),
    }
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
    } else if value == "doubao" {
        "doubao".to_string()
    } else {
        DEFAULT_ASR_PROVIDER.to_string()
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

#[derive(Debug, Clone, Serialize)]
pub struct OverlayConfigResolved {
    pub background_opacity: f64,
    pub font_size_px: u64,
    pub width_px: u64,
    pub height_px: u64,
    pub position_x: Option<i64>,
    pub position_y: Option<i64>,
}

pub fn resolve_overlay_config(s: &Settings) -> OverlayConfigResolved {
    OverlayConfigResolved {
        background_opacity: s
            .overlay_background_opacity
            .unwrap_or(DEFAULT_OVERLAY_BACKGROUND_OPACITY)
            .clamp(0.35, 0.95),
        font_size_px: s
            .overlay_font_size_px
            .unwrap_or(DEFAULT_OVERLAY_FONT_SIZE_PX)
            .clamp(18, 56),
        width_px: s
            .overlay_width_px
            .unwrap_or(DEFAULT_OVERLAY_WIDTH_PX)
            .clamp(360, 1600),
        height_px: s
            .overlay_height_px
            .unwrap_or(DEFAULT_OVERLAY_HEIGHT_PX)
            .clamp(72, 360),
        position_x: s.overlay_position_x,
        position_y: s.overlay_position_y,
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct OverlayWorkArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct OverlayPositionResolved {
    pub x: f64,
    pub y: f64,
}

pub fn resolve_overlay_position(
    config: &OverlayConfigResolved,
    work_areas: &[OverlayWorkArea],
) -> OverlayPositionResolved {
    let width = config.width_px as f64;
    let height = config.height_px as f64;
    let fallback = OverlayWorkArea {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    let area = select_overlay_work_area(config, work_areas).unwrap_or(fallback);
    let (raw_x, raw_y) = match (config.position_x, config.position_y) {
        (Some(x), Some(y)) => (x as f64, y as f64),
        _ => (
            area.x + (area.width - width) / 2.0,
            area.y + area.height - height - 96.0,
        ),
    };
    OverlayPositionResolved {
        x: raw_x.clamp(area.x, area.x + (area.width - width).max(0.0)),
        y: raw_y.clamp(area.y, area.y + (area.height - height).max(0.0)),
    }
}

fn select_overlay_work_area(
    config: &OverlayConfigResolved,
    work_areas: &[OverlayWorkArea],
) -> Option<OverlayWorkArea> {
    let saved = match (config.position_x, config.position_y) {
        (Some(x), Some(y)) => Some((x as f64, y as f64)),
        _ => None,
    };
    if let Some((x, y)) = saved {
        if let Some(area) = work_areas.iter().copied().find(|area| {
            x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
        }) {
            return Some(area);
        }
    }
    work_areas.first().copied()
}

#[cfg(test)]
mod tests {
    use super::{
        apply_patch, normalize_hotkey_primary, resolve_asr_provider, resolve_hotkey_config,
        resolve_overlay_config, resolve_overlay_position, resolve_remote_asr_concurrency,
        resolve_remote_asr_model, resolve_remote_asr_url, OverlayWorkArea, Settings, SettingsPatch,
        DEFAULT_REMOTE_ASR_URL,
    };

    #[test]
    fn apply_patch_is_partial_and_can_clear() {
        let base = Settings {
            asr_provider: Some("doubao".to_string()),
            remote_asr_url: None,
            remote_asr_model: None,
            remote_asr_concurrency: None,
            llm_base_url: Some("https://x/v1".to_string()),
            llm_model: Some("m1".to_string()),
            llm_reasoning_effort: Some("low".to_string()),
            llm_prompt: Some("prompt 1".to_string()),
            record_input_spec: None,
            rewrite_enabled: Some(false),
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
            hotkey_primary: None,
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
            llm_prompt: Some(Some("prompt 2".to_string())),
            rewrite_enabled: Some(Some(true)),
            context_history_n: Some(Some(5)),
            context_include_prev_window_meta: Some(Some(true)),
            rewrite_include_glossary: Some(Some(false)),
            auto_paste_enabled: Some(Some(false)),
            hotkey_primary: Some(Some("F9".to_string())),
            ..Default::default()
        };

        let next = apply_patch(base, p);
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
        assert_eq!(next.llm_prompt.as_deref(), Some("prompt 2"));
        assert_eq!(next.rewrite_enabled, Some(true));
        assert_eq!(next.rewrite_glossary.as_deref(), None);
        assert_eq!(next.auto_paste_enabled, Some(false));
        assert_eq!(next.hotkey_primary.as_deref(), Some("F9"));
        assert_eq!(next.context_history_n, Some(5));
        assert_eq!(next.context_include_prev_window_meta, Some(true));
        assert_eq!(next.rewrite_include_glossary, Some(false));
    }

    #[test]
    fn apply_patch_updates_overlay_fields() {
        let base = Settings {
            overlay_background_opacity: Some(0.5),
            overlay_font_size_px: Some(24),
            overlay_width_px: Some(800),
            overlay_height_px: Some(120),
            overlay_position_x: None,
            overlay_position_y: None,
            ..Default::default()
        };

        let next = apply_patch(
            base,
            SettingsPatch {
                overlay_background_opacity: Some(Some(0.82)),
                overlay_font_size_px: Some(Some(34)),
                overlay_width_px: Some(Some(1100)),
                overlay_height_px: Some(Some(180)),
                overlay_position_x: Some(Some(320)),
                overlay_position_y: Some(Some(720)),
                ..Default::default()
            },
        );

        assert_eq!(next.overlay_background_opacity, Some(0.82));
        assert_eq!(next.overlay_font_size_px, Some(34));
        assert_eq!(next.overlay_width_px, Some(1100));
        assert_eq!(next.overlay_height_px, Some(180));
        assert_eq!(next.overlay_position_x, Some(320));
        assert_eq!(next.overlay_position_y, Some(720));
    }

    #[test]
    fn resolve_overlay_config_applies_defaults_and_clamp() {
        let defaults = resolve_overlay_config(&Settings::default());
        assert_eq!(defaults.background_opacity, 0.78);
        assert_eq!(defaults.font_size_px, 32);
        assert_eq!(defaults.width_px, 960);
        assert_eq!(defaults.height_px, 160);
        assert_eq!(defaults.position_x, None);
        assert_eq!(defaults.position_y, None);

        let clamped = resolve_overlay_config(&Settings {
            overlay_background_opacity: Some(9.0),
            overlay_font_size_px: Some(200),
            overlay_width_px: Some(9999),
            overlay_height_px: Some(9999),
            overlay_position_x: Some(-40),
            overlay_position_y: Some(250),
            ..Default::default()
        });
        assert_eq!(clamped.background_opacity, 0.95);
        assert_eq!(clamped.font_size_px, 56);
        assert_eq!(clamped.width_px, 1600);
        assert_eq!(clamped.height_px, 360);
        assert_eq!(clamped.position_x, Some(-40));
        assert_eq!(clamped.position_y, Some(250));

        let clamped = resolve_overlay_config(&Settings {
            overlay_background_opacity: Some(0.1),
            overlay_font_size_px: Some(2),
            overlay_width_px: Some(200),
            overlay_height_px: Some(40),
            ..Default::default()
        });
        assert_eq!(clamped.background_opacity, 0.35);
        assert_eq!(clamped.font_size_px, 18);
        assert_eq!(clamped.width_px, 360);
        assert_eq!(clamped.height_px, 72);
    }

    #[test]
    fn resolve_overlay_position_uses_work_area_containing_saved_point() {
        let config = resolve_overlay_config(&Settings {
            overlay_width_px: Some(400),
            overlay_height_px: Some(120),
            overlay_position_x: Some(1920),
            overlay_position_y: Some(980),
            ..Default::default()
        });
        let areas = [
            OverlayWorkArea {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1040.0,
            },
            OverlayWorkArea {
                x: 1920.0,
                y: 0.0,
                width: 1920.0,
                height: 1040.0,
            },
        ];

        let pos = resolve_overlay_position(&config, &areas);

        assert_eq!(pos.x, 1920.0);
        assert_eq!(pos.y, 920.0);
    }

    #[test]
    fn resolve_remote_asr_fields_apply_defaults_and_clamp() {
        let s = Settings::default();
        assert_eq!(resolve_asr_provider(&s), "doubao");
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

    #[test]
    fn hotkey_primary_defaults_and_validates_single_keys() {
        let mut s = Settings::default();
        s.hotkeys_enabled = Some(true);
        let cfg = resolve_hotkey_config(&s).expect("hotkey config");
        assert_eq!(cfg.primary, "Alt");

        s.hotkey_primary = Some(" f9 ".to_string());
        let cfg = resolve_hotkey_config(&s).expect("hotkey config");
        assert_eq!(cfg.primary, "F9");

        assert_eq!(
            normalize_hotkey_primary(Some("control")).expect("control alias"),
            "Ctrl"
        );
        assert!(normalize_hotkey_primary(Some("Ctrl+Alt")).is_err());
    }
}
