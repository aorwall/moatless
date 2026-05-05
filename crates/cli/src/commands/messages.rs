//! Message commands: list, send.

use clap::Subcommand;
use moatless_api_schemas::message::{SendMessageRequest, UiMessage};
use moatless_api_schemas::shared::AgentMode;

use crate::client::{ClientError, MoatClient};
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum MessageAction {
    /// List messages for a task
    List { task_id: String },
    /// Send a message to a task
    Send {
        task_id: String,
        #[arg(long)]
        text: String,
        /// Agent mode: "edit" or "plan"
        #[arg(long)]
        mode: Option<String>,
    },
}

pub async fn run(
    client: &MoatClient,
    action: MessageAction,
    json: bool,
    config: &Config,
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
        MessageAction::Send {
            task_id,
            text,
            mode,
        } => {
            let req = SendMessageRequest {
                message: text,
                agent_mode: mode.map(AgentMode),
                command_name: None,
                contexts: Vec::new(),
                file_ids: Vec::new(),
                model: None,
                resume: None,
                tool_name: None,
                tool_response: None,
                tool_use_id: None,
                source_task_id: config.task_id().map(std::string::ToString::to_string),
                start_fresh: None,
            };
            let resp = client.api().send_message(&task_id, &req).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!("Message sent.");
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
