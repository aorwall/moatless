//! The doubles each orchestrator port is exercised against, and the `Harness`
//! that wires them to one `Lifecycle`.

use super::super::*;
use std::sync::Mutex;

use crate::lifecycle::dao::{ReconcileCandidate, SandboxShape};
use crate::lifecycle::reconciler::SandboxMember;

use chrono::TimeZone;
use moatless_sandbox_runtime::runtime::RuntimeError;

pub(super) fn fixed_now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("valid ts")
}

pub(super) struct TestClock;
impl WallClock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        fixed_now()
    }
}

/// Shared ordered log so tests can assert call ordering across ports.
pub(super) type Log = Arc<Mutex<Vec<String>>>;

pub(super) struct InMemoryLifecycleDao {
    pub(super) intent: Mutex<Option<TaskIntent>>,
    pub(super) next_gen: Mutex<i64>,
    pub(super) commits: Mutex<Vec<String>>,
    pub(super) marker_cleared: Mutex<bool>,
    pub(super) storage_reclaimed: Mutex<bool>,
    pub(super) other_active_members: Mutex<bool>,
    pub(super) fail_persist: bool,
    pub(super) fail_commit: bool,
}

impl InMemoryLifecycleDao {
    pub(super) fn with_intent(intent: TaskIntent) -> Arc<Self> {
        Arc::new(Self {
            intent: Mutex::new(Some(intent)),
            next_gen: Mutex::new(1),
            commits: Mutex::new(vec![]),
            marker_cleared: Mutex::new(false),
            storage_reclaimed: Mutex::new(false),
            other_active_members: Mutex::new(false),
            fail_persist: false,
            fail_commit: false,
        })
    }
    pub(super) fn missing() -> Arc<Self> {
        Arc::new(Self {
            intent: Mutex::new(None),
            next_gen: Mutex::new(1),
            commits: Mutex::new(vec![]),
            marker_cleared: Mutex::new(false),
            storage_reclaimed: Mutex::new(false),
            other_active_members: Mutex::new(false),
            fail_persist: false,
            fail_commit: false,
        })
    }
    pub(super) fn with_other_active_members(self: &Arc<Self>) -> Arc<Self> {
        *self.other_active_members.lock().expect("lock") = true;
        Arc::clone(self)
    }
    pub(super) fn current(&self) -> Option<TaskIntent> {
        self.intent.lock().expect("lock").clone()
    }
    pub(super) fn commits(&self) -> Vec<String> {
        self.commits.lock().expect("lock").clone()
    }
    pub(super) fn marker_cleared(&self) -> bool {
        *self.marker_cleared.lock().expect("lock")
    }
    pub(super) fn storage_reclaimed(&self) -> bool {
        *self.storage_reclaimed.lock().expect("lock")
    }
}

