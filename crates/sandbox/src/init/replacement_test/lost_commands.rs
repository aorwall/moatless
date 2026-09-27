//! The wake for `moat cmd` commands a pod replacement destroyed, proven at the
//! harness: which `Input` it actually accepts on a start whose workspace
//! snapshot carried a command journal.
//!
//! This is the stranding case: the agent starts a command, ends its turn as the
//! prompt tells it to, and the pod is evicted. The process, its log and the
//! armed watch die with it, and the completed turn leaves no interrupted run to
//! replay and nothing else to re-trigger the agent.

use std::sync::Arc;

use super::backend::ReplayBackend;
use super::support::{
    recorded_inputs, recording_state_gated, recording_state_with_config, settled_inputs, tmp_config,
};
use crate::commands::{CommandState, JournaledCommand, default_journal_path};
use crate::init::{InitPhase, SetupGate, WorkspaceOrigin, open_for_work};

fn journalled(id: &str, label: &str, state: CommandState) -> JournaledCommand {
    JournaledCommand {
        id: id.to_string(),
        command: "bash /tmp/verify.sh".to_string(),
        label: label.to_string(),
        started_at_unix_ms: 1,
        state,
        exit_code: None,
        log_tail: None,
    }
}

/// Write `commands` where a restored workspace snapshot would have put them.
async fn seed_journal(workspace_dir: &str, task_id: &str, commands: &[JournaledCommand]) {
    tokio::fs::create_dir_all(workspace_dir)
        .await
        .expect("workspace dir");
    tokio::fs::write(
        default_journal_path(workspace_dir, task_id),
        serde_json::to_vec(commands).expect("encode"),
    )
    .await
    .expect("seed journal");
}

#[tokio::test]
async fn a_command_lost_to_the_restart_wakes_the_agent() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    let workspace_dir = config.workspace_dir.clone();
    let task_id = config.task_id.clone();
    seed_journal(
        &workspace_dir,
        &task_id,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;

    // No pending message and no interrupted run: the turn that started the
    // command had already completed, so nothing else would re-trigger the agent.
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);

    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;

    let inputs = recorded_inputs(&recorder, 1).await;
    assert_eq!(
        inputs.len(),
        1,
        "the lost command must open exactly one turn"
    );
    assert!(
        inputs[0].text.contains("c33e9d6e"),
        "the wake must name the command: {:?}",
        inputs[0].text
    );
    assert!(
        inputs[0].text.contains("did not finish"),
        "a command cut off mid-flight is unfinished: {:?}",
        inputs[0].text
    );
}

/// The journal is cleared as it is read, so a pod that restarts again does not
/// re-announce a loss the agent has already been told about.
#[tokio::test]
async fn the_wake_fires_once_across_repeated_restarts() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    let workspace_dir = config.workspace_dir.clone();
    let task_id = config.task_id.clone();
    seed_journal(
        &workspace_dir,
        &task_id,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;

    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);
    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;
    assert_eq!(recorded_inputs(&recorder, 1).await.len(), 1);

    assert!(
        !default_journal_path(&workspace_dir, &task_id).exists(),
        "the journal must be cleared as it is read"
    );

    // A second start over the same workspace: nothing is owed any more.
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let config = tmp_config("claude-code", &tmp);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);
    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;
    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "a restart owing nothing must not open a turn"
    );
}

/// Consuming the journal deletes it locally but not from the restored S3
/// archive, leaving the tree ahead of the object.
///
/// Adopting it as the upload baseline would skip the upload on a no-op turn, so
/// a later replacement restores the stale archive and repeats the announcement.
#[tokio::test]
async fn consuming_a_journal_leaves_the_workspace_owing_an_upload() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let mut config = tmp_config("claude-code", &tmp);
    config.workspace_sync_enabled = true;
    let workspace_dir = config.workspace_dir.clone();
    let task_id = config.task_id.clone();
    seed_journal(
        &workspace_dir,
        &task_id,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;
    // A loose file the journal's removal is measured against: an otherwise empty
    // workspace fingerprints as `None` and the assertion would hold vacuously.
    tokio::fs::write(std::path::Path::new(&workspace_dir).join("notes.txt"), b"x")
        .await
        .expect("loose file");

    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, _recorder) = recording_state_with_config(config, InitPhase::Starting, backend);

    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;

    assert!(
        state.workspace_sync.baseline().await.is_none(),
        "the tree is ahead of the S3 archive, so the next upload must not be skipped"
    );
}

