//! A scheduled wakeup arms from a turn and fires after it.

use super::fixtures::*;
use super::service_fixtures::*;
use super::*;

/// A `ScheduleWakeup` `tool_use` in a completed Turn's Source blocks arms a
/// wakeup job on the `AGENT_PROFILE` path; once its (zero) delay elapses the job
/// injects the scheduled prompt back into the service.
#[tokio::test]
async fn coordinator_service_arms_and_fires_scheduled_wakeup() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
        Arc::new(c)
    };
    let agent_state = Arc::new(AgentState::new());

    let injected = Arc::new(Mutex::new(Vec::<String>::new()));
    let registry = Arc::new(ScheduledJobRegistry::new(Arc::new(CapturingInjector {
        injected: Arc::clone(&injected),
    })));

    let wakeup_block = serde_json::json!({
        "type": "tool_use",
        "id": "toolu_wake",
        "name": "ScheduleWakeup",
        "input": { "delaySeconds": 0, "prompt": "resume the task", "reason": "follow up" }
    });
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block(wakeup_block)),
        Step::Completed,
    ]);

    // Build the service directly so it uses the capturing registry.
    let (events, _received, _outbox_cancel) = recording_publisher_with_cancel(cancel.clone());
    let coordinator = Coordinator::new(HarnessRuntime::scripted(script), events.clone(), "task-1");
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
        Arc::new(crate::commands::CommandRegistry::for_test()),
        CredentialFailureDetector::default(),
        Arc::new(Dispatcher::new()),
        Default::default(),
        crate::workspace_sync::WorkspaceSyncTracker::new(),
    );
    let (tx, rx) = mpsc::channel::<AgentCommand>(64);
    tokio::spawn(service.run(rx, Heartbeat::new()));

    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.try_send(AgentCommand::Send {
        text: "go".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        agent_profile: None,
        switch_harness: false,
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
        assert!(
            tokio::time::Instant::now() < deadline,
            "wakeup prompt was not injected within 5 s; got {:?}",
            injected.lock().expect("lock")
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    cancel.cancel();
}
