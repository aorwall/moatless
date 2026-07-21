//! Postgres [`LifecycleStore`] — durable intent, generation allocation, and the
//! transactional-outbox completion commit.
//!
//! The atomic intent write collapses desired state, stop reason, archive
//! retention, and the correlation marker into **one** statement (a CTE captures
//! the pre-image so `persist_intent` can report whether intent actually changed).
//! `commit_completion` persists the deterministic completion event and clears the
//! marker in one transaction, so the deterministic event id is never live without
//! its persisted fact.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::db::DbPool;
use crate::task::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};

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
const CLEAR_MARKER_SQL: &str = r#"UPDATE tasks SET
       sandbox_operation_kind = NULL,
       sandbox_operation_generation = NULL,
       sandbox_operation_started_at = NULL,
       updated_at = NOW()
   WHERE task_id = $1"#;

/// Clear the correlation marker on `executor`. Generic over the executor so the
/// completion transaction (`&mut tx`) and the standalone clear (`&pool`) issue
/// one and the same statement.
async fn clear_marker_on<'e, E>(executor: E, task_id: &str) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(CLEAR_MARKER_SQL)
        .bind(task_id)
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

#[async_trait]
impl LifecycleStore for PgLifecycleStore {
    async fn load(&self, task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
        let row: Option<(
            TaskStatus,
            SandboxDesiredState,
            Option<SandboxStopReason>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            r#"SELECT
                status,
                sandbox_desired_state,
                sandbox_stop_reason,
                sandbox_archive_retention,
                sandbox_operation_kind,
                sandbox_operation_generation,
                sandbox_operation_started_at,
                sandbox_desired_state_updated_at
            FROM tasks
            WHERE task_id = $1"#,
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
                desired_state,
                stop_reason,
                retention: retention.as_deref().and_then(ArchiveRetention::from_db_str),
                marker: marker_from_columns(op_kind, op_generation, op_started_at),
                desired_state_updated_at,
            },
        ))
    }

    async fn allocate_generation(&self, task_id: &str) -> Result<OperationGeneration, StoreError> {
        let seq: Option<i64> = sqlx::query_scalar(
            r#"UPDATE tasks
               SET sandbox_operation_seq = sandbox_operation_seq + 1
               WHERE task_id = $1
               RETURNING sandbox_operation_seq"#,
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
        // Single atomic statement. `prev` captures the pre-image under a row lock
        // so `changed` reflects the desired-state / stop-reason / retention delta
        // — the intent-change axis — while the marker is always overwritten.
        let changed: Option<bool> = sqlx::query_scalar(
            r#"WITH prev AS (
                   SELECT sandbox_desired_state, sandbox_stop_reason, sandbox_archive_retention
                   FROM tasks
                   WHERE task_id = $1
                   FOR UPDATE
               )
               UPDATE tasks t SET
                   sandbox_desired_state = $2,
                   sandbox_stop_reason = $3,
                   sandbox_archive_retention = $4,
                   sandbox_operation_kind = $5,
                   sandbox_operation_generation = $6,
                   sandbox_operation_started_at = $7,
                   sandbox_desired_state_updated_at = NOW(),
                   updated_at = NOW()
               FROM prev
               WHERE t.task_id = $1
               RETURNING (
                   prev.sandbox_desired_state IS DISTINCT FROM $2
                   OR prev.sandbox_stop_reason IS DISTINCT FROM $3
                   OR prev.sandbox_archive_retention IS DISTINCT FROM $4
               )"#,
        )
        .bind(task_id)
        .bind(write.desired_state)
        .bind(write.stop_reason)
        .bind(write.retention.map(ArchiveRetention::as_str))
        .bind(write.marker.kind.as_str())
        .bind(write.marker.generation.value())
        .bind(write.marker.started_at)
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

    async fn commit_completion(
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
        clear_marker_on(&mut *tx, task_id)
            .await
            .map_err(|e| StoreError::with_source("clear marker on completion", e))?;

        tx.commit()
            .await
            .map_err(|e| StoreError::with_source("commit completion transaction", e))
    }

    async fn clear_marker(&self, task_id: &str) -> Result<(), StoreError> {
        clear_marker_on(&self.pool, task_id)
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
        Utc.timestamp_opt(1_700_000_000, 0).single().expect("valid ts")
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
/// applies `backend/migrations` (including 0089).
#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::super::events::EventFact;
    use super::super::test_support::seed_task as seed;
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
        assert_eq!(store.allocate_generation("t1").await.unwrap(), OperationGeneration(1));
        assert_eq!(store.allocate_generation("t1").await.unwrap(), OperationGeneration(2));
        assert_eq!(store.allocate_generation("t1").await.unwrap(), OperationGeneration(3));
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
        assert_eq!(store.persist_intent("t1", &write).await.unwrap(), IntentEffect::Changed);
        // Same intent, fresh marker → the intent axis is Unchanged even though the
        // marker moved (Changed covers only desired-state / reason / retention).
        let write2 = IntentWrite { marker: marker(OperationKind::Stop, 2), ..write.clone() };
        assert_eq!(store.persist_intent("t1", &write2).await.unwrap(), IntentEffect::Unchanged);

        let loaded = store.load("t1").await.unwrap().expect("intent");
        assert_eq!(loaded.desired_state, SandboxDesiredState::Stopped);
        assert_eq!(loaded.stop_reason, Some(SandboxStopReason::User));
        let m = loaded.marker.expect("marker");
        assert_eq!(m.kind, OperationKind::Stop);
        assert_eq!(m.generation, OperationGeneration(2));
    }

    #[sqlx::test]
    async fn remove_persists_retention_and_commit_completion_clears_marker(pool: PgPool) {
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
            fact: EventFact::Removed,
            status: SandboxStatus::Removed,
            stop_reason: None,
        };
        store.commit_completion("t1", &event).await.unwrap();
        // At-least-once retry of the same generation must not duplicate the row.
        store.commit_completion("t1", &event).await.unwrap();

        let after = store.load("t1").await.unwrap().unwrap();
        assert!(after.marker.is_none(), "completion clears the marker");
        // Desired state + retention survive the marker clear.
        assert_eq!(after.desired_state, SandboxDesiredState::Removed);
        assert_eq!(after.retention, Some(ArchiveRetention::RetainArchives));

        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM task_events WHERE event_id = $1")
                .bind("sandbox:t1:5:removed")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "deterministic id persists exactly once");
    }

    #[sqlx::test]
    async fn clear_marker_is_a_targeted_clear(pool: PgPool) {
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
        store.clear_marker("t1").await.unwrap();
        let after = store.load("t1").await.unwrap().unwrap();
        assert!(after.marker.is_none());
        // Desired state is untouched by a marker clear.
        assert_eq!(after.desired_state, SandboxDesiredState::Running);
    }
}
