//! Claude Code agent executor — wraps `claude_agent_sdk::query()` lifecycle.

use std::sync::Arc;

use claude_agent_sdk::SdkMessage;
use moatless_events::{AgentSubType, TaskTurnSubType};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::error::SandboxError;
use crate::state::SandboxState;

use super::options::build_options;
use super::types::AgentRunRequest;

/// Execute an agent run in the background.
///
/// Spawns a tokio task that runs the agent to completion, publishing
/// events to Redis along the way.
pub fn spawn_agent_execution(state: SandboxState, request: AgentRunRequest) {
    tokio::spawn(async move {
        if let Err(e) = execute_agent(&state, request).await {
            error!(error = %e, "Agent execution failed");
            state.agent.set_error(e.to_string()).await;

            // Publish error event
            publish_agent_event(
                &state,
                AgentSubType::Error,
                serde_json::json!({ "error": e.to_string() }),
            )
            .await;
        }

        // Check for deferred messages after agent finishes.
        // Only fetch if messages arrived during the run (flagged by consumer guard).
        let post_state = state.clone();
        tokio::spawn(async move {
            // Short delay to let session_init propagate to backend
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if post_state.agent.take_messages_pending().await {
                info!("Processing deferred messages after agent completion");
                crate::agent::consumer::process_pending_messages(post_state).await;
            }
        });
    });
}

/// Execute the agent and stream events.
async fn execute_agent(
    state: &SandboxState,
    request: AgentRunRequest,
) -> Result<(), SandboxError> {
    let cancel_token = CancellationToken::new();
    state.agent.set_running(cancel_token.clone()).await;

    let prompt = request.extract_text();
    let agent_type = request.agent_type.clone();
    let _agent_type_str = agent_type.as_deref().unwrap_or("claude-code");

    info!(
        task_id = %state.config.task_id,
        prompt_len = prompt.len(),
        agent_type = _agent_type_str,
        "Starting agent execution"
    );

    // Publish agent started + taskTurn.active events
    publish_agent_event(state, AgentSubType::Started, serde_json::json!({})).await;
    state
        .events
        .publish_task_turn_event(
            &state.config.task_id,
            TaskTurnSubType::Active,
            serde_json::json!({}),
        )
        .await;

    // Re-fetch credentials before each agent run to pick up rotated tokens
    if let Err(e) = crate::init::credentials::fetch_and_apply_credentials(state).await {
        warn!(error = %e, "Failed to refresh credentials, using cached");
    }

    // Read credentials from state to pass as env vars to the agent subprocess
    let credentials = state.credentials.read().await;
    let env = if credentials.is_empty() { None } else { Some(credentials.clone()) };
    drop(credentials);

    // Build options from config + request
    let options = build_options(
        &state.config,
        &request,
        cancel_token.clone(),
        Arc::clone(&state.agent),
        env,
    );

    // Start the query
    let mut handle = match claude_agent_sdk::query(&prompt, options).await {
        Ok(h) => h,
        Err(e) => {
            state.agent.set_error(e.to_string()).await;
            publish_agent_event(
                state,
                AgentSubType::Error,
                serde_json::json!({ "error": e.to_string() }),
            )
            .await;
            return Err(SandboxError::Sdk(e));
        }
    };

    // Process message stream
    let mut event_count = 0u64;
    while let Some(msg_result) = handle.messages.recv().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                warn!(error = %e, "SDK message error");
                if matches!(e, claude_agent_sdk::SdkError::Cancelled) {
                    break;
                }
                continue;
            }
        };

        event_count += 1;

        match msg {
            SdkMessage::System(init) => {
                info!(
                    session_id = %init.session_id,
                    model = %init.model,
                    "Agent initialized"
                );
                state.agent.set_session_id(init.session_id.clone()).await;

                // Publish session_init so backend can set session_id on user messages
                publish_agent_event(
                    state,
                    AgentSubType::SessionInit,
                    serde_json::json!({
                        "session_id": init.session_id,
                        "model": init.model,
                    }),
                )
                .await;
            }
            SdkMessage::Assistant(assistant) => {
                debug!(
                    uuid = %assistant.uuid,
                    blocks = assistant.message.content.len(),
                    "Assistant message received"
                );

                // Serialize the full message as the event payload
                let payload = serde_json::to_value(&assistant).unwrap_or_default();
                publish_agent_event(state, AgentSubType::Message, payload).await;
            }
            SdkMessage::Result(result) => {
                let (is_error, session_id, payload) = match &result {
                    claude_agent_sdk::types::messages::ResultMessage::Success(s) => {
                        if s.is_error {
                            warn!(
                                session_id = %s.session_id,
                                num_turns = s.num_turns,
                                duration_ms = s.duration_ms,
                                cost_usd = s.total_cost_usd,
                                "Agent completed with is_error=true"
                            );
                            warn!(output = %truncate(&s.result, 500), "Agent error output");
                        } else {
                            info!(
                                session_id = %s.session_id,
                                num_turns = s.num_turns,
                                duration_ms = s.duration_ms,
                                cost_usd = s.total_cost_usd,
                                "Agent completed successfully"
                            );
                            info!(output = %truncate(&s.result, 500), "Agent output");
                        }
                        (s.is_error, s.session_id.clone(), serde_json::to_value(s).unwrap_or_default())
                    }
                    claude_agent_sdk::types::messages::ResultMessage::Error(e) => {
                        warn!(
                            session_id = %e.session_id,
                            num_turns = e.num_turns,
                            duration_ms = e.duration_ms,
                            subtype = %e.subtype,
                            "Agent completed with error"
                        );
                        for err in &e.errors {
                            warn!(error = %err, "Agent error detail");
                        }
                        (true, e.session_id.clone(), serde_json::to_value(e).unwrap_or_default())
                    }
                };

                state.agent.set_session_id(session_id).await;

                // Always publish Success as "agent_completed" — the backend reads
                // the is_error flag from the payload and routes accordingly.
                // Only use "agent_error" for ResultMessage::Error or executor-level failures.
                let sub_type = match &result {
                    claude_agent_sdk::types::messages::ResultMessage::Success(_) => AgentSubType::Completed,
                    claude_agent_sdk::types::messages::ResultMessage::Error(_) => AgentSubType::Error,
                };
                publish_agent_event(state, sub_type, payload).await;

                // Record error in agent state so it's visible via status endpoint
                if is_error {
                    let error_text = match &result {
                        claude_agent_sdk::types::messages::ResultMessage::Success(s) => {
                            truncate(&s.result, 500).to_string()
                        }
                        claude_agent_sdk::types::messages::ResultMessage::Error(e) => {
                            e.errors.first().cloned().unwrap_or_else(|| "Unknown error".to_string())
                        }
                    };
                    state.agent.set_error(error_text).await;
                }

                info!(event_count, is_error, "Agent execution completed");
            }
            SdkMessage::User(user_msg) => {
                debug!(
                    uuid = ?user_msg.uuid,
                    blocks = user_msg.message.content.len(),
                    "User message (tool results) received"
                );
                let payload = serde_json::to_value(&user_msg).unwrap_or_default();
                publish_agent_event(state, AgentSubType::Message, payload).await;
            }
            // Forward other message types as generic events
            msg => {
                let variant = sdk_message_variant(&msg);
                debug!(event_count, variant, "SDK message");
            }
        }
    }

    state.agent.set_idle().await;

    info!(
        task_id = %state.config.task_id,
        event_count,
        "Agent execution finished"
    );

    Ok(())
}

