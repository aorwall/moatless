//! Reconciler tests against Postgres: candidate enumeration and marker recovery
//! as the real DAO answers them.

use super::*;

use sqlx::PgPool;

use super::super::dao::{IntentWrite, PgLifecycleDao};
use super::super::db::lock::PgLifecycleLock;
use super::super::error::LifecycleError;
use super::super::events::LifecycleEvent;
use super::super::orchestrator::{AgentStopPort, LifecycleEventSink, SystemWallClock};
use super::super::test_support::{seed_bare_task, seed_task_with as seed};
use moatless_sandbox_runtime::fleet::ObserveError;
use moatless_sandbox_runtime::observation_events::EventFact;
use moatless_sandbox_runtime::operation::{
    CorrelationMarker, ObservedStatus, OperationGeneration, OperationKind,
};
use moatless_sandbox_runtime::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;

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
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        self.calls.lock().unwrap().ensure += 1;
        if self.fail_ensure {
            return Err(
                moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure("boom".into()),
            );
        }
        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }
    async fn stop(
        &self,
        _t: &str,
        _c: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        self.calls.lock().unwrap().stop += 1;
        Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
    }
    async fn restart(
        &self,
        _t: &str,
        _c: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }
    async fn redeploy(
        &self,
        _t: &str,
        _c: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }
    async fn remove(
        &self,
        _t: &str,
        _r: ArchiveRetention,
        _c: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        self.calls.lock().unwrap().remove += 1;
        Ok(RuntimeOutcome::applied(ObservedStatus::Removed))
    }
    async fn reclaim_storage(
        &self,
        _t: &str,
        _rescue_upload_url: Option<&str>,
        _c: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
        // The reconciler never drives the cold transition — it converges
        // desired vs. actual, and a reclaimed Task's desired state does not
        // change.
        Ok(RuntimeOutcome::no_op(ObservedStatus::Stopped))
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
        _diagnostics: &moatless_sandbox_runtime::outcome::Diagnostics,
    ) {
    }
}

struct NoopAgentStop;
#[async_trait]
impl AgentStopPort for NoopAgentStop {
    async fn stop_agent(
        &self,
        _t: &str,
        _reason: SandboxStopReason,
    ) -> moatless_ports::sandbox::AgentStopOutcome {
        moatless_ports::sandbox::AgentStopOutcome::Stopped
    }
}

#[derive(Default)]
struct RecordingNotify {
    count: Mutex<u32>,
    targets: Mutex<Vec<String>>,
}
#[async_trait]
impl NotifyPort for RecordingNotify {
    async fn notify(&self, t: &str) {
        *self.count.lock().unwrap() += 1;
        self.targets.lock().unwrap().push(t.to_string());
    }
}

/// The production `PendingWorkReader` predicate, run against the test pool.
///
/// A stub returning a fixed list wouldn't exercise resolving a shared Sandbox's
/// members from seeded rows; matches `moatless_tasks::ports::PendingWork`,
/// restated here since that crate isn't a dependency.
struct PoolPendingWork(PgPool);
#[async_trait]
impl PendingWorkReader for PoolPendingWork {
    async fn members_awaiting_dispatch(
        &self,
        task_id: &str,
    ) -> Result<Vec<String>, moatless_types::store::StoreError> {
        sqlx::query_scalar(
            r#"SELECT peer.task_id
                 FROM tasks t
                 JOIN tasks peer ON peer.sandbox_id = t.sandbox_id
                WHERE t.task_id = $1
                  AND peer.status IN ('open', 'error')
                  AND EXISTS (
                      SELECT 1 FROM task_inbox i
                       WHERE i.task_id = peer.task_id AND i.status = 'pending'
                  )
                ORDER BY peer.created_at_db NULLS LAST, peer.task_id"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
        .map_err(|e| moatless_types::store::StoreError::with_source("pending", e))
    }
}

#[derive(Default)]
struct RecordingMemberRegistry {
    calls: Mutex<Vec<SandboxMember>>,
}
#[async_trait]
impl MemberRegistryPort for RecordingMemberRegistry {
    async fn ensure_registered(&self, member: &SandboxMember) -> Result<(), MemberRegistryError> {
        self.calls.lock().unwrap().push(member.clone());
        Ok(())
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
        Arc::new(RecordingMemberRegistry::default()),
        Arc::new(SystemWallClock),
        ReconcilerConfig {
            grace,
            ..Default::default()
        },
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "one argument per fixture collaborator"
)]
fn build_with(
    pool: PgPool,
    lock_pool: PgPool,
    runtime: Arc<RecordingRuntime>,
    observer: Arc<dyn FleetObserver>,
    notify: Arc<dyn NotifyPort>,
    member_registry: Arc<dyn MemberRegistryPort>,
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
        ReconcilerDeps {
            store,
            observer,
            notify,
            pending_work: Arc::new(PoolPendingWork(pool)),
            member_registry,
            clock,
        },
        config,
    );
    (reconciler, sink)
}

