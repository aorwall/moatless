//! The desired-state reconciler — the single async recovery owner for level
//! states.
//!
//! One tick: enumerate DB candidates (desired-Running participants + incomplete
//! markers) ∪ pending-message wakes ∪ Tasks the fleet observation shows as live,
//! observe the fleet once, and for each candidate compare intent against
//! observed via [`reconcile::decide`](super::reconcile::decide). A divergence is
//! confirmed with an authoritative per-Task read, then driven through a lifecycle
//! operation — never a direct runtime mutation. Retries are bounded by a
//! leader-local [`AttemptBudget`] with exponential backoff; permanent failures
//! and Closed are never retried.
//!
//! The reconciler holds no lease logic itself: the loop that drives
//! [`Reconciler::reconcile_once`] is lease-elected on Kubernetes and lease-free
//! single-replica on Docker.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use crate::message::dao::MessageRepository;
use crate::task::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};

use super::attempt::AttemptBudget;
use super::convergence::{ReconcileAction, SandboxObservedStatus};
use super::orchestrator::{Lifecycle, LifecycleStore, WallClock};
use super::reconcile::{decide, ReconcileInput};
use super::types::ArchiveRetention;

/// A runtime-status read failed. The reconciler logs and skips the Task this
/// tick rather than acting on missing observation.
#[derive(Debug, thiserror::Error)]
#[error("observe: {0}")]
pub struct ObserveError(pub String);

/// The reconciler's runtime-status source, batch-first.
#[async_trait]
pub trait FleetObserver: Send + Sync {
    /// One label-selector LIST per tick: coarse observed status for every Task
    /// with live resources. Tasks absent from the map have no resources and are
    /// treated as [`SandboxObservedStatus::Absent`].
    async fn observe_fleet(&self)
        -> Result<HashMap<String, SandboxObservedStatus>, ObserveError>;

    /// Authoritative per-Task status, read to confirm a divergence before acting
    /// so the reconciler never drives an operation off stale coarse data. The
    /// intent fields are the ones the provider's live-status projection needs.
    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxObservedStatus, ObserveError>;
}

/// Reconciler tuning.
#[derive(Debug, Clone)]
pub struct ReconcilerConfig {
    /// Attempts per operation generation before a transient failure gives up
    /// until a new caller operation or a wake.
    pub attempt_cap: u32,
    /// Grace after an intent change before the reconciler acts, so a caller's own
    /// in-flight operation converges without the reconciler racing it.
    pub grace: Duration,
    /// Exponential-backoff base and ceiling between transient retries.
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            attempt_cap: 5,
            grace: Duration::from_secs(30),
            backoff_base: Duration::from_secs(2),
            backoff_max: Duration::from_mins(1),
        }
    }
}

/// Per-Task retry state: the leader-local budget plus the next instant the Task
/// is eligible for another attempt (exponential backoff).
struct TaskRetry {
    budget: AttemptBudget,
    next_eligible: Option<DateTime<Utc>>,
}

impl TaskRetry {
    fn new(cap: u32) -> Self {
        Self {
            budget: AttemptBudget::new(cap),
            next_eligible: None,
        }
    }
}

/// The desired-state reconciler.
pub struct Reconciler {
    lifecycle: Lifecycle,
    store: Arc<dyn LifecycleStore>,
    messages: Arc<dyn MessageRepository>,
    observer: Arc<dyn FleetObserver>,
    clock: Arc<dyn WallClock>,
    retries: Mutex<HashMap<String, TaskRetry>>,
    config: ReconcilerConfig,
}

impl Reconciler {
    pub fn new(
        lifecycle: Lifecycle,
        store: Arc<dyn LifecycleStore>,
        messages: Arc<dyn MessageRepository>,
        observer: Arc<dyn FleetObserver>,
        clock: Arc<dyn WallClock>,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            lifecycle,
            store,
            messages,
            observer,
            clock,
            retries: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// One reconcile pass over the whole fleet.
    pub async fn reconcile_once(&self) {
        let now = self.clock.now();

        // One fleet observation per tick — also a candidate source: a live
        // resource whose intent is not Running is a divergence to converge.
        let coarse = match self.observer.observe_fleet().await {
            Ok(m) => m,
            Err(e) => {
                warn!(error = %e, "reconciler: fleet observation failed; skipping tick");
                return;
            }
        };

        let pending_map = self.pending_messages().await;

        let mut candidates: HashSet<String> = match self.store.reconcile_candidates().await {
            Ok(c) => c.into_iter().collect(),
            Err(e) => {
                warn!(error = %e, "reconciler: candidate enumeration failed");
                HashSet::new()
            }
        };
        candidates.extend(pending_map.keys().cloned());
        candidates.extend(coarse.keys().cloned());

        for task_id in &candidates {
            let pending_at = pending_map.get(task_id).copied();
            self.reconcile_task(task_id, &coarse, pending_at, now).await;
        }

        // Drop retry state for Tasks no longer in scope so the map tracks the
        // live candidate set rather than growing without bound.
        self.retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|k, _| candidates.contains(k));
    }

