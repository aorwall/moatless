//! Uploads a tracker refuses whatever changed on disk.

use super::*;

/// A restore that failed against a snapshot that exists leaves this pod holding
/// a tree that never held the work; uploading it would replace the only copy.
#[tokio::test]
async fn a_held_upload_never_overwrites_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    let backend = CountingBackend::default();
    let tracker = WorkspaceSyncTracker::new();

    // Unheld, this tree is new to the backend and ships.
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.attempts(), 1, "an unheld tracker uploads");

    tracker.hold_uploads();
    std::fs::write(repo.join("a.txt"), "edited after the failed restore").unwrap();
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.attempts(),
        1,
        "a held tracker ships nothing, whatever changed on disk"
    );
}

/// The pre-stop RPC and the shutdown flush reach a pod mid-init. Storing the
/// tree then made a half-restored `.git` the newest snapshot, which the next
/// pod restored as a checkout `git` could not read.
#[tokio::test]
async fn nothing_uploads_until_init_has_placed_the_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "half restored").unwrap();

    let config = sync_config(tmp.path(), true, false);
    let backend = CountingBackend::default();
    let tracker = WorkspaceSyncTracker::until_placed();

    flush_workspace(&config, &backend, &tracker, true).await;
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.attempts(),
        0,
        "a tree still being placed was stored"
    );

    tracker.mark_placed();
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.attempts(), 1, "a placed tree uploads");
}
