//! Actor-style agent handle — owns the CLI session and exposes `send()`.
//!
//! The outside world (dispatcher, gRPC handlers) calls `agent.send(text)`.
//! Internally, the actor manages the runtime lifecycle:
//! - Claude sessions stay open and accept injected user messages.
//! - Codex runs one turn per `codex exec` invocation and resumes on the next turn.
//!
//! The actor emits `AgentEvent`s through the shared event publisher.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::SystemTime;

use claude_agent_sdk::types::content::{ApiMessage, ContentBlock};
use claude_agent_sdk::types::messages::{MessagePriority, ResultMessage, UserMessage};
use claude_agent_sdk::{QueryHandle as ClaudeQueryHandle, SdkError, SdkMessage};
use moatless_events::{AgentSubType, SandboxSubType, TaskTurnSubType};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::events::GrpcEventPublisher;

use moatless_api_schemas::codex::{
    CodexCollabAgentState, CodexCollabAgentStatus, CodexCollabInput, CodexCollabResult,
    CodexCollabTool, CodexEditInput, CodexEditResult, CodexMcpInput, CodexMcpResult,
    CodexShellInput, CodexShellResult, CodexTodoItem, CodexTodoListInput, CodexToolStatus,
    CodexWebSearchInput, FileUpdateChange, FileUpdateKind, CODEX_TOOL_COLLAB, CODEX_TOOL_EDIT,
    CODEX_TOOL_MCP, CODEX_TOOL_SHELL, CODEX_TOOL_TODO_LIST, CODEX_TOOL_WEB_SEARCH,
};

use super::agent_state::AgentState;
use super::codex_exec::{
    self, CodexCliSession, CodexExecEvent, CollabAgentStatus as CodexExecCollabAgentStatus,
    CollabTool as CodexExecCollabTool, CollabToolCallItem, CollabToolCallStatus,
    CommandExecutionItem, CommandExecutionStatus, FileChangeItem, McpToolCallItem,
    McpToolCallStatus, PatchApplyStatus, ThreadItemDetails, TodoListItem, Usage, WebSearchItem,
};
use super::options::build_options;
use super::scheduled_job::{JobKind, ScheduledJobRegistry};
use super::types::{AgentRunRequest, ResumeSession};
use super::wakeup_job::WakeupJob;

const AGENT_TYPE_CLAUDE_CODE: &str = "claude-code";
const AGENT_TYPE_CODEX: &str = "codex";

enum ActiveHandle {
    Claude(ClaudeQueryHandle),
    Codex(CodexCliSession),
}

#[allow(clippy::large_enum_variant)] // Claude SDK messages are much larger than Codex JSON events.
enum ActiveMessage {
    Claude(Result<SdkMessage, SdkError>),
    Codex(Result<CodexExecEvent, String>),
}

#[derive(Debug, thiserror::Error)]
enum AgentRuntimeError {
    #[error(transparent)]
    Claude(#[from] claude_agent_sdk::SdkError),
    #[error("{0}")]
    Codex(String),
}

impl ActiveHandle {
    async fn send(&self, text: &str, session_id: &str) -> Result<(), AgentRuntimeError> {
        match self {
            Self::Claude(handle) => {
                let user_msg = make_user_message(text, session_id);
                handle.send_user_message(&user_msg).await?;
                Ok(())
            }
            Self::Codex(_) => Err(AgentRuntimeError::Codex(
                "Cannot inject into an active Codex turn".to_string(),
            )),
        }
    }

    fn close(&self) {
        match self {
            Self::Claude(handle) => handle.close(),
            Self::Codex(handle) => handle.cancel(),
        }
    }
}

/// Commands sent to the agent actor.
enum AgentCommand {
    /// Send a message. The ack sender is resolved with the session ID once
    /// the agent confirms it processed the message.
    Send {
        text: String,
        model: Option<String>,
        system_prompt: Option<String>,
        agent_type: Option<String>,
        agent_mode: Option<String>,
        /// Session ID from the backend for resuming after pod restart.
        resume_session_id: Option<String>,
        ack: oneshot::Sender<String>,
    },
    /// Stop the agent and shut down the actor loop.
    Stop,
}

/// Queued turn waiting for a runtime to become idle.
struct QueuedSend {
    text: String,
    model: Option<String>,
    system_prompt: Option<String>,
    agent_type: Option<String>,
    agent_mode: Option<String>,
    resume_session_id: Option<String>,
    ack: oneshot::Sender<String>,
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
    /// pod restart. Pass `None` when injecting into an active Claude session or
    /// when no prior session is known.
    ///
    /// Returns a receiver that resolves with the session ID once the agent
    /// has processed the message:
    /// - Claude injection: resolves immediately after a successful stdin write.
    /// - New Claude/Codex turn: resolves on `SessionInit` with the real session ID.
    pub fn send(
        &self,
        text: String,
        model: Option<String>,
        system_prompt: Option<String>,
        agent_type: Option<String>,
        agent_mode: Option<String>,
        resume_session_id: Option<String>,
    ) -> oneshot::Receiver<String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Send {
            text,
            model,
            system_prompt,
            agent_type,
            agent_mode,
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
    model: Option<String>,
    system_prompt: Option<String>,
    agent_type: Option<String>,
    agent_mode: Option<String>,
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
            let retryable_subtypes = ["overloaded", "rate_limit", "server_error", "capacity"];
            if retryable_subtypes.iter().any(|s| subtype.contains(s)) {
                return true;
            }
            let retryable_patterns = ["529", "503", "502", "overloaded_error", "rate_limit"];
            e.errors.iter().any(|err| {
                let lower = err.to_lowercase();
                retryable_patterns
                    .iter()
                    .any(|pattern| lower.contains(pattern))
            })
        }
        ResultMessage::Success(s) => {
            if !s.is_error || s.num_turns > 5 {
                return false;
            }
            let lower = s.result.to_lowercase();
            ["overloaded", "rate_limit", "529", "503", "502"]
                .iter()
                .any(|pattern| lower.contains(pattern))
        }
    }
}

/// Check if an SDK-level error is transient and worth retrying.
fn is_retryable_sdk_error(error: &SdkError) -> bool {
    matches!(
        error,
        SdkError::ProcessExit { .. } | SdkError::TransportClosed
    )
}

fn is_retryable_runtime_error(error: &AgentRuntimeError) -> bool {
    match error {
        AgentRuntimeError::Claude(error) => is_retryable_sdk_error(error),
        AgentRuntimeError::Codex(_) => false,
    }
}

fn normalize_agent_type(agent_type: Option<&str>) -> &'static str {
    match agent_type {
        Some(AGENT_TYPE_CODEX) => AGENT_TYPE_CODEX,
        Some(AGENT_TYPE_CLAUDE_CODE) | None => AGENT_TYPE_CLAUDE_CODE,
        Some(other) => {
            warn!(
                agent_type = other,
                "Unknown agent_type, defaulting to claude-code"
            );
            AGENT_TYPE_CLAUDE_CODE
        }
    }
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
    let jitter_norm = (nanos as f64 / u32::MAX as f64) * 2.0 - 1.0;
    let jitter = capped * JITTER_FACTOR * jitter_norm;

    let secs = (capped + jitter).max(1.0);
    std::time::Duration::from_secs_f64(secs)
}

/// Walk assistant content blocks for a `ScheduleWakeup` tool call and parse
/// it into a `WakeupJob`. Returns `None` if no such block is present or any
/// required field is missing.
fn extract_wakeup_job(content: &[ContentBlock]) -> Option<WakeupJob> {
    for block in content {
        if let ContentBlock::ToolUse { id, name, input, .. } = block {
            if name == "ScheduleWakeup" {
                return WakeupJob::from_tool_use(id, input);
            }
        }
    }
    None
}

