use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::context_pack::PreparedContext;
use crate::obs::debug;
use crate::obs::{event, Span};
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

#[derive(Debug, Clone, Serialize)]
struct ChatReq {
    model: String,
    messages: Vec<Message>,
    temperature: f32,

    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Message {
    role: String,
    content: MessageContent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
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

#[derive(Debug, Clone, Serialize)]
pub struct RewriteContextPolicy {
    pub include_history: bool,
    pub include_clipboard: bool,
    pub include_prev_window_meta: bool,
    pub include_focused_app_meta: bool,
    pub include_focused_element_meta: bool,
    pub include_input_state: bool,
    pub include_related_content: bool,
    pub include_visible_text: bool,
    pub include_glossary: bool,
}

impl Default for RewriteContextPolicy {
    fn default() -> Self {
        Self {
            include_history: false,
            include_clipboard: false,
            include_prev_window_meta: false,
            include_focused_app_meta: false,
            include_focused_element_meta: false,
            include_input_state: false,
            include_related_content: false,
            include_visible_text: false,
            include_glossary: false,
        }
    }
}

fn normalize_base_url(s: &str) -> Result<String> {
    let mut t = s.trim().trim_end_matches('/').to_string();
    if t.is_empty() {
        return Err(anyhow!(
            "E_LLM_CONFIG_BASE_URL_MISSING: llm_base_url (or TYPEVOICE_LLM_BASE_URL) is required"
        ));
    }

    // Allow users to paste full endpoint and still work.
    if let Some(stripped) = t.strip_suffix("/chat/completions") {
        t = stripped.to_string();
    }
    Ok(t.trim_end_matches('/').to_string())
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

pub fn load_config(data_dir: &std::path::Path) -> Result<LlmConfig> {
    let s = settings::load_settings_strict(data_dir)?;

    let base_url = s
        .llm_base_url
        .or_else(|| std::env::var("TYPEVOICE_LLM_BASE_URL").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            anyhow!("E_LLM_CONFIG_BASE_URL_MISSING: llm_base_url (or TYPEVOICE_LLM_BASE_URL) is required")
        })?;

    let model = s
        .llm_model
        .or_else(|| std::env::var("TYPEVOICE_LLM_MODEL").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            anyhow!("E_LLM_CONFIG_MODEL_MISSING: llm_model (or TYPEVOICE_LLM_MODEL) is required")
        })?;

    let reasoning_effort = s
        .llm_reasoning_effort
        .as_deref()
        .and_then(normalize_reasoning_effort);

    Ok(LlmConfig {
        base_url: normalize_base_url(&base_url)?,
        model,
        reasoning_effort,
    })
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
    rewrite_with_context(
        data_dir,
        task_id,
        system_prompt,
        asr_text,
        None,
        &[],
        &RewriteContextPolicy::default(),
    )
    .await
}

pub async fn rewrite_with_context(
    data_dir: &std::path::Path,
    task_id: &str,
    system_prompt: &str,
    asr_text: &str,
    ctx: Option<&PreparedContext>,
    rewrite_glossary: &[String],
    policy: &RewriteContextPolicy,
) -> Result<String> {
    let span = Span::start(
        data_dir,
        Some(task_id),
        "Rewrite",
        "LLM.rewrite",
        Some(serde_json::json!({
            "has_context": ctx.is_some(),
            "policy": policy,
        })),
    );

    let cfg = match load_config(data_dir) {
        Ok(c) => c,
        Err(e) => {
            span.err_anyhow("config", "E_LLM_CONFIG", &e, None);
            return Err(e);
        }
    };
    let key = match load_api_key() {
        Ok(k) => k,
        Err(e) => {
            span.err_anyhow("auth", "E_LLM_API_KEY", &e, None);
            return Err(e);
        }
    };
    let client = Client::new();
    let url = format!("{}/chat/completions", cfg.base_url);

    let (user_content_send, user_content_debug) =
        build_user_content(asr_text, ctx, rewrite_glossary, policy);

    // Record the exact request "shape" the model will receive (text vs multimodal parts).
    let (kind, has_image_url) = user_content_shape(&user_content_send);
    event(
        data_dir,
        Some(task_id),
        "Rewrite",
        "LLM.request.shape",
        "ok",
        Some(serde_json::json!({
            "user_content_kind": kind,
            "has_image_url": has_image_url,
            "asr_chars": asr_text.len(),
            "system_prompt_chars": system_prompt.len(),
            "glossary_count": rewrite_glossary.len(),
            "include_glossary": policy.include_glossary,
        })),
    );
    let req_send = ChatReq {
        model: cfg.model.clone(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: MessageContent::Text(system_prompt.to_string()),
            },
            Message {
                role: "user".to_string(),
                content: user_content_send,
            },
        ],
        temperature: 0.2,
        reasoning_effort: cfg.reasoning_effort.clone(),
    };

