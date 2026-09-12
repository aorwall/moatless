//! Stop: the completion commit that precedes publication, and the converged
//! no-op that publishes anyway.

use super::super::*;
use super::harness::*;
use std::sync::Mutex;

#[tokio::test]
async fn stop_against_running_publishes_stopped_and_commits_completion() {
    let h = harness(open_running());
    h.runtime
        .stop_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
    h.lifecycle
        .stop("t", SandboxStopReason::IdleReaper)
        .await
        .expect("ok");
    assert!(h.events.facts().contains(&EventFact::Stopped));
    assert_eq!(h.store.commits(), vec!["sandbox:t:1:stopped".to_string()]);
    assert!(h.store.current().expect("intent").marker.is_none());
}

#[tokio::test]
async fn stop_against_already_stopped_is_changed_noop_still_publishes() {
    let h = harness(open_stopped());
    h.runtime
        .stop_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Stopped)));
    let out = h
        .lifecycle
        .stop("t", SandboxStopReason::IdleReaper)
        .await
        .expect("ok");
    assert!(out.intent_effect.changed());
    assert!(!out.runtime_effect.applied());
    assert!(h.events.facts().contains(&EventFact::Stopped));
}

#[tokio::test]
async fn completion_commit_failure_propagates_and_does_not_publish() {
    // Durability: a failed completion transaction must not publish the event
    // nor report success; intent + marker survive for the reconciler.
    let store = Arc::new(InMemoryLifecycleDao {
        intent: Mutex::new(Some(open_running())),
        next_gen: Mutex::new(1),
        commits: Mutex::new(vec![]),
        marker_cleared: Mutex::new(false),
        storage_reclaimed: Mutex::new(false),
        other_active_members: Mutex::new(false),
        fail_persist: false,
        fail_commit: true,
    });
    let h = harness_with(store, FlakyLock::free());
    h.runtime
        .stop_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
    let res = h.lifecycle.stop("t", SandboxStopReason::User).await;
    assert!(matches!(res, Err(LifecycleError::CompletionCommit { .. })));
    // Stopped completion was NOT published; marker retained.
    assert!(!h.events.facts().contains(&EventFact::Stopped));
    assert!(h.store.current().expect("intent").marker.is_some());
}
