//! Agent command handle — exposes `send()`, `stop()`, and startup recovery.
//!
//! The outside world (dispatcher, gRPC handlers) calls `agent.send(text)`.
//! Internally, [`CoordinatorService`](super::coordinator::CoordinatorService)
//! owns the runtime lifecycle through the harness interface.
//!
//! Module layout:
//! - [`retry`] — transient-error classification and backoff
//! - [`credential_recovery`] — re-driving a prompt after a credential refresh
//! - [`codex_payloads`] — pure Codex item → event-JSON builders

pub(crate) mod codex_payloads;
pub(crate) mod credential_recovery;
pub(crate) mod retry;

use std::collections::HashMap;
use std::sync::Arc;

use moatless_events::AgentSubType;
use tokio::sync::{RwLock, mpsc, oneshot};

use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::Dispatcher;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::events::GrpcEventPublisher;

use super::agent_state::AgentState;
use super::harness::{
    ClaudeConfig, CodexConfig, ErrorClass, FailureStage, Harness, HarnessError, SessionImpact,
};
use super::scheduled_job::ScheduledJobRegistry;
use super::types::StopSource;

pub(crate) const AGENT_TYPE_CLAUDE_CODE: &str = "claude-code";
/// Claude Code over the tmux (interactive TUI) transport. Same executor family
/// as claude-code; the tmux transport is selected backend-side via the
/// `SANDBOX_USE_TMUX` env var.
const AGENT_TYPE_CLAUDE_CODE_TUI: &str = "claude-code-tui";
/// Claude Code over the subprocess/NDJSON transport (the explicit non-tmux CLI
/// variant). Treated as the claude-code family for everything except the
/// transport selection inside the Claude harness.
const AGENT_TYPE_CLAUDE_CODE_CLI: &str = "claude-code-cli";
/// Claude Code pinned to the `fable` model. Same executor family as claude-code;
/// the model is forced backend-side when the message carries none.
const AGENT_TYPE_CLAUDE_CODE_FABLE: &str = "claude-code-fable";
const AGENT_TYPE_CODEX: &str = "codex";

/// Resolve a raw agent-type string into the canonical harness identifier,
/// rejecting values this Sandbox has no harness for.
///
/// Used to validate the Task's fixed selection at Sandbox construction (the fixed-harness decision):
/// an unsupported harness is an invalid-configuration failure surfaced at
/// startup rather than a silent fall-through to Claude. The Claude transport,
/// model, and mode variants all resolve to the one Claude harness — they are
/// configuration of that harness, not separate harnesses.
pub(crate) fn resolve_agent_harness(raw: &str) -> Result<&'static str, String> {
    match raw {
        AGENT_TYPE_CODEX => Ok(AGENT_TYPE_CODEX),
        AGENT_TYPE_CLAUDE_CODE
        | AGENT_TYPE_CLAUDE_CODE_TUI
        | AGENT_TYPE_CLAUDE_CODE_CLI
        | AGENT_TYPE_CLAUDE_CODE_FABLE => Ok(AGENT_TYPE_CLAUDE_CODE),
        other => Err(format!(
            "unsupported Agent harness '{other}' (expected one of: \
             {AGENT_TYPE_CLAUDE_CODE}, {AGENT_TYPE_CLAUDE_CODE_TUI}, \
             {AGENT_TYPE_CLAUDE_CODE_CLI}, {AGENT_TYPE_CLAUDE_CODE_FABLE}, {AGENT_TYPE_CODEX})"
        )),
    }
}

/// Construct the single Agent harness fixed for this Sandbox (the fixed-harness decision),
/// selected by the resolved agent type: Codex, or Claude over the persistent
/// (tmux) or one-shot (subprocess) transport per `config.use_tmux`. An
/// unsupported type is an invalid-configuration failure surfaced at startup,
/// never a silent default.
///
/// All per-Turn configuration — including the coordinator-resolved `env` and
/// `system_prompt` — rides on the submitted `Input`, so the harness itself
/// stays fixed for the Sandbox lifetime.
pub fn build_harness(
    config: &Arc<Config>,
    agent_state: &Arc<AgentState>,
    credential_failures: &CredentialFailureDetector,
    agent_type: &str,
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
            credential_failures: credential_failures.clone(),
        }));
    }

    let claude = ClaudeConfig {
        config: Arc::clone(config),
        agent_state: Arc::clone(agent_state),
        agent_type: Some(agent_type.to_string()),
    };
    Ok(if config.use_tmux {
        Harness::claude_persistent(claude)
    } else {
        Harness::claude(claude)
    })
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

