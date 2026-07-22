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
use super::harness::{
    ClaudeConfig, CodexConfig, ErrorClass, FailureStage, Harness, HarnessError, SessionImpact,
};
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

/// Resolve a raw agent-type string into the canonical harness identifier the
/// actor branches on, rejecting values this Sandbox has no harness for.
///
/// Used to validate the Task's fixed selection at Sandbox construction (ADR-0004):
/// an unsupported harness is an invalid-configuration failure surfaced at
/// startup rather than a silent fall-through to Claude. The Claude transport,
/// model, and mode variants all resolve to the one Claude harness — they are
/// configuration of that harness, not separate harnesses.
pub(crate) fn resolve_agent_harness(raw: &str) -> Result<&'static str, String> {
    match raw {
        AGENT_TYPE_CODEX => Ok(AGENT_TYPE_CODEX),
        AGENT_TYPE_CLAUDE_CODE | AGENT_TYPE_CLAUDE_CODE_TUI | AGENT_TYPE_CLAUDE_CODE_CLI
        | AGENT_TYPE_CLAUDE_CODE_FABLE => Ok(AGENT_TYPE_CLAUDE_CODE),
        other => Err(format!(
            "unsupported Agent harness '{other}' (expected one of: \
             {AGENT_TYPE_CLAUDE_CODE}, {AGENT_TYPE_CLAUDE_CODE_TUI}, \
             {AGENT_TYPE_CLAUDE_CODE_CLI}, {AGENT_TYPE_CLAUDE_CODE_FABLE}, {AGENT_TYPE_CODEX})"
        )),
    }
}

/// Construct the single Agent harness fixed for this Sandbox (ADR-0004),
/// selected by the resolved agent type: Codex, or Claude over the persistent
/// (tmux) or one-shot (subprocess) transport per `config.use_tmux`. An
/// unsupported type is an invalid-configuration failure surfaced at startup,
/// never a silent default.
///
/// Turn-independent `env` and `system_prompt` are resolved by the caller and
/// cloned into each execution; all per-Turn configuration rides on the
/// submitted `Input`, so the harness itself stays fixed.
pub fn build_harness(
    config: &Arc<Config>,
    agent_state: &Arc<AgentState>,
    credential_failures: &CredentialFailureDetector,
    agent_type: &str,
    env: Option<HashMap<String, String>>,
    system_prompt: Option<String>,
) -> Result<Harness, HarnessError> {
    let harness = resolve_agent_harness(agent_type).map_err(|diagnostic| {
        HarnessError::new(
            FailureStage::Startup,
            ErrorClass::InvalidConfiguration,
            SessionImpact::Intact,
            diagnostic,
        )
    })?;

    if harness == AGENT_TYPE_CODEX {
        return Ok(Harness::codex(CodexConfig {
            config: Arc::clone(config),
            env,
            system_prompt,
            credential_failures: credential_failures.clone(),
        }));
    }

    let claude = ClaudeConfig {
        config: Arc::clone(config),
        agent_state: Arc::clone(agent_state),
        env,
        system_prompt,
        agent_type: Some(agent_type.to_string()),
    };
    Ok(if config.use_tmux {
        Harness::claude_persistent(claude)
    } else {
        Harness::claude(claude)
    })
}

/// Fix the harness selection on first resolution and keep it for the actor's
/// lifetime. `current` is the selection already resolved — from the Sandbox's
/// construction-time config, or (legacy migration) the first Message that
/// carried an agent type. A later Message's agent type is tolerated as input but
/// can never switch the harness, so it is adopted only when nothing is fixed yet.
fn adopt_agent_type(current: &Option<String>, incoming: Option<String>) -> Option<String> {
    match current {
        Some(_) => current.clone(),
        None => incoming,
    }
}

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

/// What the dispatcher needs to acknowledge a batch of messages: the CLI
/// session that consumed them and the agent run they belong to.
///
/// The run id is what lets the backend put messages injected mid-run on the
/// same turn as the run they joined, independently of whether the ack or the
/// run's `taskTurn.active` event reaches the backend first.
#[derive(Debug, Clone)]
pub struct RunAck {
    pub session_id: String,
    pub run_id: String,
}

