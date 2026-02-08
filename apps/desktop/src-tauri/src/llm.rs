use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String, // e.g. https://api.openai.com/v1
    pub model: String,
}

#[derive(Debug, Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
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
    let t = s.trim().trim_end_matches('/');
    if t.is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        t.to_string()
    }
}

pub fn load_config_from_env() -> LlmConfig {
    let base_url = std::env::var("TYPEVOICE_LLM_BASE_URL").unwrap_or_default();
    let model = std::env::var("TYPEVOICE_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    LlmConfig {
        base_url: normalize_base_url(&base_url),
        model,
    }
}

pub fn load_api_key() -> Result<String> {
    if let Ok(k) = std::env::var("TYPEVOICE_LLM_API_KEY") {
        if !k.trim().is_empty() {
            return Ok(k);
        }
    }
    let entry = keyring::Entry::new("typevoice", "llm_api_key").context("keyring entry init failed")?;
    let k = entry.get_password().context("keyring get failed")?;
    if k.trim().is_empty() {
        return Err(anyhow!("empty api key"));
    }
    Ok(k)
}

pub fn set_api_key(key: &str) -> Result<()> {
    let entry = keyring::Entry::new("typevoice", "llm_api_key").context("keyring entry init failed")?;
    entry
        .set_password(key)
        .context("keyring set failed")?;
    Ok(())
}

pub fn clear_api_key() -> Result<()> {
    let entry = keyring::Entry::new("typevoice", "llm_api_key").context("keyring entry init failed")?;
    // keyring v3 does not expose a cross-platform delete API. We overwrite with
    // an empty password and treat empty as "not configured".
    let _ = entry.set_password("");
    Ok(())
}

pub async fn rewrite(system_prompt: &str, asr_text: &str) -> Result<String> {
    let cfg = load_config_from_env();
    let key = load_api_key()?;
    let client = Client::new();
    let url = format!("{}/chat/completions", cfg.base_url);

    let req = ChatReq {
        model: &cfg.model,
        messages: vec![
            Message { role: "system", content: system_prompt },
            Message { role: "user", content: asr_text },
        ],
        temperature: 0.2,
    };

    let resp = client
        .post(url)
        .bearer_auth(key)
        .json(&req)
        .send()
        .await
        .context("llm http request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("llm http {status}: {body}"));
    }

    let r: ChatResp = resp.json().await.context("llm response parse failed")?;
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
