//! Reclaiming a stopped Task's volume: when the archive is new enough to skip
//! the rescue, and when the volume is kept instead.

use super::super::*;
use super::harness::*;

/// Attach an archive port to the harness's lifecycle without rebuilding it.
fn with_archive(h: &Harness, archive: Arc<dyn WorkspaceArchivePort>) -> Lifecycle {
    h.lifecycle.clone().with_workspace_archive(archive)
}

#[tokio::test]
async fn archive_newer_than_the_stop_reclaims_without_a_rescue() {
    let h = harness(open_stopped());
    h.runtime
        .reclaim_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
    let archive = RecordingArchive::heading(
        h.log.clone(),
        Ok(Some(fixed_now() + chrono::Duration::seconds(60))),
    );
    let out = with_archive(&h, archive)
        .archive_storage("t")
        .await
        .expect("reclaims");

    assert_eq!(out.runtime_effect, RuntimeEffect::Applied);
    assert_eq!(out.intent_effect, IntentEffect::Unchanged);
    // Recorded, so the next cycle does not re-head S3 and re-ask Kubernetes
    // about a task whose storage is already gone.
    assert!(h.store.storage_reclaimed());
    assert_eq!(
        h.log.lock().expect("lock").as_slice(),
        ["archived_at", "runtime_reclaim_storage:proven"]
    );
    // The volume goes, but nothing about the Task's intent moves and no
    // event is owed: a cold sandbox is still a stopped one.
    let current = h.store.current().expect("intent");
    assert_eq!(current.desired_state, SandboxDesiredState::Stopped);
    assert!(h.events.facts().is_empty());
}

#[tokio::test]
async fn stale_or_absent_archive_rescues_the_volume_first() {
    for head in [
        Ok(Some(fixed_now() - chrono::Duration::seconds(60))),
        Ok(None),
    ] {
        let h = harness(open_stopped());
        let archive = RecordingArchive::heading(h.log.clone(), head);
        with_archive(&h, archive)
            .archive_storage("t")
            .await
            .expect("reclaims after the rescue");

        assert_eq!(
            h.log.lock().expect("lock").as_slice(),
            [
                "archived_at",
                "presign_upload",
                "runtime_reclaim_storage:rescue"
            ]
        );
        assert_eq!(
            h.runtime.last_rescue_url.lock().expect("lock").as_deref(),
            Some(RESCUE_URL)
        );
    }
}

#[tokio::test]
async fn unreadable_archive_keeps_the_volume() {
    // "Cannot tell" is not "not there": a head failure must never become a
    // deletion.
    let h = harness(open_stopped());
    let archive = RecordingArchive::heading(h.log.clone(), Err("s3 timeout".into()));
    let err = with_archive(&h, archive)
        .archive_storage("t")
        .await
        .expect_err("blocked");

    assert!(matches!(err, LifecycleError::ArchiveUnproven { .. }));
    assert!(err.is_retriable(), "the next cycle must try again");
    assert_eq!(h.log.lock().expect("lock").as_slice(), ["archived_at"]);
    assert!(
        !h.store.storage_reclaimed(),
        "a blocked transition must stay a candidate"
    );
}

#[tokio::test]
async fn unpresignable_rescue_keeps_the_volume() {
    let h = harness(open_stopped());
    let archive = RecordingArchive::failing_presign(h.log.clone());
    let err = with_archive(&h, archive)
        .archive_storage("t")
        .await
        .expect_err("blocked");

    assert!(matches!(err, LifecycleError::ArchiveUnproven { .. }));
    assert_eq!(
        h.log.lock().expect("lock").as_slice(),
        ["archived_at", "presign_upload"]
    );
}

#[tokio::test]
async fn a_restarted_task_is_not_reclaimed() {
    // The caller decided this Task was cold from a pre-lock snapshot; the
    // intent re-read under the lock is what actually protects a live volume.
    let h = harness(open_running());
    let archive = RecordingArchive::heading(h.log.clone(), Ok(Some(fixed_now())));
    let out = with_archive(&h, archive)
        .archive_storage("t")
        .await
        .expect("skips");

    assert!(out.is_pure_no_op());
    assert!(h.log.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn without_an_archive_port_nothing_is_reclaimed() {
    // No S3 means the volume is the only copy — there is nowhere to cool to.
    let h = harness(open_stopped());
    let out = h.lifecycle.archive_storage("t").await.expect("skips");

    assert!(out.is_pure_no_op());
    assert!(h.log.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn an_already_cold_task_reports_no_op() {
    let h = harness(open_stopped());
    h.runtime
        .reclaim_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Stopped)));
    let archive = RecordingArchive::heading(
        h.log.clone(),
        Ok(Some(fixed_now() + chrono::Duration::seconds(60))),
    );
    let out = with_archive(&h, archive)
        .archive_storage("t")
        .await
        .expect("idempotent");

    assert!(out.is_pure_no_op());
}