/// Spawn the agent actor loop and return a handle.
#[allow(clippy::too_many_arguments)] // reason: actor wiring; refactor pending
pub fn spawn_agent(
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    events: GrpcEventPublisher,
    credentials: Arc<RwLock<HashMap<String, String>>>,
    backend_client: Arc<dyn BackendClient>,
    scheduled_jobs: Arc<ScheduledJobRegistry>,
) -> AgentHandle {
    let (command_tx, command_rx) = mpsc::unbounded_channel();

    tokio::spawn(agent_loop(
        command_rx,
        config,
        agent_state,
        events,
        credentials,
        backend_client,
        scheduled_jobs,
    ));

    AgentHandle { command_tx }
}

/// The actor event loop.
///
/// Runs a `tokio::select!` that reads runtime messages from the active session
/// and processes incoming commands. Manages the full session lifecycle.
#[allow(clippy::too_many_arguments)] // reason: actor wiring; refactor pending
async fn agent_loop(
    mut command_rx: mpsc::UnboundedReceiver<AgentCommand>,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    events: GrpcEventPublisher,
    credentials: Arc<RwLock<HashMap<String, String>>>,
    backend_client: Arc<dyn BackendClient>,
    scheduled_jobs: Arc<ScheduledJobRegistry>,
) {
    let task_id = config.task_id.clone();
    let mut handle: Option<ActiveHandle> = None;
    let mut session_id = String::new();
    let mut consecutive_errors: u32 = 0;
    let mut pending_acks: VecDeque<oneshot::Sender<String>> = VecDeque::new();
    let mut queued_sends: VecDeque<QueuedSend> = VecDeque::new();
    let mut codex_turn_output = String::new();
    let mut last_model: Option<String> = None;
    let mut last_system_prompt: Option<String> = None;
    let mut last_agent_type: Option<String> = None;
    let mut last_agent_mode: Option<String> = None;
    let mut retry_state: Option<RetryState> = None;
    let mut last_prompt: Option<String> = None;
    let mut pending_wakeup: Option<WakeupJob> = None;
    // Latest per-session cancel token — refreshed by `start_session` and reused
    // by post-turn wakeup arming so the registered wakeup honours the session's
    // cancellation scope.
    let mut run_cancel_for_loop: CancellationToken = CancellationToken::new();

    info!(task_id = %task_id, "Agent actor started");

    loop {
        if handle.is_none() && retry_state.is_none() {
            if let Some(send) = queued_sends.pop_front() {
                last_prompt = Some(send.text.clone());
                pending_acks.push_back(send.ack);

                match start_session(
                    &send.text,
                    &config,
                    Arc::clone(&agent_state),
                    &events,
                    &credentials,
                    &*backend_client,
                    &task_id,
                    &session_id,
                    send.model.as_deref(),
                    send.system_prompt.as_deref(),
                    send.agent_type.as_deref(),
                    send.agent_mode.as_deref(),
                    send.resume_session_id,
                    Arc::clone(&scheduled_jobs),
                )
                .await
                {
                    Ok((new_handle, run_cancel)) => {
                        handle = Some(new_handle);
                        run_cancel_for_loop = run_cancel;
                    }
                    Err(error) => {
                        warn!(error = %error, "Failed to start agent session");
                        drain_pending_acks(&mut pending_acks);

                        if is_retryable_runtime_error(&error) && MAX_RETRIES > 0 {
                            let backoff = compute_backoff(0);
                            info!(
                                backoff_secs = backoff.as_secs_f64(),
                                "Scheduling retry after session start failure"
                            );
                            retry_state = Some(RetryState {
                                prompt: send.text,
                                model: send.model,
                                system_prompt: send.system_prompt,
                                agent_type: send.agent_type,
                                agent_mode: send.agent_mode,
                                attempt: 0,
                                retry_at: Instant::now() + backoff,
                            });
                            events
                                .publish_agent_event(
                                    &task_id,
                                    AgentSubType::Retrying,
                                    serde_json::json!({
                                        "attempt": 0,
                                        "max_retries": MAX_RETRIES,
                                        "backoff_secs": backoff.as_secs_f64(),
                                    }),
                                )
                                .await;
                        } else {
                            consecutive_errors += 1;
                            agent_state.set_error(error.to_string()).await;
                            events
                                .publish_agent_event(
                                    &task_id,
                                    AgentSubType::Error,
                                    serde_json::json!({ "error": error.to_string() }),
                                )
                                .await;
                        }
                    }
                }

                continue;
            }
        }

        tokio::select! {
            msg = async {
                match handle.as_mut() {
                    Some(ActiveHandle::Claude(handle)) => {
                        handle.messages.recv().await.map(ActiveMessage::Claude)
                    }
                    Some(ActiveHandle::Codex(handle)) => {
                        handle.messages.recv().await.map(ActiveMessage::Codex)
                    }
                    None => std::future::pending::<Option<ActiveMessage>>().await,
                }
            } => {
                match msg {
                    Some(ActiveMessage::Claude(Ok(sdk_msg))) => {
                        let retry_candidate = if let SdkMessage::Result(ref result) = sdk_msg {
                            Some(result.clone())
                        } else {
                            None
                        };

                        process_claude_message(
                            sdk_msg,
                            &agent_state,
                            &mut pending_acks,
                            &events,
                            &task_id,
                            &mut session_id,
                            &mut handle,
                            &mut consecutive_errors,
                            &mut pending_wakeup,
                        ).await;

                        if let Some(ref result) = retry_candidate {
                            if is_retryable_result(result) {
                                let attempt = retry_state.as_ref().map_or(0, |state| state.attempt + 1);
                                if attempt < MAX_RETRIES {
                                    if let Some(ref prompt) = last_prompt {
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
                                            model: last_model.clone(),
                                            system_prompt: last_system_prompt.clone(),
                                            agent_type: last_agent_type.clone(),
                                            agent_mode: last_agent_mode.clone(),
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
                                retry_state = None;
                            }
                        }

                        // Arm the ScheduleWakeup timer if one was queued and the
                        // turn completed without error. Latest-wins: any prior
                        // pending wakeup is cancelled before the new one is
                        // registered.
                        if let Some(job) = pending_wakeup.take() {
                            let is_error = retry_candidate
                                .as_ref()
                                .is_some_and(|r| matches!(r, ResultMessage::Error(_)) || matches!(r, ResultMessage::Success(s) if s.is_error));

                            if is_error {
                                debug!("Ignoring ScheduleWakeup because turn ended with error");
                            } else {
                                let reason = job.reason().to_string();
                                info!(
                                    reason = %reason,
                                    wakeup_at = ?job.wakeup_at(),
                                    "Scheduling wakeup"
                                );
                                scheduled_jobs.cancel_by_kind(JobKind::Wakeup).await;
                                let registered = scheduled_jobs
                                    .register(Box::new(job), run_cancel_for_loop.clone())
                                    .await;
                                if registered {
                                    events.publish_agent_event(
                                        &task_id,
                                        AgentSubType::Scheduled,
                                        serde_json::json!({ "reason": reason }),
                                    ).await;
                                }
                            }
                        }
                    }
                    Some(ActiveMessage::Claude(Err(error))) => {
                        if matches!(error, claude_agent_sdk::SdkError::Cancelled) {
                            info!("Claude session cancelled");
                            handle = None;
                        } else {
                            warn!(error = %error, "Claude SDK message error");
                        }
                    }
                    Some(ActiveMessage::Codex(Ok(event))) => {
                        process_codex_message(
                            event,
                            last_model.as_deref(),
                            &agent_state,
                            &mut pending_acks,
                            &events,
                            &task_id,
                            &mut session_id,
                            &mut handle,
                            &mut consecutive_errors,
                            &mut codex_turn_output,
                        ).await;
                        retry_state = None;
                    }
                    Some(ActiveMessage::Codex(Err(error))) => {
                        handle_codex_stream_error(
                            &error,
                            last_model.as_deref(),
                            &agent_state,
                            &mut pending_acks,
                            &events,
                            &task_id,
                            &session_id,
                            &mut handle,
                            &mut consecutive_errors,
                            &mut codex_turn_output,
                        ).await;
                        retry_state = None;
                    }
                    None => {
                        debug!("Session transport closed");
                        if handle.is_some() {
                            handle = None;
                            agent_state.set_idle().await;
                        }
                    }
                }
            }

            () = async {
                match retry_state.as_ref() {
                    Some(state) => tokio::time::sleep_until(state.retry_at).await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(state) = retry_state.take() else {
                    continue;
                };

                info!(attempt = state.attempt, "Retry timer fired, starting new session");

                match start_session(
                    &state.prompt,
                    &config,
                    Arc::clone(&agent_state),
                    &events,
                    &credentials,
                    &*backend_client,
                    &task_id,
                    &session_id,
                    state.model.as_deref(),
                    state.system_prompt.as_deref(),
                    state.agent_type.as_deref(),
                    state.agent_mode.as_deref(),
                    None,
                    Arc::clone(&scheduled_jobs),
                ).await {
                    Ok((new_handle, run_cancel)) => {
                        handle = Some(new_handle);
                        run_cancel_for_loop = run_cancel;
                    }
                    Err(error) => {
                        warn!(error = %error, attempt = state.attempt, "Retry session start failed");
                        if is_retryable_runtime_error(&error) && state.attempt + 1 < MAX_RETRIES {
                            let next_attempt = state.attempt + 1;
                            let backoff = compute_backoff(next_attempt);
                            info!(
                                attempt = next_attempt,
                                backoff_secs = backoff.as_secs_f64(),
                                "Scheduling next retry after session start failure"
                            );
                            retry_state = Some(RetryState {
                                prompt: state.prompt,
                                model: state.model,
                                system_prompt: state.system_prompt,
                                agent_type: state.agent_type,
                                agent_mode: state.agent_mode,
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
                            agent_state.set_error(error.to_string()).await;
                            events.publish_agent_event(
                                &task_id,
                                AgentSubType::Error,
                                serde_json::json!({ "error": error.to_string() }),
                            ).await;
                        }
                    }
                }
            }

            cmd = command_rx.recv() => {
                match cmd {
                    Some(AgentCommand::Send { text, model, system_prompt, agent_type, agent_mode, resume_session_id, ack }) => {
                        if retry_state.is_some() {
                            info!("New send command cancels pending retry");
                            retry_state = None;
                        }

                        if model.is_some() {
                            last_model = model.clone();
                        }
                        if system_prompt.is_some() {
                            last_system_prompt = system_prompt.clone();
                        }
                        if agent_type.is_some() {
                            last_agent_type = agent_type.clone();
                        }
                        if agent_mode.is_some() {
                            last_agent_mode = agent_mode.clone();
                        }

                        const MAX_CONSECUTIVE_ERRORS: u32 = 3;
                        if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            warn!(
                                consecutive_errors,
                                "Circuit breaker: {consecutive_errors} consecutive errors, rejecting send"
                            );
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

                        let effective_send = QueuedSend {
                            text: text.clone(),
                            model: last_model.clone(),
                            system_prompt: last_system_prompt.clone(),
                            agent_type: last_agent_type.clone(),
                            agent_mode: last_agent_mode.clone(),
                            resume_session_id,
                            ack,
                        };
                        last_prompt = Some(text);

                        match handle.as_ref() {
                            Some(active_handle @ ActiveHandle::Claude(_)) => {
                                info!("Injecting message into running Claude session");
                                if let Err(error) = active_handle.send(&effective_send.text, &session_id).await {
                                    warn!(error = %error, "Failed to inject message into Claude session");
                                    drop(effective_send.ack);
                                } else {
                                    let _ = effective_send.ack.send(session_id.clone());
                                }
                            }
                            Some(ActiveHandle::Codex(_)) => {
                                info!("Queueing message until active Codex turn completes");
                                queued_sends.push_back(effective_send);
                            }
                            None => {
                                queued_sends.push_back(effective_send);
                            }
                        }
                    }
                    Some(AgentCommand::Stop) | None => {
                        info!("Agent actor stopping");
                        drop(retry_state.take());
                        drop(pending_wakeup.take());
                        while let Some(send) = queued_sends.pop_front() {
                            drop(send.ack);
                        }
                        drain_pending_acks(&mut pending_acks);
                        if let Some(active_handle) = handle.take() {
                            active_handle.close();
                        }
                        scheduled_jobs.cancel_all().await;
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
#[allow(clippy::too_many_arguments)] // reason: actor internals, will refactor later
async fn start_session(
    prompt: &str,
    config: &Config,
    agent_state: Arc<AgentState>,
    events: &GrpcEventPublisher,
    credentials: &RwLock<HashMap<String, String>>,
    backend_client: &dyn BackendClient,
    task_id: &str,
    previous_session_id: &str,
    model: Option<&str>,
    system_prompt: Option<&str>,
    agent_type: Option<&str>,
    agent_mode: Option<&str>,
    resume_session_id: Option<String>,
    scheduled_jobs: Arc<ScheduledJobRegistry>,
) -> Result<(ActiveHandle, CancellationToken), AgentRuntimeError> {
    match backend_client.get_credentials(task_id).await {
        Ok(resp) => {
            // Refresh mode: keep a locally-rotated `~/.codex/auth.json`. The
            // codex_watcher pushes rotations back to the backend; rewriting
            // here from the (pre-rotation) backend copy would clobber them
            // and trigger `refresh_token_reused` on the next codex call.
            crate::init::credentials::persist_agent_credentials(
                config,
                &resp.credentials,
                crate::init::credentials::PersistMode::Refresh,
            )
            .await;
            // Filter out git-provider tokens before passing to the agent subprocess env.
            // Git/gh ops inside the agent read tokens fresh from `~/.config/gh/hosts.yml`.
            *credentials.write().await =
                crate::init::credentials::filter_agent_credentials(&resp.credentials);
        }
        Err(error) => {
            warn!(error = %error, "Failed to refresh credentials, using cached");
        }
    }

    let creds = credentials.read().await;
    let env = if creds.is_empty() {
        None
    } else {
        Some(creds.clone())
    };
    drop(creds);

    let agent_type = normalize_agent_type(agent_type);

    let is_first_session = previous_session_id.is_empty();
    let resume = if !previous_session_id.is_empty() {
        Some(ResumeSession::WithId(previous_session_id.to_string()))
    } else if let Some(ref session_id) = resume_session_id {
        info!(resume_session_id = %session_id, "Using backend-provided session ID for resume");
        Some(ResumeSession::WithId(session_id.clone()))
    } else if agent_type == AGENT_TYPE_CLAUDE_CODE {
        Some(ResumeSession::Last)
    } else {
        None
    };

    let request = AgentRunRequest {
        content: Vec::new(),
        model: model.map(std::string::ToString::to_string),
        system_prompt: system_prompt.map(ToString::to_string),
        resume,
        agent_type: Some(agent_type.to_string()),
        agent_mode: agent_mode.map(std::string::ToString::to_string),
        is_first_session,
    };

    let cancel_token = CancellationToken::new();
    agent_state.set_running(cancel_token.clone()).await;

    events
        .publish_agent_event(task_id, AgentSubType::Started, serde_json::json!({}))
        .await;
    events
        .publish_task_turn_event(task_id, TaskTurnSubType::Active, serde_json::json!({}))
        .await;

    info!(
        task_id = %task_id,
        resume = ?request.resume,
        agent_type,
        "Starting agent session"
    );

    if agent_type == AGENT_TYPE_CODEX {
        return codex_exec::start(prompt, config, &request, env)
            .await
            .map(|h| (ActiveHandle::Codex(h), cancel_token))
            .map_err(|error| {
                AgentRuntimeError::Codex(format!("Failed to start Codex CLI: {error}"))
            });
    }

    let options = build_options(
        config,
        &request,
        cancel_token.clone(),
        Arc::clone(&agent_state),
        env.clone(),
        Arc::clone(&scheduled_jobs),
    );

    match claude_agent_sdk::query(prompt, options).await {
        Ok(handle) => Ok((ActiveHandle::Claude(handle), cancel_token)),
        Err(error) if request.resume == Some(ResumeSession::Last) => {
            warn!(error = %error, "Resume with --continue failed, starting fresh session");
            let fresh_request = AgentRunRequest {
                resume: None,
                ..request
            };
            let fresh_options = build_options(
                config,
                &fresh_request,
                cancel_token.clone(),
                agent_state,
                env,
                scheduled_jobs,
            );
            claude_agent_sdk::query(prompt, fresh_options)
                .await
                .map(|h| (ActiveHandle::Claude(h), cancel_token))
                .map_err(AgentRuntimeError::from)
        }
        Err(error) => Err(AgentRuntimeError::from(error)),
    }
}

async fn publish_session_init(
    agent_state: &AgentState,
    events: &GrpcEventPublisher,
    task_id: &str,
    pending_acks: &mut VecDeque<oneshot::Sender<String>>,
    session_id: &mut String,
    new_session_id: String,
    model: Option<String>,
) {
    *session_id = new_session_id.clone();
    agent_state.set_session_id(new_session_id.clone()).await;

    while let Some(ack) = pending_acks.pop_front() {
        let _ = ack.send(new_session_id.clone());
    }

    events
        .publish_agent_event(
            task_id,
            AgentSubType::SessionInit,
            serde_json::json!({
                "session_id": new_session_id,
                "model": model,
            }),
        )
        .await;
}

/// Process a Claude SDK message from the active session.
#[allow(clippy::too_many_arguments)] // reason: actor internals, will refactor later
async fn process_claude_message(
    msg: SdkMessage,
    agent_state: &AgentState,
    pending_acks: &mut VecDeque<oneshot::Sender<String>>,
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &mut String,
    handle: &mut Option<ActiveHandle>,
    consecutive_errors: &mut u32,
    pending_wakeup: &mut Option<WakeupJob>,
) {
    match msg {
        SdkMessage::System(init) => {
            info!(
                session_id = %init.session_id,
                model = %init.model,
                "Claude session initialized"
            );
            publish_session_init(
                agent_state,
                events,
                task_id,
                pending_acks,
                session_id,
                init.session_id,
                Some(init.model),
            )
            .await;
        }
        SdkMessage::Assistant(assistant) => {
            // Check for ScheduleWakeup tool call
            if let Some(job) = extract_wakeup_job(&assistant.message.content) {
                debug!(
                    reason = %job.reason(),
                    "ScheduleWakeup tool call detected"
                );
                *pending_wakeup = Some(job);
            }

            debug!(
                uuid = %assistant.uuid,
                blocks = assistant.message.content.len(),
                "Claude assistant message"
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

            events.publish_agent_event(task_id, sub_type, payload).await;

            *handle = None;
        }
        SdkMessage::User(user_msg) => {
            debug!(
                uuid = ?user_msg.uuid,
                blocks = user_msg.message.content.len(),
                "Claude user message"
            );
            let payload = serde_json::to_value(&user_msg).unwrap_or_default();
            events
                .publish_agent_event(task_id, AgentSubType::Message, payload)
                .await;
        }
        _ => {
            debug!("Claude SDK message (non-content)");
        }
    }
}

/// Process a Codex CLI event from the active turn.
#[allow(clippy::too_many_arguments)] // reason: actor internals, will refactor later
async fn process_codex_message(
    msg: CodexExecEvent,
    requested_model: Option<&str>,
    agent_state: &AgentState,
    pending_acks: &mut VecDeque<oneshot::Sender<String>>,
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &mut String,
    handle: &mut Option<ActiveHandle>,
    consecutive_errors: &mut u32,
    turn_output: &mut String,
) {
    match msg {
        CodexExecEvent::ThreadStarted(event) => {
            info!(
                thread_id = %event.thread_id,
                model = ?requested_model,
                "Codex session initialized"
            );
            publish_session_init(
                agent_state,
                events,
                task_id,
                pending_acks,
                session_id,
                event.thread_id,
                requested_model.map(std::string::ToString::to_string),
            )
            .await;
        }
        CodexExecEvent::TurnStarted(_) => {
            turn_output.clear();
        }
        CodexExecEvent::ItemStarted(event) | CodexExecEvent::ItemUpdated(event) => {
            match &event.item.details {
                ThreadItemDetails::TodoList(todo) => {
                    publish_codex_todo_list_call(
                        events,
                        task_id,
                        session_id,
                        &event.item.id,
                        todo,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::Other { type_name, .. } => {
                    debug!(type_name, "Codex item event (unknown kind)");
                }
                _ => {
                    debug!("Codex item event");
                }
            }
        }
        CodexExecEvent::ItemCompleted(event) => {
            let item_id = &event.item.id;
            match &event.item.details {
                ThreadItemDetails::AgentMessage(agent_message) => {
                    append_turn_output(turn_output, &agent_message.text);

                    // Codex item ids are short and only unique within a thread
                    // (item_0, item_1, ...). The task_messages table uses a
                    // global ON CONFLICT (uuid), so qualify the id with the
                    // session to avoid cross-task row collisions.
                    let uuid = format!("{session_id}_{item_id}");
                    let payload = codex_message_payload(
                        &uuid,
                        session_id,
                        &agent_message.text,
                        requested_model,
                    );
                    events
                        .publish_agent_event(task_id, AgentSubType::Message, payload)
                        .await;
                }
                ThreadItemDetails::Reasoning(reasoning) => {
                    let uuid = format!("{session_id}_{item_id}");
                    let payload = codex_reasoning_payload(
                        &uuid,
                        session_id,
                        &reasoning.text,
                        requested_model,
                    );
                    events
                        .publish_agent_event(task_id, AgentSubType::Message, payload)
                        .await;
                }
                ThreadItemDetails::CommandExecution(cmd) => {
                    publish_codex_shell_call(
                        events,
                        task_id,
                        session_id,
                        item_id,
                        cmd,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::FileChange(edit) => {
                    publish_codex_edit_call(
                        events,
                        task_id,
                        session_id,
                        item_id,
                        edit,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::TodoList(todo) => {
                    publish_codex_todo_list_call(
                        events,
                        task_id,
                        session_id,
                        item_id,
                        todo,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::McpToolCall(mcp) => {
                    publish_codex_mcp_call(
                        events,
                        task_id,
                        session_id,
                        item_id,
                        mcp,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::CollabToolCall(collab) => {
                    publish_codex_collab_call(
                        events,
                        task_id,
                        session_id,
                        item_id,
                        collab,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::WebSearch(web) => {
                    publish_codex_web_search_call(
                        events,
                        task_id,
                        session_id,
                        item_id,
                        web,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::Error(err) => {
                    append_turn_output(turn_output, &err.message);
                    publish_codex_error_item(
                        events,
                        task_id,
                        session_id,
                        item_id,
                        &err.message,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::Other { type_name, raw } => {
                    warn!(
                        item_id,
                        type_name,
                        raw = %raw,
                        "Unknown codex item type — skipped"
                    );
                }
            }
        }
        CodexExecEvent::TurnCompleted(event) => {
            let payload = codex_result_payload(
                Some(session_id.as_str()),
                false,
                "completed",
                turn_output.trim().to_string(),
                Some(&event.usage),
                requested_model,
            );

            agent_state.set_idle().await;
            *consecutive_errors = 0;

            events
                .publish_agent_event(task_id, AgentSubType::Completed, payload)
                .await;

            turn_output.clear();
            *handle = None;
        }
        CodexExecEvent::TurnFailed(event) => {
            drain_pending_acks(pending_acks);
            agent_state.set_error(event.error.message.clone()).await;
            *consecutive_errors += 1;

            let payload = codex_result_payload(
                (!session_id.is_empty()).then_some(session_id.as_str()),
                true,
                "failed",
                event.error.message,
                None,
                requested_model,
            );

            events
                .publish_agent_event(task_id, AgentSubType::Error, payload)
                .await;

            turn_output.clear();
            *handle = None;
        }
        CodexExecEvent::Error(event) => {
            drain_pending_acks(pending_acks);
            agent_state.set_error(event.message.clone()).await;
            *consecutive_errors += 1;

            let payload = codex_result_payload(
                (!session_id.is_empty()).then_some(session_id.as_str()),
                true,
                "error",
                event.message,
                None,
                requested_model,
            );

            events
                .publish_agent_event(task_id, AgentSubType::Error, payload)
                .await;

            turn_output.clear();
            *handle = None;
        }
    }
}

#[allow(clippy::too_many_arguments)] // reason: actor internals, will refactor later
async fn handle_codex_stream_error(
    error: &str,
    requested_model: Option<&str>,
    agent_state: &AgentState,
    pending_acks: &mut VecDeque<oneshot::Sender<String>>,
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    handle: &mut Option<ActiveHandle>,
    consecutive_errors: &mut u32,
    turn_output: &mut String,
) {
    warn!(error, "Codex stream error");
    drain_pending_acks(pending_acks);
    agent_state.set_error(error.to_string()).await;
    *consecutive_errors += 1;

    let payload = codex_result_payload(
        (!session_id.is_empty()).then_some(session_id),
        true,
        "error",
        error.to_string(),
        None,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Error, payload)
        .await;

    turn_output.clear();
    *handle = None;
}

fn codex_message_payload(
    uuid: &str,
    session_id: &str,
    text: &str,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{ "type": "text", "text": text }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
    })
}

/// Assistant text payload for a codex reasoning item, tagged so the UI
/// can render it collapsed.
fn codex_reasoning_payload(
    uuid: &str,
    session_id: &str,
    text: &str,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{ "type": "text", "text": text }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
        "sdk_metadata": { "codex_reasoning": true },
    })
}

/// Build an assistant message envelope containing a single `tool_use` block.
fn codex_tool_use_payload(
    uuid: &str,
    session_id: &str,
    tool_use_id: &str,
    tool_name: &str,
    tool_input: serde_json::Value,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{
            "type": "tool_use",
            "id": tool_use_id,
            "name": tool_name,
            "input": tool_input,
        }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
    })
}

/// Build a user message envelope containing a single `tool_result` block
/// referencing a prior `tool_use`. The `tool_use_result` field carries the
/// typed codex result for backend enrichment.
fn codex_tool_result_payload(
    uuid: &str,
    session_id: &str,
    tool_use_id: &str,
    content: &str,
    is_error: bool,
    tool_use_result: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }],
        },
        "tool_use_result": tool_use_result,
    })
}

/// Map a codex shell status + exit code to the shared typed status enum.
///
/// A `Completed` status with a non-zero exit code is surfaced as `Failed` so
/// the UI can flag it — codex itself only flips to `Failed` for catastrophic
/// failures, not for commands that ran successfully but returned non-zero.
fn command_execution_to_tool_status(
    status: CommandExecutionStatus,
    exit_code: Option<i32>,
) -> CodexToolStatus {
    match status {
        CommandExecutionStatus::InProgress => CodexToolStatus::InProgress,
        CommandExecutionStatus::Completed => match exit_code {
            Some(code) if code != 0 => CodexToolStatus::Failed,
            _ => CodexToolStatus::Completed,
        },
        CommandExecutionStatus::Failed => CodexToolStatus::Failed,
        CommandExecutionStatus::Declined => CodexToolStatus::Declined,
        // Forward-compat fallback: unknown status → treat as completed so the
        // turn is not stalled; the aggregated output will still render.
        CommandExecutionStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn patch_apply_to_tool_status(status: PatchApplyStatus) -> CodexToolStatus {
    match status {
        PatchApplyStatus::InProgress => CodexToolStatus::InProgress,
        PatchApplyStatus::Completed => CodexToolStatus::Completed,
        PatchApplyStatus::Failed => CodexToolStatus::Failed,
        PatchApplyStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn mcp_to_tool_status(status: McpToolCallStatus) -> CodexToolStatus {
    match status {
        McpToolCallStatus::InProgress => CodexToolStatus::InProgress,
        McpToolCallStatus::Completed => CodexToolStatus::Completed,
        McpToolCallStatus::Failed => CodexToolStatus::Failed,
        McpToolCallStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn collab_to_tool_status(status: CollabToolCallStatus) -> CodexToolStatus {
    match status {
        CollabToolCallStatus::InProgress => CodexToolStatus::InProgress,
        CollabToolCallStatus::Completed => CodexToolStatus::Completed,
        CollabToolCallStatus::Failed => CodexToolStatus::Failed,
        CollabToolCallStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn map_collab_tool(tool: CodexExecCollabTool) -> CodexCollabTool {
    match tool {
        CodexExecCollabTool::SpawnAgent => CodexCollabTool::SpawnAgent,
        CodexExecCollabTool::SendInput => CodexCollabTool::SendInput,
        CodexExecCollabTool::Wait => CodexCollabTool::Wait,
        CodexExecCollabTool::CloseAgent => CodexCollabTool::CloseAgent,
        CodexExecCollabTool::Unknown => CodexCollabTool::Unknown,
    }
}

fn map_collab_agent_status(status: CodexExecCollabAgentStatus) -> CodexCollabAgentStatus {
    match status {
        CodexExecCollabAgentStatus::PendingInit => CodexCollabAgentStatus::PendingInit,
        CodexExecCollabAgentStatus::Running => CodexCollabAgentStatus::Running,
        CodexExecCollabAgentStatus::Interrupted => CodexCollabAgentStatus::Interrupted,
        CodexExecCollabAgentStatus::Completed => CodexCollabAgentStatus::Completed,
        CodexExecCollabAgentStatus::Errored => CodexCollabAgentStatus::Errored,
        CodexExecCollabAgentStatus::Shutdown => CodexCollabAgentStatus::Shutdown,
        CodexExecCollabAgentStatus::NotFound => CodexCollabAgentStatus::NotFound,
        CodexExecCollabAgentStatus::Unknown => CodexCollabAgentStatus::Unknown,
    }
}

fn map_collab_agents_states(
    states: &HashMap<String, codex_exec::CollabAgentState>,
) -> std::collections::BTreeMap<String, CodexCollabAgentState> {
    states
        .iter()
        .map(|(id, state)| {
            (
                id.clone(),
                CodexCollabAgentState {
                    status: map_collab_agent_status(state.status),
                    message: state.message.clone(),
                },
            )
        })
        .collect()
}

fn parse_file_update_kind(raw: &str) -> FileUpdateKind {
    match raw {
        "delete" => FileUpdateKind::Delete,
        "update" => FileUpdateKind::Update,
        // Treat unknown kinds as `add` — the worst case is a misleading
        // badge rather than dropping the change.
        _ => FileUpdateKind::Add,
    }
}

fn build_shell_tool_use(cmd: &CommandExecutionItem) -> serde_json::Value {
    let input = CodexShellInput {
        command: cmd.command.clone(),
        cwd: cmd.cwd.clone(),
        command_actions: vec![],
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_shell_tool_result(
    cmd: &CommandExecutionItem,
) -> (serde_json::Value, String, bool) {
    let status = command_execution_to_tool_status(cmd.status, cmd.exit_code);
    let is_error = matches!(status, CodexToolStatus::Failed | CodexToolStatus::Declined);
    let summary_text = cmd.aggregated_output.clone().unwrap_or_default();

    let result = CodexShellResult {
        status,
        exit_code: cmd.exit_code,
        aggregated_output: cmd.aggregated_output.clone(),
        duration_ms: cmd.duration_ms,
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

fn build_edit_tool_use(edit: &FileChangeItem) -> serde_json::Value {
    let input = CodexEditInput {
        changes: edit
            .changes
            .iter()
            .map(|c| FileUpdateChange {
                path: c.path.clone(),
                kind: parse_file_update_kind(&c.kind),
                diff: c.diff.clone(),
            })
            .collect(),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_edit_tool_result(edit: &FileChangeItem) -> (serde_json::Value, String, bool) {
    let status = patch_apply_to_tool_status(edit.status);
    let is_error = matches!(status, CodexToolStatus::Failed | CodexToolStatus::Declined);

    let summary_text = if edit.changes.is_empty() {
        String::new()
    } else {
        edit.changes
            .iter()
            .map(|c| format!("{} {}", c.kind, c.path))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let result = CodexEditResult {
        status,
        changes: edit
            .changes
            .iter()
            .map(|c| FileUpdateChange {
                path: c.path.clone(),
                kind: parse_file_update_kind(&c.kind),
                diff: c.diff.clone(),
            })
            .collect(),
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

async fn publish_codex_shell_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    item_id: &str,
    cmd: &CommandExecutionItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{item_id}");
    let assistant_uuid = format!("{session_id}_{item_id}_call");
    let tool_input = build_shell_tool_use(cmd);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_SHELL,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_shell_tool_result(cmd);
    let result_uuid = format!("{session_id}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

async fn publish_codex_edit_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    item_id: &str,
    edit: &FileChangeItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{item_id}");
    let assistant_uuid = format!("{session_id}_{item_id}_call");
    let tool_input = build_edit_tool_use(edit);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_EDIT,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_edit_tool_result(edit);
    let result_uuid = format!("{session_id}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

fn build_codex_todo_list_input(todo: &TodoListItem) -> CodexTodoListInput {
    CodexTodoListInput {
        items: todo
            .items
            .iter()
            .map(|entry| CodexTodoItem {
                text: entry.text.clone(),
                completed: entry.completed,
            })
            .collect(),
    }
}

/// Publish a codex `todo_list` item as a tool_use block. Unlike shell/edit,
/// this is called on every `item.started` / `item.updated` / `item.completed`
/// for the same `item_id` — the stable `assistant_uuid` (session + item_id)
/// causes the backend DAO to upsert in place, so the UI sees one card that
/// mutates live as codex flips items to done. No `tool_result` is emitted;
/// codex's todo_list has no separate result concept.
async fn publish_codex_todo_list_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    item_id: &str,
    todo: &TodoListItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{item_id}");
    let assistant_uuid = format!("{session_id}_{item_id}_call");
    let input = build_codex_todo_list_input(todo);
    let tool_input = serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}));
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_TODO_LIST,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;
}

fn build_mcp_tool_use(item: &McpToolCallItem) -> serde_json::Value {
    let input = CodexMcpInput {
        server: item.server.clone(),
        tool: item.tool.clone(),
        arguments: item.arguments.clone(),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_mcp_tool_result(item: &McpToolCallItem) -> (serde_json::Value, String, bool) {
    let status = mcp_to_tool_status(item.status);
    let is_error = matches!(status, CodexToolStatus::Failed) || item.error.is_some();

    let (content, structured_content) = match &item.result {
        Some(result) => (
            Some(result.content.clone()),
            result.structured_content.clone(),
        ),
        None => (None, None),
    };
    let error_message = item.error.as_ref().map(|e| e.message.clone());

    let summary_text = if let Some(ref msg) = error_message {
        msg.clone()
    } else if let Some(ref content) = content {
        content
            .iter()
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    let result = CodexMcpResult {
        status,
        content,
        structured_content,
        error_message,
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

async fn publish_codex_mcp_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    item_id: &str,
    mcp: &McpToolCallItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{item_id}");
    let assistant_uuid = format!("{session_id}_{item_id}_call");
    let tool_input = build_mcp_tool_use(mcp);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_MCP,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_mcp_tool_result(mcp);
    let result_uuid = format!("{session_id}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

fn build_collab_tool_use(item: &CollabToolCallItem) -> serde_json::Value {
    let input = CodexCollabInput {
        tool: map_collab_tool(item.tool),
        sender_thread_id: item.sender_thread_id.clone(),
        receiver_thread_ids: item.receiver_thread_ids.clone(),
        prompt: item.prompt.clone(),
        agents_states: map_collab_agents_states(&item.agents_states),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_collab_tool_result(item: &CollabToolCallItem) -> (serde_json::Value, String, bool) {
    let status = collab_to_tool_status(item.status);
    let agents_states = map_collab_agents_states(&item.agents_states);
    let any_errored = agents_states
        .values()
        .any(|state| state.status == CodexCollabAgentStatus::Errored);
    let is_error = matches!(status, CodexToolStatus::Failed) || any_errored;

    let summary_text = agents_states
        .iter()
        .map(|(id, state)| {
            let status_str = serde_json::to_value(&state.status)
                .ok()
                .and_then(|v| v.as_str().map(std::string::ToString::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            format!("{id}: {status_str}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let result = CodexCollabResult {
        status,
        agents_states,
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

async fn publish_codex_collab_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    item_id: &str,
    collab: &CollabToolCallItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{item_id}");
    let assistant_uuid = format!("{session_id}_{item_id}_call");
    let tool_input = build_collab_tool_use(collab);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_COLLAB,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_collab_tool_result(collab);
    let result_uuid = format!("{session_id}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

/// Publish a `web_search` item as a single tool_use block — no result is
/// emitted because upstream has no status or payload on completion.
async fn publish_codex_web_search_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    item_id: &str,
    web: &WebSearchItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{item_id}");
    let assistant_uuid = format!("{session_id}_{item_id}_call");
    let input = CodexWebSearchInput {
        query: web.query.clone(),
        action: web.action.clone(),
    };
    let tool_input = serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}));
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_WEB_SEARCH,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;
}

/// Publish a non-fatal codex `error` item as an assistant text message,
/// flagged via `sdk_metadata.codex_error` so the UI can render it distinctly.
/// Note: this intentionally does NOT touch `agent_state.set_error()` — the
/// turn is still running; only the item itself reports a non-fatal error.
async fn publish_codex_error_item(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    item_id: &str,
    text: &str,
    requested_model: Option<&str>,
) {
    let uuid = format!("{session_id}_{item_id}");
    let payload = codex_error_message_payload(&uuid, session_id, text, requested_model);
    events
        .publish_agent_event(task_id, AgentSubType::Message, payload)
        .await;
}

/// Assistant text payload for a non-fatal codex error item.
fn codex_error_message_payload(
    uuid: &str,
    session_id: &str,
    text: &str,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{ "type": "text", "text": text }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
        "sdk_metadata": { "codex_error": true },
    })
}

fn codex_result_payload(
    session_id: Option<&str>,
    is_error: bool,
    subtype: &str,
    result: String,
    usage: Option<&Usage>,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    if let Some(session_id) = session_id.filter(|session_id| !session_id.is_empty()) {
        payload.insert("session_id".to_string(), serde_json::json!(session_id));
    }
    payload.insert("is_error".to_string(), serde_json::json!(is_error));
    payload.insert("subtype".to_string(), serde_json::json!(subtype));
    payload.insert("result".to_string(), serde_json::json!(result.clone()));
    // Mirror the Claude `extract_result_info` shape: when `is_error`, expose the
    // text under `error` so `handle_agent_error` can read it without knowing
    // SDK-specific layout (it falls back to "Unknown agent error" otherwise).
    if is_error {
        payload.insert("error".to_string(), serde_json::json!(result));
    }
    payload.insert("total_cost_usd".to_string(), serde_json::json!(0.0));
    payload.insert("num_turns".to_string(), serde_json::json!(1));
    if let Some(model) = requested_model {
        payload.insert("model".to_string(), serde_json::json!(model));
    }
    if let Some(usage) = usage {
        payload.insert(
            "usage".to_string(),
            serde_json::json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cached_input_tokens,
                "cache_creation_input_tokens": serde_json::Value::Null,
            }),
        );
    }

    serde_json::Value::Object(payload)
}

fn append_turn_output(turn_output: &mut String, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if !turn_output.is_empty() {
        turn_output.push_str("\n\n");
    }
    turn_output.push_str(text);
}

fn drain_pending_acks(pending_acks: &mut VecDeque<oneshot::Sender<String>>) {
    while let Some(ack) = pending_acks.pop_front() {
        drop(ack);
    }
}

/// Construct a `UserMessage` for injection into a running Claude session.
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
///
/// When `is_error: true` comes back in a `ResultMessage::Success` (e.g. the CLI
/// rejects the message with `"Unknown command: ..."`), we route the event through
/// the `AgentSubType::Error` path so the backend treats it as an error and the
/// frontend renders the rejection text as a red error banner. We also inject
/// `error` into the payload so `handle_agent_error` can read it without having
/// to know the SDK-specific shape.
fn extract_result_info(
    result: &ResultMessage,
    task_id: &str,
) -> (bool, String, AgentSubType, serde_json::Value) {
    match result {
        ResultMessage::Success(success) => {
            if success.is_error {
                warn!(
                    task_id = task_id,
                    session_id = %success.session_id,
                    num_turns = success.num_turns,
                    duration_ms = success.duration_ms,
                    cost_usd = success.total_cost_usd,
                    output = %truncate(&success.result, 500),
                    "Agent completed with error output"
                );
                let mut payload = serde_json::to_value(success).unwrap_or_default();
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(
                        "error".to_string(),
                        serde_json::Value::String(success.result.clone()),
                    );
                }
                return (
                    true,
                    success.session_id.clone(),
                    AgentSubType::Error,
                    payload,
                );
            }
            info!(
                task_id = task_id,
                session_id = %success.session_id,
                num_turns = success.num_turns,
                duration_ms = success.duration_ms,
                cost_usd = success.total_cost_usd,
                output = %truncate(&success.result, 500),
                "Agent completed successfully"
            );
            (
                false,
                success.session_id.clone(),
                AgentSubType::Completed,
                serde_json::to_value(success).unwrap_or_default(),
            )
        }
        ResultMessage::Error(error) => {
            warn!(
                task_id = task_id,
                session_id = %error.session_id,
                num_turns = error.num_turns,
                duration_ms = error.duration_ms,
                subtype = %error.subtype,
                errors = ?error.errors,
                "Agent completed with error"
            );
            (
                true,
                error.session_id.clone(),
                AgentSubType::Error,
                serde_json::to_value(error).unwrap_or_default(),
            )
        }
    }
}

/// Extract a human-readable error text from a result message.
fn extract_error_text(result: &ResultMessage) -> String {
    match result {
        ResultMessage::Success(success) => truncate(&success.result, 500).to_string(),
        ResultMessage::Error(error) => error
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
    use claude_agent_sdk::types::content::Usage as ClaudeUsage;
    use claude_agent_sdk::types::messages::{ResultError, ResultSuccess};

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
            usage: ClaudeUsage {
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
            usage: ClaudeUsage {
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
    fn is_retryable_result_server_error_pattern() {
        let result = make_result_error(
            "unknown",
            2,
            vec!["HTTP 503 service unavailable".to_string()],
        );
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_not_retryable_after_many_turns() {
        let result = make_result_error("overloaded", 6, vec!["overloaded_error".to_string()]);
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_success_with_error_text() {
        let result = make_result_success(true, 1, "Service overloaded (529)");
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_not_retryable_success_without_error() {
        let result = make_result_success(false, 1, "All done");
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn normalize_unknown_agent_type_defaults_to_claude() {
        assert_eq!(
            normalize_agent_type(Some("something-else")),
            AGENT_TYPE_CLAUDE_CODE
        );
    }

    #[test]
    fn codex_tool_use_payload_shapes_assistant_block() {
        let input = serde_json::json!({ "command": "echo hi" });
        let payload = codex_tool_use_payload(
            "sess_item_1_call",
            "sess",
            "sess_item_1",
            "codex.shell",
            input,
            Some("gpt-5"),
        );

        assert_eq!(payload["uuid"], "sess_item_1_call");
        assert_eq!(payload["session_id"], "sess");
        assert_eq!(payload["message"]["role"], "assistant");
        assert_eq!(payload["message"]["model"], "gpt-5");

        let block = &payload["message"]["content"][0];
        assert_eq!(block["type"], "tool_use");
        assert_eq!(block["id"], "sess_item_1");
        assert_eq!(block["name"], "codex.shell");
        assert_eq!(block["input"]["command"], "echo hi");
    }

    #[test]
    fn codex_tool_result_payload_carries_typed_result() {
        let typed = serde_json::json!({ "status": "completed", "exitCode": 0 });
        let payload = codex_tool_result_payload(
            "sess_item_1_result",
            "sess",
            "sess_item_1",
            "hi\n",
            false,
            typed,
        );

        assert_eq!(payload["uuid"], "sess_item_1_result");
        assert_eq!(payload["message"]["role"], "user");
        let block = &payload["message"]["content"][0];
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["tool_use_id"], "sess_item_1");
        assert_eq!(block["content"], "hi\n");
        assert_eq!(block["is_error"], false);
        assert_eq!(payload["tool_use_result"]["exitCode"], 0);
    }

    #[test]
    fn command_execution_status_nonzero_exit_is_failed() {
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Completed, Some(1)),
            CodexToolStatus::Failed
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Completed, Some(0)),
            CodexToolStatus::Completed
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Failed, Some(1)),
            CodexToolStatus::Failed
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::InProgress, None),
            CodexToolStatus::InProgress
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Declined, None),
            CodexToolStatus::Declined
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Unknown, None),
            CodexToolStatus::Completed
        );
    }

    #[test]
    fn build_codex_todo_list_input_preserves_items() {
        use crate::agent::codex_exec::TodoListEntry;
        let todo = TodoListItem {
            items: vec![
                TodoListEntry {
                    text: "first".to_string(),
                    completed: true,
                },
                TodoListEntry {
                    text: "second".to_string(),
                    completed: false,
                },
            ],
        };
        let input = build_codex_todo_list_input(&todo);
        assert_eq!(input.items.len(), 2);
        assert_eq!(input.items[0].text, "first");
        assert!(input.items[0].completed);
        assert_eq!(input.items[1].text, "second");
        assert!(!input.items[1].completed);
    }

    #[test]
    fn codex_todo_list_payload_uses_stable_uuid_per_item_id() {
        // Simulate two emissions for the same codex item_id: first a started
        // snapshot with all false, then an updated snapshot with one true.
        // Both should produce the same uuid so the backend upserts in place.
        let initial = serde_json::to_value(build_codex_todo_list_input(&TodoListItem {
            items: vec![
                codex_exec::TodoListEntry {
                    text: "a".to_string(),
                    completed: false,
                },
                codex_exec::TodoListEntry {
                    text: "b".to_string(),
                    completed: false,
                },
            ],
        }))
        .unwrap();
        let updated = serde_json::to_value(build_codex_todo_list_input(&TodoListItem {
            items: vec![
                codex_exec::TodoListEntry {
                    text: "a".to_string(),
                    completed: true,
                },
                codex_exec::TodoListEntry {
                    text: "b".to_string(),
                    completed: false,
                },
            ],
        }))
        .unwrap();

        let p1 = codex_tool_use_payload(
            &format!("{}_{}_call", "sess", "item_1"),
            "sess",
            &format!("{}_{}", "sess", "item_1"),
            CODEX_TOOL_TODO_LIST,
            initial,
            Some("gpt-5"),
        );
        let p2 = codex_tool_use_payload(
            &format!("{}_{}_call", "sess", "item_1"),
            "sess",
            &format!("{}_{}", "sess", "item_1"),
            CODEX_TOOL_TODO_LIST,
            updated,
            Some("gpt-5"),
        );

        assert_eq!(p1["uuid"], p2["uuid"]);
        assert_eq!(p1["uuid"], "sess_item_1_call");
        assert_eq!(p1["message"]["content"][0]["name"], CODEX_TOOL_TODO_LIST);
        // Payloads differ only in the tool_input items array.
        assert_eq!(p1["message"]["content"][0]["input"]["items"][0]["completed"], false);
        assert_eq!(p2["message"]["content"][0]["input"]["items"][0]["completed"], true);
    }

    #[test]
    fn build_shell_tool_result_marks_error_on_nonzero_exit() {
        let cmd = CommandExecutionItem {
            command: "false".to_string(),
            cwd: None,
            aggregated_output: Some(String::new()),
            exit_code: Some(1),
            status: CommandExecutionStatus::Failed,
            duration_ms: None,
        };
        let (value, _, is_error) = build_shell_tool_result(&cmd);
        assert!(is_error);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["exitCode"], 1);
    }

    #[test]
    fn build_mcp_tool_result_marks_error_when_error_present() {
        let item = McpToolCallItem {
            server: "db".to_string(),
            tool: "query".to_string(),
            arguments: serde_json::json!({}),
            result: None,
            error: Some(codex_exec::McpToolCallItemError {
                message: "connection refused".to_string(),
            }),
            status: McpToolCallStatus::Failed,
        };
        let (value, summary, is_error) = build_mcp_tool_result(&item);
        assert!(is_error);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["errorMessage"], "connection refused");
        assert_eq!(summary, "connection refused");
    }

    #[test]
    fn build_mcp_tool_result_ok_when_status_completed() {
        let item = McpToolCallItem {
            server: "atlassian".to_string(),
            tool: "jira_search".to_string(),
            arguments: serde_json::json!({}),
            result: Some(codex_exec::McpToolCallItemResult {
                content: vec![serde_json::json!({"type":"text","text":"hit"})],
                structured_content: None,
            }),
            error: None,
            status: McpToolCallStatus::Completed,
        };
        let (value, summary, is_error) = build_mcp_tool_result(&item);
        assert!(!is_error);
        assert_eq!(value["status"], "completed");
        assert_eq!(summary, "hit");
    }

    #[test]
    fn build_collab_tool_result_marks_error_on_errored_agent() {
        let mut agents = HashMap::new();
        agents.insert(
            "sub_a".to_string(),
            codex_exec::CollabAgentState {
                status: CodexExecCollabAgentStatus::Errored,
                message: Some("crashed".to_string()),
            },
        );
        let item = CollabToolCallItem {
            tool: CodexExecCollabTool::SpawnAgent,
            sender_thread_id: "main".to_string(),
            receiver_thread_ids: vec!["sub_a".to_string()],
            prompt: None,
            agents_states: agents,
            status: CollabToolCallStatus::Completed,
        };
        let (_, _, is_error) = build_collab_tool_result(&item);
        assert!(is_error);
    }

    #[test]
    fn codex_error_message_payload_sets_metadata_flag() {
        let payload = codex_error_message_payload(
            "sess_item_8",
            "sess",
            "tool returned warning",
            Some("gpt-5"),
        );
        assert_eq!(payload["sdk_metadata"]["codex_error"], true);
        assert_eq!(payload["message"]["content"][0]["text"], "tool returned warning");
        assert_eq!(payload["message"]["role"], "assistant");
    }

    #[test]
    fn codex_reasoning_payload_sets_metadata_flag() {
        let payload = codex_reasoning_payload(
            "sess_item_0",
            "sess",
            "thinking...",
            Some("gpt-5"),
        );
        assert_eq!(payload["sdk_metadata"]["codex_reasoning"], true);
        assert_eq!(payload["message"]["content"][0]["type"], "text");
        assert_eq!(payload["message"]["content"][0]["text"], "thinking...");
    }

    #[test]
    fn codex_result_payload_includes_usage_shape() {
        let payload = codex_result_payload(
            Some("thread-1"),
            false,
            "completed",
            "done".to_string(),
            Some(&Usage {
                input_tokens: 12,
                cached_input_tokens: 3,
                output_tokens: 4,
            }),
            Some("gpt-5"),
        );

        assert_eq!(payload["session_id"], "thread-1");
        assert_eq!(payload["is_error"], false);
        assert!(payload.get("error").is_none());
        assert_eq!(payload["usage"]["input_tokens"], 12);
        assert_eq!(payload["usage"]["cache_read_input_tokens"], 3);
        assert_eq!(payload["model"], "gpt-5");
    }

    #[test]
    fn codex_result_payload_injects_error_key_when_failed() {
        let payload = codex_result_payload(
            Some("thread-2"),
            true,
            "failed",
            "refresh_token_reused".to_string(),
            None,
            Some("gpt-5"),
        );

        assert_eq!(payload["is_error"], true);
        assert_eq!(payload["result"], "refresh_token_reused");
        // `error` mirrors `result` so backend `handle_agent_error` (which reads
        // payload["error"]) renders the failure text instead of "Unknown agent error".
        assert_eq!(payload["error"], "refresh_token_reused");
    }

    // ── extract_wakeup_job ──────────────────────────────────────────

    #[test]
    fn extract_wakeup_job_returns_some_for_valid_tool_call() {
        let content = vec![ContentBlock::ToolUse {
            id: "toolu_123".to_string(),
            name: "ScheduleWakeup".to_string(),
            input: serde_json::json!({
                "delaySeconds": 120,
                "prompt": "check build status",
                "reason": "waiting for build to complete"
            }),
        }];
        let job = extract_wakeup_job(&content).expect("expected wakeup job");
        assert_eq!(job.id(), "toolu_123");
        assert_eq!(job.reason(), "waiting for build to complete");
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_other_tool() {
        let content = vec![ContentBlock::ToolUse {
            id: "toolu_456".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({ "command": "ls -la" }),
        }];
        assert!(extract_wakeup_job(&content).is_none());
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_text_block() {
        let content = vec![ContentBlock::Text {
            text: "Some text".to_string(),
        }];
        assert!(extract_wakeup_job(&content).is_none());
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_missing_fields() {
        let content = vec![ContentBlock::ToolUse {
            id: "toolu_789".to_string(),
            name: "ScheduleWakeup".to_string(),
            input: serde_json::json!({
                "delaySeconds": 60
                // missing prompt and reason
            }),
        }];
        assert!(extract_wakeup_job(&content).is_none());
    }

    #[test]
    fn extract_wakeup_job_finds_in_mixed_content() {
        let content = vec![
            ContentBlock::Text {
                text: "I'll schedule a wakeup".to_string(),
            },
            ContentBlock::ToolUse {
                id: "toolu_first".to_string(),
                name: "Bash".to_string(),
                input: serde_json::json!({ "command": "echo hi" }),
            },
            ContentBlock::ToolUse {
                id: "toolu_wakeup".to_string(),
                name: "ScheduleWakeup".to_string(),
                input: serde_json::json!({
                    "delaySeconds": 300,
                    "prompt": "resume work",
                    "reason": "cache cooldown"
                }),
            },
        ];
        let job = extract_wakeup_job(&content).expect("expected wakeup job");
        assert_eq!(job.id(), "toolu_wakeup");
        assert_eq!(job.reason(), "cache cooldown");
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_empty_content() {
        let content: Vec<ContentBlock> = vec![];
        assert!(extract_wakeup_job(&content).is_none());
    }

    // ── extract_result_info ─────────────────────────────────────────

    #[test]
    fn extract_result_info_success_is_error_true_routes_to_error_subtype() {
        let result = make_result_success(true, 1, "Unknown command: /foo");
        let (is_error, session_id, sub_type, payload) = extract_result_info(&result, "task-1");
        assert!(is_error);
        assert_eq!(session_id, "test-session");
        assert_eq!(sub_type, AgentSubType::Error);
        assert_eq!(payload["error"], "Unknown command: /foo");
        assert_eq!(payload["result"], "Unknown command: /foo");
        assert_eq!(payload["is_error"], true);
    }

    #[test]
    fn extract_result_info_success_is_error_false_unchanged() {
        let result = make_result_success(false, 1, "All done");
        let (is_error, _, sub_type, payload) = extract_result_info(&result, "task-1");
        assert!(!is_error);
        assert_eq!(sub_type, AgentSubType::Completed);
        assert!(payload.get("error").is_none());
        assert_eq!(payload["result"], "All done");
    }

    #[test]
    fn extract_result_info_explicit_error_variant_unchanged() {
        let result = make_result_error("rate_limit", 1, vec!["429".to_string()]);
        let (is_error, _, sub_type, payload) = extract_result_info(&result, "task-1");
        assert!(is_error);
        assert_eq!(sub_type, AgentSubType::Error);
        assert_eq!(payload["subtype"], "rate_limit");
    }
}