#[async_trait]
impl LifecycleDao for InMemoryLifecycleDao {
    async fn load(&self, _task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
        Ok(self.intent.lock().expect("lock").clone())
    }
    async fn reconcile_candidates(&self) -> Result<Vec<ReconcileCandidate>, StoreError> {
        Ok(vec![])
    }
    async fn running_sandbox_members(&self) -> Result<Vec<SandboxMember>, StoreError> {
        Ok(vec![])
    }
    async fn sandbox_shape(&self, _sandbox_id: &str) -> Result<Option<SandboxShape>, StoreError> {
        Ok(None)
    }
    async fn has_other_active_members(
        &self,
        _sandbox_id: &str,
        _task_id: &str,
    ) -> Result<bool, StoreError> {
        Ok(*self.other_active_members.lock().expect("lock"))
    }
    async fn allocate_generation(&self, _task_id: &str) -> Result<OperationGeneration, StoreError> {
        let mut g = self.next_gen.lock().expect("lock");
        let out = OperationGeneration(*g);
        *g += 1;
        Ok(out)
    }
    async fn persist_intent(
        &self,
        _task_id: &str,
        write: &IntentWrite,
    ) -> Result<IntentEffect, StoreError> {
        if self.fail_persist {
            return Err(StoreError::new("persist failed"));
        }
        let mut slot = self.intent.lock().expect("lock");
        let effect = match slot.as_ref() {
            Some(cur)
                if cur.desired_state == write.desired_state
                    && cur.stop_reason == write.stop_reason
                    && cur.retention == write.retention =>
            {
                IntentEffect::Unchanged
            }
            _ => IntentEffect::Changed,
        };
        let status = slot.as_ref().map_or(TaskStatus::Open, |c| c.status);
        let sandbox_id = slot
            .as_ref()
            .map_or_else(|| "t".to_string(), |c| c.sandbox_id.clone());
        let workspace_id = slot
            .as_ref()
            .map_or_else(|| "ws_repo-x".to_string(), |c| c.workspace_id.clone());
        *slot = Some(TaskIntent {
            status,
            visibility: Visibility::Private,
            scope: Scope::User,
            owner_user_id: Some("user-1".to_string()),
            reporter_user_id: None,
            sandbox_id,
            workspace_id,
            headless: false,
            desired_state: write.desired_state,
            stop_reason: write.stop_reason,
            retention: write.retention,
            marker: Some(write.marker),
            desired_state_updated_at: fixed_now(),
        });
        Ok(effect)
    }
    async fn commit_fact_and_clear_marker(
        &self,
        _sandbox_id: &str,
        _task_id: &str,
        event: &LifecycleEvent,
    ) -> Result<(), StoreError> {
        if self.fail_commit {
            return Err(StoreError::new("commit failed"));
        }
        self.commits
            .lock()
            .expect("lock")
            .push(event.event_id.clone());
        // Committing a completion clears the marker atomically.
        if let Some(cur) = self.intent.lock().expect("lock").as_mut() {
            if cur
                .marker
                .is_some_and(|marker| marker.generation == event.generation)
            {
                cur.marker = None;
            }
        }
        Ok(())
    }
    async fn stopped_with_storage(
        &self,
        _task_ids: &[String],
    ) -> Result<Vec<(String, DateTime<Utc>)>, StoreError> {
        let intent = self.intent.lock().expect("lock").clone();
        Ok(intent
            .filter(|i| i.desired_state == SandboxDesiredState::Stopped)
            .filter(|_| !*self.storage_reclaimed.lock().expect("lock"))
            .map(|i| vec![("t".to_string(), i.desired_state_updated_at)])
            .unwrap_or_default())
    }
    async fn mark_storage_reclaimed(&self, _task_id: &str) -> Result<(), StoreError> {
        *self.storage_reclaimed.lock().expect("lock") = true;
        Ok(())
    }
    async fn record_running_commands(
        &self,
        _task_id: &str,
        _running: i32,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    async fn record_unreported_commands(
        &self,
        _task_id: &str,
        _commands: &[OutstandingCommand],
        _revision: i64,
    ) -> Result<(), StoreError> {
        Ok(())
    }
    async fn take_unreported_commands(
        &self,
        _task_id: &str,
    ) -> Result<Vec<OutstandingCommand>, StoreError> {
        Ok(Vec::new())
    }
    async fn bump_token_generation(
        &self,
        _sandbox_id: &str,
    ) -> Result<Option<(i64, Option<String>)>, StoreError> {
        Ok(None)
    }
    async fn switch_driver(
        &self,
        _sandbox_id: &str,
        _user_id: &str,
    ) -> Result<Option<crate::lifecycle::DriverChange>, StoreError> {
        Ok(None)
    }
    async fn clear_marker(
        &self,
        _task_id: &str,
        generation: OperationGeneration,
    ) -> Result<(), StoreError> {
        *self.marker_cleared.lock().expect("lock") = true;
        if let Some(cur) = self.intent.lock().expect("lock").as_mut() {
            if cur
                .marker
                .is_some_and(|marker| marker.generation == generation)
            {
                cur.marker = None;
            }
        }
        Ok(())
    }
}

pub(super) struct RecordingAgentStop {
    pub(super) log: Log,
}
#[async_trait]
impl AgentStopPort for RecordingAgentStop {
    async fn stop_agent(
        &self,
        _task_id: &str,
        _reason: SandboxStopReason,
    ) -> moatless_ports::sandbox::AgentStopOutcome {
        self.log.lock().expect("lock").push("stop_agent".into());
        moatless_ports::sandbox::AgentStopOutcome::Stopped
    }
}

pub(super) struct RecordingWorkspaceSync {
    pub(super) log: Log,
}
#[async_trait]
impl WorkspaceSyncPort for RecordingWorkspaceSync {
    async fn sync_workspace(&self, _task_id: &str) {
        self.log.lock().expect("lock").push("sync_workspace".into());
    }
}

/// A port that fails the way a real one does — by never answering — to prove
/// the transition does not depend on it.
pub(super) struct FailingWorkspaceSync;
#[async_trait]
impl WorkspaceSyncPort for FailingWorkspaceSync {
    async fn sync_workspace(&self, _task_id: &str) {
        // Swallowed by the adapter in production; nothing reaches lifecycle.
    }
}

/// Configurable archive double. The head answer and the presign answer are the
/// only two inputs the cold decision is made from, so both are settable and
/// both are logged into the shared order log.
pub(super) struct RecordingArchive {
    pub(super) log: Log,
    pub(super) head: Result<Option<DateTime<Utc>>, String>,
    pub(super) presign: Result<String, String>,
}
impl RecordingArchive {
    pub(super) fn heading(log: Log, head: Result<Option<DateTime<Utc>>, String>) -> Arc<Self> {
        Arc::new(Self {
            log,
            head,
            presign: Ok(RESCUE_URL.to_string()),
        })
    }
    pub(super) fn failing_presign(log: Log) -> Arc<Self> {
        Arc::new(Self {
            log,
            head: Ok(None),
            presign: Err("presign denied".into()),
        })
    }
}
#[async_trait]
impl WorkspaceArchivePort for RecordingArchive {
    async fn archived_at(&self, _t: &str) -> Result<Option<DateTime<Utc>>, String> {
        self.log.lock().expect("lock").push("archived_at".into());
        self.head.clone()
    }
    async fn presign_upload(&self, _t: &str) -> Result<String, String> {
        self.log.lock().expect("lock").push("presign_upload".into());
        self.presign.clone()
    }
}

pub(super) const RESCUE_URL: &str = "https://s3.invalid/t/workspace.tar?sig=1";

pub(super) struct RecordingEvents {
    pub(super) published: Mutex<Vec<LifecycleEvent>>,
    pub(super) diagnostics: Mutex<Vec<moatless_sandbox_runtime::outcome::Diagnostics>>,
}
impl RecordingEvents {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            published: Mutex::new(vec![]),
            diagnostics: Mutex::new(vec![]),
        })
    }
    pub(super) fn facts(&self) -> Vec<EventFact> {
        self.published
            .lock()
            .expect("lock")
            .iter()
            .map(|e| e.fact)
            .collect()
    }
    pub(super) fn ids(&self) -> Vec<String> {
        self.published
            .lock()
            .expect("lock")
            .iter()
            .map(|e| e.event_id.clone())
            .collect()
    }
}
#[async_trait]
impl LifecycleEventSink for RecordingEvents {
    async fn publish(&self, _task_id: &str, event: &LifecycleEvent) {
        self.published.lock().expect("lock").push(event.clone());
    }

