//! Sending a message to a task, as a call rather than as a route.
//!
//! This is the whole policy of a user-initiated send: prove the sender can
//! reach the task, follow it on their behalf, persist the message, and wake the
//! sandbox — extracted so the T3 WebSocket path gets the same behavior the HTTP
//! handler had, with no request body or response to build.
//!
//! [`send_message_flow`](super::send::MessageSendService::send_message_flow) is
//! persistence alone — no access check, no wake — so adapters and loops call it
//! directly while callers acting for a person call this instead.

use crate::task::operations::TaskCommands;
use crate::task::schemas::MessageContextInput;
use moatless_ports::sandbox::SandboxWaker;
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentMode;

use super::error::MessageError;
use super::send::MessageSendService;

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
    /// Skills to activate for this turn.
    pub skills: Vec<String>,
    /// The message's uuid when the caller owns it. See
    /// [`super::send::MessageSendService::send_message_flow`].
    pub message_uuid: Option<String>,
    /// An answer to a question the agent asked, rather than a new message. Both
    /// halves are required; one alone is treated as a plain message, matching
    /// what the HTTP path did before this was extracted.
    pub tool_response: Option<ToolResponse>,
    /// Wait for the active turn to end rather than steering into it.
    pub queue: bool,
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
/// Returns the persisted user message's uuid so a client can reconcile its
/// optimistic copy against the later `message.upsert`; a tool response returns
/// `None`. Waking the sandbox is best-effort, since the message is already
/// persisted and a start failure is written into the conversation instead.
pub async fn send_user_message(
    tasks: &TaskCommands,
    message_send: &MessageSendService,
    sandbox_waker: &dyn SandboxWaker,
    sender: &Sender<'_>,
    task_id: &str,
    message: UserMessage,
) -> Result<Option<String>, MessageError> {
    tasks
        .find_by_id(task_id, sender.scope)
        .await?
        .ok_or_else(|| MessageError::NotFound(task_id.to_string()))?;

    // Auto-follow: writing to a task adds it to the sender's personal task
    // list. Best-effort — a follow failure must never block sending.
    if !sender.is_agent_context {
        if let Err(e) = tasks.follow_task(sender.user_id, task_id).await {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to auto-follow task on message send");
        }
        if let Err(e) = tasks
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
            message_send
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
            message_send
                .send_message_flow(
                    task_id,
                    &message.text,
                    message.model.as_deref(),
                    message.agent_mode.as_ref(),
                    message.effort.as_deref(),
                    &message.file_ids,
                    &message.contexts,
                    message.resume,
                    moatless_types::task::ExternalOrigin {
                        sender_id: None,
                        source_task_id: message.source_task_id.as_deref(),
                        ..moatless_types::task::ExternalOrigin::none()
                    },
                    message.start_fresh,
                    Some(sender.user_id),
                    &message.skills,
                    message.message_uuid,
                    message.queue,
                    None,
                )
                .await?
        }
    };

    if let Some(error_msg) = super::wake::wake_for_message(sandbox_waker, task_id).await {
        message_send.persist_error_result(task_id, &error_msg).await;
    }

    tracing::info!(task_id = %task_id, user_id = %sender.user_id, "Message sent to agent");

    Ok(message_uuid)
}

/// Steer a Message `sender` sent to queue into `task_id`'s active turn, and
/// wake the sandbox to fetch it. A Message already delivered is left alone.
pub async fn steer_queued_message(
    tasks: &TaskCommands,
    message_send: &MessageSendService,
    sandbox_waker: &dyn SandboxWaker,
    sender: &Sender<'_>,
    task_id: &str,
    message_uuid: &str,
) -> Result<(), MessageError> {
    tasks
        .find_by_id(task_id, sender.scope)
        .await?
        .ok_or_else(|| MessageError::NotFound(task_id.to_string()))?;
    if !message_send.steer_queued(task_id, message_uuid).await? {
        return Ok(());
    }
    if let Some(error_msg) = super::wake::wake_for_message(sandbox_waker, task_id).await {
        message_send.persist_error_result(task_id, &error_msg).await;
    }
    Ok(())
}
