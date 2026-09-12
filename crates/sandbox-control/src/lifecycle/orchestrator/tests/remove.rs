//! Remove: what it accepts, what retention it keeps, what a recovery reuses,
//! and the generation an error id is scoped to.

use super::super::*;
use super::harness::*;
use moatless_sandbox_runtime::runtime::RuntimeError;
use std::sync::Mutex;

#[tokio::test]
async fn successful_remove_emits_removing_then_removed_and_commits() {
    let h = harness(open_running());
    h.runtime
        .remove_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
    h.lifecycle
        .remove("t", ArchiveRetention::DeleteArchives)
        .await
        .expect("ok");
    assert_eq!(
        h.events.facts(),
        vec![EventFact::Removing, EventFact::Removed]
    );
    assert_eq!(
        h.store.current().expect("intent").retention,
        Some(ArchiveRetention::DeleteArchives)
    );
    assert_eq!(h.store.commits(), vec!["sandbox:t:1:removed".to_string()]);
}

#[tokio::test]
async fn failed_remove_keeps_removed_intent_and_publishes_error() {
    let h = harness(open_running());
    h.runtime
        .remove_res
        .lock()
        .expect("lock")
        .replace(Err(RuntimeError::NotAbsent("pvc".into())));
    let res = h
        .lifecycle
        .remove("t", ArchiveRetention::DeleteArchives)
        .await;
    assert!(matches!(res, Err(LifecycleError::Runtime { .. })));
    let cur = h.store.current().expect("intent");
    assert_eq!(cur.desired_state, SandboxDesiredState::Removed);
    assert_eq!(cur.retention, Some(ArchiveRetention::DeleteArchives));
    let facts = h.events.facts();
    assert!(facts.contains(&EventFact::Removing));
    assert!(facts.contains(&EventFact::Error));
    assert!(!facts.contains(&EventFact::Removed));
    assert!(h.store.commits().is_empty());
}

#[tokio::test]
async fn remove_recovery_reuses_generation_and_persisted_retention() {
    // A prior remove persisted Removed + RetainArchives with an incomplete
    // Remove marker at generation 5. The recovery retry reuses generation 5
    // (stable deterministic id) and the persisted retention — it does not
    // allocate a fresh generation nor take the caller's DeleteArchives.
    let prior_marker =
        CorrelationMarker::new(OperationKind::Remove, OperationGeneration(5), fixed_now());
    let store = Arc::new(InMemoryLifecycleDao {
        intent: Mutex::new(Some(intent(
            TaskStatus::Open,
            SandboxDesiredState::Removed,
            Some(SandboxStopReason::Cleanup),
            Some(ArchiveRetention::RetainArchives),
            Some(prior_marker),
        ))),
        next_gen: Mutex::new(100), // would be used only if it allocated afresh
        commits: Mutex::new(vec![]),
        marker_cleared: Mutex::new(false),
        storage_reclaimed: Mutex::new(false),
        other_active_members: Mutex::new(false),
        fail_persist: false,
        fail_commit: false,
    });
    let h = harness_with(store, FlakyLock::free());
    h.runtime
        .remove_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
    // Caller passes DeleteArchives, but recovery must keep RetainArchives.
    h.lifecycle
        .remove("t", ArchiveRetention::DeleteArchives)
        .await
        .expect("ok");
    // Deterministic id reuses generation 5, not 100.
    assert_eq!(h.store.commits(), vec!["sandbox:t:5:removed".to_string()]);
    assert_eq!(
        h.store.current().expect("intent").retention,
        Some(ArchiveRetention::RetainArchives)
    );
}

#[tokio::test]
async fn idempotent_re_remove_noop_clears_marker_synchronously() {
    // finish path for remove must have a pure-no-op clear: an Unchanged +
    // NoOp remove against an already-removed target (fresh marker, not a
    // recovery) would otherwise strand its marker forever.
    let h = harness(intent(
        TaskStatus::Open,
        SandboxDesiredState::Removed,
        Some(SandboxStopReason::Cleanup),
        Some(ArchiveRetention::DeleteArchives),
        None, // no prior marker → not a recovery; a fresh no-op op
    ));
    h.runtime
        .remove_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Removed)));
    let out = h
        .lifecycle
        .remove("t", ArchiveRetention::DeleteArchives)
        .await
        .expect("ok");
    assert!(out.is_pure_no_op());
    // No completion fact owed; marker cleared synchronously, none stranded.
    assert!(h.store.marker_cleared());
    assert!(h.store.current().expect("intent").marker.is_none());
    assert!(h.store.commits().is_empty());
}

// ---- Deterministic event ids ----

#[tokio::test]
async fn error_event_id_is_generation_scoped() {
    let h = harness(open_stopped());
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Err(RuntimeError::Infrastructure("x".into())));
    let _ = h.lifecycle.start("t").await;
    assert_eq!(h.events.ids(), vec!["sandbox:t:1:error".to_string()]);
}
