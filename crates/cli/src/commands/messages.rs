//! Message commands: list, send.

use clap::Subcommand;
use moatless_api_schemas::message::{SendMessageRequest, UiMessage};
use moatless_api_schemas::shared::AgentMode;

use crate::client::{ClientError, MoatClient};
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
) -> Result<(), ClientError> {
    match action {
        MessageAction::List { task_id } => {
            let messages = client.api().get_messages(&task_id).await?;
            if json {
                output::print_json_typed(&messages);
            } else {
                format_messages(&messages.messages);
            }
        }
        MessageAction::Send { task_id, text, mode } => {
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
            // Extract text from content blocks via serde
            let preview: String = m
                .content
                .iter()
                .find_map(|block| {
                    let val = serde_json::to_value(block).ok()?;
                    val.get("text").and_then(|t| t.as_str()).map(String::from)
                })
                .unwrap_or_else(|| "-".to_string())
                .chars()
                .take(80)
                .collect();
            let created = m.timestamp.clone();
            vec![role, preview, created]
        })
        .collect();

    output::print_table(&["ROLE", "CONTENT", "CREATED"], &rows);
}
