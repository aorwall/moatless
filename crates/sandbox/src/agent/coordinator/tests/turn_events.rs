//! The `agent.*` events one turn publishes, and their order.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn fresh_turn_publishes_ordered_events_and_acks_after_acceptance() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block(text_block("hello"))),
        Step::Completed,
    ]);
    let mut coord = Coordinator::new(HarnessRuntime::scripted(script), events.clone(), "task-1");

    let (ack_tx, ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");
    coord.pump_one_execution().await;
    flush(&events).await;

    // ExecutionStarted → turn start (started + taskTurn active) + session init;
    // then the terminal completion. The block itself reaches the backend
    // through the trajectory.
    assert_eq!(
        sub_types(&received),
        vec!["started", "active", "session_init", "completed"],
    );

    // The batch is acknowledged only after InputAccepted, against the minted Run.
    let ack = ack_rx.await.expect("ack resolved");
    assert_eq!(ack.session_id, "sess-1");
    let started = only(&received, "started");
    assert_eq!(ack.run_id, started.run_id);
    assert!(!ack.run_id.is_empty());

    for req in received.lock().expect("lock").iter() {
        assert_eq!(req.run_id, ack.run_id, "run-scoped: {}", req.sub_type);
    }
}

/// A signal moves the state machine and the event stream as one act.
///
/// Checked after *every* signal, not just at the end: the agent reports
/// `Running` for exactly the span the events say the turn is open.
#[tokio::test]
async fn a_turn_moves_the_state_machine_and_the_event_stream_together() {
    use crate::agent::agent_state::AgentState;
    use crate::agent::types::AgentStatus;

    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Block(block(text_block("hello"))),
        Step::Completed,
    ]);
    let agent_state = Arc::new(AgentState::new());
    let mut coord = Coordinator::new(HarnessRuntime::scripted(script), events.clone(), "task-1");
    coord.bind(Arc::clone(&agent_state), CancellationToken::new());

    assert_eq!(
        agent_state.status().await,
        AgentStatus::Idle,
        "before the turn"
    );

    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "hi"), ack_tx)
        .await
        .expect("submit");

    while let Some(signal) = coord.next_signal().await {
        let terminal = matches!(signal, Signal::ExecutionFinished { .. });
        coord.handle_signal(signal).await;
        flush(&events).await;

        let published = sub_types(&received);
        let turn_is_open = published.contains(&"active".to_string())
            && !published.contains(&"completed".to_string());
        assert_eq!(
            agent_state.status().await == AgentStatus::Running,
            turn_is_open,
            "state machine disagrees with the events after {published:?}"
        );

        if terminal {
            break;
        }
    }

    assert_eq!(
        sub_types(&received),
        vec!["started", "active", "session_init", "completed"],
    );
    assert_eq!(agent_state.status().await, AgentStatus::Idle);
    assert_eq!(
        agent_state.current_run_id().await.as_deref(),
        Some(coord.current_run_id()),
        "the state machine names the Run the events were stamped with"
    );
}

#[tokio::test]
async fn failed_execution_publishes_error_and_no_completion() {
    let (events, received, _cancel) = recording_publisher();
    let script = Script::new(vec![
        Step::SessionReady(SessionId("sess-1".to_string())),
        Step::AcceptInput,
        Step::Failed(failure()),
    ]);
    let mut coord = Coordinator::new(HarnessRuntime::scripted(script), events.clone(), "task-1");
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

/// The backend reads `reason` to decide a Loop's agent fallback, and the UI
/// shows the reset time from the error text.
#[test]
fn a_usage_limit_error_names_its_reason_and_reset_time() {
    let resets_at = chrono::DateTime::parse_from_rfc3339("2026-10-03T18:05:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let error = HarnessError::usage_limit(
        FailureStage::Execution,
        SessionImpact::Intact,
        "You've hit your usage limit.",
        Some(resets_at),
    );
    let payload = super::super::seam::error_payload(&error);
    assert_eq!(payload["reason"], "usage_limit");
    assert_eq!(payload["resetsAt"], "2026-10-03T18:05:00Z");
    assert_eq!(
        payload["error"],
        "You've hit your usage limit. (usage limit resets at 2026-10-03T18:05:00Z)"
    );

    let unknown_reset = HarnessError::usage_limit(
        FailureStage::Execution,
        SessionImpact::Intact,
        "You've hit your usage limit.",
        None,
    );
    let payload = super::super::seam::error_payload(&unknown_reset);
    assert_eq!(payload["reason"], "usage_limit");
    assert!(payload.get("resetsAt").is_none());
}

/// A throttle reaches `agent.error` only once its retries are spent, and a Loop
/// falls back from it as from a spent allowance.
#[test]
fn a_throttle_that_outlasted_its_retries_is_reported_as_a_usage_limit() {
    let error = HarnessError::new(
        FailureStage::Execution,
        ErrorClass::ProviderThrottle,
        SessionImpact::Intact,
        "exceeded retry limit, last status: 429 Too Many Requests",
    );
    assert!(crate::agent::retry::is_retryable_harness_error(&error));
    let payload = super::super::seam::error_payload(&error);
    assert_eq!(payload["reason"], "usage_limit");
}

/// An unavailable provider reaches `agent.error` only once its retries are
/// spent, under its own reason.
#[test]
fn an_unavailable_provider_is_reported_under_its_own_reason() {
    let error = HarnessError::new(
        FailureStage::Execution,
        ErrorClass::ProviderUnavailable,
        SessionImpact::Intact,
        "auth_unavailable: no auth available (providers=codex)",
    );
    assert!(crate::agent::retry::is_retryable_harness_error(&error));
    let payload = super::super::seam::error_payload(&error);
    assert_eq!(payload["reason"], "provider_unavailable");
}
