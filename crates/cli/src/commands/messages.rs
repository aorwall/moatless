//! Message commands: list, reply, react.

use clap::Subcommand;
use moatless_api_schemas::message::{AddMessageReactionRequest, ReplyToMessageRequest, UiMessage};

use crate::client::MoatClient;
use crate::commands::input::{read_json_arg, read_text_or_stdin, upload_files};
use crate::output;

#[derive(Subcommand)]
pub enum MessageAction {
    /// List messages for a task
    List {
        task_id: String,
        /// Page size (default 50, max 200 — clamped by the backend).
        #[arg(long)]
        limit: Option<i32>,
        /// Cursor: fetch messages older than this internal id. Use the value
        /// from a previous page's pagination hint to walk back in history.
        #[arg(long)]
        before_id: Option<i32>,
        /// Print a full-text transcript (role, timestamp, complete message
        /// text, tool markers) instead of the preview table. Useful for
        /// recovering conversation context.
        #[arg(long)]
        full: bool,
    },

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
        /// Adapter-native rich payload, inline JSON or `@file`, naming the
        /// adapter it addresses (e.g. `{"kind":"slack","blocks":[...]}`).
        /// `--text` is still sent and is what the notification and task
        /// timeline show.
        #[arg(long)]
        rich: Option<String>,
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
        MessageAction::List {
            task_id,
            limit,
            before_id,
            full,
        } => {
            if json {
                let messages = moat
                    .api()
                    .get_messages_page(&task_id, false, limit, before_id)
                    .await?;
                output::print_json_typed(&messages);
            } else {
                // Summary projection: text blocks are complete; only heavy
                // tool inputs / results are stripped or truncated.
                let response = moat
                    .api()
                    .get_messages_page(&task_id, true, limit, before_id)
                    .await?;
                if full {
                    print!("{}", format_transcript(&response.messages));
                } else {
                    format_messages(&response.messages);
                }
                if response.has_more {
                    if let Some(cursor) = response.next_before_id {
                        println!("\nolder messages exist — rerun with --before-id {cursor}");
                    }
                }
            }
        }
        MessageAction::Reply {
            message,
            text,
            files,
            rich,
        } => {
            let text = read_text_or_stdin(text, "Pass the reply with `--text <text>`")?;
            let file_ids = upload_files(moat, &files).await?;
            let presentation = rich
                .as_deref()
                .map(|arg| read_json_arg("--rich", arg))
                .transpose()?;
            let req = ReplyToMessageRequest {
                text,
                file_ids,
                presentation,
            };
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

/// Render messages as a full-text transcript: one block per message with a
/// role + timestamp header, complete text, and compact tool-call/result
/// markers — for conversation-context recovery, e.g. re-reading task history
/// after a lost session.
fn format_transcript(messages: &[UiMessage]) -> String {
    use std::fmt::Write as _;

    // write! to a String is infallible — `let _ =` is safe throughout.
    let mut out = String::new();
    for msg in messages {
        let mut body = String::new();
        for block in &msg.content {
            if let Some(ref text) = block.text {
                if !text.is_empty() {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(text);
                }
            } else if block.block_type == "tool_use" {
                if let Some(ref name) = block.tool_name {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    let _ = write!(body, "  [tool: {name}]");
                }
            }
        }
        if !msg.tool_results.is_empty() {
            let errors = msg
                .tool_results
                .iter()
                .filter(|r| r.is_error == Some(true))
                .count();
            if !body.is_empty() {
                body.push('\n');
            }
            if errors > 0 {
                let _ = write!(
                    body,
                    "  [{} tool result(s), {errors} error(s)]",
                    msg.tool_results.len()
                );
            } else {
                let _ = write!(body, "  [{} tool result(s)]", msg.tool_results.len());
            }
        }
        if body.is_empty() {
            // Nothing user-visible (e.g. bare result row) — skip the message
            // entirely instead of printing an empty block.
            continue;
        }
        let _ = writeln!(out, "[{}] {}\n{body}\n", msg.role, msg.timestamp);
    }
    out
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

#[cfg(test)]
mod tests {
    use moatless_api_schemas::enums::{MessageRole, MessageType};
    use moatless_api_schemas::message::{UiContentBlock, UiToolResult};

    use super::*;

    fn text_block(text: &str) -> UiContentBlock {
        UiContentBlock {
            block_type: "text".to_string(),
            text: Some(text.to_string()),
            tool_use_id: None,
            tool_name: None,
            tool_input: None,
            show_in_ui: None,
            subagent_summary: None,
        }
    }

    fn tool_use_block(name: &str) -> UiContentBlock {
        UiContentBlock {
            block_type: "tool_use".to_string(),
            text: None,
            tool_use_id: Some("toolu_1".to_string()),
            tool_name: Some(name.to_string()),
            tool_input: None,
            show_in_ui: None,
            subagent_summary: None,
        }
    }

    fn message(role: MessageRole, content: Vec<UiContentBlock>) -> UiMessage {
        UiMessage {
            uuid: "uuid-1".to_string(),
            message_type: MessageType::Assistant,
            role,
            timestamp: "2026-06-10T05:54:09Z".to_string(),
            session_id: None,
            turn_number: None,
            agent_mode: None,
            model: None,
            is_queued: None,
            content,
            contexts: vec![],
            tool_results: vec![],
            has_pending_user_input: None,
            message_id: None,
            stop_reason: None,
            stop_sequence: None,
            usage: None,
            result: None,
            sdk_metadata: None,
            external_source: None,
            parent_uuid: None,
            source_task_id: None,
            sent_by: None,
        }
    }

    #[test]
    fn transcript_renders_full_text_with_role_headers() {
        let long_text = "a".repeat(500); // well past the table's 80-char preview
        let messages = vec![
            message(MessageRole::User, vec![text_block(&long_text)]),
            message(MessageRole::Assistant, vec![text_block("reply")]),
        ];
        let out = format_transcript(&messages);
        assert!(out.contains(&format!("[user] 2026-06-10T05:54:09Z\n{long_text}")));
        assert!(out.contains("[assistant] 2026-06-10T05:54:09Z\nreply"));
    }

    #[test]
    fn transcript_marks_tool_use_and_results() {
        let mut msg = message(
            MessageRole::Assistant,
            vec![text_block("running checks"), tool_use_block("Bash")],
        );
        msg.tool_results = vec![
            UiToolResult {
                tool_use_id: "toolu_1".to_string(),
                content: Some("ok".to_string()),
                is_error: None,
                typed_result: None,
            },
            UiToolResult {
                tool_use_id: "toolu_2".to_string(),
                content: Some("boom".to_string()),
                is_error: Some(true),
                typed_result: None,
            },
        ];
        let out = format_transcript(&[msg]);
        assert!(out.contains("running checks"));
        assert!(out.contains("  [tool: Bash]"));
        assert!(out.contains("  [2 tool result(s), 1 error(s)]"));
    }

    #[test]
    fn transcript_skips_messages_with_no_visible_content() {
        let empty = message(MessageRole::Assistant, vec![]);
        assert_eq!(format_transcript(&[empty]), "");
    }
}
