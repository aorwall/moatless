//! Task event DAO — persistence for lifecycle events.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{InsertTaskEvent, TaskEvent};
use moatless_core::db::DbPool;

/// DAO trait for task event persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskEventDao: Send + Sync {
    /// Insert a single event.
    async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error>;

    /// List a task's most recent events, returned oldest-first.
    ///
    /// Selects descending so `limit` truncates the *head*, then reverses for
    /// chronological order; `offset` skips newest events, so paging walks
    /// backwards through history.
    async fn list_recent_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error>;

    /// Whether the run was already terminal when `event_id` logged — dedupes
    /// `agent.stopped`'s double publish by comparing the log's own order, not
    /// "any event but mine".
    async fn agent_run_already_terminal(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// For each task, its most recent run-state lifecycle event.
    ///
    /// `(sub_type, created_at)`, omitted if none; drives the reaper's idle-TTL
    /// floor — `started` means never reap, terminal dates it from when it last
    /// stopped.
    async fn get_latest_agent_lifecycle(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, (String, DateTime<Utc>)>, sqlx::Error>;

    /// For each task, the timestamp of its most recent `agent` event of any
    /// sub-type.
    ///
    /// The reaper's real-activity signal: a long tool-call turn with no new
    /// `messages` row still advances it, so idle-TTL tracks actual work instead
    /// of freezing.
    async fn get_latest_agent_activity(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TaskEventDao`].
pub struct PgTaskEventDao(pub DbPool);

#[async_trait]
impl TaskEventDao for PgTaskEventDao {
    async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error> {
        // Idempotent: re-delivering the same stream entry persists nothing the
        // second time, and projects nothing again. It still projects an event
        // another writer persisted, which only this path sees here.
        let mut tx = self.0.begin().await?;
        insert_in(&mut tx, event).await?;
        crate::conversation::projector::Projector
            .on_task_event(&mut tx, event)
            .await?;
        tx.commit().await
    }

    async fn list_recent_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error> {
        let types_ref: Option<&[String]> = event_types.as_deref();
        // Ordered DESC so LIMIT keeps the newest rows instead of the oldest.
        //
        // `id::bigint`: the legacy Drizzle-created `task_events` table defines
        // `id` as int4, but `TaskEvent::id` is `i64` (INT8). Without this cast
        // sqlx fails to decode every row ("Rust type `i64` ... not compatible
        // with SQL type `INT4`"), which broke the events-history endpoints
        // entirely. Casting in the query fixes decoding on both the legacy int4
        // schema and a fresh bigint schema, without a locking ALTER on a table
        // that can hold a row per tool call / streamed chunk.
        let mut rows = sqlx::query_as::<_, TaskEvent>(
            r#"SELECT
                id::bigint AS id, event_id, task_id, type AS event_type, sub_type, is_error,
                agent_profile, turn_number, payload::jsonb AS payload, timestamp, created_at
            FROM task_events
            WHERE task_id = $1
              AND ($2::text[] IS NULL OR type = ANY($2))
            ORDER BY id DESC
            LIMIT $3 OFFSET $4"#,
        )
        .bind(task_id)
        .bind(types_ref)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.0)
        .await?;
        rows.reverse();
        Ok(rows)
    }

    async fn agent_run_already_terminal(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<bool, sqlx::Error> {
        // The newest run-state event strictly *older* than this one. Ordering on
        // `id` rides `idx_task_events_task_id (task_id, id)` and gives the two
        // stop publishers a stable order — `created_at` need not, and "older
        // than me" rather than "not me" is what makes exactly one of them win
        // whichever the consumer dispatches first.
        //
        // The `id <` comparison is NULL when this event has no row (persistence
        // is best-effort), so the query returns nothing and the caller proceeds
        // — the same fail-open the guard has always had.
        let row: Option<(String,)> = sqlx::query_as(
            r#"SELECT sub_type
               FROM task_events
               WHERE task_id = $1
                 AND type = 'agent'
                 AND sub_type IN ('started', 'completed', 'stopped', 'error')
                 AND id < (SELECT id FROM task_events WHERE event_id = $2)
               ORDER BY id DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .bind(event_id)
        .fetch_optional(&self.0)
        .await?;

        Ok(matches!(row, Some((ref sub_type,)) if sub_type != "started"))
    }

    async fn get_latest_agent_lifecycle(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, (String, DateTime<Utc>)>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // DISTINCT ON (task_id) ordered by created_at desc takes each task's
        // single most-recent run-state event. The sub_type filter excludes
        // non-run-state agent events (message, session_init, recovery_*) so the
        // floor reflects only start/stop transitions.
        let rows: Vec<(String, String, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT DISTINCT ON (task_id) task_id, sub_type, created_at
               FROM task_events
               WHERE type = 'agent'
                 AND sub_type IN ('started', 'completed', 'stopped', 'error')
                 AND task_id = ANY($1)
               ORDER BY task_id, created_at DESC, id DESC"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(task_id, sub_type, ts)| (task_id, (sub_type, ts)))
            .collect())
    }

    async fn get_latest_agent_activity(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // MAX(created_at) over all agent events (any sub_type) captures the last
        // tool call / streamed chunk, so an in-flight turn keeps advancing the
        // reaper's activity floor even without a new conversation message.
        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT task_id, MAX(created_at) AS latest
               FROM task_events
               WHERE type = 'agent'
                 AND task_id = ANY($1)
               GROUP BY task_id"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().collect())
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;

/// Insert `event` inside the caller's transaction, idempotent on `event_id`;
/// `false` when it was already stored.
pub(crate) async fn insert_in(
    tx: &mut sqlx::PgConnection,
    event: &InsertTaskEvent,
) -> Result<bool, sqlx::Error> {
    let inserted = sqlx::query(
        r#"INSERT INTO task_events (
            event_id, task_id, type, sub_type, is_error,
            agent_profile, turn_number, payload, timestamp
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (event_id) DO NOTHING"#,
    )
    .bind(&event.event_id)
    .bind(&event.task_id)
    .bind(&event.event_type)
    .bind(&event.sub_type)
    .bind(event.is_error)
    .bind(&event.agent_profile)
    .bind(event.turn_number)
    .bind(&event.payload)
    .bind(&event.timestamp)
    .execute(tx)
    .await?
    .rows_affected();
    Ok(inserted > 0)
}
