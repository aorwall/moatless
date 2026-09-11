//! Interrupted-Run recovery for a Member that is not the one the pod booted
//! with, proven at the backend RPC (which Task the pod asks to recover) and at
//! the harness (which checkout the replayed prompt's restart note names).
//!
//! This is the stranding case for a shared sandbox. A pod hosting two Members is
//! replaced mid-turn. The reconciler re-issues `JoinTask` for the Member that
//! joined the old pod, so the Member is registered again — but only the booting
//! Member's Run used to be replayed, so the joined Member sat idle until the
//! backend's reaper closed its turn as `reconciled_orphaned_turn` and a person
//! prodded it back to work.

use std::sync::Arc;

use super::backend::ReplayBackend;
use super::support::{
    interrupted_turn, recorded_inputs, recording_state, recording_state_with_config, tmp_config,
};
use crate::init::{
    InitPhase, WorkspaceOrigin, open_for_work, recover_interrupted_run_unless_pending,
};

/// A state whose agent spawns are inert: `compose_task_context` builds a real
/// agent for a joining Member, and no `AGENT_TYPE` makes that a poison task
/// rather than an attempt to exec a CLI this test has no use for. The family
/// `tmp_config` requires is cleared straight after, and only its tmp dirs are
/// kept — `materialize_checkout` writes under `workspace_dir`.
fn joinable_state(
    tmp: &tempfile::TempDir,
    phase: InitPhase,
    backend: Arc<ReplayBackend>,
) -> (crate::state::SandboxState, String) {
    let mut config = tmp_config("claude-code", tmp);
    config.agent_type = None;
    let boot_task_id = config.task_id.clone();
    let (state, _recorder) = recording_state_with_config(config, phase, backend);
    (state, boot_task_id)
}

#[tokio::test]
async fn a_member_joining_a_ready_pod_recovers_its_own_interrupted_run() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let backend = ReplayBackend::new(
        Ok(vec![]),
        Some(interrupted_turn("restored-sess", "prior work", None)),
    );
    let (state, _boot) = joinable_state(&tmp, InitPhase::Ready, Arc::clone(&backend));

    state
        .register_task_context("joined-task".to_string(), Some("shared"))
        .await
        .expect("join");

    assert_eq!(
        backend.recovered_task_ids(),
        vec!["joined-task".to_string()],
        "the joining Member's own Run must be the one recovered"
    );
}

/// A pending Message still supersedes the Run for a joining Member, exactly as
/// it does for the booting one — the pod must not replay a Run the person has
/// already moved past.
#[tokio::test]
async fn a_pending_message_supersedes_a_joining_members_interrupted_run() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let backend = ReplayBackend::new(
        Ok(vec![super::support::text_message("new instruction", "u-1")]),
        Some(interrupted_turn("restored-sess", "prior work", None)),
    );
    let (state, _boot) = joinable_state(&tmp, InitPhase::Ready, Arc::clone(&backend));

    state
        .register_task_context("joined-task".to_string(), Some("shared"))
        .await
        .expect("join");

    assert!(
        backend.recovered_task_ids().is_empty(),
        "a pending message must suppress the joining Member's replay too"
    );
}

/// The handoff between the two recovery sites: a Member that joins before Ready
/// is recovered by `open_for_work`'s pass and not by its own registration, so
/// its Run is replayed exactly once.
#[tokio::test]
async fn a_member_joining_before_ready_is_recovered_once_by_init() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let backend = ReplayBackend::new(
        Ok(vec![]),
        Some(interrupted_turn("restored-sess", "prior work", None)),
    );
    let (state, boot) = joinable_state(&tmp, InitPhase::Starting, Arc::clone(&backend));

    state
        .register_task_context("joined-task".to_string(), Some("shared"))
        .await
        .expect("join");
    assert!(
        backend.recovered_task_ids().is_empty(),
        "registering before Ready must leave recovery to init"
    );

    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;

    let mut recovered = backend.recovered_task_ids();
    recovered.sort();
    let mut expected = vec!["joined-task".to_string(), boot];
    expected.sort();
    assert_eq!(
        recovered, expected,
        "init must recover every registered Member, each exactly once"
    );
    assert_eq!(
        *state.workspace_origin.read().await,
        WorkspaceOrigin::RestoredFromSnapshot,
        "init must publish the origin for a Member that joins after Ready"
    );
}

/// The restart note a replayed Run carries names the recovering Member's own
/// checkout. `config.repo_dir` names the Member the pod booted with, so a note
/// keyed on it sends a joined Member's agent to a tree it is not working in.
#[tokio::test]
async fn a_joined_members_restart_note_names_its_own_checkout() {
    const JOINED_CHECKOUT: &str = "/tmp/workspace/joined-member";
    let backend = ReplayBackend::new(
        Ok(vec![]),
        Some(interrupted_turn(
            "restored-sess",
            "prior work",
            Some("claude-code"),
        )),
    );
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend);

    // The boot Member's context carries the recording handle, so cloning it and
    // renaming only what a join changes reads the joined Member's prompt back.
    let mut joined = (*state.task_context(None).await.expect("boot member")).clone();
    joined.task_id = "joined-task".to_string();
    joined.checkout_dir = JOINED_CHECKOUT.to_string();

    recover_interrupted_run_unless_pending(&state, &joined, WorkspaceOrigin::RestoredFromSnapshot)
        .await;

    let inputs = recorded_inputs(&recorder, 1).await;
    assert_eq!(inputs.len(), 1, "the joined Member's run is replayed");
    assert!(
        inputs[0].text.contains(JOINED_CHECKOUT),
        "the restart note must name the joined Member's checkout: {}",
        inputs[0].text
    );
    assert!(
        !inputs[0].text.contains(&state.config.repo_dir),
        "the restart note must not name the boot Member's checkout: {}",
        inputs[0].text
    );
}
