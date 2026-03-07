//! Message commands: list, send.

use clap::Subcommand;

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
    },
}

pub async fn run(
    client: &MoatClient,
    action: MessageAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        MessageAction::List { task_id } => {
            let resp = client
                .get(&format!("/api/v1/tasks/{task_id}/messages"))
                .await?;
            if json {
                output::print_json(&resp);
            } else {
                format_messages(&resp);
            }
        }
        MessageAction::Send { task_id, text } => {
            let body = serde_json::json!({ "message": text });
            let resp = client
                .post(&format!("/api/v1/tasks/{task_id}/messages"), &body)
                .await?;
            if json {
                output::print_json(&resp);
            } else {
                println!("Message sent.");
            }
        }
    }
    Ok(())
}

fn format_messages(resp: &serde_json::Value) {
    let items = resp.as_array().cloned().unwrap_or_default();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|m| {
            let role = m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string();
            let content = m
                .get("content")
                .and_then(|v| v.as_str())
                .or_else(|| m.get("text").and_then(|v| v.as_str()))
                .unwrap_or("-");
            let preview: String = content.chars().take(80).collect();
            let created = m
                .get("createdAt")
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string();
            vec![role, preview, created]
        })
        .collect();

    output::print_table(&["ROLE", "CONTENT", "CREATED"], &rows);
}