/// Return a short label for an SdkMessage variant (for logging).
fn sdk_message_variant(msg: &SdkMessage) -> &'static str {
    match msg {
        SdkMessage::Assistant(_) => "assistant",
        SdkMessage::User(_) => "user",
        SdkMessage::UserReplay(_) => "user_replay",
        SdkMessage::Result(_) => "result",
        SdkMessage::System(_) => "system_init",
        SdkMessage::CompactBoundary(_) => "compact_boundary",
        SdkMessage::Status(_) => "status",
        SdkMessage::HookStarted(_) => "hook_started",
        SdkMessage::HookProgress(_) => "hook_progress",
        SdkMessage::HookResponse(_) => "hook_response",
        SdkMessage::TaskNotification(_) => "task_notification",
        SdkMessage::FilesPersisted(_) => "files_persisted",
        SdkMessage::StreamEvent(_) => "stream_event",
        SdkMessage::ToolProgress(_) => "tool_progress",
        SdkMessage::AuthStatus(_) => "auth_status",
        SdkMessage::ToolUseSummary(_) => "tool_use_summary",
    }
}

/// Truncate a string to at most `max` characters (on a char boundary).
fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Helper to publish an agent event via gRPC.
async fn publish_agent_event(
    state: &SandboxState,
    sub_type: AgentSubType,
    payload: serde_json::Value,
) {
    state.events
        .publish_agent_event(&state.config.task_id, sub_type, payload)
        .await;
}
