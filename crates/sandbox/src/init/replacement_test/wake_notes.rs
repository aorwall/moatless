//! The wake for a caller that took the pod down, proven at the harness: which
//! `Input` it actually accepts on a start whose workspace snapshot carried a
//! wake note.
//!
//! This is the self-restart case. The agent runs `moat tasks restart --yes`,
//! and the pod dies with the request in flight — so the command reports
//! nothing, the turn ends unfinished, and only the note says what happened.

use super::backend::ReplayBackend;
use super::support::{recorded_inputs, recording_state_with_config, settled_inputs, tmp_config};
use crate::agent::wake_note::{self, WakeNote};
use crate::commands::{CommandState, JournaledCommand, default_journal_path};
use crate::init::{InitPhase, WorkspaceOrigin, open_for_work};

/// Write a note where a restored workspace snapshot would have put it.
async fn seed_note(workspace_dir: &str, task_id: &str, text: &str) {
    tokio::fs::create_dir_all(workspace_dir)
        .await
        .expect("workspace dir");
    let notes = std::collections::BTreeMap::from([(
        task_id.to_string(),
        WakeNote {
            text: text.to_string(),
            // A real note is minutes old at most, and `take` drops one that is
            // older than its bound.
            recorded_at_unix_ms: chrono::Utc::now().timestamp_millis(),
        },
    )]);
    tokio::fs::write(
        wake_note::default_path(workspace_dir),
        serde_json::to_vec(&notes).expect("encode"),
    )
    .await
    .expect("seed note");
}

#[tokio::test]
async fn a_recorded_note_wakes_the_agent() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    seed_note(&config.workspace_dir, &config.task_id, "you restarted me").await;

    // No pending message and no interrupted run: the note is the only thing
    // that would open a turn.
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);

    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;

    let inputs = recorded_inputs(&recorder, 1).await;
    assert_eq!(inputs.len(), 1, "the note must open exactly one turn");
    assert!(
        inputs[0].text.contains("you restarted me"),
        "the caller's own wording must reach the agent: {:?}",
        inputs[0].text
    );
    assert!(
        inputs[0].text.contains("<sandbox-restart-note>"),
        "the note travels in the tag every restart wake uses: {:?}",
        inputs[0].text
    );
}

/// The note is cleared as it is read, so an unrelated later replacement does
/// not announce a restart nobody asked for.
#[tokio::test]
async fn the_wake_fires_once_across_repeated_restarts() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    let workspace_dir = config.workspace_dir.clone();
    seed_note(&workspace_dir, &config.task_id, "you restarted me").await;

    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);
    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;
    assert_eq!(recorded_inputs(&recorder, 1).await.len(), 1);

    assert!(
        !wake_note::default_path(&workspace_dir).exists(),
        "the note must be cleared as it is read"
    );

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

/// One restart opens one turn: an agent that restarted its own pod with a
/// command still running hears both facts at once.
#[tokio::test]
async fn a_note_and_a_lost_command_open_one_turn() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let config = tmp_config("claude-code", &tmp);
    let workspace_dir = config.workspace_dir.clone();
    seed_note(&workspace_dir, &config.task_id, "you restarted me").await;
    tokio::fs::write(
        default_journal_path(&workspace_dir, &config.task_id),
        serde_json::to_vec(&[JournaledCommand {
            id: "c33e9d6e".to_string(),
            command: "bash /tmp/verify.sh".to_string(),
            label: "verify".to_string(),
            started_at_unix_ms: 1,
            state: CommandState::Running,
            exit_code: None,
            log_tail: None,
        }])
        .expect("encode"),
    )
    .await
    .expect("seed journal");

    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state_with_config(config, InitPhase::Starting, backend);

    open_for_work(
        &state,
        WorkspaceOrigin::RestoredFromSnapshot { taken_at: None },
    )
    .await;

    let inputs = recorded_inputs(&recorder, 1).await;
    assert_eq!(inputs.len(), 1, "two facts, one turn: {inputs:?}");
    assert!(inputs[0].text.contains("you restarted me"), "{inputs:?}");
    assert!(inputs[0].text.contains("c33e9d6e"), "{inputs:?}");
}

/// Taking the note deletes it here while the S3 archive still holds it, so the
/// tree is ahead of the object. Adopting it would let the next replacement
/// restore the stale archive and deliver the same wake again.
#[tokio::test]
async fn consuming_a_note_leaves_the_workspace_owing_an_upload() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let mut config = tmp_config("claude-code", &tmp);
    config.workspace_sync_enabled = true;
    let workspace_dir = config.workspace_dir.clone();
    seed_note(&workspace_dir, &config.task_id, "you restarted me").await;
    // A loose file the note's removal is measured against: an otherwise empty
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
