//! Integration tests for the agent dispatcher and handle.
//!
//! Tests `dispatch_pending_messages` with a mock `BackendClient`.
//! The agent handle spawns the actor loop, but since we don't have a real
//! Claude CLI, we test the dispatcher routing, acknowledgment logic,
//! and multi-send actor channel mechanics.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moatless_sandbox::agent::agent_state::AgentState;
use moatless_sandbox::agent::handle::spawn_agent;
use moatless_sandbox::agent::types::AgentStatus;
use moatless_sandbox::backend_client::BackendClient;
use moatless_sandbox::config::Config;
use moatless_sandbox::credentials::recovery::CredentialFailureDetector;
use moatless_sandbox::credentials::Dispatcher;
use moatless_sandbox::events::GrpcEventPublisher;
use moatless_sandbox::init::InitPhase;
use moatless_sandbox::state::SandboxState;

use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, PendingMessage, PublishEventRequest,
};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

// =============================================================================
// Mock BackendClient
// =============================================================================

struct MockBackendClient {
    message_batches: Mutex<Vec<Vec<PendingMessage>>>,
    publish_count: AtomicUsize,
    ack_count: AtomicUsize,
}

impl MockBackendClient {
    fn new(batches: Vec<Vec<PendingMessage>>) -> Self {
        Self {
            message_batches: Mutex::new(batches),
            publish_count: AtomicUsize::new(0),
            ack_count: AtomicUsize::new(0),
        }
    }

    #[allow(dead_code)] // reason: used by some tests
    fn publish_count(&self) -> usize {
        self.publish_count.load(Ordering::SeqCst)
    }

    fn ack_count(&self) -> usize {
        self.ack_count.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn publish_event(&self, _req: PublishEventRequest) {
        self.publish_count.fetch_add(1, Ordering::SeqCst);
    }

    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingMessage>, tonic::Status> {
        let mut batches = self.message_batches.lock().unwrap();
        if batches.is_empty() {
            Ok(vec![])
        } else {
            Ok(batches.remove(0))
        }
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
    ) -> Result<(), tonic::Status> {
        self.ack_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn get_credentials(
        &self,
        _task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        Ok(GetCredentialsResponse {
            credentials: HashMap::new(),
            server_env: HashMap::new(),
        })
    }

    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
        Ok(String::new())
    }

    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadUploadResponse, tonic::Status> {
        Err(tonic::Status::unimplemented(
            "download_upload not used in executor tests",
        ))
    }

    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<sandbox_proto::InterruptedTurn>, tonic::Status> {
        Ok(None)
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn make_text_message(text: &str, uuid: Option<&str>) -> PendingMessage {
    PendingMessage {
        content: vec![sandbox_proto::ContentBlock {
            block: Some(sandbox_proto::content_block::Block::Text(
                sandbox_proto::TextBlock {
                    text: text.to_string(),
                },
            )),
        }],
        is_error: None,
        model: None,
        system_prompt: None,
        resume: None,
        resume_session_id: None,
        agent_type: None,
        agent_mode: None,
        uuid: uuid.map(ToString::to_string),
        file_ids: vec![],
        start_fresh: None,
    }
}

fn test_config() -> Config {
    Config {
        port: 0,
        grpc_port: 0,
        task_id: "test-task-1".to_string(),
        workspace_dir: "/tmp/test-workspace".to_string(),
        data_dir: "/tmp/test-data".to_string(),
        upload_dir: "/tmp/test-upload".to_string(),
        home_dir: "/tmp/test-home".to_string(),
        codex_home_dir: "/tmp/test-codex-home".to_string(),
        log_level: "info".to_string(),
        backend_grpc_url: "http://localhost:50051".to_string(),
        backend_api_url: None,
        user_auth_token: None,
        agent_config: moatless_sandbox::config::AgentConfig::default(),
        claude_cli_path: "/usr/bin/claude".to_string(),
        codex_cli_path: "/usr/bin/codex".to_string(),
        proxy: moatless_sandbox::config::ProxyConfig::default(),
        servers: vec![],
        claude_config_dir: "/tmp/test-claude-config".to_string(),
        git_clone_url: None,
        git_branch: None,
        git_provider: "github".to_string(),
        repo_config_raw: None,
        setup_commands: vec![],
        plugin_ids: vec![],
        plugin_repos: vec![],
        frontend_url: None,
        default_model: None,
        default_codex_model: None,
    }
}

fn test_state(backend: Arc<dyn BackendClient>) -> SandboxState {
    use moatless_sandbox::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};

    let events = GrpcEventPublisher::new(Arc::clone(&backend));
    let config = Arc::new(test_config());
    let agent_state = Arc::new(AgentState::new());
    let credentials = Arc::new(RwLock::new(HashMap::new()));

    let late_injector = Arc::new(LateInjector::new());
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));

    let credential_failures = CredentialFailureDetector::default();
    let credential_dispatcher = Arc::new(Dispatcher::new());
    let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

    let agent_handle = spawn_agent(
        Arc::clone(&config),
        Arc::clone(&agent_state),
        events.clone(),
        Arc::clone(&credentials),
        Arc::clone(&backend),
        Arc::clone(&scheduled_jobs),
        credential_failures.clone(),
        Arc::clone(&credential_dispatcher),
        Arc::clone(&system_prompt_cache),
    );
    late_injector.set(Box::new(agent_handle.clone()));

