//! The readiness gate and the startup precedence race.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::backend::ReplayBackend;
use super::support::{
    interrupted_turn, recorded_inputs, recording_state, settled_inputs, text_message,
};
use crate::agent::dispatcher::dispatch_pending_messages;
use crate::agent::prompt::{PendingPrompt, PromptResponse, PromptToolName};
use crate::agent::types::ResumeSession;
use crate::backend_client::BackendClient;
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
async fn dispatch_drives_the_pending_message_after_ready_per_family() {
    // The configured family (`config.agent_type`) — fixed for the pod across
    // replacement — selects how a bare pending Message resolves its resume: the
    // Claude family continues its last session (`ResumeSession::Last`, i.e.
    // `--continue`); Codex has no `--continue` and starts fresh. Asserting the
    // family-specific resume proves selection actually flows from config through
    // the coordinator to the harness, not that the loop merely runs twice.
    for (family, expected_resume) in [("claude-code", Some(ResumeSession::Last)), ("codex", None)] {
        let backend = ReplayBackend::new(Ok(vec![text_message("hello", "uuid-1")]), None);
        let (state, recorder) = recording_state(Some(family), InitPhase::Ready, backend.clone());

        dispatch_pending_messages(state).await;

        let inputs = recorded_inputs(&recorder, 1).await;
        assert_eq!(
            inputs.len(),
            1,
            "family {family}: exactly the pending message"
        );
        assert_eq!(
            inputs[0].text, "hello",
            "family {family}: the pending message text is what the harness accepted"
        );
        assert_eq!(
            inputs[0].resume, expected_resume,
            "family {family}: config.agent_type selects the family's resume default"
        );
        assert!(backend.get_pending_calls() >= 1);
    }
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

    // Wait for the message to reach the harness, then give any *second* (buggy)
    // delivery time to land while the racer keeps hammering the now-open gate —
    // the single-flight dispatch guard must coalesce them so it arrives once.
    let _ = recorded_inputs(&recorder, 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.cancel();
    racer.await.expect("external dispatcher task panicked");

    let inputs = settled_inputs(&recorder).await;

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
    // The pending message reaches the harness exactly once — not once per racer
    // pass. This is the regression: without serialising dispatch through the ack,
    // each concurrent dispatcher re-fetches the still-unacked row and re-submits.
    assert_eq!(
        inputs.len(),
        1,
        "the pending message must reach the harness exactly once, got {inputs:?}"
    );
    assert_eq!(inputs[0].text, "hello", "the pending message is what ran");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_resolution_is_serialised_under_the_dispatch_guard() {
    // The prompt branch must run *under* the single-flight guard: otherwise two
    // dispatchers both fetch the same prompt Message, one answers it and the loser
    // hits the "prompt already consumed" fallback and resubmits the text as normal
    // harness Input. Proven deterministically by holding the guard externally and
    // showing the prompt cannot be answered until it is released — if the branch
    // ran before acquiring the guard, it would answer while we hold it.
    let backend = ReplayBackend::new(Ok(vec![text_message("approve it", "uuid-p")]), None);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    // Prompt acknowledgement needs an active run's session + run id.
    state.agent.set_session_id("sess-1".to_string()).await;
    state.agent.set_run_id("run-1".to_string()).await;
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    state
        .agent
        .set_pending_prompt(PendingPrompt {
            tool_name: PromptToolName::AskUserQuestion,
            input: serde_json::Value::Null,
            sender: tx,
        })
        .await;

    // Hold the guard, then dispatch: it must block before the prompt branch.
    let held = state.dispatch_lock.clone().lock_owned().await;
    let dispatch = tokio::spawn(dispatch_pending_messages(state.clone()));

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        rx.try_recv().is_err(),
        "the prompt must not be answered while the dispatch guard is held"
    );
    assert!(
        state.agent.has_pending_prompt().await,
        "the prompt must remain pending until the guard is free"
    );

    // Release the guard: dispatch resolves the prompt exactly once with the
    // Message text, and it never leaks into the harness as normal Input.
    drop(held);
    dispatch.await.expect("dispatch task panicked");
    match rx
        .await
        .expect("the prompt must be answered once the guard is free")
    {
        PromptResponse::AskUserResponse { response } => assert_eq!(response, "approve it"),
        other => panic!("unexpected prompt response: {other:?}"),
    }
    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "the prompt Message must never become normal harness Input"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_ack_failure_retries_without_re_running_the_message() {
    // After the harness accepts the Input, the ack RPC fails once. The dispatcher
    // must retry the ack while holding the guard — never resubmit — so a
    // concurrent waiter cannot re-run the already-accepted Message during the
    // outage, and the row is eventually acknowledged.
    let backend = ReplayBackend::new(Ok(vec![text_message("do it", "uuid-1")]), None);
    backend.fail_next_acks(1);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    // A waiter hammering the gate throughout the ack outage.
    let stop = CancellationToken::new();
    let racer_stop = stop.clone();
    let racer_state = state.clone();
    let racer = tokio::spawn(async move {
        while !racer_stop.is_cancelled() {
            dispatch_pending_messages(racer_state.clone()).await;
            tokio::task::yield_now().await;
        }
    });

    dispatch_pending_messages(state.clone()).await;

    // Wait past one full ack-retry backoff (base 2s + up to 25% jitter) so the
    // retry has landed.
    let first = recorded_inputs(&recorder, 1).await;
    tokio::time::sleep(Duration::from_millis(3200)).await;
    stop.cancel();
    racer.await.expect("waiter task panicked");

    let inputs = settled_inputs(&recorder).await;
    assert_eq!(
        inputs.len(),
        1,
        "the Message must be delivered exactly once despite the transient ack failure, got {inputs:?}"
    );
    assert_eq!(first[0].text, "do it");
    // The retry succeeded rather than giving up: the row is acked and gone.
    assert!(
        backend.get_pending_messages("t").await.unwrap().is_empty(),
        "the ack must eventually succeed and prune the row"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sustained_ack_failure_never_re_runs_and_aborts_on_shutdown() {
    // The ack fails for the whole outage. A failed ack is never treated as
    // success: the dispatcher holds the guard and keeps retrying — never dropping
    // the still-pending row back for a hammering waiter to re-run — and aborts
    // cleanly on shutdown, leaving the row for the replacement pod. This is the
    // regression for "on exhaustion the guard drops and the row is re-run".
    let backend = ReplayBackend::new(Ok(vec![text_message("do it", "uuid-1")]), None);
    backend.fail_next_acks(usize::MAX);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    let stop = CancellationToken::new();
    let racer_stop = stop.clone();
    let racer_state = state.clone();
    let racer = tokio::spawn(async move {
        while !racer_stop.is_cancelled() {
            dispatch_pending_messages(racer_state.clone()).await;
            tokio::task::yield_now().await;
        }
    });

    dispatch_pending_messages(state.clone()).await;

    // The message reaches the harness once; while the ack keeps failing the guard
    // stays held, so the hammering waiter can never re-run it.
    let _ = recorded_inputs(&recorder, 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        settled_inputs(&recorder).await.len(),
        1,
        "a sustained ack failure must never re-run the accepted message"
    );

    // Shutdown: the retry aborts, the guard drops, and the un-acked row is left
    // pending for the replacement pod — still delivered exactly once here.
    state.lifecycle_cancel.cancel();
    tokio::time::sleep(Duration::from_millis(200)).await;
    stop.cancel();
    racer.await.expect("waiter task panicked");

    assert_eq!(
        settled_inputs(&recorder).await.len(),
        1,
        "shutdown during ack retry must not produce a second delivery"
    );
    assert!(
        !backend.get_pending_messages("t").await.unwrap().is_empty(),
        "an un-acked row stays pending for cross-pod redelivery, not silently dropped"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_ack_failure_never_becomes_normal_input() {
    // The prompt answer's ack fails throughout, while a notification hammers the
    // gate. The answer must stay a prompt answer under the same retry-until-acked
    // policy — never leak into the harness as ordinary input, and never be
    // re-fetched by the waiter — and the retry must abort on shutdown. This is the
    // regression for "one-shot prompt ack fails, the row is later submitted
    // normally".
    let backend = ReplayBackend::new(Ok(vec![text_message("approve it", "uuid-p")]), None);
    backend.fail_next_acks(usize::MAX);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    // Prompt acknowledgement needs an active run's session + run id.
    state.agent.set_session_id("sess-1".to_string()).await;
    state.agent.set_run_id("run-1".to_string()).await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .agent
        .set_pending_prompt(PendingPrompt {
            tool_name: PromptToolName::AskUserQuestion,
            input: serde_json::Value::Null,
            sender: tx,
        })
        .await;

    let stop = CancellationToken::new();
    let racer_stop = stop.clone();
    let racer_state = state.clone();
    let racer = tokio::spawn(async move {
        while !racer_stop.is_cancelled() {
            dispatch_pending_messages(racer_state.clone()).await;
            tokio::task::yield_now().await;
        }
    });

    // The prompt branch retries the ack inline under the guard, so this dispatch
    // does not return until shutdown — drive it as a task.
    let dispatch = tokio::spawn(dispatch_pending_messages(state.clone()));

    // The answer is delivered exactly once with the message text, before the ack.
    match rx.await.expect("the prompt must be answered") {
        PromptResponse::AskUserResponse { response } => assert_eq!(response, "approve it"),
        other => panic!("unexpected prompt response: {other:?}"),
    }

    // While the ack keeps failing the guard stays held: the waiter cannot re-fetch
    // the row and the answered prompt never becomes ordinary harness input.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "an answered prompt must never leak into the harness as ordinary input"
    );
    assert!(
        !state.agent.has_pending_prompt().await,
        "the prompt was consumed as an answer, not left pending for re-dispatch"
    );

    // Shutdown aborts the retry; still no ordinary input was ever produced.
    state.lifecycle_cancel.cancel();
    stop.cancel();
    racer.await.expect("waiter task panicked");
    dispatch.await.expect("dispatch task panicked");
    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "shutdown during prompt-ack retry must not produce ordinary input"
    );
}