    async fn pending_messages(&self) -> HashMap<String, DateTime<Utc>> {
        match self.messages.tasks_with_pending_user_messages().await {
            Ok(rows) => rows
                .into_iter()
                .map(|r| (r.task_id, r.newest_created_at))
                .collect(),
            Err(e) => {
                warn!(error = %e, "reconciler: pending-message scan failed");
                HashMap::new()
            }
        }
    }

    async fn reconcile_task(
        &self,
        task_id: &str,
        coarse: &HashMap<String, SandboxObservedStatus>,
        pending_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) {
        let Some(intent) = self.store.load(task_id).await.ok().flatten() else {
            return;
        };

        // Give a freshly-changed intent its grace window — a caller's own
        // detached operation is likely still converging.
        if now.signed_duration_since(intent.desired_state_updated_at)
            < chrono::Duration::from_std(self.config.grace).unwrap_or_else(|_| chrono::Duration::zero())
        {
            return;
        }

        let observed_coarse = coarse
            .get(task_id)
            .copied()
            .unwrap_or(SandboxObservedStatus::Absent);
        let coarse_decision = decide(&ReconcileInput {
            intent: intent.clone(),
            observed: observed_coarse,
            newest_pending_message_at: pending_at,
        });
        if !coarse_decision.action.is_operation() {
            return;
        }

        // Confirm the divergence authoritatively before acting.
        let observed = match self
            .observer
            .observe(task_id, intent.status, intent.desired_state)
            .await
        {
            Ok(o) => o,
            Err(e) => {
                debug!(task_id, error = %e, "reconciler: confirm observation failed; skipping");
                return;
            }
        };
        let decision = decide(&ReconcileInput {
            intent: intent.clone(),
            observed,
            newest_pending_message_at: pending_at,
        });
        if !decision.action.is_operation() {
            return;
        }

        if !self.retry_gate(task_id, intent.marker.map(|m| m.generation.value()), decision.woken, now) {
            return;
        }

        let result = match decision.action {
            ReconcileAction::Start => self.lifecycle.start(task_id).await,
            ReconcileAction::Stop => {
                let reason = intent.stop_reason.unwrap_or(SandboxStopReason::System);
                self.lifecycle.stop(task_id, reason).await
            }
            ReconcileAction::Remove => {
                let retention = intent.retention.unwrap_or(ArchiveRetention::RetainArchives);
                self.lifecycle.remove(task_id, retention).await
            }
            ReconcileAction::None | ReconcileAction::Wait => return,
        };

        self.record_outcome(task_id, decision.action, result, now);
    }

