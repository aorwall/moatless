//! Coordinator seam-mapping contract tests.
//!
//! Each test drives the real [`Coordinator`] over a scripted [`Harness`] and a
//! real [`GrpcEventPublisher`] whose outbox delivers to an in-memory recording
//! backend. Assertions are on the exact `agent.*` events published (their order,
//! sub-types, and per-block payload shape) plus the acknowledgement — the
//! externally observable contract, not the coordinator's internals.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingMessage,
    PublishEventRequest,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tonic::Status;

use crate::agent::harness::error::FailureStage;
use crate::agent::harness::source::{NativeMessageId, SourceId};
use crate::agent::harness::{
    ErrorClass, Harness, HarnessError, Input, Role, Script, SessionId, SessionImpact, SourceEvent,
    Step,
};
use crate::agent::types::ResumeSession;
use crate::backend_client::BackendClient;
use crate::events::GrpcEventPublisher;
use crate::outbox::EventOutbox;

use super::Coordinator;

/// Records every event the outbox delivers, in order. Every other backend call
/// is unused by these tests and reports unimplemented.
struct RecordingBackend {
    received: Arc<Mutex<Vec<PublishEventRequest>>>,
}

#[async_trait]
impl BackendClient for RecordingBackend {
    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), Status> {
        self.received.lock().expect("lock").push(req);
        Ok(())
    }

    async fn get_pending_messages(&self, _task_id: &str) -> Result<Vec<PendingMessage>, Status> {
        Ok(vec![])
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
    ) -> Result<(), Status> {
        Ok(())
    }

    async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
        Err(Status::unimplemented("unused"))
    }

    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), Status> {
        Err(Status::unimplemented("unused"))
    }

    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
        Err(Status::unimplemented("unused"))
    }

    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadUploadResponse, Status> {
        Err(Status::unimplemented("unused"))
    }

    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, Status> {
        Ok(None)
    }

    async fn upload_transcript(&self, _task_id: &str, _tar: Vec<u8>) -> Result<(), Status> {
        Err(Status::unimplemented("unused"))
    }

    async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, Status> {
        Ok(vec![])
    }
}

/// A publisher whose delivered events are captured in the returned buffer. The
/// cancel token keeps the outbox drainer alive for the test's lifetime.
fn recording_publisher() -> (
    GrpcEventPublisher,
    Arc<Mutex<Vec<PublishEventRequest>>>,
    CancellationToken,
) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingBackend {
        received: Arc::clone(&received),
    });
    let cancel = CancellationToken::new();
    let outbox = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());
    (GrpcEventPublisher::new(outbox), received, cancel)
}

/// A source event for one native block under a given Message id.
fn block(message_id: &str, block: serde_json::Value, role: Role) -> SourceEvent {
    SourceEvent {
        source_id: SourceId(format!(
            "{message_id}-{}",
            block["type"].as_str().unwrap_or("b")
        )),
        message_id: NativeMessageId(message_id.to_string()),
        session: SessionId("sess-1".to_string()),
        parent_tool_use_id: None,
        role,
        block,
        usage: None,
        model: None,
        diagnostics: None,
    }
}

fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "text": text })
}

fn failure() -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        ErrorClass::NativeProtocol,
        SessionImpact::Unknown,
        "boom",
    )
}

/// A resume-startup failure whose Session is gone — the shape the Claude/Codex
/// adapters report when a `--resume` target no longer loads.
fn lost_error() -> HarnessError {
    HarnessError::new(
        FailureStage::Startup,
        ErrorClass::UnavailableSession,
        SessionImpact::Lost,
        "resume target not found",
    )
}

/// A retryable transient failure: the process exited mid-run, which the retry
/// policy classifies as worth re-driving.
fn process_exit_error() -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        ErrorClass::ProcessExit,
        SessionImpact::Lost,
        "process exited",
    )
}

/// An input that asks to resume a known Session.
fn resume_input(id: &str, text: &str, session: &str) -> Input {
    let mut input = Input::text(id, text);
    input.resume = Some(ResumeSession::WithId(session.to_string()));
    input
}

/// Sub-types of the delivered events, in delivery order.
fn sub_types(received: &Arc<Mutex<Vec<PublishEventRequest>>>) -> Vec<String> {
    received
        .lock()
        .expect("lock")
        .iter()
        .map(|r| r.sub_type.clone())
        .collect()
}

/// The single delivered event with the given sub-type (panics if not exactly one).
fn only(received: &Arc<Mutex<Vec<PublishEventRequest>>>, sub_type: &str) -> PublishEventRequest {
    let guard = received.lock().expect("lock");
    let mut matches = guard.iter().filter(|r| r.sub_type == sub_type);
    let found = matches.next().expect("event present").clone();
    assert!(matches.next().is_none(), "expected exactly one {sub_type}");
    found
}

async fn flush(events: &GrpcEventPublisher) {
    events.outbox().flush(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn fresh_turn_publishes_ordered_events_and_acks_after_acceptance() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", text_block("hello"), Role::Assistant)),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");

    let (ack_tx, ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    // ExecutionStarted → turn start (started + taskTurn active) + session init;
    // then the block; then the terminal completion — in native order.
    assert_eq!(
        sub_types(&received),
        vec!["started", "active", "session_init", "message", "completed"],
    );

    // The batch is acknowledged only after InputAccepted, against the minted Run.
    let ack = ack_rx.await.expect("ack resolved");
    assert_eq!(ack.session_id, "sess-1");
    let started = only(&received, "started");
    assert_eq!(ack.run_id, started.run_id);
    assert!(!ack.run_id.is_empty());

    // Every event of the Run carries its Run id.
    for req in received.lock().expect("lock").iter() {
        assert_eq!(req.run_id, ack.run_id, "run-scoped: {}", req.sub_type);
    }
}

#[tokio::test]
async fn per_block_message_payload_matches_backend_assembly_contract() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", text_block("one"), Role::Assistant)),
        Step::Block(block(
            "msg-1",
            serde_json::json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": {} }),
            Role::Assistant,
        )),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    let messages: Vec<serde_json::Value> = received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| r.sub_type == "message")
        .map(|r| serde_json::from_str(&r.payload_json).expect("json"))
        .collect();
    assert_eq!(messages.len(), 2);

    // Both blocks share the native Message id as their grouping `uuid`, carry
    // increasing `blockIndex`, and each holds exactly its own single block.
    assert_eq!(messages[0]["uuid"], "msg-1");
    assert_eq!(messages[0]["blockIndex"], 0);
    assert_eq!(messages[0]["session_id"], "sess-1");
    assert_eq!(messages[0]["message"]["role"], "assistant");
    assert_eq!(messages[0]["message"]["content"][0]["text"], "one");
    assert_eq!(
        messages[0]["message"]["content"].as_array().unwrap().len(),
        1
    );

    assert_eq!(messages[1]["uuid"], "msg-1");
    assert_eq!(messages[1]["blockIndex"], 1);
    assert_eq!(messages[1]["message"]["content"][0]["type"], "tool_use");
    assert_eq!(messages[1]["message"]["content"][0]["name"], "Bash");
}