    let req_debug = ChatReq {
        model: cfg.model.clone(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: MessageContent::Text(system_prompt.to_string()),
            },
            Message {
                role: "user".to_string(),
                content: user_content_debug,
            },
        ],
        temperature: 0.2,
        reasoning_effort: cfg.reasoning_effort.clone(),
    };

    if debug::verbose_enabled() && debug::include_llm() {
        if let Ok(req_value) = serde_json::to_value(&req_debug) {
            let url2 = url.clone();
            let wrapper = serde_json::json!({
                "url": url2,
                "request": req_value,
            });
            let bytes = serde_json::to_vec_pretty(&wrapper).unwrap_or_default();
            if let Some(info) =
                debug::write_payload_best_effort(data_dir, task_id, "llm_request.json", bytes)
            {
                debug::emit_debug_event_best_effort(
                    data_dir,
                    "debug_llm_request",
                    task_id,
                    &info,
                    Some(format!("model={} url={}", cfg.model, url)),
                );
            }
        }
    }

    let resp = match client
        .post(url.clone())
        .bearer_auth(key)
        .json(&req_send)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let ae = anyhow!("llm http request failed: {e}");
            span.err_anyhow(
                "http",
                "E_LLM_HTTP_SEND",
                &ae,
                Some(serde_json::json!({"url": url, "model": cfg.model})),
            );
            return Err(ae);
        }
    };

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if debug::verbose_enabled() && debug::include_llm() {
        if let Some(info) = debug::write_payload_best_effort(
            data_dir,
            task_id,
            "llm_response.txt",
            body.as_bytes().to_vec(),
        ) {
            debug::emit_debug_event_best_effort(
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
        let ae = anyhow!("llm http {status}: {msg}");
        span.err_anyhow(
            "http",
            &format!("HTTP_{}", status.as_u16()),
            &ae,
            Some(serde_json::json!({"status": status.as_u16()})),
        );
        return Err(ae);
    }

    let r: ChatResp = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            let ae = anyhow!("llm response parse failed: {e}");
            span.err_anyhow(
                "parse",
                "E_LLM_PARSE",
                &ae,
                Some(serde_json::json!({"body_len": body.len()})),
            );
            return Err(ae);
        }
    };
    let choice0 = match r.choices.get(0) {
        Some(c) => c,
        None => {
            let ae = anyhow!("llm missing choices[0]");
            span.err_anyhow("parse", "E_LLM_MISSING_CHOICES", &ae, None);
            return Err(ae);
        }
    };
    let content = choice0.message.content.trim().to_string();
    if content.is_empty() {
        let ae = anyhow!("llm returned empty content");
        span.err_anyhow("logic", "E_LLM_EMPTY", &ae, None);
        return Err(ae);
    }
    span.ok(Some(serde_json::json!({
        "status": status.as_u16(),
        "content_chars": content.len(),
        "model": cfg.model,
    })));
    Ok(content)
}

fn user_content_shape(content: &MessageContent) -> (&'static str, bool) {
    match content {
        MessageContent::Text(_) => ("text", false),
    }
}

