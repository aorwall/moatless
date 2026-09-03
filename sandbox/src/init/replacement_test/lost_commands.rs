//! The wake for `moat cmd` commands a pod replacement destroyed, proven at the
//! harness: which `Input` it actually accepts on a start whose workspace
//! snapshot carried a command journal.
//!
//! This is the stranding case. The agent starts a command, ends its turn as the
//! prompt tells it to, and the pod is evicted. The process, its log and the
//! armed watch all die with the pod, and the turn had already completed — so
//! there is no interrupted run to replay and nothing else re-triggers the agent.

use std::sync::Arc;

use super::backend::ReplayBackend;
use super::support::{recorded_inputs, recording_state_with_config, settled_inputs, tmp_config};
use crate::commands::{default_journal_path, CommandState, JournaledCommand};
use crate::init::{open_for_work, InitPhase, WorkspaceOrigin};

fn journalled(id: &str, label: &str, state: CommandState) -> JournaledCommand {
    JournaledCommand {
        id: id.to_string(),
        command: "bash /tmp/verify.sh".to_string(),
        label: label.to_string(),
        started_at_unix_ms: 1,
        state,
        exit_code: None,
    }
}

/// Write `commands` where a restored workspace snapshot would have put them.
async fn seed_journal(workspace_dir: &str, commands: &[JournaledCommand]) {
    tokio::fs::create_dir_all(workspace_dir)
        .await
        .expect("workspace dir");
    tokio::fs::write(
        default_journal_path(workspace_dir),
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
    seed_journal(
        &workspace_dir,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;

    // No pending message and no interrupted run: the turn that started the
    // command had already completed, so nothing else would re-trigger the agent.
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);

    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;

    let inputs = recorded_inputs(&recorder, 1).await;
    assert_eq!(inputs.len(), 1, "the lost command must open exactly one turn");
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
    seed_journal(
        &workspace_dir,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;

    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);
    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;
    assert_eq!(recorded_inputs(&recorder, 1).await.len(), 1);

    assert!(
        !default_journal_path(&workspace_dir).exists(),
        "the journal must be cleared as it is read"
    );

    // A second start over the same workspace: nothing is owed any more.
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let config = tmp_config("claude-code", &tmp);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);
    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;
    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "a restart owing nothing must not open a turn"
    );
}

/// Consuming the journal deletes it here but not from the S3 archive this pod
/// restored, so the tree is already ahead of the object. Adopting it as the
/// upload baseline would let a turn that edits nothing skip the upload, and the
/// next replacement would restore the stale archive and announce the same loss
/// again — breaking the once-only guarantee across a second restart.
#[tokio::test]
async fn consuming_a_journal_leaves_the_workspace_owing_an_upload() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let mut config = tmp_config("claude-code", &tmp);
    config.workspace_sync_enabled = true;
    let workspace_dir = config.workspace_dir.clone();
    seed_journal(
        &workspace_dir,
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

    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;

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

    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;

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

    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;

    assert!(settled_inputs(&recorder).await.is_empty());
}

/// The lost-command wake is additional to the pending message, not a
/// replacement for it: the agent has to hear about both.
#[tokio::test]
async fn a_pending_message_does_not_swallow_the_lost_command_wake() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    let workspace_dir = config.workspace_dir.clone();
    seed_journal(
        &workspace_dir,
        &[journalled("c33e9d6e", "verify", CommandState::Running)],
    )
    .await;

    let backend = ReplayBackend::new(
        Ok(vec![super::support::text_message("status?", "uuid-1")]),
        None,
    );
    let (state, recorder) =
        recording_state_with_config(config, InitPhase::Starting, Arc::clone(&backend) as _);

    open_for_work(&state, WorkspaceOrigin::RestoredFromSnapshot).await;

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
