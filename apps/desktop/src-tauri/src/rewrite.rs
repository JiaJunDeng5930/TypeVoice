use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::ports::{PortError, PortResult};
use crate::ui_events::{UiEvent, UiEventMailbox, UiEventStatus};
use crate::{
    context_capture, context_pack, data_dir, history, llm, settings, task_manager, templates,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteTextRequest {
    pub transcript_id: String,
    pub text: String,
    pub template_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteResult {
    pub transcript_id: String,
    pub final_text: String,
    pub rewrite_ms: u128,
    pub template_id: Option<String>,
}

pub async fn rewrite_text(
    mailbox: &UiEventMailbox,
    task_state: &task_manager::TaskManager,
    req: RewriteTextRequest,
) -> PortResult<RewriteResult> {
    let data_dir =
        data_dir::data_dir().map_err(|e| PortError::from_message("E_DATA_DIR", e.to_string()))?;
    let task_id = req.transcript_id.trim();
    if task_id.is_empty() {
        return Err(PortError::new(
            "E_REWRITE_TRANSCRIPT_ID_MISSING",
            "transcript_id is required",
        ));
    }
    if req.text.trim().is_empty() {
        return Err(PortError::new("E_REWRITE_EMPTY_TEXT", "text is required"));
    }
    let s = settings::load_settings_strict(&data_dir)
        .map_err(|e| PortError::from_message("E_SETTINGS_INVALID", e.to_string()))?;
    if !s.rewrite_enabled.unwrap_or(false) {
        return Err(PortError::new(
            "E_REWRITE_DISABLED",
            "rewrite is disabled in settings",
        ));
    }
    let template_id = req
        .template_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            s.rewrite_template_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
        })
        .ok_or_else(|| {
            PortError::new(
                "E_SETTINGS_REWRITE_TEMPLATE_MISSING",
                "rewrite_template_id is required",
            )
        })?;
    let template = templates::get_template(&data_dir, &template_id)
        .map_err(|e| PortError::from_message("E_TEMPLATE_NOT_FOUND", e.to_string()))?;
    let ctx_cfg = context_capture::config_from_settings(&s);
    let ctx_snap = rewrite_context(task_state, &data_dir, task_id, &ctx_cfg);
    let prepared = context_pack::prepare(&req.text, &ctx_snap, &ctx_cfg.budget);
    let policy = llm::RewriteContextPolicy {
        include_history: ctx_cfg.include_history,
        include_clipboard: ctx_cfg.include_clipboard,
        include_prev_window_meta: ctx_cfg.include_prev_window_meta,
        include_prev_window_screenshot: ctx_cfg.include_prev_window_screenshot
            && prepared.screenshot.is_some(),
        include_glossary: s.rewrite_include_glossary.unwrap_or(true),
    };
    let glossary = sanitize_rewrite_glossary(s.rewrite_glossary);
    let glossary_ref: &[String] = if policy.include_glossary {
        &glossary
    } else {
        &[]
    };

    mailbox.send(UiEvent::stage(
        task_id,
        "Rewrite",
        UiEventStatus::Started,
        "llm",
    ));
    let started = Instant::now();
    let final_text = match llm::rewrite_with_context(
        &data_dir,
        task_id,
        &template.system_prompt,
        &req.text,
        Some(&prepared),
        glossary_ref,
        &policy,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            let err = PortError::from_message("E_LLM_FAILED", e.to_string());
            mailbox.send(UiEvent::stage_with_elapsed(
                task_id,
                "Rewrite",
                UiEventStatus::Failed,
                err.message.clone(),
                Some(started.elapsed().as_millis()),
                Some(err.code.clone()),
            ));
            return Err(err);
        }
    };
    let rewrite_ms = started.elapsed().as_millis();
    history::update_final_text(
        &data_dir.join("history.sqlite3"),
        task_id,
        &final_text,
        Some(&template_id),
    )
    .map_err(|e| PortError::from_message("E_HISTORY_UPDATE", e.to_string()))?;
    mailbox.send(UiEvent::stage_with_elapsed(
        task_id,
        "Rewrite",
        UiEventStatus::Completed,
        "ok",
        Some(rewrite_ms),
        None,
    ));
    let result = RewriteResult {
        transcript_id: task_id.to_string(),
        final_text,
        rewrite_ms,
        template_id: Some(template_id),
    };
    mailbox.send(UiEvent::completed(
        task_id,
        "rewrite.completed",
        "rewrite completed",
        serde_json::to_value(&result).unwrap_or_default(),
    ));
    Ok(result)
}

fn rewrite_context(
    task_state: &task_manager::TaskManager,
    data_dir: &std::path::Path,
    task_id: &str,
    ctx_cfg: &context_capture::ContextConfig,
) -> context_pack::ContextSnapshot {
    let mut capture_cfg = ctx_cfg.clone();
    let pre = task_state.take_pending_hotkey_context_for_rewrite(task_id);
    if pre.is_some() {
        capture_cfg.include_prev_window_screenshot = false;
        capture_cfg.include_prev_window_meta = false;
    }
    let mut snap =
        task_state.capture_snapshot_best_effort_with_config(data_dir, task_id, &capture_cfg);
    if let Some(pre) = pre {
        if ctx_cfg.include_prev_window_meta {
            snap.prev_window = pre.prev_window;
        }
        if ctx_cfg.include_prev_window_screenshot {
            snap.screenshot = pre.screenshot;
        }
    }
    if !ctx_cfg.include_history {
        snap.recent_history.clear();
    }
    if !ctx_cfg.include_clipboard {
        snap.clipboard_text = None;
    }
    if !ctx_cfg.include_prev_window_meta {
        snap.prev_window = None;
    }
    if !ctx_cfg.include_prev_window_screenshot || !ctx_cfg.llm_supports_vision {
        snap.screenshot = None;
    }
    snap
}

fn sanitize_rewrite_glossary(glossary: Option<Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for item in glossary.unwrap_or_default() {
        let v = item.trim();
        if !v.is_empty() {
            out.push(v.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_result_keeps_transcript_identity() {
        let result = RewriteResult {
            transcript_id: "task-1".to_string(),
            final_text: "rewritten".to_string(),
            rewrite_ms: 15,
            template_id: Some("template-1".to_string()),
        };

        assert_eq!(result.transcript_id, "task-1");
        assert_eq!(result.final_text, "rewritten");
        assert_eq!(result.template_id.as_deref(), Some("template-1"));
    }
}