    async fn publish_diagnostics(
        &self,
        _task_id: &str,
        _generation: OperationGeneration,
        _observed: ObservedStatus,
        diagnostics: &moatless_sandbox_runtime::outcome::Diagnostics,
    ) {
        self.diagnostics
            .lock()
            .expect("lock")
            .push(diagnostics.clone());
    }
}

/// Configurable recording runtime adapter.
pub(super) struct RecordingRuntime {
    pub(super) log: Log,
    pub(super) ensure: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    pub(super) stop_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    pub(super) restart_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    pub(super) redeploy_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    pub(super) remove_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    pub(super) reclaim_res: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    pub(super) last_ctx: Mutex<Option<RuntimeOperationContext>>,
    /// The rescue URL the last `reclaim_storage` was handed, if any. Whether
    /// one was handed over at all is in the log line, which is the fact the
    /// freshness proof is actually about.
    pub(super) last_rescue_url: Mutex<Option<String>>,
}
impl RecordingRuntime {
    pub(super) fn new(log: Log) -> Self {
        Self {
            log,
            ensure: Mutex::new(None),
            stop_res: Mutex::new(None),
            restart_res: Mutex::new(None),
            redeploy_res: Mutex::new(None),
            remove_res: Mutex::new(None),
            reclaim_res: Mutex::new(None),
            last_ctx: Mutex::new(None),
            last_rescue_url: Mutex::new(None),
        }
    }
    pub(super) fn take(
        slot: &Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        slot.lock()
            .expect("lock")
            .take()
            .unwrap_or_else(|| Ok(RuntimeOutcome::no_op(ObservedStatus::Ready)))
    }
}
#[async_trait]
impl SandboxRuntime for RecordingRuntime {
    async fn ensure_running(
        &self,
        _t: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        self.log.lock().expect("lock").push("ensure_running".into());
        *self.last_ctx.lock().expect("lock") = Some(ctx);
        Self::take(&self.ensure)
    }
    async fn stop(
        &self,
        _t: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        self.log.lock().expect("lock").push("runtime_stop".into());
        *self.last_ctx.lock().expect("lock") = Some(ctx);
        Self::take(&self.stop_res)
    }
    async fn restart(
        &self,
        _t: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        self.log
            .lock()
            .expect("lock")
            .push("runtime_restart".into());
        *self.last_ctx.lock().expect("lock") = Some(ctx);
        Self::take(&self.restart_res)
    }
    async fn redeploy(
        &self,
        _t: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        self.log
            .lock()
            .expect("lock")
            .push("runtime_redeploy".into());
        *self.last_ctx.lock().expect("lock") = Some(ctx);
        Self::take(&self.redeploy_res)
    }
    async fn remove(
        &self,
        _t: &str,
        _r: ArchiveRetention,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        self.log.lock().expect("lock").push("runtime_remove".into());
        *self.last_ctx.lock().expect("lock") = Some(ctx);
        Self::take(&self.remove_res)
    }
    async fn reclaim_storage(
        &self,
        _t: &str,
        rescue_upload_url: Option<&str>,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        let mode = if rescue_upload_url.is_some() {
            "rescue"
        } else {
            "proven"
        };
        self.log
            .lock()
            .expect("lock")
            .push(format!("runtime_reclaim_storage:{mode}"));
        *self.last_ctx.lock().expect("lock") = Some(ctx);
        *self.last_rescue_url.lock().expect("lock") = rescue_upload_url.map(str::to_string);
        Self::take(&self.reclaim_res)
    }
}

