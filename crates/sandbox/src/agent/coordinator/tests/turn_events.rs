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

    // Every event of the Run carries its Run id.
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
