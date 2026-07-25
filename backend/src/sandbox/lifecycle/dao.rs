//! Postgres [`LifecycleStore`] — durable intent, generation allocation, and the
//! transactional-outbox completion commit.
//!
//! The atomic intent write collapses desired state, stop reason, archive
//! retention, and the correlation marker into **one** statement (a CTE captures
//! the pre-image so `persist_intent` can report whether intent actually changed).
//! `commit_fact_and_clear_marker` persists the deterministic terminal fact and clears the
//! marker in one transaction, so the deterministic event id is never live without
//! its persisted fact.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::db::DbPool;
use crate::task::models::{
    sandbox_intent_epoch, SandboxDesiredState, SandboxIntentView, SandboxStopReason, TaskStatus,
};

use super::events::LifecycleEvent;
use super::orchestrator::{IntentWrite, LifecycleStore, StoreError, TaskIntent};
use super::types::{
    ArchiveRetention, CorrelationMarker, IntentEffect, OperationGeneration, OperationKind,
};

/// Postgres-backed lifecycle store over the main pool.
pub struct PgLifecycleStore {
    pool: DbPool,
}

impl PgLifecycleStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

/// The statement that clears the correlation marker (kind/generation/started_at).
/// Shared by the standalone clear and the completion transaction so the two
/// cannot drift.
const CLEAR_MARKER_SQL: &str = r#"UPDATE sandboxes SET
       operation_kind = NULL,
       operation_generation = NULL,
       operation_started_at = NULL,
       updated_at = NOW()
   WHERE task_id = $1 AND operation_generation = $2"#;

/// Clear the correlation marker on `executor`. Generic over the executor so the
/// completion transaction (`&mut tx`) and the standalone clear (`&pool`) issue
/// one and the same statement.
async fn clear_marker_on<'e, E>(
    executor: E,
    task_id: &str,
    generation: OperationGeneration,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(CLEAR_MARKER_SQL)
        .bind(task_id)
        .bind(generation.value())
        .execute(executor)
        .await
        .map(|_| ())
}

/// Reconstruct the persisted correlation marker from its three columns. Present
/// iff `sandbox_operation_kind` is set.
fn marker_from_columns(
    kind: Option<String>,
    generation: Option<i64>,
    started_at: Option<DateTime<Utc>>,
) -> Option<CorrelationMarker> {
    match (kind, generation, started_at) {
        (Some(kind), Some(generation), Some(started_at)) => OperationKind::from_db_str(&kind)
            .map(|kind| CorrelationMarker::new(kind, OperationGeneration(generation), started_at)),
        _ => None,
    }
}

#[derive(sqlx::FromRow)]
struct SandboxIntentViewRow {
    task_id: String,
    started_at: Option<DateTime<Utc>>,
    desired_state: Option<SandboxDesiredState>,
    stop_reason: Option<SandboxStopReason>,
    desired_state_updated_at: Option<DateTime<Utc>>,
}

/// Batch-load the task-response projection of Sandbox intent. This module owns
/// the lazy-row semantic: an existing Task without a Sandbox row is returned as
/// converged Stopped at the no-intent epoch.
pub(crate) async fn load_sandbox_intent_views(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<std::collections::HashMap<String, SandboxIntentView>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let rows: Vec<SandboxIntentViewRow> = sqlx::query_as(
        r#"SELECT t.task_id, s.started_at, s.desired_state, s.stop_reason,
                  s.desired_state_updated_at
           FROM tasks t
           LEFT JOIN sandboxes s ON s.task_id = t.task_id
           WHERE t.task_id = ANY($1)"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.task_id,
                SandboxIntentView {
                    started_at: row.started_at,
                    desired_state: row.desired_state.unwrap_or(SandboxDesiredState::Stopped),
                    stop_reason: row.stop_reason,
                    desired_state_updated_at: row
                        .desired_state_updated_at
                        .unwrap_or_else(sandbox_intent_epoch),
                },
            )
        })
        .collect())
}

