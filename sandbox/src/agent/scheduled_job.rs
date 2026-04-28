//! Unified registry for sandbox-local "deferred work" jobs.
//!
//! Two concrete jobs share this surface:
//! - `MonitorJob` — the cron poller driving the Claude Code `Monitor` tool.
//! - `WakeupJob` — the one-shot timer driving the `ScheduleWakeup` tool.
//!
//! Each job runs in its own tokio task and injects synthetic user messages
//! back into the running Claude session via `JobInjector`. The registry owns
//! cancellation tokens, deduplicates by job id, and exposes a single status
//! snapshot for the gRPC `GetAgentStatus` endpoint.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::handle::AgentHandle;

/// Trait that lets the registry hand a synthetic user message back to the
/// running agent. Implemented in production by `AgentHandle`; tests provide a
/// capturing double.
pub trait JobInjector: Send + Sync + 'static {
    fn inject(&self, text: String);
}

impl JobInjector for AgentHandle {
    fn inject(&self, text: String) {
        // The job has nothing to ack; drop the receiver so the actor doesn't
        // wait. The actor's own session-id resolution still works.
        drop(self.send(text, None, None, None, None, None));
    }
}

/// Late-init injector — needed to break the circular dependency between
/// `ScheduledJobRegistry` (which the actor needs at construction) and
/// `AgentHandle` (which the registry needs to inject messages). Construct
/// the wrapper, build the registry around it, spawn the actor, then call
/// `set` with the resulting `AgentHandle`.
pub struct LateInjector {
    inner: std::sync::OnceLock<Box<dyn JobInjector>>,
}

impl LateInjector {
    pub fn new() -> Self {
        Self {
            inner: std::sync::OnceLock::new(),
        }
    }

    /// Install the real injector. Subsequent calls are no-ops.
    pub fn set(&self, injector: Box<dyn JobInjector>) {
        let _ = self.inner.set(injector);
    }
}

impl Default for LateInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl JobInjector for LateInjector {
    fn inject(&self, text: String) {
        if let Some(injector) = self.inner.get() {
            injector.inject(text);
        } else {
            warn!("Scheduled job result dropped: injector not yet initialised");
        }
    }
}

/// Discriminator for cancel-by-kind and status reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Monitor,
    Wakeup,
}

/// Read-only view of one Monitor job — kept here (not in `monitor_job.rs`) so
/// the registry can store a single tagged-union snapshot type without
/// dragging the monitor module into every consumer.
#[derive(Debug, Clone)]
pub struct MonitorSnapshot {
    pub id: String,
    pub registered_at_unix_ms: i64,
    pub description: String,
    pub command_preview: String,
    pub timeout_ms: u64,
    pub persistent: bool,
}

/// Read-only view of one Wakeup job.
#[derive(Debug, Clone)]
pub struct WakeupSnapshot {
    pub id: String,
    pub registered_at_unix_ms: i64,
    pub reason: String,
    pub prompt_preview: String,
    pub wakeup_at_unix_ms: i64,
}

/// Tagged union of registered job snapshots.
#[derive(Debug, Clone)]
pub enum JobSnapshot {
    Monitor(MonitorSnapshot),
    Wakeup(WakeupSnapshot),
}

/// Per-job context handed to `ScheduledJob::run`.
pub struct JobContext {
    /// Cancels just this job (e.g. latest-wins replacement).
    pub job_cancel: CancellationToken,
    /// Cancels every job for the current agent run.
    pub run_cancel: CancellationToken,
    /// Used to inject synthetic user messages back into the session.
    pub injector: Arc<dyn JobInjector>,
}

/// A unit of deferred work owned by the `ScheduledJobRegistry`.
#[async_trait::async_trait]
pub trait ScheduledJob: Send + Sync + 'static {
    /// Stable key. Monitor: tool_use_id. Wakeup: ScheduleWakeup tool_use_id.
    fn id(&self) -> &str;
    fn kind(&self) -> JobKind;
    fn snapshot(&self) -> JobSnapshot;
    /// Run until done or cancelled. Inject results via `ctx.injector`.
    async fn run(self: Box<Self>, ctx: JobContext);
}

struct JobEntry {
    cancel: CancellationToken,
    kind: JobKind,
    snapshot: JobSnapshot,
    _join: JoinHandle<()>,
}

/// Registry of in-flight scheduled jobs. Keyed by `ScheduledJob::id`.
pub struct ScheduledJobRegistry {
    injector: Arc<dyn JobInjector>,
    jobs: Arc<RwLock<HashMap<String, JobEntry>>>,
}

