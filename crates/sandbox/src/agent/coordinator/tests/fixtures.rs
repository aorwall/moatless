//! Recording backend and the values every coordinator case runs against.

use super::*;

/// Records every event the outbox delivers, in order. Every other backend call
/// is unused by these tests and reports unimplemented.
pub(super) struct RecordingBackend {
    pub(super) received: Arc<Mutex<Vec<PublishEventRequest>>>,
}

#[async_trait]
impl BackendClient for RecordingBackend {
    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), Status> {
        self.received.lock().expect("lock").push(req);
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
        _message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
        _answered_prompt: Option<&str>,
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
pub(super) fn recording_publisher() -> (
    GrpcEventPublisher,
    Arc<Mutex<Vec<PublishEventRequest>>>,
    CancellationToken,
) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingBackend {
        received: Arc::clone(&received),
    });
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());
    (GrpcEventPublisher::new(outbox), received, cancel)
}

/// A source event for one native block under a given Message id.
pub(super) fn block(message_id: &str, block: serde_json::Value, role: Role) -> SourceEvent {
    SourceEvent {
        source_id: SourceId(format!(
            "{message_id}-{}",
            block["type"].as_str().unwrap_or("b")
        )),
        message_id: NativeMessageId(message_id.to_string()),
        session: SessionId("sess-1".to_string()),
        parent_tool_use_id: None,
        role,
        block: serde_json::from_value(block).expect("test block is a canonical content block"),
        tool_use_result: None,
        usage: None,
        model: None,
        diagnostics: None,
    }
}

pub(super) fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "text": text })
}

pub(super) fn failure() -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        ErrorClass::NativeProtocol,
        SessionImpact::Unknown,
        "boom",
    )
}

/// A resume-startup failure whose Session is gone — the shape the Claude/Codex
/// adapters report when a `--resume` target no longer loads.
pub(super) fn lost_error() -> HarnessError {
    HarnessError::new(
        FailureStage::Startup,
        ErrorClass::UnavailableSession,
        SessionImpact::Lost,
        "resume target not found",
    )
}

/// A retryable transient failure: the process exited mid-run, which the retry
/// policy classifies as worth re-driving.
pub(super) fn process_exit_error() -> HarnessError {
    HarnessError::new(
        FailureStage::Execution,
        ErrorClass::ProcessExit,
        SessionImpact::Lost,
        "process exited",
    )
}

/// An input that asks to resume a known Session.
pub(super) fn resume_input(id: &str, text: &str, session: &str) -> Input {
    let mut input = Input::text(id, text);
    input.resume = Some(ResumeSession::WithId(session.to_string()));
    input
}

/// Sub-types of the delivered events, in delivery order.
pub(super) fn sub_types(received: &Arc<Mutex<Vec<PublishEventRequest>>>) -> Vec<String> {
    received
        .lock()
        .expect("lock")
        .iter()
        .map(|r| r.sub_type.clone())
        .collect()
}

/// The single delivered event with the given sub-type (panics if not exactly
/// one).
pub(super) fn only(
    received: &Arc<Mutex<Vec<PublishEventRequest>>>,
    sub_type: &str,
) -> PublishEventRequest {
    let guard = received.lock().expect("lock");
    let mut matches = guard.iter().filter(|r| r.sub_type == sub_type);
    let found = matches.next().expect("event present").clone();
    assert!(matches.next().is_none(), "expected exactly one {sub_type}");
    found
}

pub(super) async fn flush(events: &GrpcEventPublisher) {
    events.outbox().flush(Duration::from_secs(2)).await;
}

/// Count delivered events with the given sub-type.
pub(super) fn count_sub_type(
    received: &Arc<Mutex<Vec<PublishEventRequest>>>,
    sub_type: &str,
) -> usize {
    received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| r.sub_type == sub_type)
        .count()
}
