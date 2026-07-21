//! The lifecycle contract against local Postgres.
//!
//! Drives the `Lifecycle` orchestrator over the real `PgLifecycleStore` and
//! `PgLifecycleLock` with a recording runtime, exercising the Postgres half of
//! the contract: intent persistence, deterministic completion + marker
//! acknowledgement, genuine per-Task serialization across pools, bounded
//! lock-retry exhaustion, the Error-resumable eligibility row, and the bad-plan
//! start case.
//!
//! Gated on `db-tests`; run with
//! `cargo test -p moatless-vibe-backend --features db-tests --lib`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tokio::sync::Notify;

use crate::task::models::SandboxStopReason;

use super::events::{EventFact, LifecycleEvent};
use super::{
    AgentStopPort, ArchiveRetention, Lifecycle, LifecycleError, LifecycleEventSink, LifecycleStore,
    ObservedStatus, OperationKind, PgLifecycleLock, PgLifecycleStore, RuntimeError,
    RuntimeOperationContext, RuntimeOutcome, SandboxRuntime, SystemWallClock,
};

// ---- Fixtures ----

async fn seed(pool: &PgPool, task_id: &str, status: &str, desired: &str) {
    sqlx::query("INSERT INTO teams (id, name, slug) VALUES ('team-x', 'Team X', 'team-x') ON CONFLICT (id) DO NOTHING")
        .execute(pool)
        .await
        .expect("seed team");
    sqlx::query(
        r#"INSERT INTO tasks (task_id, created_at, last_message_at, agent_type, repository_id,
               status, total_cost_usd, visibility, scope, team_id, created_by, owner_user_id,
               sandbox_desired_state)
           VALUES ($1, NOW()::text, NOW()::text, 'claude-code', 'repo-x', $2, 0,
               'private', 'user', 'team-x', 'user-1', 'user-1', $3)"#,
    )
    .bind(task_id)
    .bind(status)
    .bind(desired)
    .execute(pool)
    .await
    .expect("seed task");
}

/// Build two pools (store + lock) on the same isolated test database, mirroring
/// production's dedicated lock-only pool.
async fn pools(opts: &PgPoolOptions, conn: &PgConnectOptions) -> (PgPool, PgPool) {
    let store = opts
        .clone()
        .max_connections(10)
        .connect_with(conn.clone())
        .await
        .expect("store pool");
    let lock = opts
        .clone()
        .max_connections(6)
        .connect_with(conn.clone())
        .await
        .expect("lock pool");
    (store, lock)
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
    fn take(slot: &Mutex<Option<Result<RuntimeOutcome, RuntimeError>>>, default: ObservedStatus) -> Result<RuntimeOutcome, RuntimeError> {
        slot.lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok(RuntimeOutcome::applied(default)))
    }
}

#[async_trait]
impl SandboxRuntime for RecordingRuntime {
    async fn ensure_running(&self, _t: &str, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
        *self.last_generation.lock().unwrap() = Some(ctx.operation_generation.value());
        if let Some(gate) = &self.gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        Self::take(&self.ensure, ObservedStatus::Starting)
    }
    async fn stop(&self, _t: &str, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
        *self.last_generation.lock().unwrap() = Some(ctx.operation_generation.value());
        Self::take(&self.stop, ObservedStatus::Stopped)
    }
    async fn restart(&self, _t: &str, _ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }
    async fn redeploy(&self, _t: &str, _ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
        Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
    }
    async fn remove(&self, _t: &str, _r: ArchiveRetention, ctx: RuntimeOperationContext) -> Result<RuntimeOutcome, RuntimeError> {
        *self.last_generation.lock().unwrap() = Some(ctx.operation_generation.value());
        Self::take(&self.remove, ObservedStatus::Removed)
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
}

struct NoopAgentStop;
#[async_trait]
impl AgentStopPort for NoopAgentStop {
    async fn stop_agent(&self, _task_id: &str) {}
}

struct Harness {
    lifecycle: Lifecycle,
    store: Arc<PgLifecycleStore>,
    runtime: Arc<RecordingRuntime>,
    sink: Arc<CollectSink>,
    pool: PgPool,
}

fn build(store_pool: PgPool, lock_pool: PgPool, runtime: RecordingRuntime, lock_attempts: u32) -> Harness {
    let store = Arc::new(PgLifecycleStore::new(store_pool.clone()));
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

// The store's intent read is `pub(crate)` behind the port; read columns directly.
async fn desired(pool: &PgPool, task_id: &str) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT sandbox_desired_state, sandbox_stop_reason, sandbox_operation_kind FROM tasks WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// ---- Contract (Postgres half) ----

#[sqlx::test]
async fn start_persists_running_and_publishes_initializing(opts: PgPoolOptions, conn: PgConnectOptions) {
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
    assert!(h.sink.facts.lock().unwrap().contains(&EventFact::Initializing));
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

    h.lifecycle.stop("t1", SandboxStopReason::IdleReaper).await.expect("stop ok");
    let (state, reason, kind) = desired(&h.pool, "t1").await;
    assert_eq!(state, "stopped");
    assert_eq!(reason.as_deref(), Some("idle_reaper"));
    assert_eq!(kind, None, "completion clears the marker");
    assert!(h.sink.facts.lock().unwrap().contains(&EventFact::Stopped));

    // Exactly one persisted Stopped fact for generation 1.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_events WHERE event_id = 'sandbox:t1:1:stopped'")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn remove_publishes_removing_then_removed_and_keeps_retention(opts: PgPoolOptions, conn: PgConnectOptions) {
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "running").await;
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new().with_remove(Ok(RuntimeOutcome::applied(ObservedStatus::Removed))),
        1,
    );

    h.lifecycle.remove("t1", ArchiveRetention::DeleteArchives).await.expect("remove ok");
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
    let (store_pool, lock_pool) = pools(&opts, &conn).await;
    seed(&store_pool, "t1", "open", "stopped").await;
    let gate = Arc::new(Gate::default());
    let h = build(
        store_pool,
        lock_pool,
        RecordingRuntime::new()
            .with_ensure(Ok(RuntimeOutcome::applied(ObservedStatus::Ready)))
            .gated(gate.clone()),
        1,
    );

    // A acquires the lock and blocks inside the runtime.
    let lc = h.lifecycle.clone();
    let a = tokio::spawn(async move { lc.start("t1").await });
    gate.entered.notified().await;

    // B contends; with a single lock attempt it exhausts immediately.
    let b = h.lifecycle.start("t1").await;
    match b {
        Err(LifecycleError::OperationInProgress { kind }) => assert_eq!(kind, OperationKind::Start),
        other => panic!("expected OperationInProgress, got {other:?}"),
    }

    // Release A; it completes successfully.
    gate.release.notify_one();
    a.await.unwrap().expect("A completes");
}

#[sqlx::test]
async fn bounded_lock_retry_exhausts_to_operation_in_progress(opts: PgPoolOptions, conn: PgConnectOptions) {
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
    assert!(matches!(b, Err(LifecycleError::OperationInProgress { kind: OperationKind::Start })));

    gate.release.notify_one();
    a.await.unwrap().expect("A completes");
}