/// Commands sent to the agent lifecycle owner.
pub(crate) enum AgentCommand {
    /// Send a message. The ack sender is resolved with the session and run IDs
    /// once the agent confirms it processed the message.
    Send {
        text: String,
        model: Option<String>,
        agent_mode: Option<String>,
        /// Reasoning effort for the turn, in the agent's own vocabulary.
        effort: Option<String>,
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
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: String,
    },
    /// Stop the agent. The `ack` (when present) resolves once the lifecycle
    /// owner has published the single terminal `Stopped` event and closed the
    /// Turn, so the caller learns the stop was processed rather than queued.
    /// `source` is echoed on the terminal event so the backend can attribute
    /// the stop (user vs. idle reaper vs. cleanup).
    Stop {
        source: StopSource,
        ack: Option<oneshot::Sender<()>>,
    },
}

/// Handle to the agent lifecycle owner — the only interface the outside world needs.
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
    #[allow(clippy::too_many_arguments)] // reason: one parameter per per-Turn field the command carries
    pub fn send(
        &self,
        text: String,
        model: Option<String>,
        _agent_type: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: Option<String>,
        start_fresh: bool,
    ) -> oneshot::Receiver<RunAck> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Send {
            text,
            model,
            agent_mode,
            effort,
            resume_session_id,
            start_fresh,
            ack: ack_tx,
        });
        ack_rx
    }

    /// Stop the agent. Returns a receiver that resolves once the lifecycle
    /// owner has published the single terminal `Stopped` event and closed the
    /// Turn — so a caller (e.g. the gRPC `stop_agent` handler) can wait for the
    /// stop to be *processed*, not merely queued.
    pub fn stop(&self, source: StopSource) -> oneshot::Receiver<()> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.command_tx.send(AgentCommand::Stop {
            source,
            ack: Some(ack_tx),
        });
        ack_rx
    }

    /// Whether the lifecycle owner has exited (its command receiver was dropped).
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
        _agent_type: Option<String>,
        agent_mode: Option<String>,
        effort: Option<String>,
        resume_session_id: String,
    ) {
        let _ = self.command_tx.send(AgentCommand::Recover {
            text,
            agent_mode,
            effort,
            resume_session_id,
        });
    }
}

#[cfg(test)]
impl AgentHandle {
    /// Build a handle backed by a fresh command channel, returning the receiver
    /// so a test can observe exactly which `AgentCommand`s the init/dispatch path
    /// enqueues. This channel is the single FIFO seam the lifecycle owner drains
    /// into the harness, so command order is the order the harness accepts input.
    pub(crate) fn test_channel() -> (Self, mpsc::UnboundedReceiver<AgentCommand>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        (Self { command_tx }, command_rx)
    }
}

/// Spawn the coordinator-backed agent lifecycle owner and return a handle.
#[allow(clippy::too_many_arguments)] // reason: lifecycle wiring; every argument is a distinct dependency
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
    let (command_tx, command_rx) = mpsc::unbounded_channel::<AgentCommand>();
    let agent_type = config.agent_type.as_deref().unwrap_or("");

    match build_harness(&config, &agent_state, &credential_failures, agent_type) {
        Ok(harness) => {
            let coordinator = super::coordinator::Coordinator::new(
                harness,
                events.clone(),
                config.task_id.clone(),
            );
            let service = super::coordinator::CoordinatorService::new(
                coordinator,
                Arc::clone(&agent_state),
                Arc::clone(&config),
                Arc::clone(&credentials),
                Arc::clone(&backend_client),
                Arc::clone(&system_prompt_cache),
                Arc::clone(&scheduled_jobs),
                // The same detector instance the harness populates, so the
                // service sees the credential flags Codex raises mid-Turn.
                credential_failures.clone(),
                Arc::clone(&credential_dispatcher),
            );
            tokio::spawn(service.run(command_rx, heartbeat));
        }
        Err(harness_err) => {
            // Invalid configuration: spawn a poison task that publishes one
            // classified error event and then exits, dropping `command_rx` so
            // `is_closed()` becomes true and `/livez` restarts the pod.
            let task_id = config.task_id.clone();
            let diagnostic = harness_err.diagnostic.clone();
            tokio::spawn(async move {
                events
                    .publish_agent_event(
                        &task_id,
                        AgentSubType::Error,
                        serde_json::json!({
                            "error": diagnostic,
                            "reason": "invalid_configuration",
                        }),
                    )
                    .await;
                drop(command_rx);
            });
        }
    }

    AgentHandle { command_tx }
}

