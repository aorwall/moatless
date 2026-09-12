//! A resume the harness reports as lost re-drives one Turn.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn resume_run_maps_recovery_events_onto_the_interrupted_turns_run_id() {
    let (events, received, _cancel) = recording_publisher();
    let mut coord = Coordinator::new(
        Harness::scripted(Script::new(vec![])),
        events.clone(),
        "task-1",
    );

    // The recovery path adopts the interrupted turn's run_id before the resumed
    // execution starts, so every event maps back to that turn instead of forking
    // a new one the reaper would later orphan.
    coord.resume_run("run-orig".to_string());

    let (ack_tx, _ack_rx) = oneshot::channel();
    coord
        .submit(Input::text("in-1", "resumed prompt"), ack_tx)
        .await
        .expect("submit");
    coord
        .handle_signal(Signal::ExecutionStarted {
            agent_turn_id: None,
            session: SessionId("sess-1".to_string()),
            cause: Cause::Input(InputId("in-1".to_string())),
        })
        .await;
    flush(&events).await;

    for req in received.lock().expect("lock").iter() {
        assert_eq!(
            req.run_id, "run-orig",
            "recovery reuses the interrupted turn's run_id: {}",
            req.sub_type
        );
    }
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
    let out = context_recovery_prompt("task-xyz", "fix the bug");
    assert!(out.contains("moat messages list task-xyz"), "got {out}");
    assert!(out.contains("fix the bug"), "got {out}");
    assert!(out.contains("context-recovery"), "got {out}");
}
