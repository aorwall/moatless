//! Sending a message to a task, as a call rather than as a route.
//!
//! This is the whole policy of a user-initiated send: prove the sender can
//! reach the task, follow it on their behalf, persist the message, and wake the
//! sandbox. It lived inside the HTTP handler until the T3 compatibility surface
//! needed the same behaviour from a WebSocket frame, where there is no request
//! body to parse and no response to build.
//!
//! [`send_message_flow`](super::send::MessageSendService::send_message_flow) below this is the
//! persistence step alone; it neither checks access nor wakes anything, which is
//! why the adapters and loops call it directly and callers acting for a
//! person call this instead.

use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::schemas::MessageContextInput;
use moatless_backend_core::scope::ScopeContext;

use super::error::MessageError;
use super::send;

/// What a person is sending, independent of how it reached the process.
#[derive(Debug, Default)]
pub struct UserMessage {
    pub text: String,
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
    /// Reasoning effort for the turn, in the agent's own vocabulary. `None`
    /// leaves the agent on its own default.
    pub effort: Option<String>,
    pub file_ids: Vec<String>,
    pub contexts: Vec<MessageContextInput>,
    pub resume: bool,
    pub source_task_id: Option<String>,
    pub start_fresh: bool,
    /// The message's uuid when the caller owns it. See
    /// [`send::MessageSendService::send_message_flow`].
    pub message_uuid: Option<String>,
    /// An answer to a question the agent asked, rather than a new message.
    /// Both halves are required; one alone is treated as a plain message,
    /// matching what the HTTP path did before this was extracted.
    pub tool_response: Option<ToolResponse>,
}

/// A response to an `AskUserQuestion` / `ExitPlanMode` tool call.
#[derive(Debug)]
pub struct ToolResponse {
    pub tool_use_id: String,
    pub tool_name: String,
    pub payload: Option<serde_json::Value>,
}

/// Who is sending, in the terms this module needs.
pub struct Sender<'a> {
    pub user_id: &'a str,
    pub scope: &'a ScopeContext,
    /// Agent and system contexts do not map to a followable user, so they skip
    /// the auto-follow.
    pub is_agent_context: bool,
}

/// Send `message` to `task_id` on `sender`'s behalf.
///
/// Returns the persisted user message's uuid so a client can reconcile its own
/// optimistic copy against the later `message.upsert`. A tool response produces
/// no single user message and therefore returns `None`.
///
/// Waking the sandbox is best-effort and deliberately not fatal: the message is
/// already persisted, and a failure to start is written into the conversation
/// where the person reading the task will see it.
pub async fn send_user_message(
    state: &AppState,
    sender: &Sender<'_>,
    task_id: &str,
    message: UserMessage,
) -> Result<Option<String>, MessageError> {
    state
        .tasks
        .find_by_id(task_id, sender.scope)
        .await?
        .ok_or_else(|| MessageError::NotFound(task_id.to_string()))?;

    // Auto-follow: writing to a task adds it to the sender's personal task
    // list. Best-effort — a follow failure must never block sending.
    if !sender.is_agent_context {
        if let Err(e) = state.tasks.follow_task(sender.user_id, task_id).await {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to auto-follow task on message send");
        }
        if let Err(e) = state
            .tasks
            .clear_snoozed_for_follower(sender.user_id, task_id)
            .await
        {
            tracing::warn!(
                error = %e,
                task_id = %task_id,
                user_id = %sender.user_id,
                "Failed to clear sender snooze on message send"
            );
        }
    }

    let message_uuid = match message.tool_response {
        Some(response) => {
            state
                .message_send
                .send_tool_response_flow(
                    task_id,
                    &response.tool_use_id,
                    &response.tool_name,
                    response.payload.as_ref(),
                    Some(sender.user_id),
                )
                .await?;
            None
        }
        None => {
            state
                .message_send
                .send_message_flow(
                    task_id,
                    &message.text,
                    message.model.as_deref(),
                    message.agent_mode.as_ref(),
                    message.effort.as_deref(),
                    &message.file_ids,
                    &message.contexts,
                    message.resume,
                    send::ExternalOrigin {
                        source_task_id: message.source_task_id.as_deref(),
                        ..send::ExternalOrigin::none()
                    },
                    message.start_fresh,
                    Some(sender.user_id),
                    message.message_uuid,
                )
                .await?
        }
    };

    if let Err(error_msg) = crate::sandbox::notify::ensure_sandbox_and_notify(state, task_id).await
    {
        state
            .message_send
            .persist_error_result(task_id, &error_msg)
            .await;
    }

    tracing::info!(task_id = %task_id, user_id = %sender.user_id, "Message sent to agent");

    Ok(message_uuid)
}
