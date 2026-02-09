use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
    match load_settings(data_dir) {
        Ok(s) => s,
        Err(e) => {
            let p = settings_path(data_dir);
            if p.exists() {
                let backup = data_dir.join(format!("settings.json.corrupt.{}", now_ms()));
                if let Err(re) = fs::rename(&p, &backup) {
                    eprintln!(
                        "settings.json corrupt, and failed to back it up (src={}, dst={}): {re:#}",
                        p.display(),
                        backup.display()
                    );
                } else {
                    eprintln!(
                        "settings.json corrupt; moved to {} (error: {:#})",
                        backup.display(),
                        e
                    );
                }
            } else {
                eprintln!("settings load failed (missing file): {e:#}");
            }
            Settings::default()
        }
    }
}

pub fn save_settings(data_dir: &Path, settings: &Settings) -> Result<()> {
    std::fs::create_dir_all(data_dir).context("create data dir failed")?;
    let p = settings_path(data_dir);
    let s = serde_json::to_string_pretty(settings).context("serialize settings failed")?;
    fs::write(&p, s).context("write settings.json failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_patch, Settings, SettingsPatch};

    #[test]
    fn apply_patch_is_partial_and_can_clear() {
        let base = Settings {
            asr_model: Some("asr".to_string()),
            llm_base_url: Some("https://x/v1".to_string()),
            llm_model: Some("m1".to_string()),
            llm_reasoning_effort: Some("low".to_string()),
            rewrite_enabled: Some(false),
            rewrite_template_id: Some("t1".to_string()),
        };

        let p = SettingsPatch {
            llm_model: Some(Some("m2".to_string())),
            llm_reasoning_effort: Some(None),
            rewrite_enabled: Some(Some(true)),
            rewrite_template_id: Some(None),
            ..Default::default()
        };

        let next = apply_patch(base, p);
        assert_eq!(next.asr_model.as_deref(), Some("asr"));
        assert_eq!(next.llm_base_url.as_deref(), Some("https://x/v1"));
        assert_eq!(next.llm_model.as_deref(), Some("m2"));
        assert_eq!(next.llm_reasoning_effort, None);
        assert_eq!(next.rewrite_enabled, Some(true));
        assert_eq!(next.rewrite_template_id, None);
    }
}
