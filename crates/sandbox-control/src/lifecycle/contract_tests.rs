//! The lifecycle contract against local Postgres.
//!
//! Drives the `Lifecycle` orchestrator over the real `PgLifecycleDao` and
//! `PgLifecycleLock` with a recording runtime, exercising the Postgres half of
//! the contract: intent persistence, deterministic completion + marker
//! acknowledgement, genuine per-Task serialization across distinct lock pools,
//! bounded lock-retry exhaustion, the Error-resumable eligibility row, and the
//! orchestrator's permanent-rejection (bad-plan) handling.
//!
//! Gated on `db-tests`; run with
//! `cargo test -p moatless-backend --features db-tests --lib`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::sync::Notify;

use moatless_types::sandbox::SandboxStopReason;

use super::dao::PgLifecycleDao;
use super::events::LifecycleEvent;
use super::lock::PgLifecycleLock;
use super::orchestrator::{AgentStopPort, LifecycleDao, LifecycleEventSink, OutstandingCommand};
use super::test_support::seed_task_with as seed;
use moatless_sandbox_runtime::observation_events::EventFact;
use moatless_sandbox_runtime::operation::{ObservedStatus, OperationKind};
use moatless_sandbox_runtime::runtime::RuntimeError;
use moatless_sandbox_runtime::runtime::{RuntimeOperationContext, RuntimeOutcome, SandboxRuntime};
use moatless_types::sandbox::ArchiveRetention;

use super::{Lifecycle, LifecycleError, SystemWallClock};

// ---- Fixtures ----

/// Build two pools (store + lock) on the same isolated test database, mirroring
/// production's dedicated lock-only pool.
async fn pools(opts: &PgPoolOptions, conn: &PgConnectOptions) -> (PgPool, PgPool) {
    let store = opts
        .clone()
        .max_connections(10)
        .connect_with(conn.clone())
        .await
        .expect("store pool");
    let lock = lock_pool(opts, conn).await;
    (store, lock)
}

/// A dedicated lock-only pool on the isolated test database. A second one models
/// a second backend replica for cross-pool serialization.
async fn lock_pool(opts: &PgPoolOptions, conn: &PgConnectOptions) -> PgPool {
    opts.clone()
        .max_connections(6)
        .connect_with(conn.clone())
        .await
        .expect("lock pool")
}

// ---- Recording ports ----

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}

struct RecordingRuntime {
    ensure: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    stop: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    remove: Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
    last_generation: Mutex<Option<i64>>,
    gate: Option<Arc<Gate>>,
}

impl RecordingRuntime {
    fn new() -> Self {
        Self {
            ensure: Mutex::new(None),
            stop: Mutex::new(None),
            remove: Mutex::new(None),
            last_generation: Mutex::new(None),
            gate: None,
        }
    }
    fn with_ensure(self, r: Result<RuntimeOutcome, RuntimeError>) -> Self {
        *self.ensure.lock().unwrap() = Some(r);
        self
    }
    fn with_stop(self, r: Result<RuntimeOutcome, RuntimeError>) -> Self {
        *self.stop.lock().unwrap() = Some(r);
        self
    }
    fn with_remove(self, r: Result<RuntimeOutcome, RuntimeError>) -> Self {
        *self.remove.lock().unwrap() = Some(r);
        self
    }
    fn gated(mut self, gate: Arc<Gate>) -> Self {
        self.gate = Some(gate);
        self
    }
    fn take(
        slot: &Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>,
        default: ObservedStatus,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        slot.lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok(RuntimeOutcome::applied(default)))
    }
}

#[async_trait]
impl SandboxRuntime for RecordingRuntime {
    async fn ensure_running(
        &self,
        _t: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        *self.last_generation.lock().unwrap() = Some(ctx.operation_generation.value());
        if let Some(gate) = &self.gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        Self::take(&self.ensure, ObservedStatus::Starting)
    }
    async fn stop(
        &self,
        _t: &str,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        *self.last_generation.lock().unwrap() = Some(ctx.operation_generation.value());
        Self::take(&self.stop, ObservedStatus::Stopped)
    }
    async fn restart(
        &self,
        _t: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }
    async fn redeploy(
        &self,
        _t: &str,
        _ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }
    async fn remove(
        &self,
        _t: &str,
        _r: ArchiveRetention,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        *self.last_generation.lock().unwrap() = Some(ctx.operation_generation.value());
        Self::take(&self.remove, ObservedStatus::Removed)
    }
    async fn reclaim_storage(
        &self,
        _t: &str,
        _rescue_upload_url: Option<&str>,
        ctx: RuntimeOperationContext,
    ) -> Result<RuntimeOutcome, RuntimeError> {
        *self.last_generation.lock().unwrap() = Some(ctx.operation_generation.value());
        Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
    }
}

