//! Generic agent executor trait and orchestrator.
//!
//! The `AgentExecutor` trait defines a pure streaming interface: request in,
//! `AgentEvent`s out via channel. The `run_agent()` orchestrator consumes that
//! channel and handles everything else: state transitions, message acknowledgment,
//! and ALL event publishing to the backend. Zero state mutations or publishing
//! inside the executor.

use moatless_events::{AgentSubType, SandboxSubType, TaskTurnSubType};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::error::SandboxError;
use crate::state::SandboxState;

use super::types::{AgentRunRequest, AgentStatus, ContentBlock, ResumeSession};

/// Events emitted by an agent executor during a run.
///
/// The executor sends these through a channel; the caller handles
/// state updates, acknowledgment, and publishing.
pub enum AgentEvent {
    /// Agent session initialized — carry session_id for acknowledgment.
    SessionInit { session_id: String, model: String },
    /// Streaming content event (assistant message, user tool result, etc.).
    Message {
        sub_type: AgentSubType,
        payload: serde_json::Value,
    },
    /// Agent produced a final result. Carries completion metadata.
    Result {
        is_error: bool,
        sub_type: AgentSubType,
        payload: serde_json::Value,
    },
}

/// A pure streaming agent executor.
///
/// Contract:
/// - Caller sets `AgentState` to Running before calling.
/// - Executor MAY set `session_id` on `AgentState` (intrinsic agent identity).
/// - Executor does NOT set lifecycle state (running/idle/error) — caller does.
/// - Executor does NOT publish to backend — sends `AgentEvent`s via channel.
/// - Executor does NOT acknowledge messages — caller does on `SessionInit`.
#[async_trait::async_trait]
pub trait AgentExecutor: Send + Sync {
    /// Run the agent to completion, streaming events through `events_tx`.
    async fn execute(
        &self,
        state: &SandboxState,
        request: AgentRunRequest,
        cancel_token: CancellationToken,
        events_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<(), SandboxError>;
}

/// Spawn the self-contained agent orchestrator loop.
///
/// Each iteration: fetch pending messages → execute → check for more → loop.
/// Handles state transitions, event publishing, message acknowledgment, and
/// circuit breaker — the executor is a pure event source.
pub fn run_agent(state: SandboxState) {
    tokio::spawn(async move {
        run_agent_inner(state).await;
    });
}

async fn run_agent_inner(state: SandboxState) {
    let task_id = state.config.task_id.clone();

    // Guard: if agent is already running (another run_agent task),
    // flag messages as pending and return — the running task will
    // pick them up after its current execution.
    if state.agent.status().await == AgentStatus::Running {
        debug!("Agent is already running, flagging messages as pending");
        state.agent.set_messages_pending().await;
        return;
    }

    loop {
        // 1. Fetch pending messages — if none, we're done
        let (request, uuids) = match fetch_pending_batch(&state).await {
            Some(batch) => batch,
            None => break,
        };

        // 2. Circuit breaker — stop after 3 consecutive errors
        const MAX_CONSECUTIVE_ERRORS: u32 = 3;
        let consecutive = state.agent.consecutive_errors().await;
        if consecutive >= MAX_CONSECUTIVE_ERRORS {
            warn!(
                consecutive_errors = consecutive,
                "Circuit breaker: {consecutive} consecutive agent errors, publishing sandbox.error"
            );
            state
                .events
                .publish_sandbox_event(
                    &task_id,
                    SandboxSubType::Error,
                    serde_json::json!({
                        "error": format!(
                            "Agent failed {consecutive} consecutive times. Last error: {}",
                            state.agent.last_error().await.unwrap_or_default()
                        )
                    }),
                )
                .await;
            break;
        }

        // 3. State → Running
        let cancel_token = CancellationToken::new();
        state.agent.set_running(cancel_token.clone()).await;

        // 4. Publish lifecycle: started
        state
            .events
            .publish_agent_event(&task_id, AgentSubType::Started, serde_json::json!({}))
            .await;
        state
            .events
            .publish_task_turn_event(&task_id, TaskTurnSubType::Active, serde_json::json!({}))
            .await;

        // 5. Create event channel + handler
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let handler_state = state.clone();
        let handler_task_id = task_id.clone();
        let event_handler = tokio::spawn(async move {
            let mut pending_uuids = uuids;
            while let Some(event) = events_rx.recv().await {
                match event {
                    AgentEvent::SessionInit { session_id, model } => {
                        // Acknowledge pending messages with the real session ID
                        if !pending_uuids.is_empty() {
                            let uuids = std::mem::take(&mut pending_uuids);
                            let client = handler_state.backend_client.clone();
                            let tid = handler_task_id.clone();
                            let sid = session_id.clone();
                            let uuid_count = uuids.len();
                            tokio::spawn(async move {
                                match client.acknowledge_messages(&tid, uuids, &sid).await {
                                    Ok(()) => {
                                        info!(
                                            task_id = %tid,
                                            session_id = %sid,
                                            count = uuid_count,
                                            "Acknowledged pending messages"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            task_id = %tid,
                                            error = %e,
                                            "Failed to acknowledge pending messages"
                                        );
                                    }
                                }
                            });
                        }

                        handler_state
                            .events
                            .publish_agent_event(
                                &handler_task_id,
                                AgentSubType::SessionInit,
                                serde_json::json!({
                                    "session_id": session_id,
                                    "model": model,
                                }),
                            )
                            .await;
                    }
                    AgentEvent::Message { sub_type, payload } => {
                        handler_state
                            .events
                            .publish_agent_event(&handler_task_id, sub_type, payload)
                            .await;
                    }
                    AgentEvent::Result {
                        is_error: _,
                        sub_type,
                        payload,
                    } => {
                        handler_state
                            .events
                            .publish_agent_event(&handler_task_id, sub_type, payload)
                            .await;
                    }
                }
            }
        });

        // 6. Execute
        let result = state
            .executor
            .execute(&state, request, cancel_token, events_tx)
            .await;

        // Wait for event handler to drain
        let _ = event_handler.await;

        // 7. Check stop FIRST — stop_agent may have set Stopped while
        //    executor was finishing. Must check before set_idle/set_error
        //    to avoid overwriting Stopped.
        if state.agent.status().await == AgentStatus::Stopped {
            break;
        }

        // 8. State transition
        match result {
            Ok(()) => {
                state.agent.set_idle().await;
            }
            Err(ref e) => {
                state.agent.set_error(e.to_string()).await;
                state
                    .events
                    .publish_agent_event(
                        &task_id,
                        AgentSubType::Error,
                        serde_json::json!({ "error": e.to_string() }),
                    )
                    .await;
            }
        }

        // 9. If no messages arrived during execution, done
        if !state.agent.take_messages_pending().await {
            break;
        }
        // Otherwise loop continues → fetch next batch
        info!("Processing deferred messages after agent completion");
    }
}

/// Fetch pending messages from backend and batch into a request + UUIDs.
async fn fetch_pending_batch(state: &SandboxState) -> Option<(AgentRunRequest, Vec<String>)> {
    let messages = match state
        .backend_client
        .get_pending_messages(&state.config.task_id)
        .await
    {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages via gRPC");
            return None;
        }
    };
    if messages.is_empty() {
        debug!("No pending messages");
        return None;
    }

    let internal: Vec<InternalPendingMessage> = messages
        .into_iter()
        .map(InternalPendingMessage::from_proto)
        .collect();

    info!(count = internal.len(), "Received pending messages");

    let uuids: Vec<String> = internal.iter().filter_map(|m| m.uuid.clone()).collect();
    let request = batch_messages(internal);
    Some((request, uuids))
}

/// Internal pending message type (converted from proto).
struct InternalPendingMessage {
    content: Vec<ContentBlock>,
    is_error: bool,
    model: Option<String>,
    system_prompt: Option<String>,
    resume: Option<ResumeSession>,
    agent_type: Option<String>,
    /// Message UUID for per-message acknowledgment.
    uuid: Option<String>,
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
            uuid: msg.uuid,
        }
    }
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