#[async_trait]
impl LifecycleStore for PgLifecycleStore {
    async fn load(&self, task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
        // Left join: a Task with no Sandbox row is not TaskNotFound — it reads as
        // converged Stopped (never requested) with an epoch intent timestamp.
        let row: Option<(
            TaskStatus,
            Option<SandboxDesiredState>,
            Option<SandboxStopReason>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            r#"SELECT
                t.status,
                s.desired_state,
                s.stop_reason,
                s.archive_retention,
                s.operation_kind,
                s.operation_generation,
                s.operation_started_at,
                s.desired_state_updated_at
            FROM tasks t
            LEFT JOIN sandboxes s ON s.task_id = t.task_id
            WHERE t.task_id = $1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load lifecycle intent", e))?;

        Ok(row.map(
            |(
                status,
                desired_state,
                stop_reason,
                retention,
                op_kind,
                op_generation,
                op_started_at,
                desired_state_updated_at,
            )| TaskIntent {
                status,
                desired_state: desired_state.unwrap_or(SandboxDesiredState::Stopped),
                stop_reason,
                retention: retention.as_deref().and_then(ArchiveRetention::from_db_str),
                marker: marker_from_columns(op_kind, op_generation, op_started_at),
                desired_state_updated_at: desired_state_updated_at
                    .unwrap_or_else(sandbox_intent_epoch),
            },
        ))
    }

    async fn reconcile_candidates(&self) -> Result<Vec<String>, StoreError> {
        // Desired-Running participants (fleet observation may find them down)
        // ∪ incomplete-marker Sandboxes (a crashed operation). Backed by
        // `sandboxes_desired_running_idx` and `sandboxes_incomplete_operation_idx`.
        sqlx::query_scalar(
            r#"SELECT s.task_id FROM sandboxes s
               JOIN tasks t ON t.task_id = s.task_id
               WHERE (s.desired_state = 'running' AND t.status IN ('open', 'error'))
                  OR s.operation_generation IS NOT NULL"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load reconcile candidates", e))
    }

    async fn allocate_generation(&self, task_id: &str) -> Result<OperationGeneration, StoreError> {
        // Allocation is also the Sandbox row's lazy birth: the first operation on
        // a Task inserts its row (seq 1); later operations increment. Serialized
        // per Task by the lifecycle lock, so no concurrent allocation races.
        let seq: Option<i64> = sqlx::query_scalar(
            r#"INSERT INTO sandboxes (task_id, operation_seq)
               VALUES ($1, 1)
               ON CONFLICT (task_id) DO UPDATE
                   SET operation_seq = sandboxes.operation_seq + 1,
                       updated_at = NOW()
               RETURNING operation_seq"#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("allocate operation generation", e))?;

        seq.map(OperationGeneration).ok_or_else(|| {
            StoreError::new(format!("task {task_id} not found allocating generation"))
        })
    }

    async fn persist_intent(
        &self,
        task_id: &str,
        write: &IntentWrite,
    ) -> Result<IntentEffect, StoreError> {
        // Single atomic statement, an upsert so the Sandbox row is born through
        // lifecycle and nowhere else. `prev` captures the pre-image under a row
        // lock so `changed` reflects the desired-state / stop-reason / retention
        // delta — the intent-change axis — while the marker is always
        // overwritten. A first write (empty `prev`) is `Changed` by definition.
        let changed: Option<bool> = sqlx::query_scalar(
            r#"WITH prev AS (
                   SELECT desired_state, stop_reason, archive_retention
                   FROM sandboxes
                   WHERE task_id = $1
                   FOR UPDATE
               ),
               upsert AS (
                   INSERT INTO sandboxes AS s (
                       task_id, desired_state, stop_reason, archive_retention,
                       operation_kind, operation_generation, operation_started_at,
                       started_at, desired_state_updated_at, updated_at
                   ) VALUES (
                       $1, $2, $3, $4, $5, $6, $7,
                       CASE WHEN $8 THEN NOW() ELSE NULL END, NOW(), NOW()
                   )
                   ON CONFLICT (task_id) DO UPDATE SET
                       desired_state = EXCLUDED.desired_state,
                       stop_reason = EXCLUDED.stop_reason,
                       archive_retention = EXCLUDED.archive_retention,
                       operation_kind = EXCLUDED.operation_kind,
                       operation_generation = EXCLUDED.operation_generation,
                       operation_started_at = EXCLUDED.operation_started_at,
                       started_at = CASE WHEN $8 THEN NOW() ELSE s.started_at END,
                       desired_state_updated_at = NOW(),
                       updated_at = NOW()
                   RETURNING 1
               )
               SELECT
                   (SELECT desired_state FROM prev) IS DISTINCT FROM $2
                   OR (SELECT stop_reason FROM prev) IS DISTINCT FROM $3
                   OR (SELECT archive_retention FROM prev) IS DISTINCT FROM $4"#,
        )
        .bind(task_id)
        .bind(write.desired_state)
        .bind(write.stop_reason)
        .bind(write.retention.map(ArchiveRetention::as_str))
        .bind(write.marker.kind.as_str())
        .bind(write.marker.generation.value())
        .bind(write.marker.started_at)
        // Lifecycle owns the start timestamp: bump it whenever intent moves
        // toward Running (start/restart/redeploy), regardless of caller path.
        .bind(write.desired_state == SandboxDesiredState::Running)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("persist lifecycle intent", e))?;