#[derive(Default)]
struct CollectSink {
    facts: Mutex<Vec<EventFact>>,
}
#[async_trait]
impl LifecycleEventSink for CollectSink {
    async fn publish(&self, _task_id: &str, event: &LifecycleEvent) {
        self.facts.lock().unwrap().push(event.fact);
    }

    async fn publish_diagnostics(
        &self,
        _task_id: &str,
        _generation: moatless_sandbox_runtime::operation::OperationGeneration,
        _observed: ObservedStatus,
        _diagnostics: &moatless_sandbox_runtime::outcome::Diagnostics,
    ) {
    }
}

struct NoopAgentStop;
#[async_trait]
impl AgentStopPort for NoopAgentStop {
    async fn stop_agent(&self, _task_id: &str, _reason: SandboxStopReason) {}
}

struct Harness {
    lifecycle: Lifecycle,
    store: Arc<PgLifecycleDao>,
    runtime: Arc<RecordingRuntime>,
    sink: Arc<CollectSink>,
    pool: PgPool,
}

fn build(
    store_pool: PgPool,
    lock_pool: PgPool,
    runtime: RecordingRuntime,
    lock_attempts: u32,
) -> Harness {
    let store = Arc::new(PgLifecycleDao::new(store_pool.clone()));
    let runtime = Arc::new(runtime);
    let sink = Arc::new(CollectSink::default());
    let lifecycle = Lifecycle::new(
        store.clone(),
        runtime.clone(),
        Arc::new(NoopAgentStop),
        sink.clone(),
        Arc::new(PgLifecycleLock::new(lock_pool)),
        Arc::new(SystemWallClock),
    )
    .with_lock_attempts(lock_attempts);
    Harness {
        lifecycle,
        store,
        runtime,
        sink,
        pool: store_pool,
    }
}

// The store's intent read is `pub(crate)` behind the port; read the Sandbox row
// columns directly.
async fn desired(pool: &PgPool, task_id: &str) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT desired_state, stop_reason, operation_kind FROM sandboxes WHERE sandbox_id = $1",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// ---- Contract (Postgres half) ----

#[sqlx::test]
async fn start_persists_running_and_publishes_initializing(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "stopped").await;
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new().with_ensure(Ok(RuntimeOutcome::applied(ObservedStatus::Starting))),
        1,
    );

    let out = h.lifecycle.start("t1").await.expect("start ok");
    assert_eq!(out.observed, ObservedStatus::Starting);
    let (state, reason, _kind) = desired(&h.pool, "t1").await;
    assert_eq!(state, "running");
    assert_eq!(reason, None);
    assert!(
        h.sink
            .facts
            .lock()
            .unwrap()
            .contains(&EventFact::Initializing)
    );
    // The allocated generation reached the runtime context.
    assert_eq!(*h.runtime.last_generation.lock().unwrap(), Some(1));
    // A start that ends Starting leaves its marker for the watcher's convergence.
    let loaded = h.store.load("t1").await.unwrap().unwrap();
    assert_eq!(loaded.marker.map(|m| m.kind), Some(OperationKind::Start));
}

#[sqlx::test]
async fn stop_commits_completion_and_clears_marker(opts: PgPoolOptions, conn: PgConnectOptions) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "running").await;
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new().with_stop(Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))),
        1,
    );

    h.lifecycle
        .stop("t1", SandboxStopReason::IdleReaper)
        .await
        .expect("stop ok");
    let (state, reason, kind) = desired(&h.pool, "t1").await;
    assert_eq!(state, "stopped");
    assert_eq!(reason.as_deref(), Some("idle_reaper"));
    assert_eq!(kind, None, "completion clears the marker");
    assert!(h.sink.facts.lock().unwrap().contains(&EventFact::Stopped));

    // Exactly one persisted Stopped fact for generation 1.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_events WHERE event_id = 'sandbox:t1:1:stopped'",
    )
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

// The running count the column holds, and the number of listing-change rows the
// task has accumulated — the two things `record_running_commands` decides.
async fn commands_state(pool: &PgPool, task_id: &str) -> (i32, i64) {
    let running: i32 =
        sqlx::query_scalar("SELECT running_commands FROM sandboxes WHERE sandbox_id = $1")
            .bind(task_id)
            .fetch_one(pool)
            .await
            .unwrap();
    let changes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM task_listing_changes WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(pool)
            .await
            .unwrap();
    (running, changes)
}

