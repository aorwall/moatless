//! Message dispatcher — routes incoming message notifications.
//!
//! Invoked from:
//! - `init::run_init()` after `InitPhase::Ready` is set
//! - `grpc_server::notify_messages()` when backend signals new messages
//!
//! The dispatcher is a thin router — it does NOT check or modify agent state.
//! It only routes incoming message notifications to the right handler:
//! 1. Pending prompt exists → fetch messages, resolve prompt via oneshot
//! 2. Otherwise → call `run_agent()` which handles everything else

use tracing::{info, warn};

use crate::state::SandboxState;

use super::executor::run_agent;
use super::prompt::PromptResponse;
use super::types::ContentBlock;

/// Dispatch pending messages to the right handler (single cycle).
///
/// If the agent is waiting for user input, resolves the pending prompt.
/// Otherwise hands off to `run_agent()` which handles guard logic,
/// fetching, executing, and looping.
pub async fn dispatch_pending_messages(state: SandboxState) {
    // If agent is waiting for user input, resolve the prompt
    if state.agent.has_pending_prompt().await {
        resolve_pending_prompt(&state).await;
        return;
    }

    // Otherwise hand off to run_agent — it handles the guard,
    // fetching, executing, and looping.
    run_agent(state);
}

/// Resolve a pending interactive prompt with the first pending message.
async fn resolve_pending_prompt(state: &SandboxState) {
    let task_id = &state.config.task_id;

    // Fetch pending messages from backend via gRPC
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
        // Prompt was already consumed between our check and take
        info!("Pending prompt was already consumed, handing off to run_agent");
        run_agent(state.clone());
        return;
    };

    // Convert first message to content blocks and extract text
    let first_msg = &proto_messages[0];
    let content = super::types::content_blocks_from_proto(first_msg.content.clone());
    let text = extract_text_from_content(&content);

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

/// Extract text from content blocks.
fn extract_text_from_content(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{ImageSource, ImageSourceType};

    #[test]
    fn extract_text_joins_text_blocks() {
        let content = vec![
            ContentBlock::Text {
                text: "hello".to_string(),
            },
            ContentBlock::Text {
                text: "world".to_string(),
            },
        ];
        assert_eq!(extract_text_from_content(&content), "hello\nworld");
    }

    #[test]
    fn extract_text_skips_non_text_blocks() {
        let content = vec![
            ContentBlock::Text {
                text: "before".to_string(),
            },
            ContentBlock::Image {
                source: ImageSource {
                    source_type: ImageSourceType::Base64,
                    media_type: "image/png".to_string(),
                    data: "abc".to_string(),
                },
            },
            ContentBlock::Text {
                text: "after".to_string(),
            },
        ];
        assert_eq!(extract_text_from_content(&content), "before\nafter");
    }

    #[test]
    fn extract_text_empty_content() {
        assert_eq!(extract_text_from_content(&[]), "");
    }

    #[test]
    fn extract_text_single_block() {
        let content = vec![ContentBlock::Text {
            text: "only".to_string(),
        }];
        assert_eq!(extract_text_from_content(&content), "only");
    }
}