/// The ordinary restart still adopts: a workspace nobody touched is not worth
/// shipping back, and gating that on the journal must not cost every start an
/// upload.
#[tokio::test]
async fn a_restart_with_no_journal_still_adopts_the_upload_baseline() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let mut config = tmp_config("claude-code", &tmp);
    config.workspace_sync_enabled = true;
    let workspace_dir = config.workspace_dir.clone();
    tokio::fs::create_dir_all(&workspace_dir)
        .await
        .expect("workspace dir");
    tokio::fs::write(std::path::Path::new(&workspace_dir).join("notes.txt"), b"x")
        .await
        .expect("loose file");

    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, _recorder) = recording_state_with_config(config, InitPhase::Starting, backend);

    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;

    assert!(
        state.workspace_sync.baseline().await.is_some(),
        "an untouched restore has nothing new to ship"
    );
}

/// A restart with no commands outstanding is the common case, and it must not
/// spend a turn saying so.
#[tokio::test]
async fn a_restart_with_no_journal_opens_no_turn() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    tokio::fs::create_dir_all(&config.workspace_dir)
        .await
        .expect("workspace dir");

    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);

    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;

    assert!(settled_inputs(&recorder).await.is_empty());
}

/// The lost-command wake is additional to the pending message, not a
/// replacement for it: the agent has to hear about both.
#[tokio::test]
async fn a_pending_message_does_not_swallow_the_lost_command_wake() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    let workspace_dir = config.workspace_dir.clone();
    let task_id = config.task_id.clone();
    seed_journal(
        &workspace_dir,
        &task_id,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;

    let backend = ReplayBackend::new(
        Ok(vec![super::support::text_message("status?", "uuid-1")]),
        None,
    );
    let (state, recorder) =
        recording_state_with_config(config, InitPhase::Starting, Arc::clone(&backend) as _);

    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;

    let inputs = recorded_inputs(&recorder, 2).await;
    assert_eq!(inputs.len(), 2, "both the message and the wake must run");
    assert!(
        inputs.iter().any(|i| i.text.contains("status?")),
        "the user's message must still run: {inputs:?}"
    );
    assert!(
        inputs.iter().any(|i| i.text.contains("c33e9d6e")),
        "the lost command must still be reported: {inputs:?}"
    );
}

/// Workspace setup runs after `open_for_work` returns, and the harness accepts
/// no turn until it has. The wake must not wait on the pending message's turn,
/// or setup never starts and neither turn runs.
#[tokio::test]
async fn the_lost_command_wake_does_not_hold_up_workspace_setup() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    let workspace_dir = config.workspace_dir.clone();
    let task_id = config.task_id.clone();
    seed_journal(
        &workspace_dir,
        &task_id,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;

    let backend = ReplayBackend::new(
        Ok(vec![super::support::text_message("status?", "uuid-1")]),
        None,
    );
    let setup_gate = SetupGate::closed();
    let (state, recorder) = recording_state_gated(
        config,
        InitPhase::Starting,
        Arc::clone(&backend) as _,
        setup_gate.clone(),
    );

    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        open_for_work(
            &state,
            WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
        ),
    )
    .await
    .expect("open_for_work must return while setup is still pending");

    setup_gate.open();

    let inputs = recorded_inputs(&recorder, 2).await;
    assert_eq!(inputs.len(), 2, "both the message and the wake must run");
    assert!(inputs[0].text.contains("status?"), "{inputs:?}");
    assert!(inputs[1].text.contains("c33e9d6e"), "{inputs:?}");
}
