//! Interrupted-Run vs pending-Message precedence, proven at the harness: which
//! `Input` the harness actually accepts (and, for a probe error, that none is).

use super::support::{
    interrupted_turn, recorded_inputs, recording_state, settled_inputs, text_message, ReplayBackend,
};
use crate::agent::dispatcher::dispatch_pending_messages;
use crate::agent::harness::Input;
use crate::agent::types::ResumeSession;
use crate::init::{recover_interrupted_run_unless_pending, InitPhase, WorkspaceOrigin};

fn resumes_restored_session(input: &Input) -> bool {
    matches!(&input.resume, Some(ResumeSession::WithId(id)) if id == "restored-sess")
}

#[tokio::test]
async fn no_pending_message_replays_the_interrupted_run_for_each_family() {
    // For each selected family, the restored Run is replayed: the harness accepts
    // an Input carrying the restored session id and prompt.
    for family in ["claude-code", "codex"] {
        let backend = ReplayBackend::new(
            Ok(vec![]),
            Some(interrupted_turn("restored-sess", "prior work", Some(family))),
        );
        let (state, recorder) = recording_state(Some(family), InitPhase::Ready, backend.clone());

        recover_interrupted_run_unless_pending(&state, WorkspaceOrigin::Persisted).await;

        let inputs = recorded_inputs(&recorder, 1).await;
        assert_eq!(inputs.len(), 1, "family {family}: exactly the interrupted run");
        assert_eq!(
            inputs[0].text, "prior work",
            "family {family}: the harness runs the restored prompt"
        );
        assert!(
            resumes_restored_session(&inputs[0]),
            "family {family}: the harness resumes the restored session"
        );
        assert_eq!(
            backend.recover_calls(),
            1,
            "family {family}: the interrupted run is probed exactly once"
        );
    }
}

#[tokio::test]
async fn pending_message_supersedes_the_interrupted_run() {
    // The interrupted Run exists, but a newer pending Message is present. The
    // replay must be skipped, and the pending Message is what the harness runs.
    let backend = ReplayBackend::new(
        Ok(vec![text_message("new instruction", "uuid-new")]),
        Some(interrupted_turn(
            "restored-sess",
            "prior work",
            Some("claude-code"),
        )),
    );
    let (state, recorder) =
        recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    recover_interrupted_run_unless_pending(&state, WorkspaceOrigin::Persisted).await;
    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "a pending message must suppress the interrupted-run replay"
    );
    assert_eq!(
        backend.recover_calls(),
        0,
        "the interrupted run must not even be probed when a message is pending"
    );

    // The authoritative dispatch then drives the pending message, not the Run.
    dispatch_pending_messages(state).await;
    let inputs = recorded_inputs(&recorder, 1).await;
    assert!(
        inputs.iter().any(|i| i.text == "new instruction"),
        "the pending message is what runs"
    );
    assert!(
        inputs.iter().all(|i| !resumes_restored_session(i)),
        "no stale restored-session Recover may run"
    );
}

#[tokio::test]
async fn pending_probe_error_fails_closed_and_skips_replay() {
    // An unknown probe result must NOT replay the stale Run: doing so could
    // resume it and then race a newer Message once the backend recovers.
    let backend = ReplayBackend::new(
        Err(()),
        Some(interrupted_turn(
            "restored-sess",
            "prior work",
            Some("claude-code"),
        )),
    );
    let (state, recorder) =
        recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());

    recover_interrupted_run_unless_pending(&state, WorkspaceOrigin::Persisted).await;

    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "a failed probe must fail closed — no interrupted-run replay"
    );
    assert_eq!(
        backend.recover_calls(),
        0,
        "a failed probe must not drive interrupted-run recovery"
    );
}
