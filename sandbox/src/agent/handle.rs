//! Actor-style agent handle — owns the CLI session and exposes `send()`.
//!
//! The outside world (dispatcher, gRPC handlers) calls `agent.send(text)`.
//! Internally, the actor manages the `QueryHandle` lifecycle:
//! - If a session is active → inject the message via `send_user_message()`
//! - If no session → start a new one via `query()`
//!
//! The actor emits `AgentEvent`s through a channel for the event handler
//! to publish to the backend.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use claude_agent_sdk::types::content::{ApiMessage, ContentBlock};
use claude_agent_sdk::types::messages::{MessagePriority, ResultMessage, UserMessage};
use claude_agent_sdk::{QueryHandle, SdkMessage};
use moatless_events::{AgentSubType, SandboxSubType, TaskTurnSubType};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::events::GrpcEventPublisher;

use super::agent_state::AgentState;
use super::options::build_options;
use super::types::AgentRunRequest;

/// Commands sent to the agent actor.
enum AgentCommand {
    /// Send a message. The ack sender is resolved with the session ID once
    /// the agent confirms it processed the message.
    Send {
        text: String,
        system_prompt: Option<String>,
        ack: oneshot::Sender<String>,
    },
    /// Stop the agent and shut down the actor loop.
    Stop,
}

/// Handle to the agent actor — the only interface the outside world needs.
#[derive(Clone)]
pub struct AgentHandle {
    command_tx: mpsc::UnboundedSender<AgentCommand>,
}

impl AgentHandle {
    /// Send a message to the agent.
    ///
    /// Returns a receiver that resolves with the session ID once the agent
    /// has processed the message:
    /// - **Injection** (session active): resolves immediately after successful stdin write.
    /// - **New session**: resolves on `SessionInit` with the real session ID.
    pub fn send(&self, text: String, system_prompt: Option<String>) -> oneshot::Receiver<String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Send { text, system_prompt, ack: ack_tx });
        ack_rx
    }

    /// Stop the agent and shut down the actor.
    pub fn stop(&self) {
        let _ = self.command_tx.send(AgentCommand::Stop);
    }
}

/// Spawn the agent actor loop and return a handle.
pub fn spawn_agent(
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    events: GrpcEventPublisher,
    credentials: Arc<RwLock<HashMap<String, String>>>,
    backend_client: Arc<dyn BackendClient>,
) -> AgentHandle {
    let (command_tx, command_rx) = mpsc::unbounded_channel();

    tokio::spawn(agent_loop(
        command_rx,
        config,
        agent_state,
        events,
        credentials,
        backend_client,
    ));

    AgentHandle { command_tx }
}

