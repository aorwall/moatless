//! Actor-style agent handle — owns the CLI session and exposes `send()`.
//!
//! The outside world (dispatcher, gRPC handlers) calls `agent.send(text)`.
//! Internally, the actor manages the runtime lifecycle:
//! - Claude sessions stay open and accept injected user messages.
//! - Codex runs one turn per `codex exec` invocation and resumes on the next turn.
//!
//! The actor emits `AgentEvent`s through the shared event publisher.
//!
//! Module layout:
//! - [`actor`] — the `tokio::select!` event loop
//! - [`session`] — starting/fingerprinting CLI sessions
//! - [`retry`] — transient-error classification and backoff
//! - [`credential_recovery`] — re-driving a prompt after a credential refresh
//! - [`claude`] / [`codex`] — per-runtime message processing
//! - [`codex_payloads`] — pure Codex item → event-JSON builders

mod actor;
mod claude;
mod codex;
mod codex_payloads;
mod credential_recovery;
mod retry;
mod session;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use claude_agent_sdk::QueryHandle as ClaudeQueryHandle;
use tokio::sync::{mpsc, oneshot, RwLock};

use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::Dispatcher;
use crate::events::GrpcEventPublisher;

use super::agent_state::AgentState;
use super::codex_exec::CodexCliSession;
use super::scheduled_job::ScheduledJobRegistry;

const AGENT_TYPE_CLAUDE_CODE: &str = "claude-code";
/// Claude Code over the tmux (interactive TUI) transport. Same executor family
/// as claude-code; the tmux transport is selected backend-side via the
/// `SANDBOX_USE_TMUX` env var.
const AGENT_TYPE_CLAUDE_CODE_TUI: &str = "claude-code-tui";
/// Claude Code over the subprocess/NDJSON transport (the explicit non-tmux CLI
/// variant). Treated as the claude-code family for everything except the
/// transport selection in `start_session`.
const AGENT_TYPE_CLAUDE_CODE_CLI: &str = "claude-code-cli";
/// Claude Code pinned to the `fable` model. Same executor family as claude-code;
/// the model is forced backend-side when the message carries none.
const AGENT_TYPE_CLAUDE_CODE_FABLE: &str = "claude-code-fable";
const AGENT_TYPE_CODEX: &str = "codex";

enum ActiveHandle {
    Claude(ClaudeQueryHandle),
    Codex(CodexCliSession),
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
                let user_msg = claude::make_user_message(text, session_id);
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
        agent_type: Option<String>,
        agent_mode: Option<String>,
        /// Session ID from the backend for resuming after pod restart.
        resume_session_id: Option<String>,
        ack: oneshot::Sender<String>,
    },
    /// Replay an interrupted run on startup. Behaves identically to `Send`
    /// except the resume session ID is forced from the backend's recovery
    /// payload — the existing CLI `--resume <session_id>` path drives the
    /// rest. Acks are dropped (no caller waiting on a session_id).
    Recover {
        text: String,
        agent_type: Option<String>,
        agent_mode: Option<String>,
        resume_session_id: String,
    },
    /// Stop the agent and shut down the actor loop.
    Stop,
}

/// Queued turn waiting for a runtime to become idle.
struct QueuedSend {
    text: String,
    model: Option<String>,
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
        agent_type: Option<String>,
        agent_mode: Option<String>,
        resume_session_id: Option<String>,
    ) -> oneshot::Receiver<String> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Send {
            text,
            model,
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

    /// Replay an interrupted agent run on sandbox startup.
    ///
    /// `resume_session_id` is forced through to the CLI as `--resume <id>`,
    /// so the run picks up the on-disk Claude session JSONL state from the
    /// PVC. No ack — recovery is best-effort and the caller does not wait
    /// for a session id.
    pub fn recover(
        &self,
        text: String,
        agent_type: Option<String>,
        agent_mode: Option<String>,
        resume_session_id: String,
    ) {
        let _ = self.command_tx.send(AgentCommand::Recover {
            text,
            agent_type,
            agent_mode,
            resume_session_id,
        });
    }
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
    credential_failures: CredentialFailureDetector,
    credential_dispatcher: Arc<Dispatcher>,
    system_prompt_cache: Arc<RwLock<Option<String>>>,
) -> AgentHandle {
    let (command_tx, command_rx) = mpsc::unbounded_channel();

    tokio::spawn(actor::agent_loop(
        command_rx,
        config,
        agent_state,
        events,
        credentials,
        backend_client,
        scheduled_jobs,
        credential_failures,
        credential_dispatcher,
        system_prompt_cache,
    ));

    AgentHandle { command_tx }
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

/// Truncate a string to at most `max` characters (on a char boundary).
fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}
