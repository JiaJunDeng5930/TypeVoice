use std::time::Instant;

use typevoice_core::workflow::{ContextPlan, RewritePlan};

pub use typevoice_core::workflow::RewriteResult;

use crate::ports::{PortError, PortResult};
use crate::{context_pack, data_dir, llm};

#[derive(Debug, Clone)]
pub struct RewriteTextRequest {
    pub transcript_id: String,
    pub text: String,
}

pub async fn rewrite_text_with_plan(
    pre_captured_context: Option<context_pack::ContextSnapshot>,
    req: RewriteTextRequest,
    plan: &RewritePlan,
    context: &ContextPlan,
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
    if !plan.enabled {
        return Err(PortError::new(
            "E_REWRITE_DISABLED",
            "rewrite is disabled for this run",
        ));
    }
    if plan.prompt.trim().is_empty() {
        return Err(PortError::new(
            "E_SETTINGS_LLM_PROMPT_MISSING",
            "llm_prompt is required",
        ));
    }

    let budget = context_pack::ContextBudget {
        max_history_items: context.history_n,
        history_window_ms: context.history_window_ms,
        ..Default::default()
    };
    let mut ctx_snap = pre_captured_context.unwrap_or_default();
    if !context.include_history {
        ctx_snap.recent_history.clear();
    }
    if !context.include_clipboard {
        ctx_snap.clipboard_text = None;
    }
    if !context.include_prev_window_meta {
        ctx_snap.prev_window = None;
    }
    if !context.include_prev_window_screenshot || !plan.supports_vision {
        ctx_snap.screenshot = None;
    }
    let prepared = context_pack::prepare(&req.text, &ctx_snap, &budget);
    let policy = llm::RewriteContextPolicy {
        include_history: context.include_history,
        include_clipboard: context.include_clipboard,
        include_prev_window_meta: context.include_prev_window_meta,
        include_prev_window_screenshot: context.include_prev_window_screenshot
            && prepared.screenshot.is_some(),
        include_glossary: plan.include_glossary,
    };
    let glossary = plan
        .glossary
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let glossary_ref: &[String] = if plan.include_glossary {
        &glossary
    } else {
        &[]
    };
    let config = llm::config_from_values(
        &plan.base_url,
        &plan.model,
        plan.reasoning_effort.as_deref(),
    )
    .map_err(|e| PortError::from_message("E_LLM_CONFIG", e.to_string()))?;

    let started = Instant::now();
    let final_text = llm::rewrite_with_context_config(
        &data_dir,
        task_id,
        &config,
        llm::RewriteRequest {
            system_prompt: &plan.prompt,
            asr_text: &req.text,
            context: Some(&prepared),
            glossary: glossary_ref,
            policy: &policy,
        },
    )
    .await
    .map_err(|e| PortError::from_message("E_LLM_FAILED", e.to_string()))?;
    Ok(RewriteResult {
        transcript_id: task_id.to_string(),
        final_text,
        rewrite_ms: started.elapsed().as_millis(),
    })
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
        };

        assert_eq!(result.transcript_id, "task-1");
        assert_eq!(result.final_text, "rewritten");
    }
}