#[tokio::test]
async fn ack_is_withheld_until_input_accepted() {
    let (events, _received, _cancel) = recording_publisher();
    // No `AcceptInput` step, so the harness never reports the input accepted.
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::Block(block("msg-1", text_block("hello"), Role::Assistant)),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let (ack_tx, mut ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;

    // The Run ran to completion, but with no InputAccepted the batch stays
    // unacknowledged — the ack channel is still open and empty.
    assert!(matches!(
        ack_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn failed_execution_publishes_error_and_no_completion() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Failed(failure()),
    ]);
    let mut coord = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    let types = sub_types(&received);
    assert!(types.contains(&"error".to_string()), "got {types:?}");
    assert!(!types.contains(&"completed".to_string()), "got {types:?}");

    let error: serde_json::Value =
        serde_json::from_str(&only(&received, "error").payload_json).expect("json");
    assert_eq!(error["error"], "boom");
    assert_eq!(error["reason"], "native_protocol");
}

#[tokio::test]
async fn rejected_input_opens_no_run_and_drops_the_ack() {
    let (events, received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(
        Harness::scripted(Script::rejecting(failure())),
        events.clone(),
        "task-1",
    );
    let (ack_tx, ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    // No ExecutionStarted preceded the rejection, so no Run was opened: no turn
    // start is published. A classified failure is surfaced.
    let types = sub_types(&received);
    assert!(!types.contains(&"started".to_string()), "got {types:?}");
    assert!(!types.contains(&"active".to_string()), "got {types:?}");
    assert!(types.contains(&"error".to_string()), "got {types:?}");

    // The ack is dropped: the dispatcher observes a closed channel and leaves
    // the batch unacknowledged.
    assert!(matches!(
        ack_rx.await,
        Err(oneshot::error::RecvError { .. })
    ));
}

#[tokio::test]
async fn lost_resume_recovers_with_a_fresh_turn_and_keeps_the_ack() {
    let (events, received, _cancel) = recording_publisher();
    // The resume attempt is rejected as Lost; the fresh recovery re-drive
    // succeeds, so the Message is never dropped.
    let scripts = vec![
        Script::rejecting(lost_error()),
        Script::new(vec![
            Step::SessionReady(SessionId("sess-2".to_string())),
            Step::AcceptInput,
            Step::Block(block("msg-1", text_block("hi again"), Role::Assistant)),
            Step::Completed,
        ]),
    ];
    let mut coord = Coordinator::new(
        Harness::scripted_sequence(scripts),
        events.clone(),
        "task-1",
    );

    let (ack_tx, ack_rx) = oneshot::channel();
    coord
        .submit(
            resume_input("in-1", "do the thing", "stale-session"),
            ack_tx,
        )
        .await
        .expect("submit");
    // First pump consumes the Lost rejection and re-drives recovery; the second
    // runs the recovery execution to completion.
    coord.pump_one_execution().await;
    coord.pump_one_execution().await;
    flush(&events).await;

    // The context-loss notice precedes a full fresh Turn.
    assert_eq!(
        sub_types(&received),
        vec![
            "session_context_lost",
            "started",
            "active",
            "session_init",
            "message",
            "completed"
        ],
    );

    // The notice names the stale Session and the classified diagnostic.
    let notice: serde_json::Value =
        serde_json::from_str(&only(&received, "session_context_lost").payload_json).expect("json");
    assert_eq!(notice["stale_session_id"], "stale-session");
    assert_eq!(notice["reason"], "transcript_unavailable");

    // The pending Message was never dropped: it acks on the recovered Turn's
    // acceptance, against the recovery Run and Session.
    let ack = ack_rx.await.expect("ack resolved on recovery");
    assert_eq!(ack.session_id, "sess-2");
    let started = only(&received, "started");
    assert_eq!(ack.run_id, started.run_id);

    // The notice and the recovered Turn all carry the one Run id.
    for req in received.lock().expect("lock").iter() {
        assert_eq!(req.run_id, ack.run_id, "run-scoped: {}", req.sub_type);
    }
}

#[tokio::test]
async fn lost_rejection_without_a_resume_request_drops_the_ack() {
    let (events, received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(
        Harness::scripted(Script::rejecting(lost_error())),
        events.clone(),
        "task-1",
    );
    let (ack_tx, ack_rx) = oneshot::channel();
    // No resume was requested, so a Lost rejection is a genuine failure — not a
    // recoverable resume miss. It must not trigger context-loss recovery.
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    let types = sub_types(&received);
    assert!(
        !types.contains(&"session_context_lost".to_string()),
        "got {types:?}"
    );
    assert!(types.contains(&"error".to_string()), "got {types:?}");
    assert!(matches!(
        ack_rx.await,
        Err(oneshot::error::RecvError { .. })
    ));
}

#[tokio::test]
async fn a_recovery_that_fails_again_is_not_retried() {
    let (events, received, _cancel) = recording_publisher();
    // The resume fails as Lost; the fresh recovery re-drive also fails. Recovery
    // is bounded to one attempt, so the second failure ends the input.
    let scripts = vec![
        Script::rejecting(lost_error()),
        Script::rejecting(failure()),
    ];
    let mut coord = Coordinator::new(
        Harness::scripted_sequence(scripts),
        events.clone(),
        "task-1",
    );
    let (ack_tx, ack_rx) = oneshot::channel();
    coord
        .submit(
            resume_input("in-1", "do the thing", "stale-session"),
            ack_tx,
        )
        .await
        .expect("submit");
    coord.pump_one_execution().await; // Lost → recover
    coord.pump_one_execution().await; // recovery fails → give up
    flush(&events).await;

    let types = sub_types(&received);
    // Exactly one context-loss notice (no second recovery), plus a terminal error.
    assert_eq!(
        types
            .iter()
            .filter(|t| *t == "session_context_lost")
            .count(),
        1,
        "got {types:?}"
    );
    assert!(types.contains(&"error".to_string()), "got {types:?}");
    // The ack is finally dropped once recovery is exhausted.
    assert!(matches!(
        ack_rx.await,
        Err(oneshot::error::RecvError { .. })
    ));
}

#[test]
fn context_recovery_prompt_wraps_the_original_prompt() {
    let out = super::context_recovery_prompt("task-xyz", "fix the bug");
    assert!(out.contains("moat messages list task-xyz"), "got {out}");
    assert!(out.contains("fix the bug"), "got {out}");
    assert!(out.contains("context-recovery"), "got {out}");
}

// ── build_resume: resolve the restored session for a replayed/next Turn ──────
//
// Session state is restored to disk during init before Ready, and the harness
// is fixed for the Sandbox, so a resumed run must target the restored Session
// id for whichever family the pod runs. These prove that resolution for both
// Claude and Codex.

#[test]
fn build_resume_prefers_last_session_id_for_claude() {
    // The pod's own last-known Session id wins over any backend-provided id.
    let resume = super::build_resume(
        "restored-claude-session",
        Some("backend-resume-id"),
        super::AGENT_TYPE_CLAUDE_CODE,
        false,
    );
    assert_eq!(
        resume,
        Some(ResumeSession::WithId("restored-claude-session".to_string()))
    );
}

#[test]
fn build_resume_prefers_last_session_id_for_codex() {
    let resume = super::build_resume(
        "restored-codex-rollout",
        Some("backend-resume-id"),
        "codex",
        false,
    );
    assert_eq!(
        resume,
        Some(ResumeSession::WithId("restored-codex-rollout".to_string()))
    );
}

#[test]
fn build_resume_uses_backend_id_on_fresh_pod_for_both_families() {
    // Fresh pod with no local Session: the backend-provided resume id (from the
    // restored run) is honored for both families.
    for family in [super::AGENT_TYPE_CLAUDE_CODE, "codex"] {
        let resume = super::build_resume("", Some("backend-resume-id"), family, false);
        assert_eq!(
            resume,
            Some(ResumeSession::WithId("backend-resume-id".to_string())),
            "family {family}"
        );
    }
}

#[test]
fn build_resume_start_fresh_overrides_restored_session_for_both_families() {
    for family in [super::AGENT_TYPE_CLAUDE_CODE, "codex"] {
        let resume =
            super::build_resume("restored-session", Some("backend-resume-id"), family, true);
        assert_eq!(resume, None, "family {family}");
    }
}

#[test]
fn build_resume_falls_back_to_continue_for_claude_only() {
    // No local Session and no backend id: Claude gets `--continue`, Codex (which
    // has no `--continue`) starts fresh.
    assert_eq!(
        super::build_resume("", None, super::AGENT_TYPE_CLAUDE_CODE, false),
        Some(ResumeSession::Last)
    );
    assert_eq!(super::build_resume("", None, "codex", false), None);
}

// ── CoordinatorService integration tests ────────────────────────────────────

use std::collections::HashMap;

use tokio::sync::{mpsc, RwLock};

use crate::agent::agent_state::AgentState;
use crate::agent::handle::AgentCommand;
use crate::agent::heartbeat::Heartbeat;
use crate::agent::scheduled_job::{JobInjector, LateInjector, ScheduledJobRegistry};
use crate::config::{test_support::minimal_config, Config};
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::{CredentialSink, Dispatcher};

use super::CoordinatorService;

/// Build a `CoordinatorService` over a scripted harness with the recording
/// publisher, returning the command sender and the event recording buffer.
fn service_with_script(
    script: Script,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    cancel: CancellationToken,
) -> (
    mpsc::UnboundedSender<AgentCommand>,
    Arc<Mutex<Vec<PublishEventRequest>>>,
    GrpcEventPublisher,
) {
    service_with_harness(Harness::scripted(script), config, agent_state, cancel)
}

/// Like `service_with_script` but takes a pre-built `Harness` so tests can drive
/// a scripted *sequence* (e.g. fail-then-succeed for a retry).
fn service_with_harness(
    harness: Harness,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    cancel: CancellationToken,
) -> (
    mpsc::UnboundedSender<AgentCommand>,
    Arc<Mutex<Vec<PublishEventRequest>>>,
    GrpcEventPublisher,
) {
    let (events, received, _outbox_cancel) = recording_publisher_with_cancel(cancel);
    let coordinator = Coordinator::new(harness, events.clone(), "task-1");
    let service = CoordinatorService::new(
        coordinator,
        Arc::clone(&agent_state),
        Arc::clone(&config),
        Arc::new(RwLock::new(HashMap::new())),
        Arc::new(RecordingBackend {
            received: Arc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(RwLock::new(None)),
        Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        CredentialFailureDetector::default(),
        Arc::new(Dispatcher::new()),
    );
    let (tx, rx) = mpsc::unbounded_channel::<AgentCommand>();
    let heartbeat = Heartbeat::new();
    tokio::spawn(service.run(rx, heartbeat));
    (tx, received, events)
}

/// A job injector that records every injected prompt, for asserting that a
/// fired `ScheduleWakeup` re-enters the service.
struct CapturingInjector {
    injected: Arc<Mutex<Vec<String>>>,
}

impl JobInjector for CapturingInjector {
    fn inject(&self, text: String) {
        self.injected.lock().expect("lock").push(text);
    }
}

/// Like `recording_publisher` but accepts an external cancel token so the
/// outbox drainer lives for the caller's test scope.
fn recording_publisher_with_cancel(
    cancel: CancellationToken,
) -> (
    GrpcEventPublisher,
    Arc<Mutex<Vec<PublishEventRequest>>>,
    CancellationToken,
) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingBackend {
        received: Arc::clone(&received),
    });
    let outbox = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());
    (GrpcEventPublisher::new(outbox), received, cancel)
}

/// Drive a Submit through a successful scripted execution and assert that
/// `agent_state.status()` ends `Idle` and a `completed` event was published.
#[tokio::test]
async fn coordinator_service_submit_completed_sets_idle_and_publishes_completed() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", text_block("response"), Role::Assistant)),
        Step::Completed,
    ]);
    let (tx, received, events) = service_with_script(
        script,
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    // Send a Submit command with a throwaway ack.
    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "do the thing".to_string(),
        model: None,
        agent_type: None,
        agent_mode: None,
        resume_session_id: None,
        start_fresh: false,
        ack: ack_tx,
    })
    .expect("channel open");

    // Wait for `completed` event to arrive in the recording buffer.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        let types = sub_types(&received);
        if types.contains(&"completed".to_string()) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("completed event not published within 5 s; got {types:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Agent state must be Idle after a successful completion.
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    cancel.cancel();
}

