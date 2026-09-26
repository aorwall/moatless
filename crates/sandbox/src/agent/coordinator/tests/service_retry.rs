//! Retry with backoff, and the consecutive-error circuit breaker.

use super::fixtures::*;
use super::service_fixtures::*;
use super::*;

/// Drive a Failed execution and assert `agent_state.last_error()` is set.
#[tokio::test]
async fn coordinator_service_failed_execution_sets_error_on_agent_state() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
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
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
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
        assert!(
            tokio::time::Instant::now() < deadline,
            "error event not published within 5 s; got {types:?}"
        );
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

/// A retryable transient failure (`ProcessExit`) is not terminal: the service
/// publishes `retrying`, keeps the Run, and re-drives the same Turn — the
/// retried attempt completes and the agent ends `Idle`. All events carry the
/// one Run id (the retry reuses it), proving retry preserves Turn identity.
#[tokio::test(start_paused = true)]
async fn coordinator_service_retryable_failure_retries_then_completes() {
    use crate::agent::types::AgentStatus;

    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
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
        HarnessRuntime::scripted_sequence(scripts),
        Arc::clone(&config),
        Arc::clone(&agent_state),
        cancel.clone(),
    );

    let (ack_tx, _ack_rx) = oneshot::channel();
    tx.send(AgentCommand::Send {
        text: "do the thing".to_string(),
        model: None,
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
        ack: ack_tx,
    })
    .expect("channel open");

    // Paused time auto-advances through the ~5 s attempt-0 backoff, so this
    // deadline is virtual.
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
        assert!(
            tokio::time::Instant::now() < deadline,
            "retried turn did not complete within 30 s; got {types:?}"
        );
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

/// After `MAX_CONSECUTIVE_ERRORS` terminal failures in a row, the circuit
/// breaker trips.
///
/// The next `Send` is rejected without reaching the harness: its ack is
/// dropped, and a sandbox `error` event names the consecutive-failure count.
#[tokio::test]
async fn coordinator_service_circuit_breaker_trips_after_consecutive_errors() {
    let cancel = CancellationToken::new();
    let config = {
        let mut c = minimal_config();
        c.agent_profile = Some("claude-code".to_string());
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
        HarnessRuntime::scripted_sequence(scripts),
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
            agent_mode: None,
            effort: None,
            resume_session_id: None,
            start_fresh: false,
            fork: None,
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
        agent_mode: None,
        effort: None,
        resume_session_id: None,
        start_fresh: false,
        fork: None,
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
