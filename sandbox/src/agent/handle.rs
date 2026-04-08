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
use std::time::SystemTime;

use claude_agent_sdk::types::content::{ApiMessage, ContentBlock};
use claude_agent_sdk::types::messages::{MessagePriority, ResultMessage, UserMessage};
use claude_agent_sdk::{QueryHandle, SdkError, SdkMessage};
use moatless_events::{AgentSubType, SandboxSubType, TaskTurnSubType};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::Instant;
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
        /// Session ID from the backend for resuming after pod restart.
        resume_session_id: Option<String>,
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
    /// `resume_session_id` is the backend-tracked session ID for resuming after
    /// pod restart. Pass `None` when injecting into an active session or when
    /// no prior session is known.
    ///
    /// Returns a receiver that resolves with the session ID once the agent
    /// has processed the message:
    /// - **Injection** (session active): resolves immediately after successful stdin write.
    /// - **New session**: resolves on `SessionInit` with the real session ID.
    pub fn send(
        &self,
        text: String,
        system_prompt: Option<String>,
        resume_session_id: Option<String>,
    ) -> oneshot::Receiver<String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Send {
            text,
            system_prompt,
            resume_session_id,
            ack: ack_tx,
        });
        ack_rx
    }

    /// Stop the agent and shut down the actor.
    pub fn stop(&self) {
        let _ = self.command_tx.send(AgentCommand::Stop);
    }
}

// ── Retry constants ──────────────────────────────────────────────
const MAX_RETRIES: u32 = 5;
const BACKOFF_BASE_SECS: f64 = 5.0;
const BACKOFF_CAP_SECS: f64 = 60.0;
const JITTER_FACTOR: f64 = 0.2;

/// Pending retry state for transient agent errors.
struct RetryState {
    prompt: String,
    system_prompt: Option<String>,
    attempt: u32,
    retry_at: Instant,
}

/// Check if a `ResultMessage` represents a transient error worth retrying.
///
/// Retryable when the API returned overloaded/rate-limit/server errors and the
/// session barely started (≤5 turns), meaning there's little work to lose.
fn is_retryable_result(result: &ResultMessage) -> bool {
    match result {
        ResultMessage::Error(e) => {
            if e.num_turns > 5 {
                return false;
            }
            let subtype = e.subtype.as_str();
            let retryable_subtypes = [
                "overloaded",
                "rate_limit",
                "server_error",
                "capacity",
            ];
            if retryable_subtypes.iter().any(|s| subtype.contains(s)) {
                return true;
            }
            let retryable_patterns = ["529", "503", "502", "overloaded_error", "rate_limit"];
            e.errors.iter().any(|err| {
                let lower = err.to_lowercase();
                retryable_patterns.iter().any(|p| lower.contains(p))
            })
        }
        ResultMessage::Success(s) => {
            if !s.is_error || s.num_turns > 5 {
                return false;
            }
            let lower = s.result.to_lowercase();
            ["overloaded", "rate_limit", "529", "503", "502"]
                .iter()
                .any(|p| lower.contains(p))
        }
    }
}

/// Check if an SDK-level error is transient and worth retrying.
fn is_retryable_sdk_error(error: &SdkError) -> bool {
    matches!(error, SdkError::ProcessExit { .. } | SdkError::TransportClosed)
}