pub(super) struct TestGuard;
impl LockGuard for TestGuard {}

/// A lock that denies `denies` times, then grants; holder kind is fixed.
pub(super) struct FlakyLock {
    pub(super) denies: Mutex<u32>,
    pub(super) holder: OperationKind,
}
impl FlakyLock {
    pub(super) fn free() -> Arc<Self> {
        Arc::new(Self {
            denies: Mutex::new(0),
            holder: OperationKind::Stop,
        })
    }
    pub(super) fn denying(times: u32, holder: OperationKind) -> Arc<Self> {
        Arc::new(Self {
            denies: Mutex::new(times),
            holder,
        })
    }
}
#[async_trait]
impl LifecycleLock for FlakyLock {
    async fn try_acquire(
        &self,
        _t: &str,
        _kind: OperationKind,
    ) -> Result<Box<dyn LockGuard>, OperationKind> {
        let mut d = self.denies.lock().expect("lock");
        if *d > 0 {
            *d -= 1;
            Err(self.holder)
        } else {
            Ok(Box::new(TestGuard))
        }
    }
}

// ---- Builders ----

pub(super) fn intent(
    status: TaskStatus,
    desired: SandboxDesiredState,
    reason: Option<SandboxStopReason>,
    retention: Option<ArchiveRetention>,
    marker: Option<CorrelationMarker>,
) -> TaskIntent {
    TaskIntent {
        status,
        visibility: Visibility::Private,
        scope: Scope::User,
        owner_user_id: Some("user-1".to_string()),
        reporter_user_id: None,
        sandbox_id: "t".to_string(),
        workspace_id: "ws_repo-x".to_string(),
        headless: false,
        desired_state: desired,
        stop_reason: reason,
        retention,
        marker,
        desired_state_updated_at: fixed_now(),
    }
}

pub(super) fn open_running() -> TaskIntent {
    intent(
        TaskStatus::Open,
        SandboxDesiredState::Running,
        None,
        None,
        None,
    )
}
pub(super) fn open_stopped() -> TaskIntent {
    intent(
        TaskStatus::Open,
        SandboxDesiredState::Stopped,
        Some(SandboxStopReason::User),
        None,
        None,
    )
}
pub(super) fn closed_removed() -> TaskIntent {
    intent(
        TaskStatus::Closed,
        SandboxDesiredState::Removed,
        Some(SandboxStopReason::Cleanup),
        Some(ArchiveRetention::RetainArchives),
        None,
    )
}

pub(super) struct Harness {
    pub(super) lifecycle: Lifecycle,
    pub(super) store: Arc<InMemoryLifecycleDao>,
    pub(super) runtime: Arc<RecordingRuntime>,
    pub(super) events: Arc<RecordingEvents>,
    pub(super) log: Log,
}

pub(super) fn harness_with(
    store: Arc<InMemoryLifecycleDao>,
    lock: Arc<dyn LifecycleLock>,
) -> Harness {
    let log: Log = Arc::new(Mutex::new(vec![]));
    let runtime = Arc::new(RecordingRuntime::new(log.clone()));
    let events = RecordingEvents::new();
    let agent_stop = Arc::new(RecordingAgentStop { log: log.clone() });
    let lifecycle = Lifecycle::new(
        store.clone(),
        runtime.clone(),
        agent_stop,
        events.clone(),
        lock,
        Arc::new(TestClock),
    )
    .with_workspace_sync(Arc::new(RecordingWorkspaceSync { log: log.clone() }));
    Harness {
        lifecycle,
        store,
        runtime,
        events,
        log,
    }
}

pub(super) fn harness(intent: TaskIntent) -> Harness {
    harness_with(InMemoryLifecycleDao::with_intent(intent), FlakyLock::free())
}