impl ScheduledJobRegistry {
    pub fn new(injector: Arc<dyn JobInjector>) -> Self {
        Self {
            injector,
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register and spawn a job. Returns `false` (and leaves the existing job
    /// untouched) if a job with the same id is already running.
    pub async fn register(
        &self,
        job: Box<dyn ScheduledJob>,
        run_cancel: CancellationToken,
    ) -> bool {
        let id = job.id().to_string();
        let mut jobs = self.jobs.write().await;
        if jobs.contains_key(&id) {
            warn!(id = %id, "Scheduled job already registered, ignoring duplicate");
            return false;
        }

        let kind = job.kind();
        let snapshot = job.snapshot();
        let cancel = CancellationToken::new();
        let ctx = JobContext {
            job_cancel: cancel.clone(),
            run_cancel,
            injector: Arc::clone(&self.injector),
        };

        info!(id = %id, ?kind, "Registering scheduled job");

        let jobs_for_cleanup = Arc::clone(&self.jobs);
        let cleanup_id = id.clone();
        let join = tokio::spawn(async move {
            job.run(ctx).await;
            jobs_for_cleanup.write().await.remove(&cleanup_id);
        });

        jobs.insert(
            id,
            JobEntry {
                cancel,
                kind,
                snapshot,
                _join: join,
            },
        );
        true
    }

    /// Snapshot of every currently registered job.
    pub async fn list(&self) -> Vec<JobSnapshot> {
        self.jobs
            .read()
            .await
            .values()
            .map(|entry| entry.snapshot.clone())
            .collect()
    }

    /// Cancel every registered job. Called when the agent run ends.
    pub async fn cancel_all(&self) {
        let jobs = self.jobs.read().await;
        if jobs.is_empty() {
            return;
        }
        info!(count = jobs.len(), "Cancelling all registered scheduled jobs");
        for entry in jobs.values() {
            entry.cancel.cancel();
        }
    }

    /// Cancel only the jobs of a given kind. Used for latest-wins
    /// replacement (e.g. a new ScheduleWakeup supersedes the previous one).
    pub async fn cancel_by_kind(&self, kind: JobKind) {
        let jobs = self.jobs.read().await;
        for entry in jobs.values() {
            if entry.kind == kind {
                entry.cancel.cancel();
            }
        }
    }

    /// Number of currently registered jobs (observability + tests).
    pub async fn len(&self) -> usize {
        self.jobs.read().await.len()
    }

    /// True when no jobs are registered.
    pub async fn is_empty(&self) -> bool {
        self.jobs.read().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::time::Instant;

    use super::super::monitor_job::{MonitorJob, MonitorRequest};
    use super::super::wakeup_job::WakeupJob;
    use super::*;

    #[derive(Default, Clone)]
    struct CapturingInjector {
        messages: Arc<Mutex<Vec<String>>>,
    }

    impl CapturingInjector {
        fn snapshot(&self) -> Vec<String> {
            self.messages.lock().expect("mutex poisoned").clone()
        }
    }

    impl JobInjector for CapturingInjector {
        fn inject(&self, text: String) {
            self.messages.lock().expect("mutex poisoned").push(text);
        }
    }

    async fn wait_until<F>(timeout: Duration, mut predicate: F) -> bool
    where
        F: FnMut() -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        predicate()
    }

    fn monitor_request(tool_use_id: &str, command: &str) -> MonitorRequest {
        MonitorRequest {
            tool_use_id: tool_use_id.to_string(),
            command: command.to_string(),
            description: "test".to_string(),
            timeout_ms: 60_000,
            persistent: false,
        }
    }

    #[tokio::test]
    async fn wakeup_fires_after_delay_and_injects_prompt() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        let job = WakeupJob::new_for_test(
            "toolu_wake_1".to_string(),
            "long build".to_string(),
            "continue work".to_string(),
            Instant::now() + Duration::from_millis(50),
        );
        let registered = registry
            .register(Box::new(job), run_cancel.clone())
            .await;
        assert!(registered);

        let ok = wait_until(Duration::from_millis(500), || {
            !injector.snapshot().is_empty()
        })
        .await;
        assert!(ok, "expected the wakeup prompt to be injected");
        let messages = injector.snapshot();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "continue work");
    }

    #[tokio::test]
    async fn wakeup_cancelled_before_firing_injects_nothing() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        let job = WakeupJob::new_for_test(
            "toolu_wake_2".to_string(),
            "long build".to_string(),
            "should not fire".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        let registered = registry
            .register(Box::new(job), run_cancel.clone())
            .await;
        assert!(registered);

        run_cancel.cancel();

        // Give the cancellation a moment to propagate and self-clean.
        for _ in 0..20 {
            if registry.is_empty().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(registry.is_empty().await, "registry should self-clean after cancel");
        assert!(injector.snapshot().is_empty(), "no prompt should have been injected");
    }

    #[tokio::test]
    async fn cancel_by_kind_only_cancels_matching_jobs() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector));
        let run_cancel = CancellationToken::new();

