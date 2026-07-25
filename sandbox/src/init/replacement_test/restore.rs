//! Session-state archive restore on startup (`transcript_sync::download_on_start`),
//! and the restore-then-replay lifecycle that observes restoration completing
//! before the harness accepts the resumed turn.
//!
//! Scope note on `SessionImpact::Lost`: the missing/corrupt-archive recovery is
//! proven as a chain of three deterministic links, each tested where it lives:
//!   1. restore leaves no session on disk, non-fatally — the tests below;
//!   2. a resume whose transcript is absent classifies as
//!      `(UnavailableSession, Lost)` — `claude::tests::resume_startup_failure_is_
//!      unavailable_session_and_lost` (and the Codex equivalent in `codex::classify`);
//!   3. a `Lost` verdict drives one fresh same-family turn and keeps the ack —
//!      `coordinator::tests::lost_resume_recovers_with_a_fresh_turn_and_keeps_the_ack`.
//!
//! Link 2 is deterministic: the Claude Session module checks the restored
//! transcript before spawning the CLI, while the native `TransportClosed` resume
//! path is classified separately. Real CLI launch compatibility remains in the
//! small transport smoke suite; it is not needed to prove missing-file policy.

use std::sync::Arc;

use super::backend::ReplayBackend;
use super::support::{
    combined_archive, interrupted_turn, recorded_inputs, recording_state_with_config,
    restore_state, tmp_config,
};
use crate::agent::types::ResumeSession;
use crate::init::transcript_sync::{codex_sessions_dir, download_on_start, transcript_dir};
use crate::init::{open_for_work, InitPhase, WorkspaceOrigin};

#[tokio::test]
async fn download_on_start_restores_claude_and_codex_session_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    tokio::fs::create_dir_all(tmp.path().join("workspace"))
        .await
        .unwrap();
    let config = Arc::new(tmp_config("claude-code", &tmp));

    let backend = ReplayBackend::with_archive(combined_archive().await);
    let state = restore_state(Arc::clone(&config), InitPhase::Starting, backend);

    download_on_start(&state).await;

    let claude_dst = transcript_dir(&config);
    let codex_dst = codex_sessions_dir(&config);
    assert_eq!(
        tokio::fs::read_to_string(claude_dst.join("session.jsonl"))
            .await
            .unwrap(),
        "{\"turn\":1}\n"
    );
    assert_eq!(
        tokio::fs::read_to_string(codex_dst.join("2026").join("rollout-xyz.jsonl"))
            .await
            .unwrap(),
        "{\"thread\":\"xyz\"}\n"
    );
}

#[tokio::test]
async fn download_on_start_is_a_noop_when_no_archive_is_stored() {
    // A missing archive (fresh task, or S3 empty) restores nothing but still
    // creates the dirs, and must not fail — the CLI then legitimately has no
    // session to resume.
    let tmp = tempfile::TempDir::new().unwrap();
    tokio::fs::create_dir_all(tmp.path().join("workspace"))
        .await
        .unwrap();
    let config = Arc::new(tmp_config("codex", &tmp));

    let backend = ReplayBackend::with_archive(Vec::new());
    let state = restore_state(Arc::clone(&config), InitPhase::Starting, backend);

    download_on_start(&state).await;

    let codex_dst = codex_sessions_dir(&config);
    let mut entries = tokio::fs::read_dir(&codex_dst).await.unwrap();
    assert!(
        entries.next_entry().await.unwrap().is_none(),
        "no archive → nothing restored"
    );
}

#[tokio::test]
async fn download_on_start_survives_a_corrupt_archive() {
    // A corrupt archive must be non-fatal and restore nothing, so a subsequent
    // `--resume` hits a genuinely missing session (whereupon the coordinator's
    // SessionImpact::Lost recovery takes over — see the coordinator tests).
    let tmp = tempfile::TempDir::new().unwrap();
    tokio::fs::create_dir_all(tmp.path().join("workspace"))
        .await
        .unwrap();
    let config = Arc::new(tmp_config("claude-code", &tmp));

    let backend = ReplayBackend::with_archive(b"not a tar archive at all".to_vec());
    let state = restore_state(Arc::clone(&config), InitPhase::Starting, backend);

    // Must not panic.
    download_on_start(&state).await;

    let claude_dst = transcript_dir(&config);
    let mut entries = tokio::fs::read_dir(&claude_dst).await.unwrap();
    assert!(
        entries.next_entry().await.unwrap().is_none(),
        "a corrupt archive must restore nothing"
    );
}

#[tokio::test]
async fn restore_precedes_the_harness_accepting_the_resumed_turn() {
    // The whole point of the seam: a replaced pod restores Session state to disk
    // *before* the harness is handed the resumed turn — otherwise the agent would
    // `--resume` against history that has not landed yet. This exercises one
    // lifecycle: download_on_start (restore) → open_for_work (recover), and
    // observes that the restored session file is present on disk *before* the
    // harness accepts the interrupted run.
    for family in ["claude-code", "codex"] {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("workspace"))
            .await
            .unwrap();
        let config = tmp_config(family, &tmp);

        let backend = ReplayBackend::with_archive_and_interrupted(
            combined_archive().await,
            interrupted_turn("restored-sess", "prior work", Some(family)),
        );
        let (state, recorder) =
            recording_state_with_config(config.clone(), InitPhase::Starting, backend.clone());

        // Restore first, and confirm the Session state actually landed on disk.
        download_on_start(&state).await;
        let restored = match family {
            "claude-code" => transcript_dir(&config).join("session.jsonl"),
            _ => codex_sessions_dir(&config)
                .join("2026")
                .join("rollout-xyz.jsonl"),
        };
        assert!(
            tokio::fs::try_exists(&restored).await.unwrap(),
            "family {family}: restore must complete before the turn is dispatched"
        );

        // Only now open for work: the interrupted run replays into the harness.
        open_for_work(&state, WorkspaceOrigin::Persisted).await;

        let inputs = recorded_inputs(&recorder, 1).await;
        assert_eq!(inputs.len(), 1, "family {family}: exactly the interrupted run");
        assert_eq!(
            inputs[0].text, "prior work",
            "family {family}: the harness runs the restored prompt"
        );
        assert!(
            matches!(&inputs[0].resume, Some(ResumeSession::WithId(id)) if id == "restored-sess"),
            "family {family}: the harness resumes the restored session"
        );
        // The restored file is still present after acceptance — restoration was a
        // precondition of the turn, not a race against it.
        assert!(
            tokio::fs::try_exists(&restored).await.unwrap(),
            "family {family}: restored session state persists across acceptance"
        );
        assert_eq!(
            backend.recover_calls(),
            1,
            "family {family}: the interrupted run is probed exactly once"
        );
    }
}
