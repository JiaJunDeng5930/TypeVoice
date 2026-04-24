use anyhow::{anyhow, Result};

use crate::llm::ApiKeyStatus;

const KEYRING_SERVICE: &str = "typevoice";
const APP_KEY_USER: &str = "doubao_asr_app_key";
const ACCESS_KEY_USER: &str = "doubao_asr_access_key";
const APP_KEY_ENV: &str = "TYPEVOICE_DOUBAO_ASR_APP_KEY";
const ACCESS_KEY_ENV: &str = "TYPEVOICE_DOUBAO_ASR_ACCESS_KEY";

#[derive(Debug, Clone)]
pub struct DoubaoCredentials {
    pub app_key: String,
    pub access_key: String,
}

pub fn set_credentials(app_key: &str, access_key: &str) -> Result<()> {
    let app_key = app_key.trim();
    let access_key = access_key.trim();
    if app_key.is_empty() {
        return Err(anyhow!("E_DOUBAO_ASR_APP_KEY_MISSING: app key is required"));
    }
    if access_key.is_empty() {
        return Err(anyhow!(
            "E_DOUBAO_ASR_ACCESS_KEY_MISSING: access key is required"
        ));
    }
    keyring::Entry::new(KEYRING_SERVICE, APP_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password(app_key)
        .map_err(|e| anyhow!("{e:?}"))?;
    keyring::Entry::new(KEYRING_SERVICE, ACCESS_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password(access_key)
        .map_err(|e| anyhow!("{e:?}"))?;
    Ok(())
}

pub fn clear_credentials() -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, APP_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password("")
        .map_err(|e| anyhow!("{e:?}"))?;
    keyring::Entry::new(KEYRING_SERVICE, ACCESS_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .set_password("")
        .map_err(|e| anyhow!("{e:?}"))?;
    Ok(())
}

pub fn credentials_status() -> ApiKeyStatus {
    if env_credentials().is_some() {
        return ApiKeyStatus {
            configured: true,
            source: "env".to_string(),
            reason: None,
        };
    }
    match keyring_credentials() {
        Ok(Some(_)) => ApiKeyStatus {
            configured: true,
            source: "keyring".to_string(),
            reason: None,
        },
        Ok(None) => ApiKeyStatus {
            configured: false,
            source: "keyring".to_string(),
            reason: Some("empty".to_string()),
        },
        Err(e) => ApiKeyStatus {
            configured: false,
            source: "keyring".to_string(),
            reason: Some(e.to_string()),
        },
    }
}

pub fn load_credentials() -> Result<DoubaoCredentials> {
    if let Some(v) = env_credentials() {
        return Ok(v);
    }
    match keyring_credentials()? {
        Some(v) => Ok(v),
        None => Err(anyhow!(
            "E_DOUBAO_ASR_CREDENTIALS_MISSING: doubao ASR credentials are missing"
        )),
    }
}

fn env_credentials() -> Option<DoubaoCredentials> {
    let app_key = std::env::var(APP_KEY_ENV).ok()?.trim().to_string();
    let access_key = std::env::var(ACCESS_KEY_ENV).ok()?.trim().to_string();
    if app_key.is_empty() || access_key.is_empty() {
        return None;
    }
    Some(DoubaoCredentials {
        app_key,
        access_key,
    })
}

fn keyring_credentials() -> Result<Option<DoubaoCredentials>> {
    let app_key = keyring::Entry::new(KEYRING_SERVICE, APP_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .get_password()
        .unwrap_or_default()
        .trim()
        .to_string();
    let access_key = keyring::Entry::new(KEYRING_SERVICE, ACCESS_KEY_USER)
        .map_err(|e| anyhow!("{e:?}"))?
        .get_password()
        .unwrap_or_default()
        .trim()
        .to_string();
    if app_key.is_empty() || access_key.is_empty() {
        return Ok(None);
    }
    Ok(Some(DoubaoCredentials {
        app_key,
        access_key,
    }))
}
