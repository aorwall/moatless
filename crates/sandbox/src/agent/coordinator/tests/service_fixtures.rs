//! Building a `CoordinatorService` over a scripted harness.

use super::fixtures::*;
use super::*;

/// Build a `CoordinatorService` over a scripted harness with the recording
/// publisher, returning the command sender and the event recording buffer.
pub(super) fn service_with_script(
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

/// Like `service_with_script` but takes a pre-built `Harness` so tests can
/// drive a scripted *sequence* (e.g. fail-then-succeed for a retry).
pub(super) fn service_with_harness(
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
        Arc::new(crate::commands::CommandRegistry::for_test()),
        CredentialFailureDetector::default(),
        Arc::new(Dispatcher::new()),
        crate::workspace_sync::WorkspaceSyncTracker::new(),
    );
    let (tx, rx) = mpsc::unbounded_channel::<AgentCommand>();
    let heartbeat = Heartbeat::new();
    tokio::spawn(service.run(rx, heartbeat));
    (tx, received, events)
}

/// A job injector that records every injected prompt, for asserting that a
/// fired `ScheduleWakeup` re-enters the service.
pub(super) struct CapturingInjector {
    pub(super) injected: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl JobInjector for CapturingInjector {
    async fn inject(&self, _idempotency_key: &str, text: String) {
        self.injected.lock().expect("lock").push(text);
    }
}

/// Like `recording_publisher` but accepts an external cancel token so the
/// outbox drainer lives for the caller's test scope.
pub(super) fn recording_publisher_with_cancel(
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
    let (outbox, _drainer) = EventOutbox::spawn(backend, "task-1".to_string(), cancel.clone());
    (GrpcEventPublisher::new(outbox), received, cancel)
}

/// Drive a Send whose execution parks on `WaitForCancel`, then wait until the
/// Turn is running (its `session_init` event is delivered).
pub(super) async fn start_turn_and_wait_running(
    tx: &mpsc::UnboundedSender<AgentCommand>,
    received: &Arc<Mutex<Vec<PublishEventRequest>>>,
    events: &GrpcEventPublisher,
) {
    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "go".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        ack: ack_tx,
    })
    .expect("channel open");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        flush(events).await;
        if sub_types(received).contains(&"session_init".to_string()) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "turn did not start within 5 s; got {:?}",
            sub_types(received)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
