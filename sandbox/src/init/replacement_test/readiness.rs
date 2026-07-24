//! The readiness gate and the startup precedence race.

use tokio_util::sync::CancellationToken;

use super::support::{
    interrupted_turn, recorded_inputs, recording_state, settled_inputs, text_message, ReplayBackend,
};
use crate::agent::dispatcher::dispatch_pending_messages;
use crate::agent::types::ResumeSession;
use crate::init::{open_for_work, InitPhase, WorkspaceOrigin};

/// True when `input` is the interrupted-run replay for the restored session.
fn is_restored_recover(input: &crate::agent::harness::Input) -> bool {
    matches!(&input.resume, Some(ResumeSession::WithId(id)) if id == "restored-sess")
}

#[tokio::test]
async fn dispatch_is_skipped_before_ready() {
    // A notification that races init must not fetch messages or drive the
    // harness: the harness may still be resuming un-restored Session state.
    let backend = ReplayBackend::new(Ok(vec![text_message("hello", "uuid-1")]), None);
    let (state, recorder) =
        recording_state(Some("claude-code"), InitPhase::Starting, backend.clone());

    dispatch_pending_messages(state).await;

    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "no input may reach the harness before Ready"
    );
    assert_eq!(
        backend.get_pending_calls(),
        0,
        "the gate short-circuits before the backend is even probed"
    );
}

#[tokio::test]
async fn dispatch_drives_the_pending_message_after_ready() {
    let backend = ReplayBackend::new(Ok(vec![text_message("hello", "uuid-1")]), None);
    let (state, recorder) =
        recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    dispatch_pending_messages(state).await;

    let inputs = recorded_inputs(&recorder, 1).await;
    assert_eq!(inputs.len(), 1, "exactly the pending message");
    assert_eq!(
        inputs[0].text, "hello",
        "the pending message text is what the harness accepted"
    );
    assert!(backend.get_pending_calls() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_dispatch_cannot_preempt_the_precedence_decision() {
    // Both a pending Message and an interrupted Run exist. A racing external
    // dispatch (a gRPC notification) hammers the gate concurrently with
    // `open_for_work`. The gate must keep external dispatch out until the
    // precedence decision completes; otherwise the racer could consume and ack
    // the pending Message before the probe, leaving the probe to see no pending
    // row and enqueue a now-stale Recover behind the Message.
    let backend = ReplayBackend::new(
        Ok(vec![text_message("hello", "uuid-1")]),
        Some(interrupted_turn(
            "restored-sess",
            "prior work",
            Some("claude-code"),
        )),
    );
    let (state, recorder) =
        recording_state(Some("claude-code"), InitPhase::Starting, backend.clone());

    // A concurrent external dispatcher, hammering the gate throughout init.
    let stop = CancellationToken::new();
    let racer_stop = stop.clone();
    let racer_state = state.clone();
    let racer = tokio::spawn(async move {
        while !racer_stop.is_cancelled() {
            dispatch_pending_messages(racer_state.clone()).await;
            tokio::task::yield_now().await;
        }
    });

    // Cross into "open for work": precedence decided while gated, then Ready,
    // then the one authoritative dispatch.
    open_for_work(&state, WorkspaceOrigin::Persisted).await;

    let inputs = recorded_inputs(&recorder, 1).await;
    stop.cancel();
    let _ = racer.await;

    // Pending was present at decision time, so the interrupted run was never
    // even probed…
    assert_eq!(
        backend.recover_calls(),
        0,
        "a pending message must suppress the interrupted-run probe"
    );
    // …and no stale Recover for the restored session ever reached the harness.
    assert!(
        inputs.iter().all(|i| !is_restored_recover(i)),
        "no stale Recover may reach the harness"
    );
    // The pending message is what actually ran.
    assert!(
        inputs.iter().any(|i| i.text == "hello"),
        "the pending message must run"
    );
}
