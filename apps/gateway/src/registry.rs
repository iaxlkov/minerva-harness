use minerva_server_control::policy::Action;
/// Every registered RPC resolves inside the authenticated, assigned workspace worker.
/// Object IDs are local keys in that worker's isolated store; a UUID never selects another worker.
pub fn method_action(method: &str) -> Option<Action> {
    Some(match method {
        "plugins.models" | "session_list" | "model_providers" | "list_plugins" | "tools.list"
        | "interactions.list" | "mcp.list" => Action::List,
        "session_get" | "session_load" | "runs.get" | "shell_settings" | "workspace.get"
        | "context.status" | "attachments.get" => Action::Read,
        "session_create" | "session_save" | "session_set_model" | "attachments.put" => {
            Action::WriteFiles
        }
        "session_run" | "runs.start" | "run_shell_command" | "context.compact" => Action::Start,
        "session_cancel" | "runs.cancel" => Action::Cancel,
        "save_model_provider"
        | "discover_models"
        | "delete_model_provider"
        | "save_shell_settings"
        | "workspace.save"
        | "interactions.answer" => Action::Configure,
        "set_plugin_enabled"
        | "set_optional_plugins_enabled"
        | "plugins.call"
        | "plugins.ui"
        | "mcp.save"
        | "mcp.delete" => Action::ManagePlugins,
        _ => return None,
    })
}
#[derive(Debug, thiserror::Error)]
#[error("browser scope override is not allowed")]
pub struct ParamsError;
pub fn validate_params(params: &serde_json::Value) -> Result<(), ParamsError> {
    let object = params.as_object().ok_or(ParamsError)?;
    for key in [
        "workspaceId",
        "workspace_id",
        "tenantId",
        "tenant_id",
        "userId",
        "user_id",
        "workerId",
        "worker_id",
        "assignmentId",
        "assignment_id",
        "workerUrl",
        "worker_url",
    ] {
        if object.contains_key(key) {
            return Err(ParamsError);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_compat_method_has_explicit_policy_and_unknown_methods_deny() {
        for method in [
            "session_list",
            "session_create",
            "session_get",
            "session_load",
            "session_save",
            "session_set_model",
            "session_run",
            "session_cancel",
            "model_providers",
            "save_model_provider",
            "discover_models",
            "delete_model_provider",
            "list_plugins",
            "set_plugin_enabled",
            "set_optional_plugins_enabled",
            "shell_settings",
            "save_shell_settings",
            "run_shell_command",
            "runs.start",
            "runs.get",
            "runs.cancel",
            "plugins.call",
        ] {
            assert!(method_action(method).is_some(), "{method}");
        }
        assert_eq!(method_action("unregistered"), None);
        assert_eq!(method_action("run_shell_command"), Some(Action::Start));
        assert_eq!(
            method_action("set_plugin_enabled"),
            Some(Action::ManagePlugins)
        );
    }
    #[test]
    fn browser_cannot_supply_scope_or_assignment_in_rpc_params() {
        for key in [
            "workspaceId",
            "tenantId",
            "userId",
            "workerId",
            "assignmentId",
            "workerUrl",
        ] {
            assert!(validate_params(&serde_json::json!({key:"forged"})).is_err());
        }
        assert!(
            validate_params(&serde_json::json!({"sessionId":"local-id","runId":"local-run"}))
                .is_ok()
        );
    }
}