        match changed {
            Some(true) => Ok(IntentEffect::Changed),
            Some(false) => Ok(IntentEffect::Unchanged),
            None => Err(StoreError::new(format!(
                "task {task_id} not found persisting intent"
            ))),
        }
    }

    async fn commit_fact_and_clear_marker(
        &self,
        task_id: &str,
        event: &LifecycleEvent,
    ) -> Result<(), StoreError> {
        let payload = serde_json::to_value(event.payload())
            .map_err(|e| StoreError::with_source("serialize completion payload", e))?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::with_source("begin completion transaction", e))?;

        // Persist the deterministic completion fact. ON CONFLICT keeps the commit
        // idempotent across at-least-once retries of the same generation.
        sqlx::query(
            r#"INSERT INTO task_events (
                   event_id, task_id, type, sub_type, is_error,
                   agent_type, turn_number, payload, timestamp
               ) VALUES ($1, $2, 'sandbox', $3, $4, NULL, NULL, $5, $6)
               ON CONFLICT (event_id) DO NOTHING"#,
        )
        .bind(&event.event_id)
        .bind(task_id)
        .bind(event.sub_type())
        .bind(event.is_error())
        .bind(&payload)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|e| StoreError::with_source("persist completion fact", e))?;

        // Clear the marker in the same transaction: the fact is durable and the
        // operation is acknowledged together.
        clear_marker_on(&mut *tx, task_id, event.generation)
            .await
            .map_err(|e| StoreError::with_source("clear marker on completion", e))?;

        tx.commit()
            .await
            .map_err(|e| StoreError::with_source("commit completion transaction", e))
    }

    async fn clear_marker(
        &self,
        task_id: &str,
        generation: OperationGeneration,
    ) -> Result<(), StoreError> {
        clear_marker_on(&self.pool, task_id, generation)
            .await
            .map_err(|e| StoreError::with_source("clear marker", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("valid ts")
    }

    #[test]
    fn marker_reconstructs_only_when_all_columns_present() {
        let full = marker_from_columns(Some("stop".to_string()), Some(7), Some(ts()));
        let marker = full.expect("full marker");
        assert_eq!(marker.kind, OperationKind::Stop);
        assert_eq!(marker.generation, OperationGeneration(7));

        // A partial marker (any column NULL) is no marker.
        assert!(marker_from_columns(None, Some(7), Some(ts())).is_none());
        assert!(marker_from_columns(Some("stop".to_string()), None, Some(ts())).is_none());
        assert!(marker_from_columns(Some("stop".to_string()), Some(7), None).is_none());
        // An unknown kind slug is not a valid marker.
        assert!(marker_from_columns(Some("bogus".to_string()), Some(7), Some(ts())).is_none());
    }
}

/// Postgres-backed store tests. Gated on `db-tests` and run in isolation
/// (`cargo test -p moatless-vibe-backend --features db-tests --lib`) against a
/// live Postgres via `#[sqlx::test]`, which provisions an isolated database and
/// applies `backend/migrations` (including the `sandboxes` table).
#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::super::events::EventFact;
    use super::super::test_support::{seed_bare_task, seed_task as seed};
    use super::*;
    use moatless_events::SandboxStatus;
    use sqlx::PgPool;

    fn marker(kind: OperationKind, generation: i64) -> CorrelationMarker {
        CorrelationMarker::new(kind, OperationGeneration(generation), Utc::now())
    }

    #[sqlx::test]
    async fn allocate_generation_is_monotonic(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleStore::new(pool);
        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(1)
        );
        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(2)
        );
        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(3)
        );
    }

    #[sqlx::test]
    async fn persist_intent_change_then_unchanged_and_load_round_trips(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleStore::new(pool);
        let write = IntentWrite {
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: Some(SandboxStopReason::User),
            retention: None,
            marker: marker(OperationKind::Stop, 1),
        };
        assert_eq!(
            store.persist_intent("t1", &write).await.unwrap(),
            IntentEffect::Changed
        );
        // Same intent, fresh marker → the intent axis is Unchanged even though the
        // marker moved (Changed covers only desired-state / reason / retention).
        let write2 = IntentWrite {
            marker: marker(OperationKind::Stop, 2),
            ..write.clone()
        };
        assert_eq!(
            store.persist_intent("t1", &write2).await.unwrap(),
            IntentEffect::Unchanged
        );

        let loaded = store.load("t1").await.unwrap().expect("intent");
        assert_eq!(loaded.desired_state, SandboxDesiredState::Stopped);
        assert_eq!(loaded.stop_reason, Some(SandboxStopReason::User));
        let m = loaded.marker.expect("marker");
        assert_eq!(m.kind, OperationKind::Stop);
        assert_eq!(m.generation, OperationGeneration(2));
    }

    #[sqlx::test]
    async fn remove_persists_retention_and_fact_commit_clears_marker(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleStore::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Removed,
                    stop_reason: Some(SandboxStopReason::Cleanup),
                    retention: Some(ArchiveRetention::RetainArchives),
                    marker: marker(OperationKind::Remove, 5),
                },
            )
            .await
            .unwrap();

        let event = LifecycleEvent {
            event_id: "sandbox:t1:5:removed".to_string(),
            generation: OperationGeneration(5),
            fact: EventFact::Removed,
            status: SandboxStatus::Removed,
            stop_reason: None,
        };
        store
            .commit_fact_and_clear_marker("t1", &event)
            .await
            .unwrap();
        // At-least-once retry of the same generation must not duplicate the row.
        store
            .commit_fact_and_clear_marker("t1", &event)
            .await
            .unwrap();

        let after = store.load("t1").await.unwrap().unwrap();
        assert!(after.marker.is_none(), "completion clears the marker");
        // Desired state + retention survive the marker clear.
        assert_eq!(after.desired_state, SandboxDesiredState::Removed);
        assert_eq!(after.retention, Some(ArchiveRetention::RetainArchives));

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_events WHERE event_id = $1")
            .bind("sandbox:t1:5:removed")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "deterministic id persists exactly once");
    }

    #[sqlx::test]
    async fn clear_marker_only_clears_the_matching_generation(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleStore::new(pool);
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Start, 1),
                },
            )
            .await
            .unwrap();
        assert!(store.load("t1").await.unwrap().unwrap().marker.is_some());
        store
            .clear_marker("t1", OperationGeneration(2))
            .await
            .unwrap();
        assert!(
            store.load("t1").await.unwrap().unwrap().marker.is_some(),
            "late acknowledgement must not clear a newer marker"
        );
        store
            .clear_marker("t1", OperationGeneration(1))
            .await
            .unwrap();
        let after = store.load("t1").await.unwrap().unwrap();
        assert!(after.marker.is_none());
        // Desired state is untouched by a marker clear.
        assert_eq!(after.desired_state, SandboxDesiredState::Running);
    }

    #[sqlx::test]
    async fn late_fact_commit_does_not_clear_a_newer_generation(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleStore::new(pool);
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Restart, 2),
                },
            )
            .await
            .unwrap();

        let old_event = LifecycleEvent {
            event_id: "sandbox:t1:1:error".to_string(),
            generation: OperationGeneration(1),
            fact: EventFact::Error,
            status: SandboxStatus::Error,
            stop_reason: None,
        };
        store
            .commit_fact_and_clear_marker("t1", &old_event)
            .await
            .unwrap();

        let after = store.load("t1").await.unwrap().unwrap();
        assert_eq!(
            after.marker.map(|marker| marker.generation),
            Some(OperationGeneration(2))
        );
    }

    #[sqlx::test]
    async fn reconcile_candidates_covers_desired_running_participants_and_incomplete_markers(
        pool: PgPool,
    ) {
        use super::super::test_support::seed_task_with;
        // Desired Running + Open → candidate.
        seed_task_with(&pool, "run-open", "open", "running").await;
        // Desired Running + Error → candidate (Error participates).
        seed_task_with(&pool, "run-error", "error", "running").await;
        // Desired Running + Closed → not a candidate (does not participate).
        seed_task_with(&pool, "run-closed", "closed", "running").await;
        // Desired Stopped + Open → not a candidate on the desired-Running branch…
        seed_task_with(&pool, "stopped-open", "open", "stopped").await;
        // …until it carries an incomplete operation marker (a crashed stop/remove).
        let store = PgLifecycleStore::new(pool.clone());
        store
            .persist_intent(
                "stopped-open",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Stopped,
                    stop_reason: Some(SandboxStopReason::User),
                    retention: None,
                    marker: marker(OperationKind::Stop, 1),
                },
            )
            .await
            .unwrap();

        let mut got = store.reconcile_candidates().await.unwrap();
        got.sort();
        assert_eq!(got, vec!["run-error", "run-open", "stopped-open"]);
    }

    #[sqlx::test]
    async fn toward_running_persist_stamps_started_at_and_non_running_preserves_it(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleStore::new(pool.clone());
        let started_at = |pool: PgPool| async move {
            sqlx::query_scalar::<_, Option<chrono::DateTime<Utc>>>(
                "SELECT started_at FROM sandboxes WHERE task_id = 't1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        assert!(
            started_at(pool.clone()).await.is_none(),
            "seed leaves it unset"
        );

        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Start, 1),
                },
            )
            .await
            .unwrap();
        let stamped = started_at(pool.clone())
            .await
            .expect("toward-running stamps started_at");

        // A later stop keeps the original start timestamp — only toward-running writes it.
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Stopped,
                    stop_reason: Some(SandboxStopReason::User),
                    retention: None,
                    marker: marker(OperationKind::Stop, 2),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            started_at(pool).await,
            Some(stamped),
            "non-running persist preserves started_at"
        );
    }

    // ---- Persistence matrix ----

    async fn sandbox_row_count(pool: &PgPool, task_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM sandboxes WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn first_operation_creates_the_sandbox_row(pool: PgPool) {
        // The atomic intent write is the row's lazy birth: a Task with no
        // Sandbox row gains one on its first lifecycle operation.
        seed_bare_task(&pool, "t1", "open").await;
        assert_eq!(
            sandbox_row_count(&pool, "t1").await,
            0,
            "no row before any operation"
        );

        let store = PgLifecycleStore::new(pool.clone());
        let effect = store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Start, 1),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            effect,
            IntentEffect::Changed,
            "first write is a change from absence"
        );
        assert_eq!(
            sandbox_row_count(&pool, "t1").await,
            1,
            "the upsert created the row"
        );
    }

    #[sqlx::test]
    async fn no_row_task_loads_as_converged_stopped_and_is_no_candidate(pool: PgPool) {
        // Absence is meaningful: no row means never-requested = converged Stopped
        // with an epoch intent timestamp (so any Message supersedes it), and the
        // Task is not a reconciler candidate.
        seed_bare_task(&pool, "t1", "open").await;
        let store = PgLifecycleStore::new(pool.clone());

        let intent = store.load("t1").await.unwrap().expect("task exists");
        assert_eq!(intent.desired_state, SandboxDesiredState::Stopped);
        assert!(intent.marker.is_none());
        assert_eq!(intent.stop_reason, None);
        assert_eq!(
            intent.desired_state_updated_at,
            sandbox_intent_epoch(),
            "no intent timestamp"
        );

        assert!(
            store.reconcile_candidates().await.unwrap().is_empty(),
            "a no-row Task is converged, not a candidate",
        );
    }

    #[sqlx::test]
    async fn generation_allocation_births_a_no_intent_row_at_epoch(pool: PgPool) {
        seed_bare_task(&pool, "t1", "open").await;
        let store = PgLifecycleStore::new(pool.clone());

        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(1)
        );

        let intent = store.load("t1").await.unwrap().expect("task exists");
        assert_eq!(intent.desired_state, SandboxDesiredState::Stopped);
        assert_eq!(intent.desired_state_updated_at, sandbox_intent_epoch());

        let views = load_sandbox_intent_views(&pool, &[String::from("t1")])
            .await
            .unwrap();
        let view = views.get("t1").expect("existing task has a projection");
        assert_eq!(view.desired_state, SandboxDesiredState::Stopped);
        assert!(
            !view.has_explicit_intent(),
            "allocation alone must not look like an explicit stop"
        );
    }

    #[sqlx::test]
    async fn sandbox_row_cascades_with_task_deletion(pool: PgPool) {
        seed(&pool, "t1").await;
        assert_eq!(sandbox_row_count(&pool, "t1").await, 1);
        sqlx::query("DELETE FROM tasks WHERE task_id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            sandbox_row_count(&pool, "t1").await,
            0,
            "ON DELETE CASCADE removes the Sandbox row with its Task",
        );
    }
}