#[sqlx::test]
async fn running_commands_persists_the_count_and_notifies_only_on_the_busy_axis(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "running").await;
    let h = build(store_pool, lock_pool, RecordingRuntime::new(), 1);

    // A fresh row starts at zero; writing zero again crosses nothing.
    h.store.record_running_commands("t1", 0).await.unwrap();
    assert_eq!(commands_state(&h.pool, "t1").await, (0, 0));

    // 0 -> 2 crosses into busy: the count lands and one listing change is queued.
    h.store.record_running_commands("t1", 2).await.unwrap();
    assert_eq!(commands_state(&h.pool, "t1").await, (2, 1));

    // 2 -> 1 stays busy: the count updates but no new change is queued.
    h.store.record_running_commands("t1", 1).await.unwrap();
    assert_eq!(commands_state(&h.pool, "t1").await, (1, 1));

    // 1 -> 0 crosses back to idle: the count clears and a second change queues.
    h.store.record_running_commands("t1", 0).await.unwrap();
    assert_eq!(commands_state(&h.pool, "t1").await, (0, 2));
}

fn cmd(id: &str, state: &str) -> OutstandingCommand {
    OutstandingCommand {
        id: id.to_string(),
        command: format!("cargo test {id}"),
        label: id.to_string(),
        started_at_unix_ms: 1,
        state: state.to_string(),
        exit_code: None,
    }
}

#[sqlx::test]
async fn unreported_commands_is_absolute_guarded_by_revision_and_cleared_by_the_take(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "running").await;
    let h = build(store_pool, lock_pool, RecordingRuntime::new(), 1);

    h.store
        .record_unreported_commands("t1", &[cmd("a", "running")], 1)
        .await
        .unwrap();

    // Absolute: the second write replaces the first rather than adding to it.
    h.store
        .record_unreported_commands("t1", &[cmd("a", "exited"), cmd("b", "running")], 2)
        .await
        .unwrap();

    // The registry lock orders the revisions, the RPC does not, so a write that
    // arrives late is dropped rather than resurrecting a discharged command.
    h.store
        .record_unreported_commands("t1", &[cmd("a", "running")], 1)
        .await
        .unwrap();

    let taken = h.store.take_unreported_commands("t1").await.unwrap();
    assert_eq!(
        taken,
        vec![cmd("a", "exited"), cmd("b", "running")],
        "the highest revision is what the replacement pod reads"
    );

    // The take cleared the row, so no restart announces the same loss twice.
    assert!(
        h.store
            .take_unreported_commands("t1")
            .await
            .unwrap()
            .is_empty()
    );

    // The take leaves the revision behind, so a write still in flight from the
    // dying pod cannot resurrect what was just announced.
    h.store
        .record_unreported_commands("t1", &[cmd("a", "running")], 2)
        .await
        .unwrap();
    assert!(
        h.store
            .take_unreported_commands("t1")
            .await
            .unwrap()
            .is_empty(),
        "a late write at a spent revision is dropped, not replayed"
    );
}

/// Two Members of one shared sandbox resolve to one `sandboxes` row, so a
/// sandbox-keyed row would let each one's absolute write erase the other's.
#[sqlx::test]
async fn each_member_keeps_its_own_commands_and_its_own_revision(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "running").await;
    seed(&store_pool, "t2", "open", "running").await;
    let h = build(store_pool, lock_pool, RecordingRuntime::new(), 1);

    h.store
        .record_unreported_commands("t1", &[cmd("a", "running")], 40)
        .await
        .unwrap();
    // A lower revision than t1's, and it must still land: the counters are
    // per-registry and never comparable across Members.
    h.store
        .record_unreported_commands("t2", &[cmd("b", "exited")], 7)
        .await
        .unwrap();

    assert_eq!(
        h.store.take_unreported_commands("t1").await.unwrap(),
        vec![cmd("a", "running")]
    );
    assert_eq!(
        h.store.take_unreported_commands("t2").await.unwrap(),
        vec![cmd("b", "exited")]
    );
}

#[sqlx::test]
async fn moving_the_desired_state_resets_a_stranded_running_count(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "stopped").await;
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new().with_ensure(Ok(RuntimeOutcome::applied(ObservedStatus::Starting))),
        1,
    );

    // A pod that died mid-command leaves the count high with no one to decrement it.
    h.store.record_running_commands("t1", 3).await.unwrap();
    assert_eq!(commands_state(&h.pool, "t1").await.0, 3);

    // Moving the desired state (a fresh pod's lifecycle) zeroes the stale count.
    h.lifecycle.start("t1").await.expect("start ok");
    assert_eq!(
        commands_state(&h.pool, "t1").await.0,
        0,
        "persist_intent resets a stranded count when desired_state moves"
    );
}

