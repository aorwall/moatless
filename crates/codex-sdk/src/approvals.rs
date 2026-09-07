use async_trait::async_trait;
use codex_app_server_protocol::CommandExecutionApprovalDecision;
use codex_app_server_protocol::CommandExecutionRequestApprovalParams;
use codex_app_server_protocol::FileChangeApprovalDecision;
use codex_app_server_protocol::FileChangeRequestApprovalParams;
use codex_app_server_protocol::PermissionsRequestApprovalParams;
use codex_app_server_protocol::PermissionsRequestApprovalResponse;

/// Trait for handling approval requests from the Codex agent.
///
/// This is the Codex equivalent of the Claude SDK's `PermissionHandler`.
/// Codex has three distinct approval types (command execution, file changes,
/// and permission escalation) compared to Claude's single `can_use_tool`.
#[async_trait]
pub trait ApprovalHandler: Send + Sync {
    /// Called when the agent wants to execute a shell command.
    async fn on_command_approval(
        &self,
        params: &CommandExecutionRequestApprovalParams,
    ) -> CommandExecutionApprovalDecision;

    /// Called when the agent wants to modify files.
    async fn on_file_change_approval(
        &self,
        params: &FileChangeRequestApprovalParams,
    ) -> FileChangeApprovalDecision;

    /// Called when the agent requests additional permissions.
    /// Defaults to denying the request.
    async fn on_permissions_approval(
        &self,
        _params: &PermissionsRequestApprovalParams,
    ) -> Option<PermissionsRequestApprovalResponse> {
        None
    }
}

/// An approval handler that approves all requests.
pub struct AutoApproveHandler;

#[async_trait]
impl ApprovalHandler for AutoApproveHandler {
    async fn on_command_approval(
        &self,
        _params: &CommandExecutionRequestApprovalParams,
    ) -> CommandExecutionApprovalDecision {
        CommandExecutionApprovalDecision::Accept
    }

    async fn on_file_change_approval(
        &self,
        _params: &FileChangeRequestApprovalParams,
    ) -> FileChangeApprovalDecision {
        FileChangeApprovalDecision::Accept
    }
}

/// An approval handler that denies all requests (interrupts the turn).
pub struct DenyAllHandler;

#[async_trait]
impl ApprovalHandler for DenyAllHandler {
    async fn on_command_approval(
        &self,
        _params: &CommandExecutionRequestApprovalParams,
    ) -> CommandExecutionApprovalDecision {
        CommandExecutionApprovalDecision::Cancel
    }

    async fn on_file_change_approval(
        &self,
        _params: &FileChangeRequestApprovalParams,
    ) -> FileChangeApprovalDecision {
        FileChangeApprovalDecision::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_params() -> CommandExecutionRequestApprovalParams {
        serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "turnId": "turn1",
            "itemId": "item1",
            "startedAtMs": 0,
        }))
        .unwrap()
    }

    fn file_change_params() -> FileChangeRequestApprovalParams {
        serde_json::from_value(serde_json::json!({
            "threadId": "t1",
            "turnId": "turn1",
            "itemId": "item1",
            "startedAtMs": 0,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn test_auto_approve_command() {
        let handler = AutoApproveHandler;
        let decision = handler.on_command_approval(&command_params()).await;
        assert!(
            matches!(decision, CommandExecutionApprovalDecision::Accept),
            "expected Accept, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn test_auto_approve_file_change() {
        let handler = AutoApproveHandler;
        let decision = handler.on_file_change_approval(&file_change_params()).await;
        assert!(
            matches!(decision, FileChangeApprovalDecision::Accept),
            "expected Accept, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn test_deny_all_command() {
        let handler = DenyAllHandler;
        let decision = handler.on_command_approval(&command_params()).await;
        assert!(
            matches!(decision, CommandExecutionApprovalDecision::Cancel),
            "expected Cancel, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn test_deny_all_file_change() {
        let handler = DenyAllHandler;
        let decision = handler.on_file_change_approval(&file_change_params()).await;
        assert!(
            matches!(decision, FileChangeApprovalDecision::Cancel),
            "expected Cancel, got {decision:?}"
        );
    }
}