fn build_rewrite_glossary_section(rewrite_glossary: &[String]) -> Option<String> {
    if rewrite_glossary.is_empty() {
        return None;
    }
    let mut out = String::new();
    for item in rewrite_glossary {
        let v = item.trim();
        if v.is_empty() {
            continue;
        }
        out.push_str("- ");
        out.push_str(v);
        out.push('\n');
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn bool_text(v: bool) -> &'static str {
    if v {
        "enabled"
    } else {
        "disabled"
    }
}

fn policy_to_markdown(policy: &RewriteContextPolicy) -> String {
    format!(
        "### CONTEXT_POLICY\n- history: {}\n- clipboard: {}\n- focused_app_meta: {}\n- focused_element_meta: {}\n- input_state: {}\n- related_content: {}\n- visible_text: {}\n- glossary: {}\n",
        bool_text(policy.include_history),
        bool_text(policy.include_clipboard),
        bool_text(policy.include_focused_app_meta),
        bool_text(policy.include_focused_element_meta),
        bool_text(policy.include_input_state),
        bool_text(policy.include_related_content),
        bool_text(policy.include_visible_text),
        bool_text(policy.include_glossary),
    )
}

fn extract_prepared_context_block(prepared: &PreparedContext) -> String {
    const MARKER: &str = "### CONTEXT\n";
    match prepared.user_text.find(MARKER) {
        Some(pos) => prepared.user_text[pos + MARKER.len()..].trim().to_string(),
        None => String::new(),
    }
}

fn build_rewrite_user_text(
    asr_text: &str,
    ctx: Option<&PreparedContext>,
    rewrite_glossary: &[String],
    policy: &RewriteContextPolicy,
) -> String {
    let mut out = String::new();
    out.push_str("### TRANSCRIPT\n");
    out.push_str(asr_text.trim());
    out.push('\n');
    out.push('\n');
    out.push_str(&policy_to_markdown(policy));

    if policy.include_glossary {
        if let Some(g) = build_rewrite_glossary_section(rewrite_glossary) {
            out.push_str("\n### GLOSSARY\n");
            out.push_str(&g);
        }
    }

    let include_context_sections = policy.include_history
        || policy.include_clipboard
        || policy.include_prev_window_meta
        || policy.include_focused_app_meta
        || policy.include_focused_element_meta
        || policy.include_input_state
        || policy.include_related_content
        || policy.include_visible_text;
    if include_context_sections {
        if let Some(c) = ctx {
            let context_block = extract_prepared_context_block(c);
            if !context_block.is_empty() {
                out.push_str("\n### CONTEXT\n");
                out.push_str(&context_block);
            }
        }
    }

    out.trim_end().to_string()
}

fn build_user_content(
    asr_text: &str,
    ctx: Option<&PreparedContext>,
    rewrite_glossary: &[String],
    policy: &RewriteContextPolicy,
) -> (MessageContent, MessageContent) {
    let send_text = build_rewrite_user_text(asr_text, ctx, rewrite_glossary, policy);
    let debug_text = send_text.clone();
    (
        MessageContent::Text(send_text),
        MessageContent::Text(debug_text),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        api_key_status, build_rewrite_user_text, build_user_content, normalize_base_url,
        MessageContent, RewriteContextPolicy,
    };
    use crate::context_pack::PreparedContext;

    #[test]
    fn normalize_base_url_handles_empty_and_endpoint_suffix() {
        assert!(normalize_base_url("").is_err());
        assert_eq!(
            normalize_base_url(" https://api.openai.com/v1/ ").expect("base"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            normalize_base_url("http://api.server/v1/chat/completions").expect("base"),
            "http://api.server/v1"
        );
        assert_eq!(
            normalize_base_url("http://api.server/v1/chat/completions/").expect("base"),
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

    #[test]
    fn build_rewrite_user_text_is_text_only_and_has_no_screenshot_policy() {
        let prepared = PreparedContext {
            user_text: "### TRANSCRIPT\nhello\n\n### CONTEXT\nbody".to_string(),
        };
        let policy = RewriteContextPolicy {
            include_history: true,
            include_clipboard: true,
            include_prev_window_meta: false,
            include_focused_app_meta: true,
            include_focused_element_meta: true,
            include_input_state: true,
            include_related_content: true,
            include_visible_text: true,
            include_glossary: false,
        };
        let text = build_rewrite_user_text("hello", Some(&prepared), &[], &policy);
        assert!(text.contains("focused_app_meta: enabled"));
        assert!(text.contains("focused_element_meta: enabled"));
        assert!(text.contains("input_state: enabled"));
        assert!(text.contains("related_content: enabled"));
        assert!(text.contains("visible_text: enabled"));
        assert!(!text.contains("screenshot"));
    }

    #[test]
    fn build_user_content_never_emits_image_parts() {
        let ctx = PreparedContext {
            user_text: "### TRANSCRIPT\nhello\n\n### CONTEXT\nbody".to_string(),
        };
        let policy = RewriteContextPolicy {
            include_history: false,
            include_clipboard: false,
            include_prev_window_meta: false,
            include_focused_app_meta: true,
            include_focused_element_meta: false,
            include_input_state: false,
            include_related_content: false,
            include_visible_text: false,
            include_glossary: false,
        };
        let (send, debug) = build_user_content("hello", Some(&ctx), &[], &policy);
        assert!(matches!(send, MessageContent::Text(_)));
        assert!(matches!(debug, MessageContent::Text(_)));
    }

    #[test]
    fn build_rewrite_user_text_keeps_window_only_context() {
        let prepared = PreparedContext {
            user_text: "### TRANSCRIPT\nhello\n\n### CONTEXT\nFOCUSED WINDOW\n- title: notes"
                .to_string(),
        };
        let policy = RewriteContextPolicy {
            include_history: false,
            include_clipboard: false,
            include_prev_window_meta: true,
            include_focused_app_meta: false,
            include_focused_element_meta: false,
            include_input_state: false,
            include_related_content: false,
            include_visible_text: false,
            include_glossary: false,
        };

        let text = build_rewrite_user_text("hello", Some(&prepared), &[], &policy);

        assert!(text.contains("FOCUSED WINDOW"));
    }
}