#[sqlx::test]
async fn remove_publishes_removing_then_removed_and_keeps_retention(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "running").await;
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new().with_remove(Ok(RuntimeOutcome::applied(ObservedStatus::Removed))),
        1,
    );

    h.lifecycle
        .remove("t1", ArchiveRetention::DeleteArchives)
        .await
        .expect("remove ok");
    let facts = h.sink.facts.lock().unwrap().clone();
    assert_eq!(facts, vec![EventFact::Removing, EventFact::Removed]);
    let loaded = h.store.load("t1").await.unwrap().unwrap();
    assert_eq!(loaded.retention, Some(ArchiveRetention::DeleteArchives));
    assert!(loaded.marker.is_none(), "completion clears the marker");
}

// ---- Eligibility ----

#[sqlx::test]
async fn error_status_task_start_is_resumable(opts: PgPoolOptions, conn: PgConnectOptions) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "error", "stopped").await;
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new().with_ensure(Ok(RuntimeOutcome::applied(ObservedStatus::Starting))),
        1,
    );
    h.lifecycle.start("t1").await.expect("error task resumes");
    assert_eq!(desired(&h.pool, "t1").await.0, "running");
}

#[sqlx::test]
async fn closed_task_start_is_rejected(opts: PgPoolOptions, conn: PgConnectOptions) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "closed", "removed").await;
    let h = build(store_pool, lock_pool, RecordingRuntime::new(), 1);
    let err = h.lifecycle.start("t1").await.unwrap_err();
    assert!(matches!(err, LifecycleError::NotAllowed { .. }));
}

// ---- Bad plan ----

// The recording runtime injects the permanent `Rejected`, so this covers the
// orchestrator's response to a bad plan (intent kept, Error fact, non-retriable).
// The real plan path — the native runtime adapter's unbuildable-plan →
// `Rejected` mapping — is covered by `plan::db_tests::bad_plan_is_permanent_rejected`.
#[sqlx::test]
async fn bad_plan_start_is_permanent_and_keeps_intent(opts: PgPoolOptions, conn: PgConnectOptions) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "stopped").await;
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new().with_ensure(Err(RuntimeError::Rejected("unbuildable plan".into()))),
        1,
    );
    let err = h.lifecycle.start("t1").await.unwrap_err();
    assert!(!err.is_retriable(), "bad plan is permanent");
    // Intent is Running (written before the effect) and its Error fact persisted.
    assert_eq!(desired(&h.pool, "t1").await.0, "running");
    assert!(h.sink.facts.lock().unwrap().contains(&EventFact::Error));
}

// ---- Race matrix: genuine concurrency + lock-retry exhaustion ----

#[sqlx::test]
async fn concurrent_starts_serialize_and_contender_gets_operation_in_progress(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_a) = pools(&opts, &conn).await;
    // A second, distinct lock pool models a second backend replica: the two
    // lifecycles serialize across separate lock connection pools, not by sharing
    // one — genuine cross-replica serialization against the same database.
    let lock_b = lock_pool(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "stopped").await;
    let gate = Arc::new(Gate::default());
    let ha = build(
        store_pool.clone(),
        lock_a,
        RecordingRuntime::new()
            .with_ensure(Ok(RuntimeOutcome::applied(ObservedStatus::Ready)))
            .gated(gate.clone()),
        1,
    );
    let hb = build(store_pool, lock_b, RecordingRuntime::new(), 1);

    // Replica A acquires the lock and blocks inside the runtime.
    let lc = ha.lifecycle.clone();
    let a = tokio::spawn(async move { lc.start("t1").await });
    gate.entered.notified().await;

    // Replica B contends over its own lock pool; with a single lock attempt it
    // exhausts immediately and reads A's marker for the holder kind.
    let b = hb.lifecycle.start("t1").await;
    match b {
        Err(LifecycleError::OperationInProgress { kind }) => assert_eq!(kind, OperationKind::Start),
        other => panic!("expected OperationInProgress, got {other:?}"),
    }

    // Release A; it completes successfully.
    gate.release.notify_one();
    a.await.unwrap().expect("A completes");
}

#[sqlx::test]
async fn bounded_lock_retry_exhausts_to_operation_in_progress(
    opts: PgPoolOptions,
    conn: PgConnectOptions,
) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "stopped").await;
    let gate = Arc::new(Gate::default());
    // Three attempts still exhaust while A holds the lock.
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new()
            .with_ensure(Ok(RuntimeOutcome::applied(ObservedStatus::Ready)))
            .gated(gate.clone()),
        3,
    );
    let lc = h.lifecycle.clone();
    let a = tokio::spawn(async move { lc.start("t1").await });
    gate.entered.notified().await;

    let b = h.lifecycle.stop("t1", SandboxStopReason::User).await;
    assert!(matches!(
        b,
        Err(LifecycleError::OperationInProgress {
            kind: OperationKind::Start
        })
    ));

    gate.release.notify_one();
    a.await.unwrap().expect("A completes");
}
