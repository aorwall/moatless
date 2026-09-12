//! Which transitions a Task admits, by the intent it currently sits at.

use super::super::*;
use super::harness::*;

#[tokio::test]
async fn closed_task_rejects_start_restart_redeploy() {
    for op in ["start", "restart", "redeploy"] {
        let h = harness(closed_removed());
        let res = match op {
            "start" => h.lifecycle.start("t").await,
            "restart" => h.lifecycle.restart("t").await,
            _ => h.lifecycle.redeploy("t").await,
        };
        assert!(
            matches!(res, Err(LifecycleError::NotAllowed { .. })),
            "{op}"
        );
        assert!(h.log.lock().expect("lock").is_empty(), "{op}");
    }
}

#[tokio::test]
async fn error_status_start_is_resumable() {
    // Error tasks are resumable: a start succeeds.
    let h = harness(intent(
        TaskStatus::Error,
        SandboxDesiredState::Stopped,
        Some(SandboxStopReason::Error),
        None,
        None,
    ));
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
    let out = h.lifecycle.start("t").await.expect("ok");
    assert_eq!(out.observed, ObservedStatus::Starting);
    assert_eq!(
        h.store.current().expect("intent").desired_state,
        SandboxDesiredState::Running
    );
}

#[tokio::test]
async fn closed_removed_stop_preserves_removed_intent() {
    // stop is idempotent for Closed, but must not clobber Removed intent or
    // discard its retention (the archive-deletion obligation).
    let h = harness(closed_removed());
    let out = h
        .lifecycle
        .stop("t", SandboxStopReason::User)
        .await
        .expect("ok");
    assert!(out.is_pure_no_op());
    let cur = h.store.current().expect("intent");
    assert_eq!(cur.desired_state, SandboxDesiredState::Removed);
    assert_eq!(cur.retention, Some(ArchiveRetention::RetainArchives));
    // No runtime effect, no events.
    assert!(h.log.lock().expect("lock").is_empty());
    assert!(h.events.facts().is_empty());
}

#[tokio::test]
async fn closed_task_permits_remove() {
    let h = harness(closed_removed());
    h.runtime
        .remove_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Removed)));
    assert!(
        h.lifecycle
            .remove("t", ArchiveRetention::DeleteArchives)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn missing_task_is_not_found() {
    let h = harness_with(InMemoryLifecycleDao::missing(), FlakyLock::free());
    assert!(matches!(
        h.lifecycle.start("t").await,
        Err(LifecycleError::TaskNotFound(_))
    ));
}
