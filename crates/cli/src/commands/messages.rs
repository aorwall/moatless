//! Message commands: list, reply, react.

use clap::Subcommand;
use moatless_api_schemas::message::{
    AddMessageReactionRequest, ReplyToMessageRequest, UiMessage,
};

use crate::client::MoatClient;
use crate::commands::misc::{read_text_or_stdin, upload_files};
use crate::output;

#[derive(Subcommand)]
pub enum MessageAction {
    /// List messages for a task
    List { task_id: String },

    /// Reply to a specific message. The backend picks the right destination
    /// (Slack thread, Linear comment, GitHub PR review reply, Telegram reply).
    Reply {
        /// Internal message id (uuid from `task_messages`).
        #[arg(long)]
        message: String,
        /// Reply text. Reads stdin if omitted and stdin is piped.
        #[arg(long)]
        text: Option<String>,
        /// File(s) to attach. Repeatable.
        #[arg(long = "file")]
        files: Vec<String>,
    },

    /// React to a specific message with an emoji.
    React {
        /// Internal message id (uuid from `task_messages`).
        #[arg(long)]
        message: String,
        /// Emoji shortcode (e.g. "thumbsup", "+1", "rocket").
        #[arg(long, short = 'e')]
        emoji: String,
        /// Remove a previously added reaction instead of adding one.
        #[arg(long)]
        remove: bool,
    },
}

pub async fn run(
    moat: &MoatClient,
    action: MessageAction,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        MessageAction::List { task_id } => {
            if json {
                let messages = moat.api().get_messages(&task_id).await?;
                output::print_json_typed(&messages);
            } else {
                let messages = moat.api().get_messages_summary(&task_id).await?;
                format_messages(&messages.messages);
            }
        }
        MessageAction::Reply {
            message,
            text,
            files,
        } => {
            let text = read_text_or_stdin(text)?;
            let file_ids = upload_files(moat, &files).await?;
            let req = ReplyToMessageRequest { text, file_ids };
            let resp = moat.api().reply_to_message(&message, &req).await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                match resp.external_id {
                    Some(ext) => println!("Reply sent to message {message} (external id: {ext})."),
                    None => println!("Reply sent to message {message}."),
                }
            }
        }
        MessageAction::React {
            message,
            emoji,
            remove,
        } => {
            if remove {
                moat.api().remove_message_reaction(&message, &emoji).await?;
                if json {
                    output::print_json_typed(&serde_json::json!({
                        "ok": true,
                        "removed": true,
                    }));
                } else {
                    println!("Removed reaction {emoji} from message {message}.");
                }
            } else {
                let req = AddMessageReactionRequest {
                    emoji: emoji.clone(),
                };
                let resp = moat.api().add_message_reaction(&message, &req).await?;
                if json {
                    output::print_json_typed(&resp);
                } else {
                    println!("Reacted with {emoji} to message {message}.");
                }
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
            vec![m.uuid.clone(), role, preview, session, created]
        })
        .collect();

    output::print_table(&["ID", "ROLE", "CONTENT", "SESSION", "TIMESTAMP"], &rows);
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
