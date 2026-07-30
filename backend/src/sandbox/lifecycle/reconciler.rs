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

use crate::message::dao::MessageDao;
use crate::task::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};

use super::attempt::AttemptBudget;
use super::convergence::{ReconcileAction, SandboxObservedStatus};
use super::orchestrator::{Lifecycle, LifecycleDao, WallClock};
use super::reconcile::{decide, ReconcileInput};
use super::types::{ArchiveRetention, OperationGeneration};

/// Runtime observation paired with the lifecycle generation stamped on the
/// currently observed pod/container. Legacy resources have no generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeObservation {
    pub status: SandboxObservedStatus,
    pub generation: Option<OperationGeneration>,
}

impl RuntimeObservation {
    pub fn new(status: SandboxObservedStatus, generation: Option<OperationGeneration>) -> Self {
        Self { status, generation }
    }
}

impl From<SandboxObservedStatus> for RuntimeObservation {
    fn from(status: SandboxObservedStatus) -> Self {
        Self::new(status, None)
    }
}

/// A runtime-status read failed. The reconciler logs and skips the Task this
/// tick rather than acting on missing observation. Keeps the underlying source
/// (a K8s/Docker observation error) so fleet-LIST debugging retains the chain.
#[derive(Debug, thiserror::Error)]
#[error("{context}")]
pub struct ObserveError {
    pub context: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl ObserveError {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            source: None,
        }
    }

    pub fn with_source(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// The reconciler's runtime-status source, batch-first.
#[async_trait]
pub trait FleetObserver: Send + Sync {
    /// One label-selector LIST per tick: coarse observed status for every Task
    /// with live resources. Tasks absent from the map have no resources and are
    /// treated as [`SandboxObservedStatus::Absent`].
    async fn observe_fleet(&self) -> Result<HashMap<String, RuntimeObservation>, ObserveError>;

    /// Authoritative per-Task status, read to confirm a divergence before acting
    /// so the reconciler never drives an operation off stale coarse data. The
    /// intent fields are the ones the observer's live-status projection needs.
    async fn observe(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<RuntimeObservation, ObserveError>;
}

/// Delivers a pending-message wake to an already-Ready sandbox over gRPC.
///
/// A wake whose gRPC notification is lost while the sandbox is already Ready is
/// not rescued by a start (the sandbox never restarts), so the reconciler
/// re-notifies a woken Ready Task until its pending rows are acknowledged.
/// Idempotent and fire-and-forget.
#[async_trait]
pub trait NotifyPort: Send + Sync {
    async fn notify(&self, task_id: &str);
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
    /// The newest pending-message timestamp that has already reset this budget.
    /// A wake resets the cap only when a *newer* message arrives — the same
    /// pending row (still undelivered every tick until Ready) must not.
    last_wake_at: Option<DateTime<Utc>>,
    /// Ready re-notification backoff, keyed off the same wake advance as the
    /// operation cap.
    next_notify: Option<DateTime<Utc>>,
    notify_attempts: u32,
}

impl TaskRetry {
    fn new(cap: u32) -> Self {
        Self {
            budget: AttemptBudget::new(cap),
            next_eligible: None,
            last_wake_at: None,
            next_notify: None,
            notify_attempts: 0,
        }
    }
}

/// The desired-state reconciler.
pub struct Reconciler {
    lifecycle: Lifecycle,
    store: Arc<dyn LifecycleDao>,
    messages: Arc<dyn MessageDao>,
    observer: Arc<dyn FleetObserver>,
    notify: Arc<dyn NotifyPort>,
    clock: Arc<dyn WallClock>,
    retries: Mutex<HashMap<String, TaskRetry>>,
    config: ReconcilerConfig,
}

impl Reconciler {
    pub(crate) fn new(
        lifecycle: Lifecycle,
        store: Arc<dyn LifecycleDao>,
        messages: Arc<dyn MessageDao>,
        observer: Arc<dyn FleetObserver>,
        notify: Arc<dyn NotifyPort>,
        clock: Arc<dyn WallClock>,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            lifecycle,
            store,
            messages,
            observer,
            notify,
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
        coarse: &HashMap<String, RuntimeObservation>,
        pending_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) {
        let mut intent = match self.store.load(task_id).await {
            Ok(Some(intent)) => intent,
            Ok(None) => return,
            Err(e) => {
                warn!(task_id, error = %e, "reconciler: intent load failed; skipping task");
                return;
            }
        };

        // Give a freshly-changed intent its grace window — a caller's own
        // detached operation is likely still converging. An oversized configured
        // grace saturates to MAX (fail closed: keep waiting) rather than
        // collapsing to no window.
        if now.signed_duration_since(intent.desired_state_updated_at)
            < chrono::Duration::from_std(self.config.grace).unwrap_or(chrono::Duration::MAX)
        {
            return;
        }

        let observed_coarse = coarse
            .get(task_id)
            .copied()
            .unwrap_or_else(|| SandboxObservedStatus::Absent.into());
        let coarse_decision = decide(&ReconcileInput {
            intent: intent.clone(),
            observed: observed_coarse.status,
            newest_pending_message_at: pending_at,
        });
        // A woken Task always confirms — even when coarse shows no operation it may
        // be an already-Ready sandbox owed a re-notification.
        if intent.marker.is_none()
            && !coarse_decision.woken
            && !coarse_decision.action.is_operation()
        {
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

        if let Some(marker) = intent.marker {
            let replacement_ready = observed.status == SandboxObservedStatus::Ready
                && observed.generation == Some(marker.generation);
            let ack = super::marker::acknowledge(
                &marker,
                super::marker::AckContext {
                    observed: observed.status,
                    replacement_ready,
                    ..Default::default()
                },
            );
            if ack.clears() {
                if let Err(e) = self.store.clear_marker(task_id, marker.generation).await {
                    warn!(task_id, error = %e, "reconciler: failed to acknowledge converged marker");
                }
                return;
            }

            if marker.kind.is_edge() {
                if !super::marker::is_stale(&marker, now, super::marker::EDGE_ABANDON_AFTER) {
                    // Caller-owned edges are never replayed. Give the fresh
                    // replacement time to appear before level recovery acts.
                    return;
                }
                if observed.status == SandboxObservedStatus::Starting
                    && observed.generation == Some(marker.generation)
                {
                    // The replacement generation is still advancing. Its age
                    // cannot by itself prove that the edge was abandoned.
                    return;
                }
                if let Err(e) = self
                    .lifecycle
                    .abandon_edge(task_id, marker.kind, marker.generation)
                    .await
                {
                    warn!(task_id, error = %e, "reconciler: failed to durably abandon stale edge");
                    return;
                }
                // The atomic abandonment commit cleared the old edge marker.
                // Any level recovery below must allocate a fresh generation.
                intent.marker = None;
            }
        }

        let decision = decide(&ReconcileInput {
            intent: intent.clone(),
            observed: observed.status,
            newest_pending_message_at: pending_at,
        });

        // A newly observed wake resets the operation cap and the notify backoff
        // (never the same pending row that stays undelivered every tick).
        if decision.woken {
            self.note_wake(task_id, pending_at);
        }

        if decision.action.is_operation() {
            if !self.retry_gate(task_id, intent.marker.map(|m| m.generation.value()), now) {
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
            if let Some(last_error) = self.record_outcome(task_id, decision.action, result, now) {
                self.publish_recovery_exhausted(task_id, decision.action, &last_error)
                    .await;
            }
        } else if decision.woken && observed.status == SandboxObservedStatus::Ready {
            // The wake landed on an already-Ready sandbox: re-notify under the
            // per-Task backoff until the pending rows are acknowledged (which
            // drops the Task from the pending set and ends the wake).
            self.maybe_notify(task_id, now).await;
        }
    }

    /// Reset the operation cap and notify backoff when a genuinely new message
    /// arrives — the same undelivered pending row (present every tick until
    /// Ready) must not.
    fn note_wake(&self, task_id: &str, wake_at: Option<DateTime<Utc>>) {
        let Some(wake_at) = wake_at else { return };
        let mut guard = self
            .retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));
        if retry.last_wake_at.is_none_or(|prev| wake_at > prev) {
            retry.budget.reset_for_wake();
            retry.next_eligible = None;
            retry.next_notify = None;
            retry.notify_attempts = 0;
            retry.last_wake_at = Some(wake_at);
        }
    }

    /// Re-notify a Ready sandbox of its pending wake, spaced by leader-local
    /// exponential backoff keyed by Task.
    async fn maybe_notify(&self, task_id: &str, now: DateTime<Utc>) {
        let due = {
            let mut guard = self
                .retries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let retry = guard
                .entry(task_id.to_string())
                .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));
            if retry.next_notify.is_some_and(|t| now < t) {
                false
            } else {
                retry.notify_attempts = retry.notify_attempts.saturating_add(1);
                let delay = super::attempt::backoff(
                    retry.notify_attempts,
                    self.config.backoff_base,
                    self.config.backoff_max,
                );
                retry.next_notify = Some(
                    now + chrono::Duration::from_std(delay)
                        .unwrap_or_else(|_| chrono::Duration::seconds(60)),
                );
                true
            }
        };
        if due {
            debug!(
                task_id,
                "reconciler: re-notifying Ready sandbox of pending wake"
            );
            self.notify.notify(task_id).await;
        }
    }

    /// Decide whether to drive an operation this tick under the attempt cap +
    /// backoff. Mutates the Task's budget (`begin`) before gating. Wake resets
    /// are handled by [`note_wake`](Self::note_wake) before this runs.
    fn retry_gate(&self, task_id: &str, generation: Option<i64>, now: DateTime<Utc>) -> bool {
        let mut guard = self
            .retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));

        if let Some(g) = generation {
            retry.budget.begin(super::types::OperationGeneration(g));
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

    /// Record one operation's result against the Task's budget.
    ///
    /// Returns the rendered last error when this call is the one that **spends**
    /// the budget, so the caller can publish the terminal `recovery_exhausted`
    /// fact exactly once per generation. Returns `None` otherwise, including on
    /// a budget that was already spent.
    fn record_outcome(
        &self,
        task_id: &str,
        action: ReconcileAction,
        result: Result<super::outcome::LifecycleOutcome, super::error::LifecycleError>,
        now: DateTime<Utc>,
    ) -> Option<String> {
        let mut guard = self
            .retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));

        match result {
            Ok(_) => {
                retry.next_eligible = None;
                debug!(task_id, ?action, "reconciler: drove lifecycle operation");
                None
            }
            Err(e) => {
                let was_exhausted = retry.budget.exhausted();
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
                (!was_exhausted && retry.budget.exhausted()).then(|| e.to_string())
            }
        }
    }

    /// Publish the terminal give-up fact. Without it, a Task whose recovery is
    /// spent shows one deduped `error` and then silence forever — the reader
    /// cannot tell "still retrying" from "never coming back on its own".
    ///
    /// Scoped to the marker's generation, so it needs one: a first attempt that
    /// spends the budget before any marker is persisted has nothing stable to
    /// key on and is left to the logs.
    async fn publish_recovery_exhausted(
        &self,
        task_id: &str,
        action: ReconcileAction,
        last_error: &str,
    ) {
        let marker = match self.store.load(task_id).await {
            Ok(Some(intent)) => intent.marker,
            Ok(None) => None,
            Err(e) => {
                warn!(task_id, error = %e, "reconciler: intent load failed; cannot publish recovery_exhausted");
                return;
            }
        };
        let Some(marker) = marker else {
            warn!(
                task_id,
                ?action,
                "reconciler: recovery exhausted with no marker; not publishing"
            );
            return;
        };
        self.lifecycle
            .publish_recovery_exhausted(
                task_id,
                marker.kind,
                marker.generation,
                self.config.attempt_cap,
                last_error,
            )
            .await;
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;

    use sqlx::PgPool;

    use crate::message::dao::PgMessageDao;

    use super::super::dao::PgLifecycleDao;
    use super::super::events::{EventFact, LifecycleEvent};
    use super::super::lock::PgLifecycleLock;
    use super::super::orchestrator::{
        AgentStopPort, IntentWrite, LifecycleEventSink, SystemWallClock,
    };
    use super::super::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
    use super::super::test_support::{seed_bare_task, seed_task_with as seed};
    use super::super::types::{CorrelationMarker, OperationGeneration};
    use super::super::types::{ObservedStatus, OperationKind};

    // ---- Recording ports ----

    #[derive(Default)]
    struct RuntimeCalls {
        ensure: u32,
        stop: u32,
        remove: u32,
    }

    struct RecordingRuntime {
        calls: Mutex<RuntimeCalls>,
        /// When set, every `ensure_running` returns this transient failure.
        fail_ensure: bool,
    }

    impl RecordingRuntime {
        fn new() -> Self {
            Self {
                calls: Mutex::new(RuntimeCalls::default()),
                fail_ensure: false,
            }
        }
        fn failing_ensure() -> Self {
            Self {
                calls: Mutex::new(RuntimeCalls::default()),
                fail_ensure: true,
            }
        }
        fn calls(&self) -> (u32, u32, u32) {
            let c = self.calls.lock().unwrap();
            (c.ensure, c.stop, c.remove)
        }
    }

    #[async_trait]
    impl SandboxRuntime for RecordingRuntime {
        async fn ensure_running(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            self.calls.lock().unwrap().ensure += 1;
            if self.fail_ensure {
                return Err(super::super::error::RuntimeError::Infrastructure(
                    "boom".into(),
                ));
            }
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn stop(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            self.calls.lock().unwrap().stop += 1;
            Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
        }
        async fn restart(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn redeploy(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn remove(
            &self,
            _t: &str,
            _r: ArchiveRetention,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, super::super::error::RuntimeError> {
            self.calls.lock().unwrap().remove += 1;
            Ok(RuntimeOutcome::applied(ObservedStatus::Removed))
        }
    }

    #[derive(Default)]
    struct CollectSink {
        facts: Mutex<Vec<EventFact>>,
        published: Mutex<Vec<LifecycleEvent>>,
    }
    #[async_trait]
    impl LifecycleEventSink for CollectSink {
        async fn publish(&self, _t: &str, event: &LifecycleEvent) {
            self.facts.lock().unwrap().push(event.fact);
            self.published.lock().unwrap().push(event.clone());
        }

        async fn publish_diagnostics(
            &self,
            _task_id: &str,
            _generation: OperationGeneration,
            _observed: ObservedStatus,
            _diagnostics: &super::super::outcome::Diagnostics,
        ) {
        }
    }

    struct NoopAgentStop;
    #[async_trait]
    impl AgentStopPort for NoopAgentStop {
        async fn stop_agent(&self, _t: &str, _reason: SandboxStopReason) {}
    }

    #[derive(Default)]
    struct RecordingNotify {
        count: Mutex<u32>,
    }
    #[async_trait]
    impl NotifyPort for RecordingNotify {
        async fn notify(&self, _t: &str) {
            *self.count.lock().unwrap() += 1;
        }
    }

    /// A fixed fleet + confirm status the whole tick sees.
    struct FixedObserver {
        fleet: HashMap<String, RuntimeObservation>,
        confirm: RuntimeObservation,
    }
    #[async_trait]
    impl FleetObserver for FixedObserver {
        async fn observe_fleet(&self) -> Result<HashMap<String, RuntimeObservation>, ObserveError> {
            Ok(self.fleet.clone())
        }
        async fn observe(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<RuntimeObservation, ObserveError> {
            Ok(self.confirm)
        }
    }

    /// A clock that jumps forward a fixed step on every read, so successive
    /// reconcile ticks always land past the exponential backoff window.
    struct StepClock {
        base: DateTime<Utc>,
        step: chrono::Duration,
        reads: Mutex<i32>,
    }
    impl WallClock for StepClock {
        fn now(&self) -> DateTime<Utc> {
            let mut r = self.reads.lock().unwrap();
            let t = self.base + self.step * *r;
            *r += 1;
            t
        }
    }

    fn build(
        pool: PgPool,
        lock_pool: PgPool,
        runtime: Arc<RecordingRuntime>,
        observer: Arc<dyn FleetObserver>,
        grace: Duration,
    ) -> (Reconciler, Arc<CollectSink>) {
        build_with(
            pool,
            lock_pool,
            runtime,
            observer,
            Arc::new(RecordingNotify::default()),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace,
                ..Default::default()
            },
        )
    }

    fn build_with(
        pool: PgPool,
        lock_pool: PgPool,
        runtime: Arc<RecordingRuntime>,
        observer: Arc<dyn FleetObserver>,
        notify: Arc<dyn NotifyPort>,
        clock: Arc<dyn WallClock>,
        config: ReconcilerConfig,
    ) -> (Reconciler, Arc<CollectSink>) {
        let store = Arc::new(PgLifecycleDao::new(pool.clone()));
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
            Arc::new(PgMessageDao(pool)),
            observer,
            notify,
            clock,
            config,
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
            fleet: fleet
                .iter()
                .map(|(k, v)| (k.to_string(), (*v).into()))
                .collect(),
            confirm: confirm.into(),
        })
    }

    fn observer_with_generation(
        task_id: &str,
        status: SandboxObservedStatus,
        generation: Option<OperationGeneration>,
    ) -> Arc<dyn FleetObserver> {
        let observation = RuntimeObservation::new(status, generation);
        Arc::new(FixedObserver {
            fleet: HashMap::from([(task_id.to_string(), observation)]),
            confirm: observation,
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
        assert!(sink
            .facts
            .lock()
            .unwrap()
            .contains(&EventFact::Initializing));
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
            sqlx::query_scalar("SELECT desired_state FROM sandboxes WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(desired, "running", "wake flipped intent to running");
    }

    #[sqlx::test]
    async fn pending_message_wakes_a_task_without_a_sandbox_row(pool: PgPool) {
        seed_bare_task(&pool, "t1", "open").await;
        seed_pending_user_message(&pool, "t1", Utc::now()).await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().0, 1, "durable pending Message drove start");
        let desired: String =
            sqlx::query_scalar("SELECT desired_state FROM sandboxes WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(desired, "running");
    }

    #[sqlx::test]
    async fn a_persistent_wake_still_honours_the_attempt_cap(pool: PgPool) {
        // One pending message supersedes the stop, but the start keeps failing
        // transiently. The same undelivered row must not reset the budget every
        // tick — retries stay bounded by the cap instead of running forever.
        seed(&pool, "t1", "open", "stopped").await;
        seed_pending_user_message(&pool, "t1", Utc::now() + chrono::Duration::hours(1)).await;
        let runtime = Arc::new(RecordingRuntime::failing_ensure());
        // Each tick jumps an hour so the exponential backoff never gates —
        // isolating the wake-reset behaviour as the only thing bounding retries.
        let clock = Arc::new(StepClock {
            base: DateTime::from_timestamp(2_000_000_000, 0).unwrap(),
            step: chrono::Duration::hours(1),
            reads: Mutex::new(0),
        });
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Arc::new(RecordingNotify::default()),
            clock,
            ReconcilerConfig {
                grace: Duration::ZERO,
                attempt_cap: 3,
                ..Default::default()
            },
        );

        for _ in 0..8 {
            rec.reconcile_once().await;
        }

        // Bounded near the cap. Without the newest-message guard the wake would
        // reset every tick and drive all 8.
        let ensure = runtime.calls().0;
        assert!(
            ensure <= 5,
            "ensure driven {ensure} times; the cap should bound it"
        );
        assert!(
            ensure >= 3,
            "ensure should retry up to the cap, got {ensure}"
        );
    }

    #[sqlx::test]
    async fn spending_the_attempt_cap_publishes_one_recovery_exhausted_fact(pool: PgPool) {
        // Desired Running, nothing running, every start failing: the retries all
        // reuse the generation, so their `Error` facts dedup to one id and the
        // give-up is otherwise indistinguishable from silence. The terminal fact
        // is what makes "not coming back on its own" readable.
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::failing_ensure());
        let clock = Arc::new(StepClock {
            base: DateTime::from_timestamp(2_000_000_000, 0).unwrap(),
            step: chrono::Duration::hours(1),
            reads: Mutex::new(0),
        });
        let (rec, sink) = build_with(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Arc::new(RecordingNotify::default()),
            clock,
            ReconcilerConfig {
                grace: Duration::ZERO,
                attempt_cap: 2,
                ..Default::default()
            },
        );

        for _ in 0..6 {
            rec.reconcile_once().await;
        }

        let published = sink.published.lock().unwrap().clone();
        let exhausted: Vec<_> = published
            .iter()
            .filter(|e| e.fact == EventFact::RecoveryExhausted)
            .collect();
        assert_eq!(
            exhausted.len(),
            1,
            "exactly one terminal fact per spent generation, got {}",
            exhausted.len()
        );
        let payload = exhausted[0].payload();
        assert_eq!(
            payload.lifecycle_event.as_deref(),
            Some("recovery_exhausted")
        );
        // It names the operation and carries the last failure, not a placeholder.
        assert!(
            payload.message.contains("start"),
            "unexpected message: {}",
            payload.message
        );
        assert!(payload
            .error
            .as_deref()
            .is_some_and(|e| e.contains("2 attempts failed")));
    }

    #[sqlx::test]
    async fn re_notifies_a_woken_ready_sandbox_bounded_by_backoff(pool: PgPool) {
        // Desired Running, observed Ready → converged, so no lifecycle operation.
        // A pending user message keeps the Task woken: the wake's gRPC notify may
        // have been lost while the sandbox was already Ready, so the reconciler
        // re-notifies. Backoff bounds it — a second immediate tick is spaced out.
        seed(&pool, "t1", "open", "running").await;
        seed_pending_user_message(&pool, "t1", Utc::now()).await;
        let runtime = Arc::new(RecordingRuntime::new());
        let notify = Arc::new(RecordingNotify::default());
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
            notify.clone(),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace: Duration::ZERO,
                ..Default::default()
            },
        );

        rec.reconcile_once().await;
        rec.reconcile_once().await;

        assert_eq!(
            runtime.calls(),
            (0, 0, 0),
            "converged — no lifecycle operation driven"
        );
        assert_eq!(
            *notify.count.lock().unwrap(),
            1,
            "notify fired once; the second immediate tick is backoff-suppressed",
        );
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
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().1, 1, "stop driven once");
    }

    #[sqlx::test]
    async fn removes_when_desired_removed_and_still_live(pool: PgPool) {
        seed(&pool, "t1", "closed", "removed").await;
        // Persist a Removed intent with a marker so it is a DB candidate.
        let store = PgLifecycleDao::new(pool.clone());
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
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
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

    #[sqlx::test]
    async fn replacement_generation_ready_acknowledges_edge_marker(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Restart,
                        OperationGeneration(7),
                        Utc::now(),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation(
                "t1",
                SandboxObservedStatus::Ready,
                Some(OperationGeneration(7)),
            ),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker, None);
        assert_eq!(runtime.calls(), (0, 0, 0));
    }

    #[sqlx::test]
    async fn fresh_edge_marker_retains_suppression_and_is_not_replayed(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Redeploy,
                        OperationGeneration(8),
                        Utc::now(),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation("t1", SandboxObservedStatus::Starting, None),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker.as_deref(), Some("redeploy"));
        assert_eq!(runtime.calls(), (0, 0, 0));
        assert!(sink.facts.lock().unwrap().is_empty());
    }

    #[sqlx::test]
    async fn stale_edge_persists_error_before_clearing_marker(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Restart,
                        OperationGeneration(9),
                        Utc::now() - chrono::Duration::minutes(20),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation("t1", SandboxObservedStatus::Starting, None),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker, None);
        let is_error: bool = sqlx::query_scalar(
            "SELECT is_error FROM task_events WHERE event_id = 'sandbox:t1:9:error'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(is_error);
        assert_eq!(runtime.calls(), (0, 0, 0));
        assert_eq!(*sink.facts.lock().unwrap(), vec![EventFact::Error]);
    }

    #[sqlx::test]
    async fn edge_between_suppression_and_abandonment_bounds_is_retained(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Restart,
                        OperationGeneration(10),
                        Utc::now() - chrono::Duration::minutes(10),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation("t1", SandboxObservedStatus::Starting, None),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker.as_deref(), Some("restart"));
        assert!(sink.facts.lock().unwrap().is_empty());
        assert_eq!(runtime.calls(), (0, 0, 0));
    }

    #[sqlx::test]
    async fn old_edge_with_matching_replacement_still_starting_is_retained(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Redeploy,
                        OperationGeneration(11),
                        Utc::now() - chrono::Duration::minutes(20),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation(
                "t1",
                SandboxObservedStatus::Starting,
                Some(OperationGeneration(11)),
            ),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE task_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker.as_deref(), Some("redeploy"));
        assert!(sink.facts.lock().unwrap().is_empty());
        assert_eq!(runtime.calls(), (0, 0, 0));
    }
}
