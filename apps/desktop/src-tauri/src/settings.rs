use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::trace::Span;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub asr_model: Option<String>, // local dir or HF repo id

    // LLM settings (non-sensitive). API key is stored in OS keyring.
    pub llm_base_url: Option<String>, // e.g. https://api.openai.com/v1
    pub llm_model: Option<String>,    // e.g. gpt-4o-mini
    pub llm_reasoning_effort: Option<String>, // e.g. none|minimal|low|medium|high|xhigh

    // UX settings
    pub rewrite_enabled: Option<bool>,
    pub rewrite_template_id: Option<String>,

    // Context settings (for LLM rewrite)
    pub context_include_history: Option<bool>,
    pub context_history_n: Option<i64>,
    pub context_history_window_ms: Option<i64>,
    pub context_include_clipboard: Option<bool>,
    pub context_include_prev_window_screenshot: Option<bool>,
    pub llm_supports_vision: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsPatch {
    // Outer Option: whether to update this field.
    // Inner Option: Some(value)=set, None=clear.
    pub asr_model: Option<Option<String>>,

    pub llm_base_url: Option<Option<String>>,
    pub llm_model: Option<Option<String>>,
    pub llm_reasoning_effort: Option<Option<String>>,

    pub rewrite_enabled: Option<Option<bool>>,
    pub rewrite_template_id: Option<Option<String>>,

    pub context_include_history: Option<Option<bool>>,
    pub context_history_n: Option<Option<i64>>,
    pub context_history_window_ms: Option<Option<i64>>,
    pub context_include_clipboard: Option<Option<bool>>,
    pub context_include_prev_window_screenshot: Option<Option<bool>>,
    pub llm_supports_vision: Option<Option<bool>>,
}

pub fn apply_patch(mut s: Settings, p: SettingsPatch) -> Settings {
    if let Some(v) = p.asr_model {
        s.asr_model = v;
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
    if let Some(v) = p.rewrite_enabled {
        s.rewrite_enabled = v;
    }
    if let Some(v) = p.rewrite_template_id {
        s.rewrite_template_id = v;
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
    if let Some(v) = p.llm_supports_vision {
        s.llm_supports_vision = v;
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

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn load_settings_or_recover(data_dir: &Path) -> Settings {
    let span = Span::start(data_dir, None, "Settings", "SETTINGS.load_or_recover", None);
    match load_settings(data_dir) {
        Ok(s) => {
            span.ok(Some(serde_json::json!({"status": "ok"})));
            s
        }
        Err(e) => {
            let mut backup_ok = false;
            let p = settings_path(data_dir);
            let had_file = p.exists();
            if had_file {
                let backup = data_dir.join(format!("settings.json.corrupt.{}", now_ms()));
                if let Err(re) = fs::rename(&p, &backup) {
                    crate::safe_eprintln!(
                        "settings.json corrupt, and failed to back it up (src={}, dst={}): {re:#}",
                        p.display(),
                        backup.display()
                    );
                } else {
                    backup_ok = true;
                    crate::safe_eprintln!(
                        "settings.json corrupt; moved to {} (error: {:#})",
                        backup.display(),
                        e
                    );
                }
            } else {
                crate::safe_eprintln!("settings load failed (missing file): {e:#}");
            }

            span.err_anyhow(
                "parse",
                "E_SETTINGS_LOAD",
                &e,
                Some(serde_json::json!({
                    "had_file": had_file,
                    "backup_ok": backup_ok,
                })),
            );

            Settings::default()
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{apply_patch, Settings, SettingsPatch};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn apply_patch_is_partial_and_can_clear() {
        let base = Settings {
            asr_model: Some("asr".to_string()),
            llm_base_url: Some("https://x/v1".to_string()),
            llm_model: Some("m1".to_string()),
            llm_reasoning_effort: Some("low".to_string()),
            rewrite_enabled: Some(false),
            rewrite_template_id: Some("t1".to_string()),
            context_include_history: None,
            context_history_n: None,
            context_history_window_ms: None,
            context_include_clipboard: None,
            context_include_prev_window_screenshot: None,
            llm_supports_vision: None,
        };

        let p = SettingsPatch {
            llm_model: Some(Some("m2".to_string())),
            llm_reasoning_effort: Some(None),
            rewrite_enabled: Some(Some(true)),
            rewrite_template_id: Some(None),
            context_history_n: Some(Some(5)),
            ..Default::default()
        };

        let next = apply_patch(base, p);
        assert_eq!(next.asr_model.as_deref(), Some("asr"));
        assert_eq!(next.llm_base_url.as_deref(), Some("https://x/v1"));
        assert_eq!(next.llm_model.as_deref(), Some("m2"));
        assert_eq!(next.llm_reasoning_effort, None);
        assert_eq!(next.rewrite_enabled, Some(true));
        assert_eq!(next.rewrite_template_id, None);
        assert_eq!(next.context_history_n, Some(5));
    }

    #[test]
    fn load_settings_or_recover_moves_corrupt_file_and_returns_default() {
        let td = tempdir().expect("tempdir");
        let data_dir = td.path().join("data");
        fs::create_dir_all(&data_dir).expect("mkdir");
        fs::write(data_dir.join("settings.json"), "{not-json").expect("write");

        let s = super::load_settings_or_recover(&data_dir);
        assert_eq!(s.asr_model, None);
        assert!(!data_dir.join("settings.json").exists());

        let entries: Vec<_> = fs::read_dir(&data_dir)
            .expect("read_dir")
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries
            .iter()
            .any(|n| n.starts_with("settings.json.corrupt.")));
    }
}