/// A pending inbox item, plus the Sandbox wake signal its send path writes.
///
/// The two are one transaction in production
/// (`moatless_tasks::inbox::InboxService::submit`); the fixture
/// records both so the reconciler sees the shape it reads.
async fn seed_pending_user_message(pool: &PgPool, task_id: &str, created_at: DateTime<Utc>) {
    sqlx::query(
        r#"INSERT INTO task_inbox (id, task_id, text, origin, created_at)
           VALUES (gen_random_uuid(), $1, '', 'system', $2)"#,
    )
    .bind(task_id)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("seed pending message");

    sqlx::query(
        r#"UPDATE sandboxes s
              SET pending_work_at = GREATEST(s.pending_work_at, $2)
             FROM tasks t
            WHERE t.task_id = $1
              AND s.sandbox_id = t.sandbox_id"#,
    )
    .bind(task_id)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("queue sandbox wake signal");
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
    assert!(
        sink.facts
            .lock()
            .unwrap()
            .contains(&EventFact::Initializing)
    );
}

#[sqlx::test]
async fn wakes_a_stopped_task_with_a_newer_pending_message(pool: PgPool) {
    seed(&pool, "t1", "open", "stopped").await;
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
        sqlx::query_scalar("SELECT desired_state FROM sandboxes WHERE sandbox_id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(desired, "running", "wake flipped intent to running");
}

#[sqlx::test]
async fn pending_message_wakes_a_never_operated_sandbox(pool: PgPool) {
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
        sqlx::query_scalar("SELECT desired_state FROM sandboxes WHERE sandbox_id = 't1'")
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
        Arc::new(RecordingMemberRegistry::default()),
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
        Arc::new(RecordingMemberRegistry::default()),
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
    assert!(
        payload
            .error
            .as_deref()
            .is_some_and(|e| e.contains("2 attempts failed"))
    );
}

#[sqlx::test]
async fn lock_contention_neither_spends_the_budget_nor_gates_the_next_tick(pool: PgPool) {
    // The idle reaper's stop holds the per-Task lock while it flushes the
    // workspace, and the reconciler ticks inside that window. Contention is
    // the holder driving this same generation, so the budget must survive
    // it: spending it published a `recovery_exhausted` error against a stop
    // that then completed, and refused to drive the operation again.
    seed(&pool, "t1", "open", "stopped").await;
    let (rec, _sink) = build_with(
        pool.clone(),
        pool,
        Arc::new(RecordingRuntime::new()),
        observer(&[], SandboxObservedStatus::Absent),
        Arc::new(RecordingNotify::default()),
        Arc::new(RecordingMemberRegistry::default()),
        Arc::new(SystemWallClock),
        ReconcilerConfig {
            grace: Duration::ZERO,
            attempt_cap: 2,
            ..Default::default()
        },
    );

    let now = Utc::now();
    for _ in 0..5 {
        assert!(
            rec.retry_gate("t1", Some(1), now),
            "contention must leave the next tick eligible"
        );
        let spent = rec.record_outcome(
            "t1",
            ReconcileAction::Stop,
            Err(LifecycleError::OperationInProgress {
                kind: OperationKind::Stop,
            }),
            now,
        );
        assert!(
            spent.is_none(),
            "contention must not spend the budget or publish a give-up"
        );
    }
}

#[sqlx::test]
async fn a_permanent_failure_reports_the_one_attempt_it_spent(pool: PgPool) {
    // The give-up event used to publish the cap rather than the count, so a
    // permanent failure on the first attempt read as five failed retries.
    seed(&pool, "t1", "open", "stopped").await;
    let (rec, _sink) = build_with(
        pool.clone(),
        pool,
        Arc::new(RecordingRuntime::new()),
        observer(&[], SandboxObservedStatus::Absent),
        Arc::new(RecordingNotify::default()),
        Arc::new(RecordingMemberRegistry::default()),
        Arc::new(SystemWallClock),
        ReconcilerConfig {
            grace: Duration::ZERO,
            attempt_cap: 5,
            ..Default::default()
        },
    );

    let now = Utc::now();
    assert!(rec.retry_gate("t1", Some(1), now));
    let spent = rec
        .record_outcome(
            "t1",
            ReconcileAction::Stop,
            Err(LifecycleError::TaskNotFound("t1".into())),
            now,
        )
        .expect("a permanent failure spends the budget");

    assert_eq!(spent.attempts, 1);
    assert!(
        !rec.retry_gate("t1", Some(1), now),
        "a permanent failure still blocks this generation"
    );
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
        Arc::new(RecordingMemberRegistry::default()),
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
async fn the_wake_reaches_the_shared_sandbox_member_holding_the_message(pool: PgPool) {
    // Two Tasks on one Sandbox: "t1" created it, "t2" joined it (a
    // `sameSandbox` fork). Only "t2" has an unclaimed message. The candidate
    // enumeration collapses the Sandbox to its oldest member, "t1", so a
    // notify aimed at the candidate would dispatch the wrong Task and leave
    // "t2" waiting forever while its message kept the wake set.
    seed(&pool, "t1", "open", "running").await;
    seed_bare_task(&pool, "t2", "open").await;
    sqlx::query("UPDATE tasks SET sandbox_id = 't1' WHERE task_id = 't2'")
        .execute(&pool)
        .await
        .unwrap();
    seed_pending_user_message(&pool, "t2", Utc::now()).await;

    let notify = Arc::new(RecordingNotify::default());
    let (rec, _sink) = build_with(
        pool.clone(),
        pool,
        Arc::new(RecordingRuntime::new()),
        observer(
            &[("t1", SandboxObservedStatus::Ready)],
            SandboxObservedStatus::Ready,
        ),
        notify.clone(),
        Arc::new(RecordingMemberRegistry::default()),
        Arc::new(SystemWallClock),
        ReconcilerConfig {
            grace: Duration::ZERO,
            ..Default::default()
        },
    );

    rec.reconcile_once().await;

    assert_eq!(
        notify.targets.lock().unwrap().as_slice(),
        ["t2"],
        "the wake goes to the member whose message is unclaimed, not the candidate"
    );
}

#[sqlx::test]
async fn reissues_join_task_for_all_running_sandbox_members(pool: PgPool) {
    seed(&pool, "t1", "open", "running").await;
    seed_bare_task(&pool, "t2", "open").await;
    // `t2` joins `t1`'s pod under its own harness. The pod's `AGENT_PROFILE`
    // names `t1` only, so each re-registration has to carry the Member's.
    sqlx::query("UPDATE tasks SET sandbox_id = 't1', agent_profile = 'codex' WHERE task_id = 't2'")
        .execute(&pool)
        .await
        .unwrap();
    let runtime = Arc::new(RecordingRuntime::new());
    let registry = Arc::new(RecordingMemberRegistry::default());
    let (rec, _sink) = build_with(
        pool.clone(),
        pool,
        runtime.clone(),
        observer(
            &[("t1", SandboxObservedStatus::Ready)],
            SandboxObservedStatus::Ready,
        ),
        Arc::new(RecordingNotify::default()),
        registry.clone(),
        Arc::new(SystemWallClock),
        ReconcilerConfig {
            grace: Duration::ZERO,
            ..Default::default()
        },
    );

    rec.reconcile_once().await;

    let mut calls = registry.calls.lock().unwrap().clone();
    calls.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    assert_eq!(
        calls,
        vec![
            SandboxMember {
                task_id: "t1".to_string(),
                sandbox_id: "t1".to_string(),
                checkout_mode: moatless_types::task::CheckoutMode::Shared,
                agent_profile: AgentProfile::ClaudeCode,
            },
            SandboxMember {
                task_id: "t2".to_string(),
                sandbox_id: "t1".to_string(),
                checkout_mode: moatless_types::task::CheckoutMode::Shared,
                agent_profile: AgentProfile::Codex,
            },
        ],
        "reconciler rebuilds in-pod members from DB membership after restart, each under \
         its own harness"
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
                desired_state: moatless_types::sandbox::SandboxDesiredState::Removed,
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
        Duration::from_hours(1),
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
        sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
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
        sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
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
        sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
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
        sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
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
        sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(marker.as_deref(), Some("redeploy"));
    assert!(sink.facts.lock().unwrap().is_empty());
    assert_eq!(runtime.calls(), (0, 0, 0));
}
