//! Message consumer — fetches pending messages from the backend via gRPC.
//!
//! Runs as a background task, waking when notified via `NotifyMessages`
//! gRPC call, with a 30-second polling fallback to catch lost notifications.

use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn};

use crate::state::SandboxState;

use super::claude_code::spawn_agent_execution;
use super::prompt::PromptResponse;
use super::types::{AgentRunRequest, AgentStatus, ContentBlock};

/// Run the message consumer loop.
///
/// This task polls the backend via gRPC for pending user messages and either:
/// - Resolves an active interactive prompt (AskUserQuestion/ExitPlanMode)
/// - Starts a new agent execution with the pending messages
pub async fn run_consumer(state: SandboxState) {
    // Wait for initialization to complete before polling messages
    {
        use crate::init::InitPhase;
        let init_phase = state.init_phase.clone();
        loop {
            let phase = init_phase.read().await.clone();
            match phase {
                InitPhase::Ready => break,
                InitPhase::Failed(ref reason) => {
                    warn!(reason = %reason, "Init failed, message consumer exiting");
                    return;
                }
                _ => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    let task_id = state.config.task_id.clone();

    info!(task_id = %task_id, "Message consumer started (gRPC)");

    // Self-notify so the first loop iteration immediately fetches
    // any messages that were pending before the consumer started.
    state.consumer_notify.notify_one();

    loop {
        // Wait for notification or poll every 30 seconds (catches lost notifications)
        let _notified = timeout(
            Duration::from_secs(30),
            state.consumer_notify.notified(),
        )
        .await;
        debug!("Consumer woken (notification or poll timeout)");

        // Check agent status BEFORE fetching to prevent premature acknowledgment.
        // If agent is running, skip the fetch entirely — messages stay pending in DB.
        // The consumer will be re-notified when the agent completes (self-notify).
        let status = state.agent.status().await;
        if status == AgentStatus::Running {
            // Exception: if there's a pending interactive prompt, we still need to
            // fetch messages to resolve it (AskUserQuestion / ExitPlanMode).
            if !state.agent.has_pending_prompt().await {
                debug!("Agent is running, deferring message fetch");
                continue;
            }
        }

        // Fetch pending messages from backend via gRPC
        let proto_messages = match state.backend_client
            .get_pending_messages(&task_id)
            .await
        {
            Ok(msgs) => msgs,
            Err(e) => {
                warn!(error = %e, "Failed to fetch pending messages via gRPC");
                // Schedule a retry after a short delay
                let notify = state.consumer_notify.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    notify.notify_one();
                });
                continue;
            }
        };

        if proto_messages.is_empty() {
            debug!("No pending messages returned after notification");
            continue;
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
                continue;
            }
            info!("Pending prompt was already consumed, continuing with messages");
        }

        // If agent is running (prompt path fell through), defer processing
        if state.agent.status().await == AgentStatus::Running {
            debug!("Agent is running after prompt check, deferring");
            continue;
        }

        // Batch messages into a single request
        info!(message_count = messages.len(), "Batching messages and starting agent execution");
        let request = batch_messages(messages);
        spawn_agent_execution(state.clone(), request);
    }
}

/// Internal pending message type (converted from proto).
struct InternalPendingMessage {
    content: Vec<ContentBlock>,
    model: Option<String>,
    system_prompt: Option<String>,
    resume: Option<serde_json::Value>,
    agent_type: Option<String>,
}

impl InternalPendingMessage {
    fn from_proto(msg: sandbox_proto::PendingMessage) -> Self {
        let content = msg
            .content
            .into_iter()
            .filter_map(|block| {
                block.block.map(|b| match b {
                    sandbox_proto::content_block::Block::Text(t) => {
                        ContentBlock::Text { text: t.text }
                    }
                    sandbox_proto::content_block::Block::Image(img) => {
                        ContentBlock::Image {
                            source: super::types::ImageSource {
                                source_type: "url".to_string(),
                                media_type: img.media_type,
                                data: img.url,
                            },
                        }
                    }
                })
            })
            .collect();

        Self {
            content,
            model: msg.model,
            system_prompt: msg.system_prompt,
            resume: msg.resume.map(serde_json::Value::Bool),
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
        all_content.extend(msg.content);
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
