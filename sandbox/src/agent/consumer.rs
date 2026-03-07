//! Message consumer — fetches pending messages from the backend via gRPC.
//!
//! Invoked directly (not as a background loop) from:
//! - `init::run_init()` after `InitPhase::Ready` is set
//! - `grpc_server::notify_messages()` when backend signals new messages
//! - `claude_code::spawn_agent_execution()` after agent completes

use std::future::Future;
use std::pin::Pin;

use tokio::time::Duration;
use tracing::{debug, info, warn};

use crate::state::SandboxState;

use super::claude_code::spawn_agent_execution;
use super::prompt::PromptResponse;
use super::types::{AgentRunRequest, AgentStatus, ContentBlock, ResumeSession};

/// Fetch and process any pending messages from the backend (single cycle).
///
/// This is a one-shot function — it fetches once, processes, and returns.
/// Returns a boxed future to break the recursive type (retry spawns itself).
pub fn process_pending_messages(state: SandboxState) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
    process_pending_messages_inner(state).await;
    })
}

async fn process_pending_messages_inner(state: SandboxState) {
    let task_id = &state.config.task_id;

    // Guard: if agent is running and there's no pending interactive prompt,
    // skip the fetch — messages stay pending in DB until agent finishes.
    let status = state.agent.status().await;
    if status == AgentStatus::Running && !state.agent.has_pending_prompt().await {
        debug!("Agent is running, deferring message fetch");
        state.agent.set_messages_pending().await;
        return;
    }

    // Fetch pending messages from backend via gRPC
    let proto_messages = match state.backend_client.get_pending_messages(task_id).await {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages via gRPC");
            // Schedule a retry after a short delay
            let retry_state = state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                process_pending_messages(retry_state).await;
            });
            return;
        }
    };

    if proto_messages.is_empty() {
        debug!("No pending messages");
        return;
    }

    // Convert proto messages to internal types
    let messages: Vec<InternalPendingMessage> = proto_messages
        .into_iter()
        .map(InternalPendingMessage::from_proto)
        .collect();

    info!(count = messages.len(), "Received pending messages");

    // Check if there's an active interactive prompt to resolve
    if state.agent.has_pending_prompt().await {
        if let Some(prompt) = state.agent.take_pending_prompt().await {
            // Try to convert the first message into a prompt response
            let first_msg = &messages[0];
            let text = extract_text_from_content(&first_msg.content);

            info!(tool = ?prompt.tool_name, "Resolving interactive prompt with pending message");

            let response = match prompt.tool_name {
                super::prompt::PromptToolName::AskUserQuestion => {
                    PromptResponse::AskUserAnswer {
                        answers: serde_json::json!({ "text": text }),
                    }
                }
                super::prompt::PromptToolName::ExitPlanMode => {
                    if text.to_lowercase().contains("reject") || text.to_lowercase().contains("no") {
                        PromptResponse::PlanRejected {
                            feedback: text,
                        }
                    } else {
                        PromptResponse::PlanApproved
                    }
                }
            };

            let _ = prompt.sender.send(response);
            return;
        }
        info!("Pending prompt was already consumed, continuing with messages");
    }

    // If agent is running (prompt path fell through), defer processing
    if state.agent.status().await == AgentStatus::Running {
        debug!("Agent is running after prompt check, deferring");
        return;
    }

    // Circuit breaker: stop retrying after 3 consecutive agent errors.
    // Defense in depth against any future retry loop scenarios.
    const MAX_CONSECUTIVE_ERRORS: u32 = 3;
    let consecutive = state.agent.consecutive_errors().await;
    if consecutive >= MAX_CONSECUTIVE_ERRORS {
        warn!(
            consecutive_errors = consecutive,
            "Circuit breaker: {consecutive} consecutive agent errors, publishing sandbox.fatal"
        );
        state.events.publish_sandbox_event(
            task_id,
            moatless_events::SandboxSubType::Error,
            serde_json::json!({
                "error": format!(
                    "Agent failed {consecutive} consecutive times. Last error: {}",
                    state.agent.last_error().await.unwrap_or_default()
                )
            }),
        ).await;
        return;
    }

    // Batch messages into a single request
    info!(message_count = messages.len(), "Batching messages and starting agent execution");
    let request = batch_messages(messages);
    spawn_agent_execution(state.clone(), request);
}

/// Internal pending message type (converted from proto).
struct InternalPendingMessage {
    content: Vec<ContentBlock>,
    is_error: bool,
    model: Option<String>,
    system_prompt: Option<String>,
    resume: Option<ResumeSession>,
    agent_type: Option<String>,
}

impl InternalPendingMessage {
    fn from_proto(msg: sandbox_proto::PendingMessage) -> Self {
        let content = super::types::content_blocks_from_proto(msg.content);

        // Resolve resume mode:
        // 1. If resume_session_id is set → --resume <id> (specific session)
        // 2. If resume=true but no session id → --continue (latest local session)
        // 3. Otherwise → no resume
        let resume = if let Some(session_id) = msg.resume_session_id {
            Some(ResumeSession::WithId(session_id))
        } else {
            msg.resume.filter(|&b| b).map(|_| ResumeSession::Last)
        };

        Self {
            content,
            is_error: msg.is_error.unwrap_or(false),
            model: msg.model,
            system_prompt: msg.system_prompt,
            resume,
            agent_type: msg.agent_type,
        }
    }
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

/// Batch multiple pending messages into a single AgentRunRequest.
fn batch_messages(messages: Vec<InternalPendingMessage>) -> AgentRunRequest {
    let mut all_content = Vec::new();
    let mut model = None;
    let mut system_prompt = None;
    let mut resume = None;
    let mut agent_type = None;

    for msg in messages {
        if msg.is_error {
            // Prefix error messages so the agent knows they're harness errors
            for block in msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        all_content.push(ContentBlock::Text {
                            text: format!("[System Error - Sandbox Harness]\n{text}"),
                        });
                    }
                    other => all_content.push(other),
                }
            }
        } else {
            all_content.extend(msg.content);
        }
        // Last message wins for options
        if msg.model.is_some() {
            model = msg.model;
        }
        if msg.system_prompt.is_some() {
            system_prompt = msg.system_prompt;
        }
        if msg.resume.is_some() {
            resume = msg.resume;
        }
        if msg.agent_type.is_some() {
            agent_type = msg.agent_type;
        }
    }

    AgentRunRequest {
        content: all_content,
        model,
        system_prompt,
        resume,
        agent_type,
    }
}
