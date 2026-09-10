//! Unified registry for sandbox-local "deferred work" jobs.
//!
//! Today only `WakeupJob` (the one-shot timer driving the `ScheduleWakeup`
//! tool) uses this surface. Monitor crons are not registered here: the Claude
//! CLI handles `Monitor` client-side and our per-turn subprocess model means
//! events from the CLI's watcher never reach the agent — so Monitor is
//! disallowed at the CLI flag level instead.
//!
//! Each job runs in its own tokio task and injects a synthetic user message
//! back into the running Claude session through `JobContext::inject`. The
//! registry owns cancellation tokens, deduplicates by job id, and exposes a
//! single status snapshot for the gRPC `GetAgentStatus` endpoint.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent::handle::AgentHandle;

/// Trait that lets the registry hand a synthetic user message back to the
/// running agent. Implemented in production by `QueuedInjector`, which queues
/// the message with the backend so the turn it opens is recoverable; tests
/// provide the `AgentHandle` impl or a capturing double.
#[async_trait::async_trait]
pub trait JobInjector: Send + Sync + 'static {
    /// Hand `text` to the agent. `idempotency_key` is what a resend of this
    /// message dedups on, for an implementation that queues rather than sends
    /// straight at the agent: two jobs carrying identical text are distinct
    /// injections, and each must open its own turn.
    async fn inject(&self, idempotency_key: &str, text: String);
}

/// For a test that drives the agent directly. Production goes through
/// `QueuedInjector`: a direct send leaves no `user` row, and that row is what
/// `find_interrupted_turn` selects on.
#[async_trait::async_trait]
impl JobInjector for AgentHandle {
    async fn inject(&self, _idempotency_key: &str, text: String) {
        // The job has nothing to ack; drop the receiver so the lifecycle task
        // doesn't wait. The coordinator's session-id resolution still works.
        drop(self.send(text, None, None, None, None, None, false, None));
    }
}

/// Late-init injector — needed to break the circular dependency between
/// `ScheduledJobRegistry` (which the lifecycle task needs at construction) and
/// `AgentHandle` (which the registry needs to inject messages). Construct
/// the wrapper, build the registry around it, spawn the lifecycle task, then call
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

#[async_trait::async_trait]
impl JobInjector for LateInjector {
    async fn inject(&self, idempotency_key: &str, text: String) {
        if let Some(injector) = self.inner.get() {
            injector.inject(idempotency_key, text).await;
        } else {
            warn!("Scheduled job result dropped: injector not yet initialised");
        }
    }
}

/// Discriminator for cancel-by-kind and status reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Wakeup,
    /// Watches `run_in_background` jobs that outlived a turn and re-triggers the
    /// agent once they finish (subprocess mode only).
    BackgroundWatch,
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

/// Read-only view of one background-completion watch job.
#[derive(Debug, Clone)]
pub struct BackgroundWatchSnapshot {
    pub id: String,
    pub registered_at_unix_ms: i64,
    /// Number of background processes being watched when the snapshot was taken.
    pub command_count: u32,
    /// Best-effort command lines of the watched processes (for status display).
    pub commands: Vec<String>,
}

/// Tagged union of registered job snapshots.
#[derive(Debug, Clone)]
pub enum JobSnapshot {
    Wakeup(WakeupSnapshot),
    BackgroundWatch(BackgroundWatchSnapshot),
}

/// Per-job context handed to `ScheduledJob::run`.
pub struct JobContext {
    /// Cancels just this job (e.g. latest-wins replacement).
    pub job_cancel: CancellationToken,
    /// Cancels every job for the current agent run.
    pub run_cancel: CancellationToken,
    /// The registering job's `ScheduledJob::id`, which keys its injection.
    job_id: String,
    injector: Arc<dyn JobInjector>,
}

impl JobContext {
    pub fn new(
        job_id: String,
        injector: Arc<dyn JobInjector>,
        job_cancel: CancellationToken,
        run_cancel: CancellationToken,
    ) -> Self {
        Self {
            job_cancel,
            run_cancel,
            job_id,
            injector,
        }
    }

    /// Hand `text` to the agent as a synthetic user message from this job.
    pub async fn inject(&self, text: String) {
        self.injector.inject(&self.job_id, text).await;
    }
}