    /// Decide whether to drive an operation this tick under the attempt cap +
    /// backoff. Mutates the Task's budget (begin/reset) before gating.
    fn retry_gate(&self, task_id: &str, generation: Option<i64>, woken: bool, now: DateTime<Utc>) -> bool {
        let mut guard = self.retries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));

        if let Some(g) = generation {
            retry.budget.begin(super::types::OperationGeneration(g));
        }
        if woken {
            retry.budget.reset_for_wake();
            retry.next_eligible = None;
        }

        // Backing off between transient retries.
        if let Some(t) = retry.next_eligible {
            if now < t {
                return false;
            }
        }
        // A tracked generation whose cap is exhausted (or blocked by a permanent
        // failure) waits for a new caller operation or a wake. A markerless first
        // attempt has no generation to key on and is always allowed — it
        // establishes the marker the next tick keys against.
        if generation.is_some() && retry.budget.exhausted() {
            return false;
        }
        true
    }

    fn record_outcome(
        &self,
        task_id: &str,
        action: ReconcileAction,
        result: Result<super::outcome::LifecycleOutcome, super::error::LifecycleError>,
        now: DateTime<Utc>,
    ) {
        let mut guard = self.retries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));

        match result {
            Ok(_) => {
                retry.next_eligible = None;
                debug!(task_id, ?action, "reconciler: drove lifecycle operation");
            }
            Err(e) => {
                let class = e.class();
                if class.is_transient() {
                    retry.budget.record_failure();
                    let delay = retry
                        .budget
                        .backoff(self.config.backoff_base, self.config.backoff_max);
                    retry.next_eligible = Some(
                        now + chrono::Duration::from_std(delay)
                            .unwrap_or_else(|_| chrono::Duration::seconds(60)),
                    );
                    warn!(task_id, ?action, error = %e, "reconciler: transient failure; will retry");
                } else {
                    // Permanent (bad plan, Closed) or contention: never retried
                    // for this generation.
                    retry.budget.block();
                    info!(task_id, ?action, error = %e, "reconciler: non-transient failure; not retrying this generation");
                }
            }
        }
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;

    use sqlx::PgPool;

    use crate::message::dao::PgMessageRepository;

    use super::super::dao::PgLifecycleStore;
    use super::super::events::{EventFact, LifecycleEvent};
    use super::super::lock::PgLifecycleLock;
    use super::super::orchestrator::{
        AgentStopPort, IntentWrite, LifecycleEventSink, SystemWallClock,
    };
    use super::super::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
    use super::super::test_support::seed_task_with as seed;
    use super::super::types::{ObservedStatus, OperationKind};
    use super::super::{CorrelationMarker, OperationGeneration};

    // ---- Recording ports ----

    #[derive(Default)]
    struct RuntimeCalls {
        ensure: u32,
        stop: u32,
        remove: u32,
    }

    struct RecordingRuntime {
        calls: Mutex<RuntimeCalls>,
        ensure_result: Mutex<Option<Result<RuntimeOutcome, super::super::error::RuntimeError>>>,
    }

    impl RecordingRuntime {
        fn new() -> Self {
            Self {
                calls: Mutex::new(RuntimeCalls::default()),
                ensure_result: Mutex::new(None),
            }
        }
        fn calls(&self) -> (u32, u32, u32) {
            let c = self.calls.lock().unwrap();
            (c.ensure, c.stop, c.remove)
        }
    }

    #[async_trait]
    impl SandboxRuntime for RecordingRuntime {
        async fn ensure_running(&self, _t: &str, _c: RuntimeOperationContext) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            self.calls.lock().unwrap().ensure += 1;
            self.ensure_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(RuntimeOutcome::applied(ObservedStatus::Starting)))
        }
        async fn stop(&self, _t: &str, _c: RuntimeOperationContext) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            self.calls.lock().unwrap().stop += 1;
            Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
        }
        async fn restart(&self, _t: &str, _c: RuntimeOperationContext) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn redeploy(&self, _t: &str, _c: RuntimeOperationContext) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn remove(&self, _t: &str, _r: ArchiveRetention, _c: RuntimeOperationContext) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            self.calls.lock().unwrap().remove += 1;
            Ok(RuntimeOutcome::applied(ObservedStatus::Removed))
        }
    }

    #[derive(Default)]
    struct CollectSink {
        facts: Mutex<Vec<EventFact>>,
    }
    #[async_trait]
    impl LifecycleEventSink for CollectSink {
        async fn publish(&self, _t: &str, event: &LifecycleEvent) {
            self.facts.lock().unwrap().push(event.fact);
        }
    }

    struct NoopAgentStop;
    #[async_trait]
    impl AgentStopPort for NoopAgentStop {
        async fn stop_agent(&self, _t: &str) {}
    }

    /// A fixed fleet + confirm status the whole tick sees.
    struct FixedObserver {
        fleet: HashMap<String, SandboxObservedStatus>,
        confirm: SandboxObservedStatus,
    }
    #[async_trait]
    impl FleetObserver for FixedObserver {
        async fn observe_fleet(&self) -> Result<HashMap<String, SandboxObservedStatus>, ObserveError> {
            Ok(self.fleet.clone())
        }
        async fn observe(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<SandboxObservedStatus, ObserveError> {
            Ok(self.confirm)
        }
    }

    fn build(
        pool: PgPool,
        lock_pool: PgPool,
        runtime: Arc<RecordingRuntime>,
        observer: Arc<dyn FleetObserver>,
        grace: Duration,
    ) -> (Reconciler, Arc<CollectSink>) {
        let store = Arc::new(PgLifecycleStore::new(pool.clone()));
        let sink = Arc::new(CollectSink::default());
        let lifecycle = Lifecycle::new(
            store.clone(),
            runtime,
            Arc::new(NoopAgentStop),
            sink.clone(),
            Arc::new(PgLifecycleLock::new(lock_pool)),
            Arc::new(SystemWallClock),
        );
        let reconciler = Reconciler::new(
            lifecycle,
            store,
            Arc::new(PgMessageRepository(pool)),
            observer,
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace,
                ..Default::default()
            },
        );
        (reconciler, sink)
    }

    async fn seed_pending_user_message(pool: &PgPool, task_id: &str, created_at: DateTime<Utc>) {
        sqlx::query(
            r#"INSERT INTO task_messages
                   (task_id, uuid, message_type, user_type, timestamp, message_role,
                    message_content, created_at)
               VALUES ($1, $2, 'user', 'human', $3, 'user', '{}'::jsonb, $4)"#,
        )
        .bind(task_id)
        .bind(format!("{task_id}-msg"))
        .bind(created_at.to_rfc3339())
        .bind(created_at)
        .execute(pool)
        .await
        .expect("seed pending message");
    }

    fn observer(
        fleet: &[(&str, SandboxObservedStatus)],
        confirm: SandboxObservedStatus,
    ) -> Arc<dyn FleetObserver> {
        Arc::new(FixedObserver {
            fleet: fleet.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            confirm,
        })
    }

    #[sqlx::test]
    async fn starts_a_desired_running_task_observed_absent(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().0, 1, "ensure_running driven once");
        assert!(sink.facts.lock().unwrap().contains(&EventFact::Initializing));
    }

    #[sqlx::test]
    async fn wakes_a_stopped_task_with_a_newer_pending_message(pool: PgPool) {
        seed(&pool, "t1", "open", "stopped").await;
        // A message strictly newer than the stop supersedes it.
        seed_pending_user_message(&pool, "t1", Utc::now() + chrono::Duration::hours(1)).await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().0, 1, "wake drove a start");
        let desired: String =
            sqlx::query_scalar("SELECT sandbox_desired_state FROM tasks WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(desired, "running", "wake flipped intent to running");
    }

    #[sqlx::test]
    async fn stops_a_desired_stopped_task_observed_ready(pool: PgPool) {
        // Desired Stopped, but the fleet shows it live — surfaced as a candidate
        // by the fleet LIST, not by the DB candidate query.
        seed(&pool, "t1", "open", "stopped").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[("t1", SandboxObservedStatus::Ready)], SandboxObservedStatus::Ready),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().1, 1, "stop driven once");
    }

    #[sqlx::test]
    async fn removes_when_desired_removed_and_still_live(pool: PgPool) {
        seed(&pool, "t1", "closed", "removed").await;
        // Persist a Removed intent with a marker so it is a DB candidate.
        let store = PgLifecycleStore::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: crate::task::models::SandboxDesiredState::Removed,
                    stop_reason: Some(SandboxStopReason::Cleanup),
                    retention: Some(ArchiveRetention::DeleteArchives),
                    marker: CorrelationMarker::new(
                        OperationKind::Remove,
                        OperationGeneration(1),
                        Utc::now(),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[("t1", SandboxObservedStatus::Ready)], SandboxObservedStatus::Ready),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().2, 1, "remove driven once");
    }

    #[sqlx::test]
    async fn confirm_read_overrides_stale_coarse_and_skips(pool: PgPool) {
        // Coarse LIST omits the task (→ Absent → would start), but the
        // authoritative confirm read says Ready: converged, drive nothing.
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Ready),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls(), (0, 0, 0), "no operation driven");
        assert!(sink.facts.lock().unwrap().is_empty());
    }

    #[sqlx::test]
    async fn grace_window_defers_a_freshly_changed_intent(pool: PgPool) {
        // Seed sets sandbox_desired_state_updated_at to NOW(); a long grace means
        // the reconciler leaves the caller's own in-flight op alone this tick.
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::from_secs(3600),
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls(), (0, 0, 0), "grace deferred the operation");
    }
}