/// Drive a Failed execution and assert `agent_state.last_error()` is set.
#[tokio::test]
async fn coordinator_service_failed_execution_sets_error_on_agent_state() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-err".to_string())),
        Step::AcceptInput,
        Step::Failed(failure()),
    ]);
    let (tx, received, events) = service_with_script(
        script,
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "do the thing".to_string(),
        model: None,
        agent_type: None,
        agent_mode: None,
        resume_session_id: None,
        start_fresh: false,
        ack: ack_tx,
    })
    .expect("channel open");

    // Wait for `error` event.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        let types = sub_types(&received);
        if types.contains(&"error".to_string()) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("error event not published within 5 s; got {types:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // last_error must be set after a Failed outcome.
    let err = agent_state.last_error().await;
    assert!(err.is_some(), "expected last_error to be set");
    assert!(
        err.unwrap().contains("boom"),
        "diagnostic should mention 'boom'"
    );

    cancel.cancel();
}

/// A retryable transient failure (ProcessExit) is not terminal: the service
/// publishes `retrying`, keeps the Run, and re-drives the same Turn — the
/// retried attempt completes and the agent ends `Idle`. All events carry the
/// one Run id (the retry reuses it), proving retry preserves Turn identity.
#[tokio::test]
async fn coordinator_service_retryable_failure_retries_then_completes() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    // First attempt fails with a retryable ProcessExit; the retried attempt
    // completes.
    let scripts = vec![
        Script::new(vec![
            Step::SessionReady(SessionId("sess-1".to_string())),
            Step::AcceptInput,
            Step::Failed(process_exit_error()),
        ]),
        Script::new(vec![
            Step::SessionReady(SessionId("sess-1".to_string())),
            Step::AcceptInput,
            Step::Block(block("msg-1", text_block("recovered"), Role::Assistant)),
            Step::Completed,
        ]),
    ];
    let (tx, received, events) = service_with_harness(
        Harness::scripted_sequence(scripts),
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "do the thing".to_string(),
        model: None,
        agent_type: None,
        agent_mode: None,
        resume_session_id: None,
        start_fresh: false,
        ack: ack_tx,
    })
    .expect("channel open");

    // The backoff for attempt 0 is ~5 s, so allow generous headroom.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        flush(&events).await;
        let types = sub_types(&received);
        if types.contains(&"completed".to_string()) {
            break;
        }
        assert!(
            !types.contains(&"error".to_string()),
            "retryable failure must not publish a terminal error; got {types:?}"
        );
        if tokio::time::Instant::now() >= deadline {
            panic!("retried turn did not complete within 30 s; got {types:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // The transient failure surfaced as `retrying`, never a terminal `error`.
    let types = sub_types(&received);
    assert!(
        types.contains(&"retrying".to_string()),
        "expected a retrying event; got {types:?}"
    );
    assert!(
        !types.contains(&"error".to_string()),
        "no terminal error after a successful retry; got {types:?}"
    );

    // The agent ends Idle after the retried completion.
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    // Every published event shares the one Run id: the retry re-used it rather
    // than minting a fresh Run, so the whole exchange stays one Turn.
    let run_ids: std::collections::HashSet<String> = received
        .lock()
        .expect("lock")
        .iter()
        .map(|r| r.run_id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    assert_eq!(
        run_ids.len(),
        1,
        "retry must preserve a single Run id; saw {run_ids:?}"
    );

    cancel.cancel();
}

/// A `ScheduleWakeup` tool_use in a completed Turn's Source blocks arms a wakeup
/// job on the AGENT_TYPE path; once its (zero) delay elapses the job injects the
/// scheduled prompt back into the service — the harness-path equivalent of the
/// legacy actor's self-scheduling.
#[tokio::test]
async fn coordinator_service_arms_and_fires_scheduled_wakeup() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    // A registry whose injector records what a fired job re-enters.
    let injected = Arc::new(Mutex::new(Vec::<String>::new()));
    let registry = Arc::new(ScheduledJobRegistry::new(Arc::new(CapturingInjector {
        injected: Arc::clone(&injected),
    })));

    // A completed Turn that emits a ScheduleWakeup tool_use with a zero delay.
    let wakeup_block = serde_json::json!({
        "type": "tool_use",
        "id": "toolu_wake",
        "name": "ScheduleWakeup",
        "input": { "delaySeconds": 0, "prompt": "resume the task", "reason": "follow up" }
    });
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block("msg-1", wakeup_block, Role::Assistant)),
        Step::Completed,
    ]);

    // Build the service directly so it uses the capturing registry.
    let (events, _received, _outbox_cancel) = recording_publisher_with_cancel(cancel.clone());
    let coordinator = Coordinator::new(Harness::scripted(script), events.clone(), "task-1");
    let service = CoordinatorService::new(
        coordinator,
        Arc::clone(&agent_state),
        Arc::clone(&config),
        Arc::new(RwLock::new(HashMap::new())),
        Arc::new(RecordingBackend {
            received: Arc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(RwLock::new(None)),
        Arc::clone(&registry),
        CredentialFailureDetector::default(),
        Arc::new(Dispatcher::new()),
    );
    let (tx, rx) = mpsc::unbounded_channel::<AgentCommand>();
    tokio::spawn(service.run(rx, Heartbeat::new()));

    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "go".to_string(),
        model: None,
        agent_type: None,
        agent_mode: None,
        resume_session_id: None,
        start_fresh: false,
        ack: ack_tx,
    })
    .expect("channel open");

    // The zero-delay wakeup arms at turn end and fires, injecting its prompt.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if injected
            .lock()
            .expect("lock")
            .iter()
            .any(|t| t == "resume the task")
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "wakeup prompt was not injected within 5 s; got {:?}",
                injected.lock().expect("lock")
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    cancel.cancel();
}