        // Long-running monitor that won't terminate during the test.
        let monitor = MonitorJob::from_request(monitor_request("toolu_mon_1", "sleep 30"));
        registry
            .register(Box::new(monitor), run_cancel.clone())
            .await;

        let wakeup = WakeupJob::new_for_test(
            "toolu_wake_3".to_string(),
            "later".to_string(),
            "continue".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        registry
            .register(Box::new(wakeup), run_cancel.clone())
            .await;

        assert_eq!(registry.len().await, 2);

        registry.cancel_by_kind(JobKind::Wakeup).await;

        // Wait for the wakeup task to self-clean.
        for _ in 0..50 {
            if registry.len().await == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(registry.len().await, 1, "monitor should remain after cancel_by_kind(Wakeup)");

        let snapshots = registry.list().await;
        assert!(matches!(snapshots[0], JobSnapshot::Monitor(_)));

        run_cancel.cancel();
    }

    #[tokio::test]
    async fn list_returns_mixed_snapshot_variants() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector));
        let run_cancel = CancellationToken::new();

        let monitor = MonitorJob::from_request(monitor_request("toolu_mon_2", "sleep 30"));
        registry
            .register(Box::new(monitor), run_cancel.clone())
            .await;

        let wakeup = WakeupJob::new_for_test(
            "toolu_wake_4".to_string(),
            "later".to_string(),
            "continue".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        registry
            .register(Box::new(wakeup), run_cancel.clone())
            .await;

        let snapshots = registry.list().await;
        assert_eq!(snapshots.len(), 2);
        let has_monitor = snapshots
            .iter()
            .any(|s| matches!(s, JobSnapshot::Monitor(_)));
        let has_wakeup = snapshots
            .iter()
            .any(|s| matches!(s, JobSnapshot::Wakeup(_)));
        assert!(has_monitor, "expected one Monitor snapshot");
        assert!(has_wakeup, "expected one Wakeup snapshot");

        run_cancel.cancel();
    }

    #[tokio::test]
    async fn latest_wins_wakeup_replaces_previous() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector));
        let run_cancel = CancellationToken::new();

        let first = WakeupJob::new_for_test(
            "toolu_wake_5".to_string(),
            "first".to_string(),
            "first prompt".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        registry
            .register(Box::new(first), run_cancel.clone())
            .await;

        registry.cancel_by_kind(JobKind::Wakeup).await;

        // Wait for the first to self-clean before registering the second so
        // there's no possibility of mixing up snapshots.
        for _ in 0..50 {
            if registry.is_empty().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let second = WakeupJob::new_for_test(
            "toolu_wake_6".to_string(),
            "second".to_string(),
            "second prompt".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        registry
            .register(Box::new(second), run_cancel.clone())
            .await;

        let snapshots = registry.list().await;
        assert_eq!(snapshots.len(), 1);
        match &snapshots[0] {
            JobSnapshot::Wakeup(snap) => {
                assert_eq!(snap.id, "toolu_wake_6");
                assert_eq!(snap.reason, "second");
            }
            other => panic!("expected wakeup snapshot, got {other:?}"),
        }

        run_cancel.cancel();
    }

    #[tokio::test]
    async fn duplicate_registration_is_ignored() {
        let injector = CapturingInjector::default();
        let registry = ScheduledJobRegistry::new(Arc::new(injector));
        let run_cancel = CancellationToken::new();

        let first = WakeupJob::new_for_test(
            "toolu_dupe".to_string(),
            "r".to_string(),
            "p".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        let second = WakeupJob::new_for_test(
            "toolu_dupe".to_string(),
            "r2".to_string(),
            "p2".to_string(),
            Instant::now() + Duration::from_secs(60),
        );

        assert!(registry.register(Box::new(first), run_cancel.clone()).await);
        assert!(!registry.register(Box::new(second), run_cancel.clone()).await);
        assert_eq!(registry.len().await, 1);

        run_cancel.cancel();
    }
}
