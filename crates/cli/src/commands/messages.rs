//! Message commands: list.

use clap::Subcommand;
use moatless_api_schemas::message::UiMessage;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum MessageAction {
    /// List messages for a task
    List { task_id: String },
}

pub async fn run(
    client: &MoatClient,
    action: MessageAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        MessageAction::List { task_id } => {
            if json {
                let messages = client.api().get_messages(&task_id).await?;
                output::print_json_typed(&messages);
            } else {
                let messages = client.api().get_messages_summary(&task_id).await?;
                format_messages(&messages.messages);
            }
        }
    }
    Ok(())
}

fn format_messages(messages: &[UiMessage]) {
    let rows: Vec<Vec<String>> = messages
        .iter()
        .map(|m| {
            let role = m.role.to_string();
            let preview = message_preview(m);
            let session = m.session_id.as_deref().map_or_else(
                || "-".to_string(),
                |s| s.chars().take(8).collect::<String>(),
            );
            let created = m.timestamp.clone();
            vec![role, preview, session, created]
        })
        .collect();

    output::print_table(&["ROLE", "CONTENT", "SESSION", "TIMESTAMP"], &rows);
}

/// Build a short preview string for a message.
fn message_preview(msg: &UiMessage) -> String {
    // Try to find a text block first
    for block in &msg.content {
        if let Some(ref text) = block.text {
            if !text.is_empty() {
                return text.chars().take(80).collect();
            }
        }
        // Show tool_use as "[tool_name]"
        if block.block_type == "tool_use" {
            if let Some(ref name) = block.tool_name {
                return format!("[{name}]");
            }
        }
    }
    // Show tool result summary
    if !msg.tool_results.is_empty() {
        let errors = msg
            .tool_results
            .iter()
            .filter(|r| r.is_error == Some(true))
            .count();
        if errors > 0 {
            return format!(
                "[{} tool result(s), {} error(s)]",
                msg.tool_results.len(),
                errors
            );
        }
        return format!("[{} tool result(s)]", msg.tool_results.len());
    }
    // Result message
    if msg.result.is_some() {
        return "[result]".to_string();
    }
    "-".to_string()
}