/// After `MAX_CONSECUTIVE_ERRORS` terminal failures in a row, the circuit
/// breaker trips: the next `Send` is rejected without reaching the harness — its
/// ack is dropped and a sandbox `error` event names the consecutive-failure
/// count.
#[tokio::test]
async fn coordinator_service_circuit_breaker_trips_after_consecutive_errors() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    // Three pre-accept rejections in a row; a fourth script is never needed
    // because the breaker short-circuits the fourth Send before submit.
    let scripts = vec![
        Script::rejecting(failure()),
        Script::rejecting(failure()),
        Script::rejecting(failure()),
    ];
    let (tx, received, events) = service_with_harness(
        Harness::scripted_sequence(scripts),
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    // Drive three failing turns. A rejection drops the ack, so awaiting each ack
    // (which resolves to Err) sequences the sends: by the time the next Send is
    // processed the previous failure has incremented the breaker counter.
    for _ in 0..3 {
        let (ack_tx, ack_rx) = oneshot::channel();
        tx.send(AgentCommand::Send {
            text: "do the thing".to_string(),
            model: None,
            agent_type: None,
            agent_mode: None,
            resume_session_id: None,
            start_fresh: false,
            ack: ack_tx,
        })
        .expect("channel open");
        assert!(
            matches!(ack_rx.await, Err(oneshot::error::RecvError { .. })),
            "each rejected turn drops its ack"
        );
    }

    // The fourth Send trips the breaker: its ack is dropped without executing.
    let (ack_tx, ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "one more".to_string(),
        model: None,
        agent_type: None,
        agent_mode: None,
        resume_session_id: None,
        start_fresh: false,
        ack: ack_tx,
    })
    .expect("channel open");
    assert!(
        matches!(ack_rx.await, Err(oneshot::error::RecvError { .. })),
        "the breaker drops the fourth ack"
    );

    // A sandbox error event announces the breaker, naming the failure count.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        let tripped = received
            .lock()
            .expect("lock")
            .iter()
            .any(|r| r.payload_json.contains("consecutive"));
        if tripped {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let types = sub_types(&received);
            panic!("circuit-breaker event not published within 5 s; got {types:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    cancel.cancel();
}

/// Count delivered events with the given sub-type.
fn count_sub_type(received: &Arc<Mutex<Vec<PublishEventRequest>>>, sub_type: &str) -> usize {
    received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| r.sub_type == sub_type)
        .count()
}