/// Truncate a string to at most `max` characters (on a char boundary).
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        AGENT_TYPE_CLAUDE_CODE, AGENT_TYPE_CODEX, build_harness, resolve_agent_harness, spawn_agent,
    };
    use crate::agent::agent_state::AgentState;
    use crate::agent::harness::ErrorClass;
    use crate::agent::heartbeat::Heartbeat;
    use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
    use crate::backend_client::BackendClient;
    use crate::config::{Config, test_support::minimal_config};
    use crate::credentials::Dispatcher;
    use crate::credentials::recovery::CredentialFailureDetector;
    use crate::events::GrpcEventPublisher;
    use crate::outbox::EventOutbox;
    use async_trait::async_trait;
    use sandbox_proto::{
        DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery,
        PublishEventRequest,
    };
    use std::time::Duration;
    use tokio::sync::RwLock;
    use tokio_util::sync::CancellationToken;
    use tonic::Status;

    /// Minimal recording backend for spawn_agent tests.
    struct NopBackend;

    #[async_trait]
    impl BackendClient for NopBackend {
        async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), Status> {
            Ok(())
        }
        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<PendingAgentDelivery>, Status> {
            Ok(vec![])
        }
        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _uuids: Vec<String>,
            _session_id: &str,
            _run_id: &str,
        ) -> Result<(), Status> {
            Ok(())
        }
        async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn update_credential(
            &self,
            _task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<DownloadUploadResponse, Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn recover_interrupted_run(
            &self,
            _task_id: &str,
        ) -> Result<Option<InterruptedTurn>, Status> {
            Ok(None)
        }
        async fn upload_transcript(&self, _task_id: &str, _tar: Vec<u8>) -> Result<(), Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, Status> {
            Ok(vec![])
        }
    }

    fn make_publisher(cancel: CancellationToken) -> GrpcEventPublisher {
        let backend = Arc::new(NopBackend);
        let (outbox, _drainer) = EventOutbox::spawn(backend, "test-task".to_string(), cancel);
        GrpcEventPublisher::new(outbox)
    }

    /// `spawn_agent` with an unsupported `agent_type` must return an
    /// `AgentHandle` whose channel closes promptly (the poison task drops
    /// `command_rx` so `is_closed()` becomes true and `/livez` restarts the pod).
    #[tokio::test]
    async fn spawn_agent_with_invalid_agent_type_closes_channel_promptly() {
        let cancel = CancellationToken::new();
        let events = make_publisher(cancel.clone());

        let mut config = minimal_config();
        config.agent_type = Some("gpt-9000".to_string()); // not a supported harness

        let handle = spawn_agent(
            Arc::new(config),
            Arc::new(AgentState::new()),
            events,
            Arc::new(RwLock::new(std::collections::HashMap::new())),
            Arc::new(NopBackend),
            Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
            CredentialFailureDetector::default(),
            Arc::new(Dispatcher::new()),
            Arc::new(RwLock::new(None)),
            Heartbeat::new(),
        );

        // Give the poison task a moment to run and drop command_rx.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if handle.is_closed() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "handle channel did not close within 2 s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        cancel.cancel();
    }

    /// A missing `AGENT_TYPE` is a classified invalid configuration that closes
    /// the handle for liveness.
    #[tokio::test]
    async fn spawn_agent_without_agent_type_closes_channel_promptly() {
        let cancel = CancellationToken::new();
        let events = make_publisher(cancel.clone());

        let mut config = minimal_config();
        config.agent_type = None;

        let handle = spawn_agent(
            Arc::new(config),
            Arc::new(AgentState::new()),
            events,
            Arc::new(RwLock::new(std::collections::HashMap::new())),
            Arc::new(NopBackend),
            Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
            CredentialFailureDetector::default(),
            Arc::new(Dispatcher::new()),
            Arc::new(RwLock::new(None)),
            Heartbeat::new(),
        );

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if handle.is_closed() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "handle channel did not close within 2 s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        cancel.cancel();
    }

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
        let err = match build_harness(&config, &agent_state, &failures, "gpt-9000") {
            Ok(_) => panic!("unsupported harness must fail"),
            Err(err) => err,
        };
        assert_eq!(err.class, ErrorClass::InvalidConfiguration);
        assert!(err.diagnostic.contains("gpt-9000"));
    }

    #[tokio::test]
    async fn build_harness_constructs_each_supported_selection() {
        // Every supported (agent_type, transport) composes a harness rather than
        // panicking or erroring; the routing decision itself — which family each
        // raw type collapses to — is asserted by the `resolve_agent_harness` tests
        // below (the single source of truth `build_harness` matches on). The
        // behavioural proof that the composed adapter is the intended family is an
        // E2E concern (it requires actually running each CLI): fixed selection
        // surviving a restart is covered for both families by
        // `sandbox_replacement_ordering` (gated `SANDBOX_REPLACEMENT_E2E`) — not
        // by inspecting a private tag here.
        for (agent_type, use_tmux) in [
            ("codex", false),
            ("claude-code", false),
            ("claude-code", true),
            ("claude-code-cli", false),
        ] {
            let (config, agent_state, failures) = harness_inputs(use_tmux);
            // `Harness` has no `Debug`, so assert on the `is_ok()` bool.
            assert!(
                build_harness(&config, &agent_state, &failures, agent_type).is_ok(),
                "{agent_type} (tmux={use_tmux}) should construct"
            );
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
}
