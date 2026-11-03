//! Session-state archive restore on startup; restore-then-replay observes
//! restoration complete before the harness accepts the resumed turn.
//!
//! `SessionImpact::Lost` recovery is proven as three links, each tested where
//! it lives: restore leaves no session on disk non-fatally (here); an
//! absent-transcript resume classifies `(UnavailableSession, Lost)`
//! (`claude`/`codex` classify tests); and a `Lost` verdict drives one fresh
//! turn, keeping the ack (`coordinator` tests).

use std::sync::Arc;

use super::backend::ReplayBackend;
use super::support::{
    combined_archive, interrupted_turn, recorded_inputs, recording_state_with_config,
    restore_state, tmp_config,
};
use crate::agent::types::ResumeSession;
use crate::init::{InitPhase, WorkspaceOrigin, open_for_work};
use crate::transcript_sync::{
    codex_sessions_dir, download_on_start, mini_swe_sessions_dir, pi_sessions_dir, transcript_dir,
};

#[tokio::test]
async fn mini_swe_shutdown_archive_restores_into_a_replacement_home() {
    let source = tempfile::TempDir::new().unwrap();
    let source_config = Arc::new(tmp_config("mini-swe", &source));
    let backend = ReplayBackend::with_archive(Vec::new());
    let state = restore_state(source_config.clone(), InitPhase::Starting, backend.clone());
    let sessions = mini_swe_sessions_dir(&source_config);
    tokio::fs::create_dir_all(&sessions).await.unwrap();
    let journal = "{\"type\":\"session\",\"id\":\"native-id\"}\n{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"remember indigo\"}}\n";
    tokio::fs::write(sessions.join("native-id.jsonl"), journal)
        .await
        .unwrap();
    tokio::fs::write(
        source_config.mini_swe_home_dir().join("auth.json"),
        "private",
    )
    .await
    .unwrap();
    crate::transcript_sync::flush_on_shutdown(&state).await;

    let replacement = tempfile::TempDir::new().unwrap();
    let config = Arc::new(tmp_config("mini-swe", &replacement));
    let state = restore_state(config.clone(), InitPhase::Starting, backend);
    download_on_start(&state).await;

    assert_eq!(
        tokio::fs::read_to_string(mini_swe_sessions_dir(&config).join("native-id.jsonl"))
            .await
            .unwrap(),
        journal
    );
    assert!(!config.mini_swe_home_dir().join("auth.json").exists());
}

#[tokio::test]
async fn download_on_start_restores_claude_codex_and_pi_session_state() {
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
    assert_eq!(
        tokio::fs::read_to_string(pi_sessions_dir(&config).join("pi-session.jsonl"))
            .await
            .unwrap(),
        "{\"type\":\"session\"}\n"
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
    // The whole point of the seam: a replaced pod restores Session state
    // to disk before the harness gets the resumed turn — otherwise the
    // agent would `--resume` against history not yet landed. Observes that
    // the restored file exists before the harness accepts the interrupted
    // run, across download_on_start (restore) → open_for_work (recover).
    for family in ["claude-code", "codex", "pi", "mini-swe"] {
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
            "mini-swe" => mini_swe_sessions_dir(&config).join("mini-session.jsonl"),
            "pi" => pi_sessions_dir(&config).join("pi-session.jsonl"),
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
        assert_eq!(
            inputs.len(),
            1,
            "family {family}: exactly the interrupted run"
        );
        // A kept volume still owes the note: every process died with the pod.
        assert!(
            inputs[0].text.starts_with("<sandbox-restart-note>")
                && inputs[0].text.ends_with("prior work"),
            "family {family}: the harness runs the restored prompt after the note: {}",
            inputs[0].text
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
