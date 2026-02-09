use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::debug_log;
use crate::settings;

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyStatus {
    pub configured: bool,
    pub source: String, // env|keyring
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String, // e.g. https://api.openai.com/v1
    pub model: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,

    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: String,
}

fn normalize_base_url(s: &str) -> String {
    let mut t = s.trim().trim_end_matches('/').to_string();
    if t.is_empty() {
        return "https://api.openai.com/v1".to_string();
    }

    // Allow users to paste full endpoint and still work.
    if let Some(stripped) = t.strip_suffix("/chat/completions") {
        t = stripped.to_string();
    }
    t.trim_end_matches('/').to_string()
}

fn normalize_reasoning_effort(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // "default" means "do not send this field".
    if t.eq_ignore_ascii_case("default") {
        return None;
    }
    Some(t.to_string())
}

pub fn load_config(data_dir: &std::path::Path) -> LlmConfig {
    let s = settings::load_settings_or_recover(data_dir);

    let base_url = s
        .llm_base_url
        .or_else(|| std::env::var("TYPEVOICE_LLM_BASE_URL").ok())
        .unwrap_or_default();

    let model = s
        .llm_model
        .or_else(|| std::env::var("TYPEVOICE_LLM_MODEL").ok())
        .unwrap_or_else(|| "gpt-4o-mini".to_string());

    let reasoning_effort = s
        .llm_reasoning_effort
        .as_deref()
        .and_then(normalize_reasoning_effort);

    LlmConfig {
        base_url: normalize_base_url(&base_url),
        model,
        reasoning_effort,
    }
}

pub fn load_api_key() -> Result<String> {
    if let Ok(k) = std::env::var("TYPEVOICE_LLM_API_KEY") {
        if !k.trim().is_empty() {
            return Ok(k);
        }
    }
    let entry = keyring::Entry::new("typevoice", "llm_api_key")
        .map_err(|e| anyhow!("keyring entry init failed: {e:?}"))?;
    let k = entry
        .get_password()
        .map_err(|e| anyhow!("keyring get failed: {e:?}"))?;
    if k.trim().is_empty() {
        return Err(anyhow!("empty api key"));
    }
    Ok(k)
}

pub fn set_api_key(key: &str) -> Result<()> {
    let entry = keyring::Entry::new("typevoice", "llm_api_key")
        .map_err(|e| anyhow!("keyring entry init failed: {e:?}"))?;
    entry
        .set_password(key)
        .map_err(|e| anyhow!("keyring set failed: {e:?}"))?;
    Ok(())
}

pub fn clear_api_key() -> Result<()> {
    let entry = keyring::Entry::new("typevoice", "llm_api_key")
        .map_err(|e| anyhow!("keyring entry init failed: {e:?}"))?;
    // keyring v3 does not expose a cross-platform delete API. We overwrite with
    // an empty password and treat empty as "not configured".
    let _ = entry
        .set_password("")
        .map_err(|e| anyhow!("keyring set failed: {e:?}"));
    Ok(())
}

pub fn api_key_status() -> ApiKeyStatus {
    if let Ok(k) = std::env::var("TYPEVOICE_LLM_API_KEY") {
        if !k.trim().is_empty() {
            return ApiKeyStatus {
                configured: true,
                source: "env".to_string(),
                reason: None,
            };
        }
    }

    let entry = match keyring::Entry::new("typevoice", "llm_api_key") {
        Ok(e) => e,
        Err(e) => {
            return ApiKeyStatus {
                configured: false,
                source: "keyring".to_string(),
                reason: Some(format!("keyring_entry_init_failed:{e:?}")),
            };
        }
    };

    let k = match entry.get_password() {
        Ok(k) => k,
        Err(e) => {
            return ApiKeyStatus {
                configured: false,
                source: "keyring".to_string(),
                reason: Some(format!("keyring_get_failed:{e:?}")),
            };
        }
    };
    if k.trim().is_empty() {
        return ApiKeyStatus {
            configured: false,
            source: "keyring".to_string(),
            reason: Some("empty".to_string()),
        };
    }
    ApiKeyStatus {
        configured: true,
        source: "keyring".to_string(),
        reason: None,
    }
}

pub async fn rewrite(
    data_dir: &std::path::Path,
    task_id: &str,
    system_prompt: &str,
    asr_text: &str,
) -> Result<String> {
    let cfg = load_config(data_dir);
    let key = load_api_key()?;
    let client = Client::new();
    let url = format!("{}/chat/completions", cfg.base_url);

    let req = ChatReq {
        model: &cfg.model,
        messages: vec![
            Message {
                role: "system",
                content: system_prompt,
            },
            Message {
                role: "user",
                content: asr_text,
            },
        ],
        temperature: 0.2,
        reasoning_effort: cfg.reasoning_effort.as_deref(),
    };

    if debug_log::verbose_enabled() && debug_log::include_llm() {
        if let Ok(req_value) = serde_json::to_value(&req) {
            let url2 = url.clone();
            let wrapper = serde_json::json!({
                "url": url2,
                "request": req_value,
            });
            let bytes = serde_json::to_vec_pretty(&wrapper).unwrap_or_default();
            if let Some(info) =
                debug_log::write_payload_best_effort(data_dir, task_id, "llm_request.json", bytes)
            {
                debug_log::emit_debug_event_best_effort(
                    data_dir,
                    "debug_llm_request",
                    task_id,
                    &info,
                    Some(format!("model={} url={}", cfg.model, url)),
                );
            }
        }
    }

    let resp = client
        .post(url)
        .bearer_auth(key)
        .json(&req)
        .send()
        .await
        .context("llm http request failed")?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if debug_log::verbose_enabled() && debug_log::include_llm() {
        if let Some(info) = debug_log::write_payload_best_effort(
            data_dir,
            task_id,
            "llm_response.txt",
            body.as_bytes().to_vec(),
        ) {
            debug_log::emit_debug_event_best_effort(
                data_dir,
                "debug_llm_response",
                task_id,
                &info,
                Some(format!("http_status={}", status)),
            );
        }
    }

    if !status.is_success() {
        let msg = if body.len() > 1024 {
            format!("{}...(truncated)", &body[..1024])
        } else {
            body
        };
        return Err(anyhow!("llm http {status}: {msg}"));
    }

    let r: ChatResp = serde_json::from_str(&body).context("llm response parse failed")?;
    let content = r
        .choices
        .get(0)
        .ok_or_else(|| anyhow!("llm missing choices[0]"))?
        .message
        .content
        .trim()
        .to_string();
    if content.is_empty() {
        return Err(anyhow!("llm returned empty content"));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::api_key_status;
    use super::normalize_base_url;

    #[test]
    fn normalize_base_url_handles_empty_and_endpoint_suffix() {
        assert_eq!(normalize_base_url(""), "https://api.openai.com/v1");
        assert_eq!(
            normalize_base_url(" https://api.openai.com/v1/ "),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            normalize_base_url("http://api.server/v1/chat/completions"),
            "http://api.server/v1"
        );
        assert_eq!(
            normalize_base_url("http://api.server/v1/chat/completions/"),
            "http://api.server/v1"
        );
    }

    #[test]
    fn api_key_status_prefers_env_when_set() {
        std::env::set_var("TYPEVOICE_LLM_API_KEY", "test-key");
        let st = api_key_status();
        assert!(st.configured);
        assert_eq!(st.source, "env");
        std::env::remove_var("TYPEVOICE_LLM_API_KEY");
    }
}