    SandboxState {
        config,
        events,
        agent: agent_state,
        agent_handle,
        scheduled_jobs,
        backend_client: backend,
        init_phase: Arc::new(RwLock::new(InitPhase::Ready)),
        credentials,
        lifecycle_cancel: CancellationToken::new(),
        credential_dispatcher,
        credential_failures,
        codex_watcher_handle: Arc::new(RwLock::new(None)),
        system_prompt_cache,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[tokio::test]
async fn dispatch_with_no_messages_does_nothing() {
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let state = test_state(Arc::clone(&backend) as _);

    moatless_sandbox::agent::dispatcher::dispatch_pending_messages(state.clone()).await;

    // No messages → no ack, no events
    assert_eq!(backend.ack_count(), 0);
}

#[tokio::test]
async fn dispatch_does_not_ack_when_agent_fails_to_start() {
    // Without a real CLI binary, the agent actor will fail to start a session.
    // The ack oneshot is dropped on failure, so the dispatcher's spawned task
    // gets RecvError and skips the acknowledge call.
    let backend = Arc::new(MockBackendClient::new(vec![vec![make_text_message(
        "hello",
        Some("uuid-1"),
    )]]));
    let state = test_state(Arc::clone(&backend) as _);

    moatless_sandbox::agent::dispatcher::dispatch_pending_messages(state.clone()).await;

    // Give the actor time to attempt session start and fail
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(
        backend.ack_count(),
        0,
        "Should not acknowledge when agent fails to start"
    );
}

#[tokio::test]
async fn agent_handle_send_triggers_actor() {
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let state = test_state(Arc::clone(&backend) as _);

    // Send directly to agent handle — actor will try to start a session.
    // It will fail (no real CLI) but the send itself should not panic.
    // Drop the ack receiver since we don't need it.
    let _ack_rx = state
        .agent_handle
        .send("test message".to_string(), None, None, None, None);

    // Give the actor loop time to process
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Agent should have attempted to start and hit an error (no CLI binary).
    // The handle should not have panicked.
}

#[tokio::test]
async fn agent_handle_stop_exits_cleanly() {
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let state = test_state(Arc::clone(&backend) as _);

    state.agent_handle.stop();

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        state.agent.status().await,
        AgentStatus::Stopped,
        "Agent should be stopped after handle.stop()"
    );
}

// =============================================================================
// Multi-send actor channel tests
// =============================================================================

#[tokio::test]
async fn agent_handle_rapid_fire_send_no_panic() {
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let state = test_state(Arc::clone(&backend) as _);

    // Send 10 messages rapidly — actor processes them without panic or deadlock.
    // Drop ack receivers since we only care about channel stability.
    for i in 0..10 {
        let _ack_rx = state
            .agent_handle
            .send(format!("rapid message {i}"), None, None, None, None);
    }

    // Give the actor time to attempt processing
    tokio::time::sleep(Duration::from_secs(1)).await;

    state.agent_handle.stop();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        state.agent.status().await,
        AgentStatus::Stopped,
        "Agent should be stopped cleanly after rapid-fire sends"
    );
}

#[tokio::test]
async fn agent_handle_send_after_stop_does_not_panic() {
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let state = test_state(Arc::clone(&backend) as _);

    state.agent_handle.stop();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sending after stop should be silently ignored (command channel closed,
    // ack receiver will get RecvError immediately).
    let ack1 = state
        .agent_handle
        .send("should be ignored".to_string(), None, None, None, None);
    let ack2 = state
        .agent_handle
        .send("also ignored".to_string(), None, None, None, None);

    // Ack receivers should resolve with error (sender dropped)
    assert!(ack1.await.is_err(), "ack should fail after stop");
    assert!(ack2.await.is_err(), "ack should fail after stop");

    assert_eq!(
        state.agent.status().await,
        AgentStatus::Stopped,
        "Agent should remain stopped after sends to closed channel"
    );
}

#[tokio::test]
async fn agent_handle_concurrent_send_from_multiple_tasks() {
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let state = test_state(Arc::clone(&backend) as _);

    // Spawn 5 tasks that all send concurrently
    let mut join_handles = Vec::new();
    for i in 0..5 {
        let handle = state.agent_handle.clone();
        join_handles.push(tokio::spawn(async move {
            let _ack_rx = handle.send(format!("concurrent task {i}"), None, None, None, None);
        }));
    }

    // Wait for all spawned tasks to complete
    for jh in join_handles {
        jh.await.unwrap();
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    state.agent_handle.stop();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        state.agent.status().await,
        AgentStatus::Stopped,
        "Agent should stop cleanly after concurrent sends"
    );
}

#[tokio::test]
async fn dispatch_does_not_ack_without_real_session() {
    // Even with agent state set to Running, the actor has no real QueryHandle.
    // The send goes through but the actor tries to inject into a non-existent
    // session handle (handle is None in the actor), so it starts a new session
    // which fails. The ack oneshot is dropped, and the dispatcher's spawned
    // task gets RecvError — no ack fires.
    let backend = Arc::new(MockBackendClient::new(vec![vec![make_text_message(
        "injected message",
        Some("uuid-inject"),
    )]]));
    let state = test_state(Arc::clone(&backend) as _);

    // Simulate agent state as "running" — but the actor's internal handle is None
    state
        .agent
        .set_running(tokio_util::sync::CancellationToken::new())
        .await;
    state
        .agent
        .set_session_id("session-active".to_string())
        .await;

    moatless_sandbox::agent::dispatcher::dispatch_pending_messages(state.clone()).await;

    // Give the spawned ack task time to resolve
    tokio::time::sleep(Duration::from_millis(500)).await;

    // No real session → ack sender dropped → no acknowledge call
    assert_eq!(
        backend.ack_count(),
        0,
        "Should not acknowledge without a real session (ack sender dropped)"
    );
}