/// A unit of deferred work owned by the `ScheduledJobRegistry`.
#[async_trait::async_trait]
pub trait ScheduledJob: Send + Sync + 'static {
    /// Stable key. Monitor: `tool_use_id`. Wakeup: `ScheduleWakeup` `tool_use_id`.
    fn id(&self) -> &str;
    fn kind(&self) -> JobKind;
    fn snapshot(&self) -> JobSnapshot;
    /// Run until done or cancelled. Inject results via `ctx.inject`.
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
        let ctx = JobContext::new(
            id.clone(),
            Arc::clone(&self.injector),
            cancel.clone(),
            run_cancel,
        );

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
        info!(
            count = jobs.len(),
            "Cancelling all registered scheduled jobs"
        );
        for entry in jobs.values() {
            entry.cancel.cancel();
        }
    }

    /// Cancel only the jobs of a given kind. Used for latest-wins
    /// replacement (e.g. a new `ScheduleWakeup` supersedes the previous one).
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

    use super::super::background_watch_job::BackgroundWatchJob;
    use super::super::wakeup_job::WakeupJob;
    use super::*;
    use crate::commands::{CommandRegistry, CommandSpec};

    #[derive(Default, Clone)]
    struct CapturingInjector {
        messages: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl CapturingInjector {
        /// Each injection as `(idempotency_key, text)`.
        fn snapshot(&self) -> Vec<(String, String)> {
            self.messages.lock().expect("mutex poisoned").clone()
        }
    }

    #[async_trait::async_trait]
    impl JobInjector for CapturingInjector {
        async fn inject(&self, idempotency_key: &str, text: String) {
            self.messages
                .lock()
                .expect("mutex poisoned")
                .push((idempotency_key.to_string(), text));
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
        let registered = registry.register(Box::new(job), run_cancel.clone()).await;
        assert!(registered);

        let ok = wait_until(Duration::from_millis(500), || {
            !injector.snapshot().is_empty()
        })
        .await;
        assert!(ok, "expected the wakeup prompt to be injected");
        let messages = injector.snapshot();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            ("toolu_wake_1".to_string(), "continue work".to_string()),
            "the job id keys the injection, so two ticks of one prompt stay distinct"
        );
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
            Instant::now() + Duration::from_mins(1),
        );
        let registered = registry.register(Box::new(job), run_cancel.clone()).await;
        assert!(registered);

        run_cancel.cancel();

        // Give the cancellation a moment to propagate and self-clean.
        for _ in 0..20 {
            if registry.is_empty().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            registry.is_empty().await,
            "registry should self-clean after cancel"
        );
        assert!(
            injector.snapshot().is_empty(),
            "no prompt should have been injected"
        );
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
            Instant::now() + Duration::from_mins(1),
        );
        registry.register(Box::new(first), run_cancel.clone()).await;

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
            Instant::now() + Duration::from_mins(1),
        );
        registry
            .register(Box::new(second), run_cancel.clone())
            .await;

        let snapshots = registry.list().await;
        assert_eq!(snapshots.len(), 1);
        let JobSnapshot::Wakeup(snap) = &snapshots[0] else {
            panic!("expected wakeup snapshot");
        };
        assert_eq!(snap.id, "toolu_wake_6");
        assert_eq!(snap.reason, "second");

        run_cancel.cancel();
    }

    #[tokio::test]
    async fn background_watch_fires_after_a_registered_command_exits() {
        let injector = CapturingInjector::default();
        let jobs = ScheduledJobRegistry::new(Arc::new(injector.clone()));
        let run_cancel = CancellationToken::new();

        let commands = Arc::new(CommandRegistry::new(
            Duration::from_mins(1),
            std::env::temp_dir().join(format!("sched-{}", uuid::Uuid::new_v4())),
            "task-1".to_string(),
        ));
        commands
            .run(CommandSpec {
                command: "sleep 0.3".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");

        let job = BackgroundWatchJob::arm(Arc::clone(&commands))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));
        assert!(jobs.register(Box::new(job), run_cancel.clone()).await);

        assert!(
            injector.snapshot().is_empty(),
            "watch must wait for the command to finish"
        );

        let ok = wait_until(Duration::from_secs(5), || !injector.snapshot().is_empty()).await;
        assert!(ok, "expected a wake injection after the command exited");
        assert!(
            injector.snapshot()[0]
                .1
                .contains("background command has finished"),
            "wake message should name the finished command"
        );
        assert!(
            jobs.is_empty().await,
            "watch should self-clean after firing"
        );

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
            Instant::now() + Duration::from_mins(1),
        );
        let second = WakeupJob::new_for_test(
            "toolu_dupe".to_string(),
            "r2".to_string(),
            "p2".to_string(),
            Instant::now() + Duration::from_mins(1),
        );

        assert!(registry.register(Box::new(first), run_cancel.clone()).await);
        assert!(
            !registry
                .register(Box::new(second), run_cancel.clone())
                .await
        );
        assert_eq!(registry.len().await, 1);

        run_cancel.cancel();
    }
}
