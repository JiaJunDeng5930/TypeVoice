use serde::Deserialize;
use tauri::State;
use typevoice_core::workflow::{
    RunId, WorkflowCommandReply, WorkflowError, WorkflowIntent, WorkflowView,
};
use typevoice_engine::workflow_controller::WorkflowController;

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "command",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkflowCommandRequest {
    Primary { action_key: String },
    Cancel { target_run_id: RunId },
}

impl From<WorkflowCommandRequest> for WorkflowIntent {
    fn from(request: WorkflowCommandRequest) -> Self {
        match request {
            WorkflowCommandRequest::Primary { action_key } => Self::Primary { action_key },
            WorkflowCommandRequest::Cancel { target_run_id } => Self::Cancel { target_run_id },
        }
    }
}

#[tauri::command]
pub fn workflow_snapshot(workflow: State<'_, std::sync::Arc<WorkflowController>>) -> WorkflowView {
    workflow.snapshot()
}

#[tauri::command]
pub fn workflow_command(
    workflow: State<'_, std::sync::Arc<WorkflowController>>,
    req: serde_json::Value,
) -> Result<WorkflowCommandReply, WorkflowError> {
    let request = parse_workflow_command_request(req)?;
    workflow.inner().command(request.into())
}

fn parse_workflow_command_request(
    req: serde_json::Value,
) -> Result<WorkflowCommandRequest, WorkflowError> {
    serde_json::from_value(req).map_err(|_| {
        WorkflowError::new(
            "E_WORKFLOW_INTENT_INVALID",
            "workflow intent does not match the command schema",
        )
    })
}

#[cfg(test)]
pub fn command_names() -> &'static [&'static str] {
    &["workflow_snapshot", "workflow_command"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_surface_has_only_controller_entrypoints() {
        assert_eq!(command_names(), &["workflow_snapshot", "workflow_command"]);
    }

    #[test]
    fn command_envelope_is_strict() {
        let primary = parse_workflow_command_request(serde_json::json!({
            "command": "primary",
            "actionKey": "ready:initial"
        }));
        let unknown = parse_workflow_command_request(serde_json::json!({
            "command": "primary",
            "actionKey": "ready:initial",
            "legacy": true
        }));
        let missing_cancel_target = parse_workflow_command_request(serde_json::json!({
            "command": "cancel"
        }));
        assert!(primary.is_ok());
        assert_eq!(
            unknown.expect_err("unknown command field must fail").code,
            "E_WORKFLOW_INTENT_INVALID"
        );
        assert_eq!(
            missing_cancel_target
                .expect_err("Cancel without targetRunId must fail")
                .code,
            "E_WORKFLOW_INTENT_INVALID"
        );
    }
}
