use std::sync::Arc;

use async_trait::async_trait;
use codex_sdk::{
    ApprovalHandler, AskForApproval, CodexMessage, CodexOptions, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalParams, FileChangeApprovalDecision,
    FileChangeRequestApprovalParams, query,
};

/// A custom approval handler that logs and approves command executions, but
/// denies file changes.
struct ReadOnlyApprovalHandler;

#[async_trait]
impl ApprovalHandler for ReadOnlyApprovalHandler {
    async fn on_command_approval(
        &self,
        params: &CommandExecutionRequestApprovalParams,
    ) -> CommandExecutionApprovalDecision {
        println!("[approval] command: {:?}", params.command);
        CommandExecutionApprovalDecision::Accept
    }

    async fn on_file_change_approval(
        &self,
        params: &FileChangeRequestApprovalParams,
    ) -> FileChangeApprovalDecision {
        println!("[approval] file change denied (item: {})", params.item_id);
        FileChangeApprovalDecision::Decline
    }
}

#[tokio::main]
async fn main() -> codex_sdk::Result<()> {
    let mut handle = query(
        "List the current directory contents",
        CodexOptions {
            approval_policy: Some(AskForApproval::UnlessTrusted),
            approval_handler: Some(Arc::new(ReadOnlyApprovalHandler)),
            ..Default::default()
        },
    )
    .await?;

    while let Some(msg) = handle.messages.recv().await {
        match msg? {
            CodexMessage::AgentMessageDelta(delta) => {
                print!("{}", delta.delta);
            }
            CodexMessage::TurnCompleted(_) => {
                println!("\n--- Turn completed ---");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