/// Drive a Send whose execution parks on `WaitForCancel`, then wait until the
/// Turn is running (its `session_init` event is delivered).
async fn start_turn_and_wait_running(
    tx: &mpsc::UnboundedSender<AgentCommand>,
    received: &Arc<Mutex<Vec<PublishEventRequest>>>,
    events: &GrpcEventPublisher,
) {
    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "go".to_string(),
        model: None,
        agent_type: None,
        agent_mode: None,
        resume_session_id: None,
        start_fresh: false,
        ack: ack_tx,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(events).await;
        if sub_types(received).contains(&"session_init".to_string()) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "turn did not start within 5 s; got {:?}",
                sub_types(received)
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// An **active** Stop (a Turn is in flight) requests harness cancellation, drives
/// to the single `Cancelled` outcome, and publishes exactly one `stopped` event —
/// closing the Run and Turn. The gRPC-style ack resolves only after that publish.
#[tokio::test]
async fn coordinator_service_active_stop_publishes_exactly_one_stopped() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::WaitForCancel,
    ]);
    let (tx, received, events) = service_with_script(
        script,
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    start_turn_and_wait_running(&tx, &received, &events).await;

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    tx.send(AgentCommand::Stop { ack: Some(stop_tx) })
        .expect("channel open");
    stop_rx
        .await
        .expect("stop ack resolves after Stopped is published");
    flush(&events).await;

    assert_eq!(
        count_sub_type(&received, "stopped"),
        1,
        "exactly one stopped; got {:?}",
        sub_types(&received)
    );
    assert_eq!(agent_state.status().await, AgentStatus::Stopped);

    cancel.cancel();
}

/// An **idle** Stop (no Turn in flight, so no terminal signal can arrive)
/// publishes exactly one `stopped` directly and resolves the ack.
#[tokio::test]
async fn coordinator_service_idle_stop_publishes_exactly_one_stopped() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    // No steps: the harness is never driven, the agent stays idle.
    let (tx, received, events) = service_with_script(
        Script::new(vec![]),
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    tx.send(AgentCommand::Stop { ack: Some(stop_tx) })
        .expect("channel open");
    stop_rx.await.expect("idle stop ack resolves");
    flush(&events).await;

    assert_eq!(
        count_sub_type(&received, "stopped"),
        1,
        "exactly one stopped; got {:?}",
        sub_types(&received)
    );
    assert_eq!(agent_state.status().await, AgentStatus::Stopped);

    cancel.cancel();
}

