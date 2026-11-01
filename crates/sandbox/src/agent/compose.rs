use crate::agent::handle::AgentHandle;
use std::collections::HashMap;
use std::sync::Arc;

use moatless_types::agent::{AgentProfile, AgentSubType};
use tokio::sync::RwLock;

use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::Dispatcher;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::events::GrpcEventPublisher;
use crate::trajectory_tail::TrajectoryRecording;

use crate::agent::agent_state::AgentState;
use crate::agent::harness::{
    ClaudeConfig, CodexConfig, ErrorClass, FailureStage, HarnessError, HarnessRuntime,
    SessionImpact,
};
use crate::agent::scheduled_job::ScheduledJobRegistry;

/// Resolve an Agent profile id into the harness that runs it,
/// rejecting values this Sandbox has no harness for.
///
/// An unsupported harness fails at startup rather than silently falling through
/// to Claude; the Claude transport, model, and mode variants all resolve to the
/// one Claude harness.
pub(crate) fn resolve_agent_harness(raw: &str) -> Result<moatless_types::agent::Harness, String> {
    raw.parse::<AgentProfile>()
        .map(AgentProfile::harness)
        .map_err(|error| format!("unsupported Agent harness: {error}"))
}

/// Constructs the single Agent harness fixed for one Member (the fixed-harness
/// decision).
///
/// Selects Codex or Claude (over the tmux or subprocess transport per
/// `config.use_tmux`); per-Turn configuration rides on the submitted `Input`
/// instead, so the harness itself stays fixed for the Member's lifetime.
pub fn build_harness(
    config: &Arc<Config>,
    agent_state: &Arc<AgentState>,
    credential_failures: &CredentialFailureDetector,
    agent_profile: &str,
    events: Option<GrpcEventPublisher>,
    trajectory: Option<Arc<TrajectoryRecording>>,
) -> Result<HarnessRuntime, HarnessError> {
    let harness = resolve_agent_harness(agent_profile).map_err(|diagnostic| {
        HarnessError::new(
            FailureStage::Startup,
            ErrorClass::InvalidConfiguration,
            SessionImpact::Intact,
            diagnostic,
        )
    })?;

    if harness == moatless_types::agent::Harness::Codex {
        return Ok(HarnessRuntime::codex(CodexConfig {
            config: Arc::clone(config),
            credential_failures: credential_failures.clone(),
            trajectory,
        }));
    }

    let claude = ClaudeConfig {
        config: Arc::clone(config),
        agent_state: Arc::clone(agent_state),
        agent_profile: Some(agent_profile.to_string()),
        events,
        trajectory,
    };
    Ok(if config.use_tmux {
        HarnessRuntime::claude_persistent(claude)
    } else {
        HarnessRuntime::claude(claude)
    })
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
    commands: Arc<crate::commands::CommandRegistry>,
    credential_failures: CredentialFailureDetector,
    credential_dispatcher: Arc<Dispatcher>,
    system_prompt_cache: Arc<RwLock<Option<String>>>,
    heartbeat: crate::agent::heartbeat::Heartbeat,
    workspace_sync: crate::workspace_sync::WorkspaceSyncTracker,
    setup_gate: crate::init::SetupGate,
) -> AgentHandle {
    let (handle, command_rx) = AgentHandle::channel();
    let agent_profile = config.agent_profile.as_deref().unwrap_or("");
    let trajectory = TrajectoryRecording::new(Arc::clone(&backend_client));

    match build_harness(
        &config,
        &agent_state,
        &credential_failures,
        agent_profile,
        Some(events.clone()),
        Some(Arc::clone(&trajectory)),
    ) {
        Ok(harness) => {
            let coordinator = crate::agent::coordinator::Coordinator::new(
                harness,
                events.clone(),
                config.task_id.clone(),
            );
            let service = crate::agent::coordinator::CoordinatorService::new(
                coordinator,
                Arc::clone(&agent_state),
                Arc::clone(&config),
                Arc::clone(&credentials),
                Arc::clone(&backend_client),
                Arc::clone(&system_prompt_cache),
                Arc::clone(&scheduled_jobs),
                Arc::clone(&commands),
                // The same detector instance the harness populates, so the
                // service sees the credential flags Codex raises mid-Turn.
                credential_failures.clone(),
                Arc::clone(&credential_dispatcher),
                workspace_sync,
            )
            .with_harness_factory({
                let (agent_state, events) = (Arc::clone(&agent_state), events.clone());
                Box::new(move |config, agent_profile| {
                    build_harness(
                        config,
                        &agent_state,
                        &credential_failures,
                        agent_profile,
                        Some(events.clone()),
                        Some(Arc::clone(&trajectory)),
                    )
                })
            });
            tokio::spawn(service.run_after(setup_gate, command_rx, heartbeat));
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

    handle
}

/// Truncate a string to at most `max` characters (on a char boundary).
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{build_harness, resolve_agent_harness, spawn_agent};
    use crate::agent::agent_state::AgentState;
    use crate::agent::harness::ErrorClass;
    use crate::agent::heartbeat::Heartbeat;
    use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
    use crate::backend_client::BackendClient;
    use crate::backend_client::DownloadedUpload;
    use crate::config::{Config, test_support::minimal_config};
    use crate::credentials::Dispatcher;
    use crate::credentials::recovery::CredentialFailureDetector;
    use crate::events::GrpcEventPublisher;
    use crate::outbox::EventOutbox;
    use async_trait::async_trait;
    use sandbox_proto::GetCredentialsResponse;
    use std::time::Duration;
    use tokio::sync::RwLock;
    use tokio_util::sync::CancellationToken;
    use tonic::Status;

    /// Minimal recording backend for `spawn_agent` tests.
    struct NopBackend;

    #[async_trait]
    impl BackendClient for NopBackend {
        async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<DownloadedUpload, Status> {
            Err(Status::unimplemented("nop"))
        }
        async fn upload_transcript(&self, _task_id: &str, _tar: Vec<u8>) -> Result<(), Status> {
            Err(Status::unimplemented("nop"))
        }
    }

    fn make_publisher(cancel: CancellationToken) -> GrpcEventPublisher {
        let backend = Arc::new(NopBackend);
        let (outbox, _drainer) = EventOutbox::spawn(backend, "test-task".to_string(), cancel);
        GrpcEventPublisher::new(outbox)
    }

    /// `spawn_agent` with an unsupported `agent_profile` must return an
    /// `AgentHandle` whose channel closes promptly (the poison task drops
    /// `command_rx` so `is_closed()` becomes true and `/livez` restarts the
    /// pod).
    #[tokio::test]
    async fn spawn_agent_with_invalid_agent_profile_closes_channel_promptly() {
        let cancel = CancellationToken::new();
        let events = make_publisher(cancel.clone());

        let mut config = minimal_config();
        config.agent_profile = Some("gpt-9000".to_string()); // not a supported harness

        let handle = spawn_agent(
            Arc::new(config),
            Arc::new(AgentState::new()),
            events,
            Arc::new(RwLock::new(std::collections::HashMap::new())),
            Arc::new(NopBackend),
            Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
            Arc::new(crate::commands::CommandRegistry::for_test()),
            CredentialFailureDetector::default(),
            Arc::new(Dispatcher::new()),
            Arc::new(RwLock::new(None)),
            Heartbeat::new(),
            crate::workspace_sync::WorkspaceSyncTracker::new(),
            crate::init::SetupGate::opened(),
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

    /// A missing `AGENT_PROFILE` is a classified invalid configuration that closes
    /// the handle for liveness.
    #[tokio::test]
    async fn spawn_agent_without_agent_profile_closes_channel_promptly() {
        let cancel = CancellationToken::new();
        let events = make_publisher(cancel.clone());

        let mut config = minimal_config();
        config.agent_profile = None;

        let handle = spawn_agent(
            Arc::new(config),
            Arc::new(AgentState::new()),
            events,
            Arc::new(RwLock::new(std::collections::HashMap::new())),
            Arc::new(NopBackend),
            Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
            Arc::new(crate::commands::CommandRegistry::for_test()),
            CredentialFailureDetector::default(),
            Arc::new(Dispatcher::new()),
            Arc::new(RwLock::new(None)),
            Heartbeat::new(),
            crate::workspace_sync::WorkspaceSyncTracker::new(),
            crate::init::SetupGate::opened(),
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
        // `HarnessRuntime` has no `Debug`, so match rather than `expect_err`.
        let err = match build_harness(&config, &agent_state, &failures, "gpt-9000", None, None) {
            Ok(_) => panic!("unsupported harness must fail"),
            Err(err) => err,
        };
        assert_eq!(err.class, ErrorClass::InvalidConfiguration);
        assert!(err.diagnostic.contains("gpt-9000"));
    }

    #[tokio::test]
    async fn build_harness_constructs_each_supported_selection() {
        // Asserts construction only; `resolve_agent_harness` tests cover the
        // family routing. Proving the composed adapter is the right family
        // needs a running CLI, so that's `sandbox_replacement_ordering`
        // (gated `SANDBOX_REPLACEMENT_E2E`), not a check on a private tag.
        for (agent_profile, use_tmux) in [
            ("codex", false),
            ("claude-code", false),
            ("claude-code", true),
            ("claude-code-cli", false),
        ] {
            let (config, agent_state, failures) = harness_inputs(use_tmux);
            // `HarnessRuntime` has no `Debug`, so assert on the `is_ok()` bool.
            assert!(
                build_harness(&config, &agent_state, &failures, agent_profile, None, None).is_ok(),
                "{agent_profile} (tmux={use_tmux}) should construct"
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
            assert_eq!(
                resolve_agent_harness(raw).unwrap(),
                moatless_types::agent::Harness::ClaudeCode
            );
        }
    }

    #[test]
    fn resolve_agent_harness_accepts_codex() {
        assert_eq!(
            resolve_agent_harness("codex").unwrap(),
            moatless_types::agent::Harness::Codex
        );
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