/// The actor event loop.
///
/// Runs a `tokio::select!` that reads SDK messages from the active session
/// and processes incoming commands. Manages the full session lifecycle.
async fn agent_loop(
    mut command_rx: mpsc::UnboundedReceiver<AgentCommand>,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    events: GrpcEventPublisher,
    credentials: Arc<RwLock<HashMap<String, String>>>,
    backend_client: Arc<dyn BackendClient>,
) {
    let task_id = config.task_id.clone();
    let mut handle: Option<QueryHandle> = None;
    let mut session_id = String::new();
    let mut consecutive_errors: u32 = 0;
    let mut pending_acks: VecDeque<oneshot::Sender<String>> = VecDeque::new();
    let mut last_system_prompt: Option<String> = None;

    info!(task_id = %task_id, "Agent actor started");

    loop {
        tokio::select! {
            // ── Read from active session ───────────────────────────
            msg = async {
                match handle.as_mut() {
                    Some(h) => h.messages.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match msg {
                    Some(Ok(sdk_msg)) => {
                        process_sdk_message(
                            sdk_msg,
                            &agent_state,
                            &mut pending_acks,
                            &events,
                            &task_id,
                            &mut session_id,
                            &mut handle,
                            &mut consecutive_errors,
                        ).await;
                    }
                    Some(Err(e)) => {
                        if matches!(e, claude_agent_sdk::SdkError::Cancelled) {
                            info!("Session cancelled");
                            handle = None;
                        } else {
                            warn!(error = %e, "SDK message error");
                        }
                    }
                    None => {
                        // Transport closed (subprocess exited)
                        debug!("Session transport closed");
                        if handle.is_some() {
                            handle = None;
                            agent_state.set_idle().await;
                        }
                    }
                }
            }

            // ── Handle commands ────────────────────────────────────
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AgentCommand::Send { text, system_prompt, ack }) => {
                        // Store the latest system_prompt for use in start_session
                        if system_prompt.is_some() {
                            last_system_prompt = system_prompt;
                        }
                        // Circuit breaker
                        const MAX_CONSECUTIVE_ERRORS: u32 = 3;
                        if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            warn!(
                                consecutive_errors,
                                "Circuit breaker: {consecutive_errors} consecutive errors, rejecting send"
                            );
                            // Drop ack — receiver gets RecvError
                            drop(ack);
                            events.publish_sandbox_event(
                                &task_id,
                                SandboxSubType::Error,
                                serde_json::json!({
                                    "error": format!(
                                        "Agent failed {consecutive_errors} consecutive times. Last error: {}",
                                        agent_state.last_error().await.unwrap_or_default()
                                    )
                                }),
                            ).await;
                            continue;
                        }

                        if let Some(ref h) = handle {
                            // Session active — inject message
                            info!("Injecting message into running session");
                            let user_msg = make_user_message(&text, &session_id);
                            if let Err(e) = h.send_user_message(&user_msg).await {
                                warn!(error = %e, "Failed to inject message");
                                drop(ack);
                            } else {
                                // Write succeeded — resolve ack immediately
                                let _ = ack.send(session_id.clone());
                            }
                        } else {
                            // No active session — defer ack until SessionInit
                            pending_acks.push_back(ack);
                            match start_session(
                                &text,
                                &config,
                                Arc::clone(&agent_state),
                                &events,
                                &credentials,
                                &*backend_client,
                                &task_id,
                                &session_id,
                                last_system_prompt.as_deref(),
                            ).await {
                                Ok(h) => {
                                    handle = Some(h);
                                }
                                Err(e) => {
                                    warn!(error = %e, "Failed to start agent session");
                                    // Drain pending acks — session failed to start
                                    while let Some(ack) = pending_acks.pop_front() {
                                        drop(ack);
                                    }
                                    consecutive_errors += 1;
                                    agent_state.set_error(e.to_string()).await;
                                    events.publish_agent_event(
                                        &task_id,
                                        AgentSubType::Error,
                                        serde_json::json!({ "error": e.to_string() }),
                                    ).await;
                                }
                            }
                        }
                    }
                    Some(AgentCommand::Stop) | None => {
                        info!("Agent actor stopping");
                        if let Some(h) = handle.take() {
                            h.close();
                        }
                        agent_state.set_stopped().await;
                        events.publish_agent_event(
                            &task_id,
                            AgentSubType::Stopped,
                            serde_json::json!({}),
                        ).await;
                        break;
                    }
                }
            }
        }
    }

    info!(task_id = %task_id, "Agent actor exited");
}

/// Start a new CLI session.
#[allow(clippy::too_many_arguments)] // reason: actor internals, will refactor to a struct later
async fn start_session(
    prompt: &str,
    config: &Config,
    agent_state: Arc<AgentState>,
    events: &GrpcEventPublisher,
    credentials: &RwLock<HashMap<String, String>>,
    backend_client: &dyn BackendClient,
    task_id: &str,
    previous_session_id: &str,
    system_prompt: Option<&str>,
) -> Result<QueryHandle, claude_agent_sdk::SdkError> {
    // Re-fetch credentials before each run to pick up rotated tokens.
    // Best-effort — fall back to cached credentials on failure.
    match backend_client.get_credentials(task_id).await {
        Ok(resp) => {
            *credentials.write().await = resp.credentials;
        }
        Err(e) => {
            warn!(error = %e, "Failed to refresh credentials, using cached");
        }
    }

    let creds = credentials.read().await;
    let env = if creds.is_empty() {
        None
    } else {
        Some(creds.clone())
    };
    drop(creds);

    // Build a request for options (resume previous session if we have one)
    let request = AgentRunRequest {
        content: Vec::new(),
        model: None,
        system_prompt: system_prompt.map(ToString::to_string),
        resume: if previous_session_id.is_empty() {
            None
        } else {
            Some(super::types::ResumeSession::WithId(previous_session_id.to_string()))
        },
        agent_type: None,
    };

    let cancel_token = CancellationToken::new();
    agent_state.set_running(cancel_token.clone()).await;

    let options = build_options(config, &request, cancel_token, agent_state, env);

    // Publish lifecycle events
    events
        .publish_agent_event(task_id, AgentSubType::Started, serde_json::json!({}))
        .await;
    events
        .publish_task_turn_event(task_id, TaskTurnSubType::Active, serde_json::json!({}))
        .await;

    info!(task_id = %task_id, "Starting new agent session");

    claude_agent_sdk::query(prompt, options).await
}

