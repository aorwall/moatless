//! Two callers on one Task: what the lock refuses outright, what it retries,
//! and where it gives up.

use super::super::*;
use super::harness::*;
use std::sync::Mutex;

#[tokio::test]
async fn contended_lock_surfaces_operation_in_progress_with_holder_kind() {
    let store = InMemoryLifecycleDao::with_intent(open_running());
    let h = harness_with(store, FlakyLock::denying(1, OperationKind::Stop));
    let res = h.lifecycle.start("t").await;
    match res {
        Err(LifecycleError::OperationInProgress { kind }) => {
            assert_eq!(kind, OperationKind::Stop);
        }
        other => panic!("expected OperationInProgress, got {other:?}"),
    }
    assert!(h.log.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn bounded_lock_retry_eventually_succeeds() {
    // Denies twice then grants; three attempts succeed.
    let store = InMemoryLifecycleDao::with_intent(open_running());
    let log: Log = Arc::new(Mutex::new(vec![]));
    let runtime = Arc::new(RecordingRuntime::new(log.clone()));
    runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)));
    let lifecycle = Lifecycle::new(
        store,
        runtime,
        Arc::new(RecordingAgentStop { log }),
        RecordingEvents::new(),
        FlakyLock::denying(2, OperationKind::Stop),
        Arc::new(TestClock),
    )
    .with_lock_attempts(3);
    assert!(lifecycle.start("t").await.is_ok());
}

#[tokio::test]
async fn exhausted_lock_retry_surfaces_operation_in_progress() {
    let store = InMemoryLifecycleDao::with_intent(open_running());
    let h_lock = FlakyLock::denying(10, OperationKind::Remove);
    let h = harness_with(store, h_lock);
    let res = h.lifecycle.with_lock_attempts(3).start("t").await;
    assert!(matches!(
        res,
        Err(LifecycleError::OperationInProgress {
            kind: OperationKind::Remove
        })
    ));
}
