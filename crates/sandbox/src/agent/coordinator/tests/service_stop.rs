//! Stop and cancellation publish exactly one terminal event.

use super::fixtures::*;
use super::service_fixtures::*;
use super::*;

/// An **active** Stop (a Turn is in flight) requests harness cancellation,
/// drives to the single `Cancelled` outcome, and publishes exactly one
/// `stopped` event — closing the Run and Turn. The gRPC-style ack resolves only
/// after that publish.
#[tokio::test]
async fn coordinator_service_active_stop_publishes_exactly_one_stopped() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
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
    tx.try_send(AgentCommand::Stop {
        source: StopSource::User,
        ack: Some(stop_tx),
    })
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
    let stopped = only(&received, "stopped");
    let payload: serde_json::Value =
        serde_json::from_str(&stopped.payload_json).expect("valid payload");
    assert_eq!(payload["source"], "user");
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
        c.agent_profile = Some("claude-code".to_string());
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
    tx.try_send(AgentCommand::Stop {
        source: StopSource::IdleReaper,
        ack: Some(stop_tx),
    })
    .expect("channel open");
    stop_rx.await.expect("idle stop ack resolves");
    flush(&events).await;

    assert_eq!(
        count_sub_type(&received, "stopped"),
        1,
        "exactly one stopped; got {:?}",
        sub_types(&received)
    );
    let stopped = only(&received, "stopped");
    let payload: serde_json::Value =
        serde_json::from_str(&stopped.payload_json).expect("valid payload");
    assert_eq!(payload["source"], "idle_reaper");
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
        c.agent_profile = Some("claude-code".to_string());
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
    tx.try_send(AgentCommand::Stop {
        source: StopSource::User,
        ack: Some(stop_tx1),
    })
    .expect("channel open");
    tx.try_send(AgentCommand::Stop {
        source: StopSource::User,
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

/// The shutdown path: cancelling via `AgentState::stop()` (not an
/// `AgentCommand::Stop`) closes the Turn as `turn_interrupted`, never
/// `stopped`. See `seam::interrupted_turn_payload` for why.
#[tokio::test]
async fn coordinator_service_cancel_token_publishes_turn_interrupted_not_stopped() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
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
        if sub_types(&received).contains(&"turn_interrupted".to_string()) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "turn_interrupted not published within 5 s; got {:?}",
            sub_types(&received)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    flush(&events).await;

    assert_eq!(
        count_sub_type(&received, "turn_interrupted"),
        1,
        "exactly one turn_interrupted; got {:?}",
        sub_types(&received)
    );
    // The turn must stay resumable: no terminal row for the backend to read as
    // "this Turn ended".
    assert_eq!(
        count_sub_type(&received, "stopped"),
        0,
        "a shutdown self-cancel must not publish a terminal stopped; got {:?}",
        sub_types(&received)
    );
    assert_eq!(agent_state.status().await, AgentStatus::Stopped);

    cancel.cancel();
}