/// Process an SDK message from the active session.
async fn process_sdk_message(
    msg: SdkMessage,
    agent_state: &AgentState,
    pending_acks: &mut VecDeque<oneshot::Sender<String>>,
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &mut String,
    handle: &mut Option<QueryHandle>,
    consecutive_errors: &mut u32,
) {
    match msg {
        SdkMessage::System(init) => {
            info!(
                session_id = %init.session_id,
                model = %init.model,
                "Agent initialized"
            );
            *session_id = init.session_id.clone();
            agent_state.set_session_id(init.session_id.clone()).await;

            // Resolve pending acks with the real session ID
            while let Some(ack) = pending_acks.pop_front() {
                let _ = ack.send(init.session_id.clone());
            }

            events
                .publish_agent_event(
                    task_id,
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
                "Assistant message"
            );
            let payload = serde_json::to_value(&assistant).unwrap_or_default();
            events
                .publish_agent_event(task_id, AgentSubType::Message, payload)
                .await;
        }
        SdkMessage::Result(result) => {
            let (is_error, sid, sub_type, payload) = extract_result_info(&result);
            *session_id = sid;
            agent_state.set_session_id(session_id.clone()).await;

            if is_error {
                let error_text = extract_error_text(&result);
                agent_state.set_error(error_text).await;
                *consecutive_errors += 1;
            } else {
                agent_state.set_idle().await;
                *consecutive_errors = 0;
            }

            events
                .publish_agent_event(task_id, sub_type, payload)
                .await;

            info!(is_error, "Agent execution completed");

            // Session is done — drop the handle so next Send starts a new one
            *handle = None;
        }
        SdkMessage::User(user_msg) => {
            debug!(
                uuid = ?user_msg.uuid,
                blocks = user_msg.message.content.len(),
                "User message (tool results)"
            );
            let payload = serde_json::to_value(&user_msg).unwrap_or_default();
            events
                .publish_agent_event(task_id, AgentSubType::Message, payload)
                .await;
        }
        _ => {
            // Status, hooks, compact boundary, etc. — log only
            debug!("SDK message (non-content)");
        }
    }
}

/// Construct a `UserMessage` for injection into a running session.
fn make_user_message(text: &str, session_id: &str) -> UserMessage {
    UserMessage {
        uuid: None,
        session_id: session_id.to_string(),
        message: ApiMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            usage: None,
            model: None,
            id: None,
            stop_reason: None,
            stop_sequence: None,
        },
        parent_tool_use_id: None,
        is_synthetic: None,
        tool_use_result: None,
        is_meta: None,
        is_compact_summary: None,
        is_visible_in_transcript_only: None,
        plan_content: None,
        thinking_metadata: None,
        todos: None,
        summarize_metadata: None,
        source_tool_assistant_uuid: None,
        source_tool_use_id: None,
        permission_mode: None,
        priority: MessagePriority::Next,
    }
}

/// Extract result info: (is_error, session_id, sub_type, payload).
fn extract_result_info(
    result: &ResultMessage,
) -> (bool, String, AgentSubType, serde_json::Value) {
    match result {
        ResultMessage::Success(s) => {
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
            (
                s.is_error,
                s.session_id.clone(),
                AgentSubType::Completed,
                serde_json::to_value(s).unwrap_or_default(),
            )
        }
        ResultMessage::Error(e) => {
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
            (
                true,
                e.session_id.clone(),
                AgentSubType::Error,
                serde_json::to_value(e).unwrap_or_default(),
            )
        }
    }
}

/// Extract a human-readable error text from a result message.
fn extract_error_text(result: &ResultMessage) -> String {
    match result {
        ResultMessage::Success(s) => truncate(&s.result, 500).to_string(),
        ResultMessage::Error(e) => e
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown error".to_string()),
    }
}

/// Truncate a string to at most `max` characters (on a char boundary).
fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}
