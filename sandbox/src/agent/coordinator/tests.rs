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
        source_id: SourceId(format!("{message_id}-{}", block["type"].as_str().unwrap_or("b"))),
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
        Step::Block(
            block(
                "msg-1",
                serde_json::json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": {} }),
                Role::Assistant,
            ),
        ),
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
    assert_eq!(messages[0]["message"]["content"].as_array().unwrap().len(), 1);

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
    assert!(matches!(ack_rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
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
    assert!(matches!(ack_rx.await, Err(oneshot::error::RecvError { .. })));
}
