//! Message dispatcher — routes incoming message notifications.
//!
//! Invoked from:
//! - `grpc_server::execute_agent()` when backend requests execution
//! - `grpc_server::notify_messages()` when backend signals new messages
//! - `init::run_init()` after `InitPhase::Ready` is set
//!
//! The dispatcher fetches pending messages from the backend, sends text to
//! the agent actor via `agent_handle.send()`, and acknowledges the messages.
//! The agent handles whether to start a new session or inject mid-turn.

use std::sync::Arc;

use tracing::{info, warn};

use crate::state::SandboxState;

use super::prompt::PromptResponse;
use super::types::{content_blocks_from_proto, ContentBlock};

/// Dispatch pending messages to the agent.
///
/// If the agent is waiting for user input (AskUserQuestion/ExitPlanMode),
/// resolves the pending prompt. Otherwise sends text to the agent actor.
pub async fn dispatch_pending_messages(state: SandboxState) {
    // If agent is waiting for user input, resolve the prompt
    if state.agent.has_pending_prompt().await {
        resolve_pending_prompt(&state).await;
        return;
    }

    // Fetch pending messages from backend
    let task_id = &state.config.task_id;
    let messages = match state.backend_client.get_pending_messages(task_id).await {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages");
            return;
        }
    };

    if messages.is_empty() {
        return;
    }

    // Extract UUIDs, text content, and system_prompt
    let uuids: Vec<String> = messages.iter().filter_map(|m| m.uuid.clone()).collect();
    let text = extract_text_from_messages(&messages);
    let system_prompt = extract_system_prompt(&messages);

    if text.is_empty() {
        return;
    }

    info!(
        message_count = messages.len(),
        text_len = text.len(),
        has_system_prompt = system_prompt.is_some(),
        "Dispatching messages to agent"
    );

    // Send to agent actor — it handles injection vs new session.
    // Ack is deferred until the agent confirms it processed the message.
    let ack_rx = state.agent_handle.send(text, system_prompt);

    if !uuids.is_empty() {
        let task_id = task_id.to_string();
        let backend = Arc::clone(&state.backend_client);
        tokio::spawn(async move {
            match ack_rx.await {
                Ok(session_id) => {
                    if let Err(e) = backend
                        .acknowledge_messages(&task_id, uuids, &session_id)
                        .await
                    {
                        warn!(error = %e, "Failed to acknowledge messages");
                    }
                }
                Err(_) => {
                    warn!("Agent ack channel dropped — messages not acknowledged");
                }
            }
        });
    }
}

/// Resolve a pending interactive prompt with the first pending message.
async fn resolve_pending_prompt(state: &SandboxState) {
    let task_id = &state.config.task_id;

    let proto_messages = match state.backend_client.get_pending_messages(task_id).await {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages for prompt resolution");
            return;
        }
    };

    if proto_messages.is_empty() {
        return;
    }

    let Some(prompt) = state.agent.take_pending_prompt().await else {
        // Prompt was already consumed between our check and take —
        // fall through to normal dispatch
        info!("Pending prompt was already consumed, dispatching normally");
        let text = extract_text_from_messages(&proto_messages);
        // Drop the ack receiver — no UUIDs to acknowledge in this race path
        let _ = state.agent_handle.send(text, None);
        return;
    };

    let text = extract_text_from_messages(&proto_messages);

    info!(tool = ?prompt.tool_name, "Resolving interactive prompt with pending message");

    let response = match prompt.tool_name {
        super::prompt::PromptToolName::AskUserQuestion => PromptResponse::AskUserAnswer {
            answers: serde_json::json!({ "text": text }),
        },
        super::prompt::PromptToolName::ExitPlanMode => {
            if text.to_lowercase().contains("reject") || text.to_lowercase().contains("no") {
                PromptResponse::PlanRejected { feedback: text }
            } else {
                PromptResponse::PlanApproved
            }
        }
    };

    let _ = prompt.sender.send(response);
}

/// Extract the system_prompt from proto pending messages (first non-None wins).
fn extract_system_prompt(messages: &[sandbox_proto::PendingMessage]) -> Option<String> {
    messages.iter().find_map(|m| m.system_prompt.clone())
}

/// Extract text from proto pending messages.
fn extract_text_from_messages(messages: &[sandbox_proto::PendingMessage]) -> String {
    messages
        .iter()
        .flat_map(|m| {
            let blocks = content_blocks_from_proto(m.content.clone());
            blocks.into_iter().filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text),
                _ => None,
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_joins_message_content() {
        let messages = vec![
            sandbox_proto::PendingMessage {
                content: vec![sandbox_proto::ContentBlock {
                    block: Some(sandbox_proto::content_block::Block::Text(
                        sandbox_proto::TextBlock {
                            text: "hello".to_string(),
                        },
                    )),
                }],
                ..Default::default()
            },
            sandbox_proto::PendingMessage {
                content: vec![sandbox_proto::ContentBlock {
                    block: Some(sandbox_proto::content_block::Block::Text(
                        sandbox_proto::TextBlock {
                            text: "world".to_string(),
                        },
                    )),
                }],
                ..Default::default()
            },
        ];
        assert_eq!(extract_text_from_messages(&messages), "hello\nworld");
    }

    #[test]
    fn extract_text_empty_messages() {
        assert_eq!(extract_text_from_messages(&[]), "");
    }
}