/// Two Stops in a row still close the Turn with exactly one `stopped` event —
/// the second Stop is absorbed by the in-flight-stop guard.
#[tokio::test]
async fn coordinator_service_duplicate_stop_emits_exactly_one_stopped() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::WaitForCancel,
    ]);
    let (tx, received, events) = service_with_script(
        script,
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    start_turn_and_wait_running(&tx, &received, &events).await;

    // Two Stops arrive before the first resolves.
    let (stop_tx1, stop_rx1) = oneshot::channel::<()>();
    let (stop_tx2, stop_rx2) = oneshot::channel::<()>();
    tx.send(AgentCommand::Stop {
        ack: Some(stop_tx1),
    })
    .expect("channel open");
    tx.send(AgentCommand::Stop {
        ack: Some(stop_tx2),
    })
    .expect("channel open");

    stop_rx1.await.expect("first stop ack resolves");
    // The second ack is absorbed (dropped) by the in-flight guard.
    let _ = stop_rx2.await;
    flush(&events).await;

    assert_eq!(
        count_sub_type(&received, "stopped"),
        1,
        "exactly one stopped across duplicate stops; got {:?}",
        sub_types(&received)
    );

    cancel.cancel();
}

/// The shutdown path: cancelling via `AgentState::stop()` (the token) — not an
/// `AgentCommand::Stop` — must still deliver exactly one `stopped` and close the
/// Turn, proving cancellation-token delivery drives Turn closure.
#[tokio::test]
async fn coordinator_service_cancel_token_delivers_single_stopped() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_type = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::WaitForCancel,
    ]);
    let (tx, received, events) = service_with_script(
        script,
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    start_turn_and_wait_running(&tx, &received, &events).await;

    // Cancel the running Turn via the token (the shutdown.rs path).
    agent_state.stop().await.expect("stop while running");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(&events).await;
        if sub_types(&received).contains(&"stopped".to_string()) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "stopped not published within 5 s; got {:?}",
                sub_types(&received)
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    flush(&events).await;

    assert_eq!(
        count_sub_type(&received, "stopped"),
        1,
        "exactly one stopped; got {:?}",
        sub_types(&received)
    );
    assert_eq!(agent_state.status().await, AgentStatus::Stopped);

    cancel.cancel();
}

// ── Credential-recovery tests ───────────────────────────────────────────────

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// A backend whose `get_credentials` returns a configurable `CODEX_AUTH_JSON`
/// value, optionally rotating it (so each recovery pull is genuinely fresher)
/// and optionally flagging a shared detector on every call (standing in for
/// Codex re-emitting `refresh_token_reused` on each failing Turn). Every other
/// call is inert.
struct CredBackend {
    /// The `CODEX_AUTH_JSON` value returned when `rotate` is false.
    base: String,
    /// When true, return a monotonically fresh value each call instead of `base`.
    rotate: bool,
    counter: AtomicU64,
    /// When set, flip this detector's Codex flag on every `get_credentials` —
    /// simulating a Turn that fails on a stale credential.
    flag: Option<CredentialFailureDetector>,
}

#[async_trait]
impl BackendClient for CredBackend {
    async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), Status> {
        Ok(())
    }
    async fn get_pending_messages(&self, _task_id: &str) -> Result<Vec<PendingMessage>, Status> {
        Ok(vec![])
    }
    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
    ) -> Result<(), Status> {
        Ok(())
    }
    async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
        if let Some(detector) = &self.flag {
            detector.scan("refresh_token_reused");
        }
        let value = if self.rotate {
            let n = self.counter.fetch_add(1, Ordering::SeqCst);
            format!("{{\"tokens\":{{\"refresh_token\":\"tok-{n}\"}}}}")
        } else {
            self.base.clone()
        };
        Ok(GetCredentialsResponse {
            credentials: HashMap::from([("CODEX_AUTH_JSON".to_string(), value)]),
            server_env: HashMap::new(),
            ssh_keys: HashMap::new(),
        })
    }
    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), Status> {
        Ok(())
    }
    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadUploadResponse, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, Status> {
        Ok(None)
    }
    async fn upload_transcript(&self, _task_id: &str, _tar: Vec<u8>) -> Result<(), Status> {
        Ok(())
    }
    async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, Status> {
        Ok(vec![])
    }
}

/// A credential sink that writes the recovered value to a file (the same
/// `auth.json` the disk-dedup reads) and counts how many times it applied.
struct WritingSink {
    path: PathBuf,
    applied: Arc<AtomicUsize>,
}

#[async_trait]
impl CredentialSink for WritingSink {
    async fn apply(&self, value: &str) -> Result<(), String> {
        tokio::fs::write(&self.path, value)
            .await
            .map_err(|e| e.to_string())?;
        self.applied.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A temp `~/.codex` with `auth.json` seeded to `on_disk_auth`, plus a codex
/// `Config` pointed at it. The returned `TempDir` must be kept alive.
fn codex_config_with_auth(on_disk_auth: &str) -> (tempfile::TempDir, Arc<Config>, PathBuf) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_dir = tmp.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).expect("mkdir codex");
    let auth_path = codex_dir.join("auth.json");
    std::fs::write(&auth_path, on_disk_auth).expect("write auth.json");
    let mut config = minimal_config();
    config.agent_type = Some("codex".to_string());
    config.codex_home_dir = codex_dir.display().to_string();
    (tmp, Arc::new(config), auth_path)
}

/// Build a `CoordinatorService` over a scripted harness with a custom backend,
/// detector, and dispatcher — the credential-recovery collaborators.
#[allow(clippy::too_many_arguments)] // reason: test wiring mirrors the service constructor
fn service_with_credentials(
    harness: Harness,
    config: Arc<Config>,
    agent_state: Arc<AgentState>,
    backend: Arc<dyn BackendClient>,
    detector: CredentialFailureDetector,
    dispatcher: Arc<Dispatcher>,
    cancel: CancellationToken,
) -> (
    mpsc::UnboundedSender<AgentCommand>,
    Arc<Mutex<Vec<PublishEventRequest>>>,
    GrpcEventPublisher,
) {
    let (events, received, _outbox_cancel) = recording_publisher_with_cancel(cancel);
    let coordinator = Coordinator::new(harness, events.clone(), "task-1");
    let service = CoordinatorService::new(
        coordinator,
        agent_state,
        config,
        Arc::new(RwLock::new(HashMap::new())),
        backend,
        Arc::new(RwLock::new(None)),
        Arc::new(ScheduledJobRegistry::new(Arc::new(LateInjector::new()))),
        detector,
        dispatcher,
    );
    let (tx, rx) = mpsc::unbounded_channel::<AgentCommand>();
    tokio::spawn(service.run(rx, Heartbeat::new()));
    (tx, received, events)
}

