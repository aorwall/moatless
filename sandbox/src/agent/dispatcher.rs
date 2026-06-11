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
use std::time::Duration;

use tracing::{info, warn};

use crate::files::uploads::fetch_uploads_for_messages;
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

    // Pre-fetch any attached uploaded files before invoking the agent so the
    // on-disk paths advertised in the prompt are valid by the time the agent
    // reads them. Best-effort: errors are logged but don't block dispatch.
    let file_ids: Vec<String> = messages
        .iter()
        .flat_map(|m| m.file_ids.iter().cloned())
        .collect();
    if !file_ids.is_empty() {
        fetch_uploads_for_messages(
            &state.backend_client,
            task_id,
            file_ids,
            &state.config.upload_dir,
        )
        .await;
    }

    // Extract UUIDs, text content, and resume_session_id
    let uuids: Vec<String> = messages.iter().filter_map(|m| m.uuid.clone()).collect();
    let text = extract_text_from_messages(&messages);
    let model = extract_model(&messages);
    let agent_type = extract_agent_type(&messages);
    let agent_mode = extract_agent_mode(&messages);
    let resume_session_id = messages.iter().find_map(|m| m.resume_session_id.clone());

    if text.is_empty() {
        return;
    }

    info!(
        message_count = messages.len(),
        text_len = text.len(),
        model = ?model,
        agent_type = ?agent_type,
        agent_mode = ?agent_mode,
        resume_session_id = ?resume_session_id,
        "Dispatching messages to agent"
    );

    // Send to agent actor — it handles injection vs new session.
    // Ack is deferred until the agent confirms it processed the message.
    let ack_rx = state
        .agent_handle
        .send(text, model, agent_type, agent_mode, resume_session_id);

    if !uuids.is_empty() {
        let task_id = task_id.to_string();
        let backend = Arc::clone(&state.backend_client);
        let redelivery_state = state.clone();
        tokio::spawn(async move {
            let Ok(session_id) = ack_rx.await else {
                warn!("Agent ack channel dropped — messages not acknowledged");
                schedule_redispatch(redelivery_state).await;
                return;
            };
            if let Err(e) = backend
                .acknowledge_messages(&task_id, uuids, &session_id)
                .await
            {
                warn!(error = %e, "Failed to acknowledge messages");
            }
        });
    }
}

/// How long to wait before re-fetching pending messages after a dropped ack —
/// enough for a follow-up agent start attempt to succeed (and ack) or fail
/// definitively, without hammering `GetPendingMessages` in a tight loop.
const REDISPATCH_BACKOFF: Duration = Duration::from_secs(15);

/// Skip re-dispatch once the agent has accumulated this many consecutive
/// errors — mirrors the agent actor's `MAX_CONSECUTIVE_ERRORS` circuit
/// breaker so a persistently failing agent can't loop dispatch → drop →
/// re-dispatch forever.
const MAX_REDISPATCH_CONSECUTIVE_ERRORS: u32 = 3;

/// Re-fetch and re-dispatch pending messages after the agent dropped an ack.
///
/// A dropped ack means the agent died before `SessionInit`, so the messages
/// were never acknowledged (`agent_session_id` stays NULL on the backend).
/// Without this, nothing re-delivers them until the next sandbox startup —
/// in production that meant a ~10 minute gap. Bounded: each re-dispatch only
/// schedules another retry if its own ack drops again, and the consecutive-
/// error gate stops the cycle once the agent is persistently failing.
///
/// Safe against double-delivery: the backend only returns messages that are
/// still unacknowledged, so if a later attempt acked them this fetch is empty.
///
/// Not an `async fn`: the recursion (`dispatch_pending_messages` → ack task →
/// here → `dispatch_pending_messages`) must go through a type-erased boxed
/// future, otherwise the compiler cannot resolve the futures' `Send` bounds
/// through the cycle.
fn schedule_redispatch(
    state: SandboxState,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        let consecutive_errors = state.agent.consecutive_errors().await;
        if consecutive_errors >= MAX_REDISPATCH_CONSECUTIVE_ERRORS {
            warn!(
                consecutive_errors,
                "Skipping pending-message re-dispatch — agent circuit breaker active"
            );
            return;
        }
        tokio::time::sleep(REDISPATCH_BACKOFF).await;
        dispatch_pending_messages(state).await;
    })
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
        drop(state.agent_handle.send(text, None, None, None, None));
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

fn extract_model(messages: &[sandbox_proto::PendingMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| m.model.clone())
}

fn extract_agent_type(messages: &[sandbox_proto::PendingMessage]) -> Option<String> {
    messages.iter().find_map(|m| m.agent_type.clone())
}

fn extract_agent_mode(messages: &[sandbox_proto::PendingMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| m.agent_mode.clone())
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
