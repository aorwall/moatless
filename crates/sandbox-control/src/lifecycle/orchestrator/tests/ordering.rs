//! What is durable before the runtime is touched: intent on the way in, and a
//! best-effort `StopAgent` on the way out.

use super::super::*;
use super::harness::*;
use crate::lifecycle::dao::{ReconcileCandidate, SandboxShape};
use crate::lifecycle::reconciler::SandboxMember;
use std::sync::Mutex;

#[tokio::test]
async fn start_persists_running_intent_before_runtime_effect() {
    let h = harness(open_stopped());
    h.runtime
        .ensure
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Starting)));
    let out = h.lifecycle.start("t").await.expect("ok");
    let cur = h.store.current().expect("intent");
    assert_eq!(cur.desired_state, SandboxDesiredState::Running);
    assert_eq!(cur.stop_reason, None);
    assert_eq!(out.observed, ObservedStatus::Starting);
    assert!(out.intent_effect.changed());
    assert!(out.runtime_effect.applied());
}

#[tokio::test]
async fn intent_write_failure_yields_no_runtime_effect() {
    let store = Arc::new(InMemoryLifecycleDao {
        intent: Mutex::new(Some(open_stopped())),
        next_gen: Mutex::new(1),
        commits: Mutex::new(vec![]),
        marker_cleared: Mutex::new(false),
        storage_reclaimed: Mutex::new(false),
        other_active_members: Mutex::new(false),
        fail_persist: true,
        fail_commit: false,
    });
    let h = harness_with(store, FlakyLock::free());
    let res = h.lifecycle.start("t").await;
    assert!(matches!(res, Err(LifecycleError::IntentWrite { .. })));
    assert!(h.log.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn load_failure_is_intent_load_not_write() {
    // A read failure must classify as IntentLoad, not IntentWrite.
    struct FailingLoad;
    #[async_trait]
    impl LifecycleDao for FailingLoad {
        async fn load(&self, _t: &str) -> Result<Option<TaskIntent>, StoreError> {
            Err(StoreError::new("conn reset"))
        }
        async fn reconcile_candidates(&self) -> Result<Vec<ReconcileCandidate>, StoreError> {
            Ok(vec![])
        }
        async fn running_sandbox_members(&self) -> Result<Vec<SandboxMember>, StoreError> {
            Ok(vec![])
        }
        async fn sandbox_shape(
            &self,
            _sandbox_id: &str,
        ) -> Result<Option<SandboxShape>, StoreError> {
            Ok(None)
        }
        async fn has_other_active_members(
            &self,
            _sandbox_id: &str,
            _task_id: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
        async fn allocate_generation(&self, _t: &str) -> Result<OperationGeneration, StoreError> {
            Ok(OperationGeneration(1))
        }
        async fn persist_intent(
            &self,
            _t: &str,
            _w: &IntentWrite,
        ) -> Result<IntentEffect, StoreError> {
            Ok(IntentEffect::Changed)
        }
        async fn commit_fact_and_clear_marker(
            &self,
            _sandbox_id: &str,
            _t: &str,
            _e: &LifecycleEvent,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn clear_marker(
            &self,
            _t: &str,
            _generation: OperationGeneration,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn stopped_with_storage(
            &self,
            _t: &[String],
        ) -> Result<Vec<(String, DateTime<Utc>)>, StoreError> {
            Ok(vec![])
        }
        async fn mark_storage_reclaimed(&self, _t: &str) -> Result<(), StoreError> {
            Ok(())
        }
        async fn record_running_commands(&self, _t: &str, _running: i32) -> Result<(), StoreError> {
            Ok(())
        }
        async fn record_unreported_commands(
            &self,
            _t: &str,
            _commands: &[OutstandingCommand],
            _revision: i64,
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn take_unreported_commands(
            &self,
            _t: &str,
        ) -> Result<Vec<OutstandingCommand>, StoreError> {
            Ok(Vec::new())
        }
    }
    let log: Log = Arc::new(Mutex::new(vec![]));
    let lifecycle = Lifecycle::new(
        Arc::new(FailingLoad),
        Arc::new(RecordingRuntime::new(log.clone())),
        Arc::new(RecordingAgentStop { log }),
        RecordingEvents::new(),
        FlakyLock::free(),
        Arc::new(TestClock),
    );
    assert!(matches!(
        lifecycle.start("t").await,
        Err(LifecycleError::IntentLoad { .. })
    ));
}

// ---- Graceful stop ordering ----

#[tokio::test]
async fn stop_issues_agent_stop_before_runtime_effect() {
    let h = harness(open_running());
    h.runtime
        .stop_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
    h.lifecycle
        .stop("t", SandboxStopReason::User)
        .await
        .expect("ok");
    let log = h.log.lock().expect("lock").clone();
    let stop_agent = log
        .iter()
        .position(|s| s == "stop_agent")
        .expect("stop_agent");
    let runtime_stop = log
        .iter()
        .position(|s| s == "runtime_stop")
        .expect("runtime_stop");
    assert!(
        stop_agent < runtime_stop,
        "StopAgent must precede runtime stop: {log:?}"
    );

    // The workspace push sits between them: after the agent stops writing,
    // before the effect that takes the volume away.
    let sync = log
        .iter()
        .position(|s| s == "sync_workspace")
        .expect("sync_workspace");
    assert!(
        stop_agent < sync && sync < runtime_stop,
        "the workspace push belongs between the agent stop and the runtime stop: {log:?}"
    );
}

/// A remove that keeps the archives is the last chance to make the S3 copy
/// current — the PVC is about to be deleted.
#[tokio::test]
async fn remove_with_retention_pushes_the_workspace_first() {
    let h = harness(open_running());
    h.runtime
        .remove_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Removed)));
    h.lifecycle
        .remove("t", ArchiveRetention::RetainArchives)
        .await
        .expect("ok");
    let log = h.log.lock().expect("lock").clone();
    let sync = log
        .iter()
        .position(|s| s == "sync_workspace")
        .expect("sync_workspace");
    let runtime_remove = log
        .iter()
        .position(|s| s == "runtime_remove")
        .expect("runtime_remove");
    assert!(
        sync < runtime_remove,
        "the workspace push must precede the runtime remove: {log:?}"
    );
}

/// A hard delete is on its way to erasing the object the push would write.
#[tokio::test]
async fn hard_delete_skips_the_workspace_push() {
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
    let log = h.log.lock().expect("lock").clone();
    assert!(
        !log.iter().any(|s| s == "sync_workspace"),
        "a hard delete must not spend the stop budget on an upload: {log:?}"
    );
}

#[tokio::test]
async fn remove_of_shared_member_keeps_runtime_while_active_peers_remain() {
    let store = InMemoryLifecycleDao::with_intent(open_running()).with_other_active_members();
    let h = harness_with(store.clone(), FlakyLock::free());

    let outcome = h
        .lifecycle
        .remove("t", ArchiveRetention::DeleteArchives)
        .await
        .expect("member detach succeeds");

    assert_eq!(
        outcome,
        LifecycleOutcome::new(
            IntentEffect::Unchanged,
            RuntimeEffect::NoOp,
            ObservedStatus::Ready,
        )
    );
    assert_eq!(
        store.current().expect("intent").desired_state,
        SandboxDesiredState::Running,
        "member detach must not move sandbox intent to Removed"
    );
    let log = h.log.lock().expect("lock").clone();
    assert!(log.iter().any(|s| s == "stop_agent"));
    assert!(
        !log.iter().any(|s| s == "sync_workspace"),
        "peer detach must not push the whole shared workspace: {log:?}"
    );
    assert!(
        !log.iter().any(|s| s == "runtime_remove"),
        "peer detach must not remove shared runtime resources: {log:?}"
    );
}

/// The push is advisory. A port that cannot deliver must leave the stop
/// exactly as it would have been without one.
#[tokio::test]
async fn a_failing_workspace_sync_does_not_block_the_stop() {
    let log: Log = Arc::new(Mutex::new(vec![]));
    let runtime = Arc::new(RecordingRuntime::new(log.clone()));
    runtime
        .stop_res
        .lock()
        .expect("lock")
        .replace(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped)));
    let lifecycle = Lifecycle::new(
        InMemoryLifecycleDao::with_intent(open_running()),
        runtime,
        Arc::new(RecordingAgentStop { log: log.clone() }),
        RecordingEvents::new(),
        FlakyLock::free(),
        Arc::new(TestClock),
    )
    .with_workspace_sync(Arc::new(FailingWorkspaceSync));

    lifecycle
        .stop("t", SandboxStopReason::User)
        .await
        .expect("the stop still succeeds");
    assert!(
        log.lock()
            .expect("lock")
            .iter()
            .any(|s| s == "runtime_stop"),
        "the runtime effect must still run"
    );
}

#[tokio::test]
async fn remove_issues_agent_stop_before_runtime_effect() {
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
    let log = h.log.lock().expect("lock").clone();
    let stop_agent = log
        .iter()
        .position(|s| s == "stop_agent")
        .expect("stop_agent");
    let runtime_remove = log
        .iter()
        .position(|s| s == "runtime_remove")
        .expect("runtime_remove");
    assert!(
        stop_agent < runtime_remove,
        "StopAgent must precede runtime remove: {log:?}"
    );
}
