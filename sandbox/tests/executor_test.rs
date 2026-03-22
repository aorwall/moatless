//! Mock-based integration tests for the agent orchestrator loop.
//!
//! Tests `run_agent_inner` and `dispatch_pending_messages` with a mock
//! `BackendClient` and `AgentExecutor` — no real gRPC or Claude CLI needed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moatless_sandbox::agent::agent_state::AgentState;
use moatless_sandbox::agent::executor::{AgentEvent, AgentExecutor};
use moatless_sandbox::agent::types::AgentStatus;
use moatless_sandbox::backend_client::BackendClient;
use moatless_sandbox::config::Config;
use moatless_sandbox::error::SandboxError;
use moatless_sandbox::events::GrpcEventPublisher;
use moatless_sandbox::init::InitPhase;
use moatless_sandbox::state::SandboxState;

use sandbox_proto::{GetCredentialsResponse, PendingMessage, PublishEventRequest};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

// =============================================================================
// Mock BackendClient
// =============================================================================

/// A mock backend client that returns configurable pending messages.
struct MockBackendClient {
    /// Queue of message batches to return from `get_pending_messages`.
    /// Each call pops the front batch; empty vec when exhausted.
    message_batches: Mutex<Vec<Vec<PendingMessage>>>,
    /// Track how many times publish_event was called.
    publish_count: AtomicUsize,
    /// Track how many times acknowledge was called.
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

    fn publish_count(&self) -> usize {
        self.publish_count.load(Ordering::SeqCst)
    }

    #[allow(dead_code)]
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
        })
    }
}

// =============================================================================
// Mock AgentExecutor
// =============================================================================

/// A mock executor that sends predetermined events through the channel.
struct MockExecutor {
    /// If true, the executor will return an error.
    should_fail: bool,
}

#[async_trait::async_trait]
impl AgentExecutor for MockExecutor {
    async fn execute(
        &self,
        _state: &SandboxState,
        _request: moatless_sandbox::agent::types::AgentRunRequest,
        _cancel_token: CancellationToken,
        events_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<(), SandboxError> {
        // Send a SessionInit event (simulates Claude CLI starting)
        let _ = events_tx.send(AgentEvent::SessionInit {
            session_id: "mock-session-1".to_string(),
            model: "mock-model".to_string(),
        });

        if self.should_fail {
            return Err(SandboxError::Sdk(claude_agent_sdk::SdkError::Cancelled));
        }

        // Send a Result event (simulates successful completion)
        let _ = events_tx.send(AgentEvent::Result {
            is_error: false,
            sub_type: moatless_events::AgentSubType::Completed,
            payload: serde_json::json!({"result": "done"}),
        });

        Ok(())
    }
}

// =============================================================================
// Helper: build a test SandboxState
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
        uuid: uuid.map(ToString::to_string),
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
        log_level: "info".to_string(),
        backend_grpc_url: "http://localhost:50051".to_string(),
        backend_api_url: None,
        user_auth_token: None,
        agent_config: moatless_sandbox::config::AgentConfig::default(),
        claude_cli_path: "/usr/bin/claude".to_string(),
        claude_config_dir: "/tmp/test-claude-config".to_string(),
        git_clone_url: None,
        git_branch: None,
        git_provider: "github".to_string(),
        repo_config_raw: None,
        setup_commands: vec![],
        plugin_ids: vec![],
        plugin_repos: vec![],
    }
}

