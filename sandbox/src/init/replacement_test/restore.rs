//! Session-state archive restore on startup (`transcript_sync::download_on_start`).
//!
//! Scope note: these prove the restore step's file behaviour — happy-path
//! extraction, and non-fatal no-op on a missing or corrupt archive. They do NOT
//! assert the downstream `SessionImpact::Lost` classification: when restore
//! leaves no session on disk, the CLI's later `--resume` is what surfaces the
//! unavailable session, and the coordinator's classified `session_context_lost`
//! recovery (same harness family, no switch) is covered by the coordinator tests
//! (`lost_resume_recovers_with_a_fresh_turn_and_keeps_the_ack`).

use std::sync::Arc;

use super::support::{combined_archive, restore_state, tmp_config, ReplayBackend};
use crate::init::transcript_sync::{codex_sessions_dir, download_on_start, transcript_dir};
use crate::init::InitPhase;

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