/// Commands sent to the agent actor.
enum AgentCommand {
    /// Send a message. The ack sender is resolved with the session and run IDs
    /// once the agent confirms it processed the message.
    Send {
        text: String,
        model: Option<String>,
        agent_type: Option<String>,
        agent_mode: Option<String>,
        /// Session ID from the backend for resuming after pod restart.
        resume_session_id: Option<String>,
        /// The user asked for a fresh agent session: never inject into a
        /// running one, and start the new run without `--resume`.
        start_fresh: bool,
        ack: oneshot::Sender<RunAck>,
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
    start_fresh: bool,
    ack: oneshot::Sender<RunAck>,
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
    /// `start_fresh` forces a brand-new agent session: the message is never
    /// injected into a running one, and the new run starts without `--resume`.
    ///
    /// Returns a receiver that resolves with the session and run IDs once the
    /// agent has processed the message:
    /// - Claude injection: resolves immediately after a successful stdin write,
    ///   carrying the *running* run's id so the message joins its turn.
    /// - New Claude/Codex turn: resolves on `SessionInit` with the real session ID.
    pub fn send(
        &self,
        text: String,
        model: Option<String>,
        agent_type: Option<String>,
        agent_mode: Option<String>,
        resume_session_id: Option<String>,
        start_fresh: bool,
    ) -> oneshot::Receiver<RunAck> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Send {
            text,
            model,
            agent_type,
            agent_mode,
            resume_session_id,
            start_fresh,
            ack: ack_tx,
        });
        ack_rx
    }

    /// Stop the agent and shut down the actor.
    pub fn stop(&self) {
        let _ = self.command_tx.send(AgentCommand::Stop);
    }

    /// Whether the actor loop has exited (its command receiver was dropped).
    ///
    /// Used by the `/livez` liveness probe: a closed channel means no further
    /// agent turn can ever run, so the pod should be restarted.
    pub fn is_closed(&self) -> bool {
        self.command_tx.is_closed()
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
    heartbeat: super::heartbeat::Heartbeat,
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
        heartbeat,
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

fn drain_pending_acks(pending_acks: &mut VecDeque<oneshot::Sender<RunAck>>) {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        adopt_agent_type, build_harness, resolve_agent_harness, AGENT_TYPE_CLAUDE_CODE,
        AGENT_TYPE_CODEX,
    };
    use crate::agent::agent_state::AgentState;
    use crate::agent::harness::ErrorClass;
    use crate::config::{test_support::minimal_config, Config};
    use crate::credentials::recovery::CredentialFailureDetector;

    fn harness_inputs(use_tmux: bool) -> (Arc<Config>, Arc<AgentState>, CredentialFailureDetector) {
        let mut config = minimal_config();
        config.use_tmux = use_tmux;
        (
            Arc::new(config),
            Arc::new(AgentState::new()),
            CredentialFailureDetector::default(),
        )
    }

    #[tokio::test]
    async fn build_harness_rejects_unsupported_type_as_invalid_configuration() {
        let (config, agent_state, failures) = harness_inputs(false);
        // `Harness` has no `Debug`, so match rather than `expect_err`.
        let err = match build_harness(&config, &agent_state, &failures, "gpt-9000", None, None) {
            Ok(_) => panic!("unsupported harness must fail"),
            Err(err) => err,
        };
        assert_eq!(err.class, ErrorClass::InvalidConfiguration);
        assert!(err.diagnostic.contains("gpt-9000"));
    }

    #[tokio::test]
    async fn build_harness_constructs_each_supported_selection() {
        // Each supported selection constructs its fixed harness without error;
        // the variant is private (no public trait), so this proves construction,
        // not the branch taken — `resolve_agent_harness` covers the mapping.
        for (agent_type, use_tmux) in [
            ("codex", false),
            ("claude-code", false),
            ("claude-code", true),
            ("claude-code-cli", false),
        ] {
            let (config, agent_state, failures) = harness_inputs(use_tmux);
            build_harness(&config, &agent_state, &failures, agent_type, None, None)
                .unwrap_or_else(|_| panic!("{agent_type} (tmux={use_tmux}) should construct"));
        }
    }

    #[test]
    fn resolve_agent_harness_collapses_claude_variants() {
        for raw in [
            "claude-code",
            "claude-code-tui",
            "claude-code-cli",
            "claude-code-fable",
        ] {
            assert_eq!(resolve_agent_harness(raw).unwrap(), AGENT_TYPE_CLAUDE_CODE);
        }
    }

    #[test]
    fn resolve_agent_harness_accepts_codex() {
        assert_eq!(resolve_agent_harness("codex").unwrap(), AGENT_TYPE_CODEX);
    }

    #[test]
    fn resolve_agent_harness_rejects_unknown_instead_of_defaulting() {
        // An unsupported value is a classified configuration failure, never a
        // silent fall-through to Claude.
        let err = resolve_agent_harness("gpt-9000").unwrap_err();
        assert!(err.contains("gpt-9000"));
        assert!(err.contains("unsupported Agent harness"));
    }

    #[test]
    fn adopt_agent_type_freezes_the_first_resolved_selection() {
        // A construction-time (or first legacy) selection is authoritative and a
        // later Message cannot switch it.
        let fixed = Some("codex".to_string());
        assert_eq!(
            adopt_agent_type(&fixed, Some("claude-code".to_string())),
            fixed
        );
        // No fixed selection yet (legacy pod): adopt the first Message's type.
        assert_eq!(
            adopt_agent_type(&None, Some("codex".to_string())),
            Some("codex".to_string())
        );
        // Still nothing to adopt when the Message carries none.
        assert_eq!(adopt_agent_type(&None, None), None);
    }
}