fn test_state(
    backend: Arc<dyn BackendClient>,
    executor: Arc<dyn AgentExecutor>,
) -> SandboxState {
    let events = GrpcEventPublisher::new(Arc::clone(&backend));
    SandboxState {
        config: Arc::new(test_config()),
        events,
        agent: Arc::new(AgentState::new()),
        executor,
        backend_client: backend,
        init_phase: Arc::new(RwLock::new(InitPhase::Ready)),
        credentials: Arc::new(RwLock::new(HashMap::new())),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[tokio::test]
async fn run_agent_processes_messages_and_transitions_to_idle() {
    let backend = Arc::new(MockBackendClient::new(vec![
        vec![make_text_message("hello", Some("uuid-1"))],
    ]));
    let executor = Arc::new(MockExecutor { should_fail: false });
    let state = test_state(Arc::clone(&backend) as _, Arc::clone(&executor) as _);

    // Trigger run_agent
    moatless_sandbox::agent::executor::run_agent(state.clone());

    // Wait for it to complete
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Agent should be idle after processing
    assert_eq!(
        state.agent.status().await,
        AgentStatus::Idle,
        "Agent should be idle after successful execution"
    );

    // Events should have been published (at minimum: started, session_init, completed, active)
    assert!(
        backend.publish_count() >= 3,
        "Expected at least 3 published events, got {}",
        backend.publish_count()
    );
}

#[tokio::test]
async fn run_agent_circuit_breaker_after_3_errors() {
    // Each call to run_agent processes one batch then exits.
    // Consecutive errors accumulate across calls. The 4th call
    // should see 3 consecutive errors and trigger the circuit breaker.
    let backend = Arc::new(MockBackendClient::new(vec![
        vec![make_text_message("msg1", None)],
        vec![make_text_message("msg2", None)],
        vec![make_text_message("msg3", None)],
        // 4th batch: should be skipped by circuit breaker
        vec![make_text_message("msg4", None)],
    ]));
    let executor = Arc::new(MockExecutor { should_fail: true });
    let state = test_state(Arc::clone(&backend) as _, Arc::clone(&executor) as _);

    // Run agent 3 times — each run processes one batch, fails, exits
    for i in 1..=3 {
        moatless_sandbox::agent::executor::run_agent(state.clone());
        tokio::time::sleep(Duration::from_millis(200)).await;
        let errors = state.agent.consecutive_errors().await;
        assert_eq!(
            errors, i,
            "Expected {i} consecutive errors after run {i}, got {errors}"
        );
    }

    // 4th run should trigger circuit breaker and not execute
    let initial_publish_count = backend.publish_count();
    moatless_sandbox::agent::executor::run_agent(state.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Circuit breaker publishes a sandbox.error event
    assert!(
        backend.publish_count() > initial_publish_count,
        "Circuit breaker should have published sandbox.error event"
    );

    // Errors count should still be 3 (no new execution happened)
    let errors = state.agent.consecutive_errors().await;
    assert_eq!(errors, 3, "Should still be 3 consecutive errors");
}

#[tokio::test]
async fn run_agent_skips_when_already_running() {
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let executor = Arc::new(MockExecutor { should_fail: false });
    let state = test_state(Arc::clone(&backend) as _, Arc::clone(&executor) as _);

    // Set agent to Running manually
    let token = CancellationToken::new();
    state.agent.set_running(token).await;

    // Try to run agent — should just flag messages pending and return
    moatless_sandbox::agent::executor::run_agent(state.clone());

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Agent should still be running (not overridden)
    assert_eq!(state.agent.status().await, AgentStatus::Running);

    // Messages pending flag should be set
    assert!(
        state.agent.take_messages_pending().await,
        "Messages pending should be flagged when agent is already running"
    );
}

#[tokio::test]
async fn run_agent_no_messages_exits_immediately() {
    // Empty message batch — no messages to process
    let backend = Arc::new(MockBackendClient::new(vec![]));
    let executor = Arc::new(MockExecutor { should_fail: false });
    let state = test_state(Arc::clone(&backend) as _, Arc::clone(&executor) as _);

    moatless_sandbox::agent::executor::run_agent(state.clone());

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Agent should remain idle — no work to do
    assert_eq!(state.agent.status().await, AgentStatus::Idle);

    // No events should have been published
    assert_eq!(backend.publish_count(), 0);
}