/// A `Send` with a throwaway ack.
fn send_text(tx: &mpsc::UnboundedSender<AgentCommand>, text: &str) {
    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: text.to_string(),
        model: None,
        agent_type: None,
        agent_mode: None,
        resume_session_id: None,
        start_fresh: false,
        ack: ack_tx,
    })
    .expect("channel open");
}

/// A scripted Turn that accepts its input and then fails.
fn failing_turn() -> Script {
    Script::new(vec![
        Step::SessionReady(SessionId("sess-cred".to_string())),
        Step::AcceptInput,
        Step::Failed(failure()),
    ])
}

/// Poll (flushing the outbox) until `cond` holds or the deadline elapses.
async fn wait_until<F: Fn() -> bool>(events: &GrpcEventPublisher, cond: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    loop {
        flush(events).await;
        if cond() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// The terminal reconnect errors credential recovery gives up with.
fn count_reconnect_errors(received: &Arc<Mutex<Vec<PublishEventRequest>>>) -> usize {
    received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| {
            r.sub_type == "error" && r.payload_json.contains("credential_recovery_exhausted")
        })
        .count()
}

/// A fresher backend credential re-drives the SAME prompt on the SAME Run: the
/// failed Turn is suppressed to a single `retrying` (reason `credential_recovery`),
/// the recovered credential is applied, and the re-driven Turn completes — all
/// under one Run id, with no terminal `error`.
#[tokio::test]
async fn coordinator_service_credential_recovery_redrives_same_run_on_fresher_credential() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let (_tmp, config, auth_path) =
        codex_config_with_auth(r#"{"tokens":{"refresh_token":"old"}}"#);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // The backend holds a genuinely fresher credential than what's on disk.
    let backend = Arc::new(CredBackend {
        base: r#"{"tokens":{"refresh_token":"fresh"}}"#.to_string(),
        rotate: false,
        counter: AtomicU64::new(0),
        flag: None,
    });
    let applied = Arc::new(AtomicUsize::new(0));
    let mut dispatcher = Dispatcher::new();
    dispatcher.register(
        "CODEX_AUTH_JSON",
        Arc::new(WritingSink {
            path: auth_path,
            applied: Arc::clone(&applied),
        }),
    );

    // First Turn fails on the stale credential; the recovery re-drive completes.
    let scripts = vec![
        failing_turn(),
        Script::new(vec![
            Step::SessionReady(SessionId("sess-cred".to_string())),
            Step::AcceptInput,
            Step::Block(block("msg-1", text_block("recovered"), Role::Assistant)),
            Step::Completed,
        ]),
    ];
    let (tx, received, events) = service_with_credentials(
        Harness::scripted_sequence(scripts),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(dispatcher),
        cancel.clone(),
    );

    // Flag the credential failure (as Codex would on the failing Turn), then send.
    detector.scan("refresh_token_reused");
    send_text(&tx, "do the thing");

    wait_until(
        &events,
        || sub_types(&received).contains(&"completed".to_string()),
        "completed after credential-recovery retry",
    )
    .await;

    let types = sub_types(&received);
    assert!(
        types.contains(&"retrying".to_string()),
        "expected a retrying event; got {types:?}"
    );
    assert!(
        !types.contains(&"error".to_string()),
        "a recovered credential must not surface a terminal error; got {types:?}"
    );

    // The retrying event names credential recovery as the reason.
    let retrying: serde_json::Value =
        serde_json::from_str(&only(&received, "retrying").payload_json).expect("json");
    assert_eq!(retrying["reason"], "credential_recovery");

    // The recovered credential was applied.
    assert!(
        applied.load(Ordering::SeqCst) >= 1,
        "recovered credential should have been dispatched to its sink"
    );

    // The retry re-used the one Run id (same prompt, same Turn).
    let run_ids: std::collections::HashSet<String> = received
        .lock()
        .expect("lock")
        .iter()
        .map(|r| r.run_id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    assert_eq!(
        run_ids.len(),
        1,
        "recovery must preserve one Run; saw {run_ids:?}"
    );
    assert_eq!(agent_state.status().await, AgentStatus::Idle);

    cancel.cancel();
}

/// An unchanged credential (the backend has nothing fresher than what's on disk)
/// produces exactly ONE terminal reconnect error and NO retry — recovery detects
/// the dedup and surfaces the user-facing error instead of burning a re-drive.
#[tokio::test]
async fn coordinator_service_credential_recovery_unchanged_gives_one_terminal_error() {
    let cancel = CancellationToken::new();
    let on_disk = r#"{"tokens":{"refresh_token":"same"}}"#;
    let (_tmp, config, _auth) = codex_config_with_auth(on_disk);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // The backend returns exactly the on-disk value: nothing fresher.
    let backend = Arc::new(CredBackend {
        base: on_disk.to_string(),
        rotate: false,
        counter: AtomicU64::new(0),
        flag: None,
    });

    let (tx, received, events) = service_with_credentials(
        Harness::scripted(failing_turn()),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(Dispatcher::new()),
        cancel.clone(),
    );

    detector.scan("refresh_token_reused");
    send_text(&tx, "do the thing");

    wait_until(
        &events,
        || count_reconnect_errors(&received) >= 1,
        "one terminal reconnect error",
    )
    .await;
    // Let any (wrongly) scheduled retry have a chance to surface a second event.
    tokio::time::sleep(Duration::from_millis(100)).await;
    flush(&events).await;

    let types = sub_types(&received);
    assert!(
        !types.contains(&"retrying".to_string()),
        "an unchanged credential must not schedule a futile retry; got {types:?}"
    );
    assert_eq!(
        count_reconnect_errors(&received),
        1,
        "exactly one terminal reconnect error; got {types:?}"
    );

    // The single error names the provider and the no-fresher reason.
    let error: serde_json::Value = serde_json::from_str(
        &received
            .lock()
            .expect("lock")
            .iter()
            .find(|r| r.sub_type == "error")
            .expect("error present")
            .payload_json,
    )
    .expect("json");
    assert_eq!(error["reason"], "no_fresher_credential_available");
    assert_eq!(error["provider"], "Codex");
    assert!(
        error["error"]
            .as_str()
            .unwrap_or_default()
            .contains("reconnect"),
        "reconnect message; got {error:?}"
    );
    // The reconnect message is recorded as the agent's last error.
    assert!(
        agent_state
            .last_error()
            .await
            .unwrap_or_default()
            .contains("reconnect"),
        "last_error should carry the reconnect message"
    );

    cancel.cancel();
}

/// Credential recovery is capped per prompt: with an always-fresher backend and
/// a Turn that keeps failing, recovery re-drives up to the cap and then gives up
/// with a single terminal error — exactly `MAX_CREDENTIAL_RECOVERY_RETRIES`
/// `retrying` events precede it, and the Turn never completes.
#[tokio::test]
async fn coordinator_service_credential_recovery_caps_and_then_gives_up() {
    let cancel = CancellationToken::new();
    let (_tmp, config, auth_path) =
        codex_config_with_auth(r#"{"tokens":{"refresh_token":"tok-init"}}"#);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // Every pull is fresher (so recovery always "succeeds"), and every Turn is
    // re-flagged (so the credential failure recurs) — only the per-prompt cap
    // stops the loop.
    let backend = Arc::new(CredBackend {
        base: String::new(),
        rotate: true,
        counter: AtomicU64::new(0),
        flag: Some(detector.clone()),
    });
    let applied = Arc::new(AtomicUsize::new(0));
    let mut dispatcher = Dispatcher::new();
    dispatcher.register(
        "CODEX_AUTH_JSON",
        Arc::new(WritingSink {
            path: auth_path,
            applied: Arc::clone(&applied),
        }),
    );

    // Three failing Turns: two recovery re-drives, then the cap gives up.
    let scripts = vec![failing_turn(), failing_turn(), failing_turn()];
    let (tx, received, events) = service_with_credentials(
        Harness::scripted_sequence(scripts),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(dispatcher),
        cancel.clone(),
    );

    send_text(&tx, "do the thing");

    wait_until(
        &events,
        || count_reconnect_errors(&received) >= 1,
        "terminal reconnect error after cap exhaustion",
    )
    .await;

    let types = sub_types(&received);
    let retrying = types.iter().filter(|t| *t == "retrying").count();
    assert_eq!(
        retrying,
        super::MAX_CREDENTIAL_RECOVERY_RETRIES as usize,
        "exactly the cap of credential-recovery retries precede the give-up; got {types:?}"
    );
    assert_eq!(
        count_reconnect_errors(&received),
        1,
        "one terminal error after the cap; got {types:?}"
    );
    assert!(
        !types.contains(&"completed".to_string()),
        "the Turn never completed; got {types:?}"
    );

    // The give-up names cap exhaustion.
    let error: serde_json::Value = serde_json::from_str(
        &received
            .lock()
            .expect("lock")
            .iter()
            .find(|r| r.sub_type == "error")
            .expect("error present")
            .payload_json,
    )
    .expect("json");
    assert_eq!(error["reason"], "credential_recovery_exhausted");

    cancel.cancel();
}

/// Each credential-recovery give-up counts as exactly ONE consecutive failure —
/// so it takes three give-ups (not fewer) to trip the circuit breaker on the
/// fourth Send. A legacy-style double-count would trip it a Send early, so the
/// three-reconnect-errors-then-breaker sequence proves single-increment accounting.
#[tokio::test]
async fn coordinator_service_credential_giveups_count_once_toward_breaker() {
    let cancel = CancellationToken::new();
    let on_disk = r#"{"tokens":{"refresh_token":"same"}}"#;
    let (_tmp, config, _auth) = codex_config_with_auth(on_disk);
    let agent_state = Arc::new(AgentState::new());
    let detector = CredentialFailureDetector::default();

    // Unchanged credential (always gives up) and re-flagged each Turn.
    let backend = Arc::new(CredBackend {
        base: on_disk.to_string(),
        rotate: false,
        counter: AtomicU64::new(0),
        flag: Some(detector.clone()),
    });

    // Three give-up Turns; the fourth Send trips the breaker before submitting.
    let scripts = vec![failing_turn(), failing_turn(), failing_turn()];
    let (tx, received, events) = service_with_credentials(
        Harness::scripted_sequence(scripts),
        config,
        Arc::clone(&agent_state),
        backend,
        detector.clone(),
        Arc::new(Dispatcher::new()),
        cancel.clone(),
    );

    // Drive three give-ups, sequencing each Send after the prior reconnect error
    // so the breaker counter is observed one increment at a time.
    for n in 1..=3 {
        send_text(&tx, "do the thing");
        wait_until(
            &events,
            || count_reconnect_errors(&received) >= n,
            "reconnect error",
        )
        .await;
    }
    assert_eq!(
        count_reconnect_errors(&received),
        3,
        "three give-ups each publish one reconnect error"
    );

    // The fourth Send trips the breaker (3 >= MAX_CONSECUTIVE_ERRORS) — proving
    // each give-up incremented the counter exactly once.
    send_text(&tx, "one more");
    wait_until(
        &events,
        || {
            received
                .lock()
                .expect("lock")
                .iter()
                .any(|r| r.payload_json.contains("consecutive"))
        },
        "circuit breaker trips on the fourth send",
    )
    .await;

    // Still exactly three reconnect errors — the breaker short-circuited the
    // fourth Send before it could execute another Turn.
    assert_eq!(
        count_reconnect_errors(&received),
        3,
        "breaker fires on the fourth send, not a fourth give-up"
    );

    cancel.cancel();
}