/// Compute exponential backoff with jitter for a given attempt (0-indexed).
fn compute_backoff(attempt: u32) -> std::time::Duration {
    let base = BACKOFF_BASE_SECS * 2.0_f64.powi(attempt as i32);
    let capped = base.min(BACKOFF_CAP_SECS);

    // Deterministic-ish jitter from system time nanos
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // Map nanos to [-1.0, 1.0]
    let jitter_norm = (nanos as f64 / u32::MAX as f64) * 2.0 - 1.0;
    let jitter = capped * JITTER_FACTOR * jitter_norm;

    let secs = (capped + jitter).max(1.0);
    std::time::Duration::from_secs_f64(secs)
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
    let mut retry_state: Option<RetryState> = None;
    let mut last_prompt: Option<String> = None;

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
                        // Clone Result messages before processing to check retryability
                        let retry_candidate = if let SdkMessage::Result(ref result) = sdk_msg {
                            Some(result.clone())
                        } else {
                            None
                        };

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

                        // After processing, check if we should retry
                        if let Some(result) = retry_candidate {
                            if is_retryable_result(&result) {
                                let attempt = retry_state.as_ref().map_or(0, |r| r.attempt + 1);
                                if attempt < MAX_RETRIES {
                                    if let Some(ref prompt) = last_prompt {
                                        // Undo the error increment — retries shouldn't count toward circuit breaker
                                        agent_state.undo_error_increment().await;
                                        consecutive_errors = consecutive_errors.saturating_sub(1);

                                        let backoff = compute_backoff(attempt);
                                        info!(
                                            attempt,
                                            max_retries = MAX_RETRIES,
                                            backoff_secs = backoff.as_secs_f64(),
                                            "Scheduling retry for transient error"
                                        );

                                        retry_state = Some(RetryState {
                                            prompt: prompt.clone(),
                                            system_prompt: last_system_prompt.clone(),
                                            attempt,
                                            retry_at: Instant::now() + backoff,
                                        });

                                        events.publish_agent_event(
                                            &task_id,
                                            AgentSubType::Retrying,
                                            serde_json::json!({
                                                "attempt": attempt,
                                                "max_retries": MAX_RETRIES,
                                                "backoff_secs": backoff.as_secs_f64(),
                                            }),
                                        ).await;
                                    }
                                } else {
                                    info!("Max retries ({MAX_RETRIES}) exhausted");
                                    retry_state = None;
                                }
                            } else {
                                // Successful or non-retryable — clear retry state
                                retry_state = None;
                            }
                        }
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

            // ── Retry timer fires ─────────────────────────────────
            () = async {
                match retry_state.as_ref() {
                    Some(rs) => tokio::time::sleep_until(rs.retry_at).await,
                    None => std::future::pending().await,
                }
            } => {
                // SAFETY: the async branch only completes when retry_state is Some
                let Some(rs) = retry_state.take() else {
                    continue;
                };
                info!(
                    attempt = rs.attempt,
                    "Retry timer fired, starting new session"
                );

                match start_session(
                    &rs.prompt,
                    &config,
                    Arc::clone(&agent_state),
                    &events,
                    &credentials,
                    &*backend_client,
                    &task_id,
                    &session_id,
                    rs.system_prompt.as_deref(),
                    None,
                ).await {
                    Ok(h) => {
                        handle = Some(h);
                    }
                    Err(e) => {
                        warn!(error = %e, attempt = rs.attempt, "Retry session start failed");
                        if is_retryable_sdk_error(&e) && rs.attempt + 1 < MAX_RETRIES {
                            let next_attempt = rs.attempt + 1;
                            let backoff = compute_backoff(next_attempt);
                            info!(
                                attempt = next_attempt,
                                backoff_secs = backoff.as_secs_f64(),
                                "Scheduling next retry after SDK error"
                            );
                            retry_state = Some(RetryState {
                                prompt: rs.prompt,
                                system_prompt: rs.system_prompt,
                                attempt: next_attempt,
                                retry_at: Instant::now() + backoff,
                            });
                            events.publish_agent_event(
                                &task_id,
                                AgentSubType::Retrying,
                                serde_json::json!({
                                    "attempt": next_attempt,
                                    "max_retries": MAX_RETRIES,
                                    "backoff_secs": backoff.as_secs_f64(),
                                }),
                            ).await;
                        } else {
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

            // ── Handle commands ────────────────────────────────────
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AgentCommand::Send { text, system_prompt, resume_session_id, ack }) => {
                        // New send supersedes any pending retry
                        if retry_state.is_some() {
                            info!("New send command cancels pending retry");
                            retry_state = None;
                        }

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

                        // Track the prompt for retry
                        last_prompt = Some(text.clone());

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
                                resume_session_id,
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

                                    if is_retryable_sdk_error(&e) && MAX_RETRIES > 0 {
                                        let backoff = compute_backoff(0);
                                        info!(
                                            backoff_secs = backoff.as_secs_f64(),
                                            "Scheduling retry after session start failure"
                                        );
                                        retry_state = Some(RetryState {
                                            prompt: text,
                                            system_prompt: last_system_prompt.clone(),
                                            attempt: 0,
                                            retry_at: Instant::now() + backoff,
                                        });
                                        events.publish_agent_event(
                                            &task_id,
                                            AgentSubType::Retrying,
                                            serde_json::json!({
                                                "attempt": 0,
                                                "max_retries": MAX_RETRIES,
                                                "backoff_secs": backoff.as_secs_f64(),
                                            }),
                                        ).await;
                                    } else {
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
                    }
                    Some(AgentCommand::Stop) | None => {
                        info!("Agent actor stopping");
                        drop(retry_state.take());
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
    resume_session_id: Option<String>,
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

    // Resume strategy (in priority order):
    // 1. In-memory previous_session_id (within same pod lifetime) → --resume <id>
    // 2. Backend-provided resume_session_id (survives pod restarts) → --resume <id>
    // 3. Fall back to --continue (resumes latest session from disk)
    // 4. If --continue fails → start a fresh session
    let is_first_session = previous_session_id.is_empty();
    let resume = if !previous_session_id.is_empty() {
        Some(super::types::ResumeSession::WithId(
            previous_session_id.to_string(),
        ))
    } else if let Some(ref sid) = resume_session_id {
        info!(resume_session_id = %sid, "Using backend-provided session ID for resume");
        Some(super::types::ResumeSession::WithId(sid.clone()))
    } else {
        Some(super::types::ResumeSession::Last)
    };

    let request = AgentRunRequest {
        content: Vec::new(),
        model: None,
        system_prompt: system_prompt.map(ToString::to_string),
        resume,
        agent_type: None,
        is_first_session,
    };

    let cancel_token = CancellationToken::new();
    agent_state.set_running(cancel_token.clone()).await;

    let options = build_options(config, &request, cancel_token.clone(), Arc::clone(&agent_state), env.clone());

    // Publish lifecycle events
    events
        .publish_agent_event(task_id, AgentSubType::Started, serde_json::json!({}))
        .await;
    events
        .publish_task_turn_event(task_id, TaskTurnSubType::Active, serde_json::json!({}))
        .await;

    info!(task_id = %task_id, resume = ?request.resume, "Starting agent session");

    match claude_agent_sdk::query(prompt, options).await {
        Ok(handle) => Ok(handle),
        Err(e) if request.resume == Some(super::types::ResumeSession::Last) => {
            // --continue failed (e.g. no prior session on disk) — retry as new session
            warn!(error = %e, "Resume with --continue failed, starting fresh session");
            let fresh_request = AgentRunRequest {
                resume: None,
                ..request
            };
            let fresh_options =
                build_options(config, &fresh_request, cancel_token, agent_state, env);
            claude_agent_sdk::query(prompt, fresh_options).await
        }
        Err(e) => Err(e),
    }
}

/// Process an SDK message from the active session.
#[allow(clippy::too_many_arguments)] // reason: actor internals, will refactor to a struct later
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
            let (is_error, sid, sub_type, payload) = extract_result_info(&result, task_id);
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
    task_id: &str,
) -> (bool, String, AgentSubType, serde_json::Value) {
    match result {
        ResultMessage::Success(s) => {
            if s.is_error {
                warn!(
                    task_id,
                    session_id = %s.session_id,
                    num_turns = s.num_turns,
                    duration_ms = s.duration_ms,
                    cost_usd = s.total_cost_usd,
                    output = %truncate(&s.result, 500),
                    "Agent completed with error output"
                );
            } else {
                info!(
                    task_id,
                    session_id = %s.session_id,
                    num_turns = s.num_turns,
                    duration_ms = s.duration_ms,
                    cost_usd = s.total_cost_usd,
                    output = %truncate(&s.result, 500),
                    "Agent completed successfully"
                );
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
                task_id,
                session_id = %e.session_id,
                num_turns = e.num_turns,
                duration_ms = e.duration_ms,
                subtype = %e.subtype,
                errors = ?e.errors,
                "Agent completed with error"
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use claude_agent_sdk::types::content::Usage;
    use claude_agent_sdk::types::messages::{ResultError, ResultSuccess};
    use std::collections::HashMap;

    fn make_result_error(subtype: &str, num_turns: u32, errors: Vec<String>) -> ResultMessage {
        ResultMessage::Error(ResultError {
            subtype: subtype.to_string(),
            uuid: "test-uuid".to_string(),
            session_id: "test-session".to_string(),
            duration_ms: 100,
            duration_api_ms: 50,
            is_error: true,
            num_turns,
            total_cost_usd: 0.0,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                service_tier: None,
                cache_creation: None,
                server_tool_use: None,
            },
            model_usage: HashMap::new(),
            permission_denials: vec![],
            errors,
        })
    }

    fn make_result_success(is_error: bool, num_turns: u32, result: &str) -> ResultMessage {
        ResultMessage::Success(ResultSuccess {
            uuid: "test-uuid".to_string(),
            session_id: "test-session".to_string(),
            duration_ms: 100,
            duration_api_ms: 50,
            is_error,
            num_turns,
            result: result.to_string(),
            total_cost_usd: 0.0,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                service_tier: None,
                cache_creation: None,
                server_tool_use: None,
            },
            model_usage: HashMap::new(),
            permission_denials: vec![],
            structured_output: None,
        })
    }

    #[test]
    fn is_retryable_result_overloaded_error() {
        let result = make_result_error("overloaded", 1, vec!["overloaded_error".to_string()]);
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_rate_limit_subtype() {
        let result = make_result_error("rate_limit", 0, vec![]);
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_529_in_errors() {
        let result = make_result_error("unknown", 2, vec!["HTTP 529 overloaded".to_string()]);
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_503_in_errors() {
        let result = make_result_error("unknown", 1, vec!["503 Service Unavailable".to_string()]);
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_not_retryable_after_many_turns() {
        let result = make_result_error("overloaded", 10, vec!["overloaded_error".to_string()]);
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_not_retryable_generic_error() {
        let result = make_result_error("unknown", 1, vec!["something else".to_string()]);
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_success_not_retryable() {
        let result = make_result_success(false, 5, "done");
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_success_with_overloaded_error_text() {
        let result = make_result_success(true, 1, "overloaded_error: API is overloaded");
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_success_error_too_many_turns() {
        let result = make_result_success(true, 10, "overloaded");
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_sdk_error_process_exit() {
        let err = SdkError::ProcessExit {
            code: Some(1),
            signal: None,
        };
        assert!(is_retryable_sdk_error(&err));
    }

    #[test]
    fn is_retryable_sdk_error_transport_closed() {
        assert!(is_retryable_sdk_error(&SdkError::TransportClosed));
    }

    #[test]
    fn is_retryable_sdk_error_not_retryable_cancelled() {
        assert!(!is_retryable_sdk_error(&SdkError::Cancelled));
    }

    #[test]
    fn is_retryable_sdk_error_not_retryable_json() {
        let err = SdkError::Internal("test".to_string());
        assert!(!is_retryable_sdk_error(&err));
    }

    #[test]
    fn compute_backoff_increases_with_attempt() {
        let b0 = compute_backoff(0);
        let b1 = compute_backoff(1);
        let b2 = compute_backoff(2);
        // With jitter, we can't be exact, but the general trend should hold.
        // Base values: 5, 10, 20 — even with +-20% jitter, b2 > b0.
        assert!(b2 > b0, "backoff should generally increase: b0={b0:?}, b2={b2:?}");
        // All should be at least 1 second
        assert!(b0.as_secs_f64() >= 1.0);
        assert!(b1.as_secs_f64() >= 1.0);
        assert!(b2.as_secs_f64() >= 1.0);
    }

    #[test]
    fn compute_backoff_caps_at_max() {
        // Attempt 10: base = 5 * 2^10 = 5120, capped at 60
        let b = compute_backoff(10);
        // With 20% jitter on 60s cap: range is [48, 72], but clamped at min 1.0
        assert!(b.as_secs_f64() <= 72.0, "backoff should be capped: {b:?}");
    }

    #[tokio::test]
    async fn undo_error_increment_decrements() {
        let state = AgentState::new();
        state.set_error("err1".to_string()).await;
        state.set_error("err2".to_string()).await;
        assert_eq!(state.consecutive_errors().await, 2);

        state.undo_error_increment().await;
        assert_eq!(state.consecutive_errors().await, 1);
    }

    #[tokio::test]
    async fn undo_error_increment_saturates_at_zero() {
        let state = AgentState::new();
        assert_eq!(state.consecutive_errors().await, 0);

        state.undo_error_increment().await;
        assert_eq!(state.consecutive_errors().await, 0);
    }
}
