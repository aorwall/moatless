//! Task event DAO — persistence for lifecycle events.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::TaskEvent;
use crate::db::DbPool;

/// Insert parameters for a new task event.
pub struct InsertTaskEvent {
    pub event_id: String,
    pub task_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    pub agent_type: Option<String>,
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
    pub timestamp: String,
}

/// A task with an unclosed turn. The authoritative reaper query now lives on
/// the `task_turns` table ([`crate::task_turn::dao::TaskTurnRepository::list_unclosed_active_turns`]);
/// this module keeps the event-log-derived [`TaskEventRepository::active_turn_for_task`]
/// for the single-task lookup the pod watcher needs. Both share this type.
pub use crate::task_turn::models::UnclosedTurn;

/// Repository trait for task event persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskEventRepository: Send + Sync {
    /// Insert a single event.
    async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error>;

    /// List events for a task, ordered by id ascending.
    async fn list_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error>;

    /// Return the currently-active (unclosed) turn for a single task, if any.
    ///
    /// Yields `Some` only when the task's latest `taskTurn` event is `active`
    /// (no terminal event has superseded it). Used by the pod watcher to attach
    /// turn context to a `turn_interrupted` event and to suppress it when no
    /// turn is in flight. The reaper's fleet-wide equivalent now reads the
    /// authoritative `task_turns` table via
    /// [`crate::task_turn::dao::TaskTurnRepository::list_unclosed_active_turns`].
    async fn active_turn_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<UnclosedTurn>, sqlx::Error>;

    /// For each task in `task_ids`, return its single most-recent agent
    /// lifecycle event as `(sub_type, created_at)`.
    ///
    /// Considers only `type = 'agent'` rows whose `sub_type` is a run-state
    /// marker (`started` | `completed` | `stopped` | `error`); other agent
    /// sub-types (`message`, `session_init`, …) are ignored. Tasks with no such
    /// event are omitted from the map.
    ///
    /// Drives the reaper's idle-TTL floor: a `started` latest event means the
    /// agent is still running (never reap); a terminal latest event dates the
    /// floor from when the agent last stopped.
    async fn get_latest_agent_lifecycle(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, (String, DateTime<Utc>)>, sqlx::Error>;

    /// For each task in `task_ids`, return the timestamp of its most recent
    /// `type = 'agent'` event of any sub-type — including the `message` events
    /// emitted for every tool call and streamed assistant chunk.
    ///
    /// This is the reaper's real-activity signal: a long agent turn (many tool
    /// calls, no new conversation `messages` row and no lifecycle transition)
    /// still advances this timestamp, so the idle-TTL floor tracks actual work
    /// instead of freezing at the last user message / sandbox start. Tasks with
    /// no agent events are omitted from the map.
    async fn get_latest_agent_activity(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TaskEventRepository`].
pub struct PgTaskEventRepository(pub DbPool);

#[async_trait]
impl TaskEventRepository for PgTaskEventRepository {
    async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error> {
        // ON CONFLICT DO NOTHING makes insertion idempotent on event_id (which
        // carries a unique index): re-delivering the same stream entry — e.g.
        // when the consumer recovers its pending list after a disconnect —
        // persists nothing the second time instead of erroring on the duplicate.
        sqlx::query(
            r#"INSERT INTO task_events (
                event_id, task_id, type, sub_type, is_error,
                agent_type, turn_number, payload, timestamp
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (event_id) DO NOTHING"#,
        )
        .bind(&event.event_id)
        .bind(&event.task_id)
        .bind(&event.event_type)
        .bind(&event.sub_type)
        .bind(event.is_error)
        .bind(&event.agent_type)
        .bind(event.turn_number)
        .bind(&event.payload)
        .bind(&event.timestamp)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error> {
        let types_ref: Option<&[String]> = event_types.as_deref();
        sqlx::query_as::<_, TaskEvent>(
            r#"SELECT
                id, event_id, task_id, type AS event_type, sub_type, is_error,
                agent_type, turn_number, payload::jsonb AS payload, timestamp, created_at
            FROM task_events
            WHERE task_id = $1
              AND ($2::text[] IS NULL OR type = ANY($2))
            ORDER BY id ASC
            LIMIT $3 OFFSET $4"#,
        )
        .bind(task_id)
        .bind(types_ref)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.0)
        .await
    }

    async fn active_turn_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<UnclosedTurn>, sqlx::Error> {
        // Take this task's single most-recent taskTurn event (the inner LIMIT 1),
        // then keep it only when that latest row is `active` — a terminal event
        // would have produced a newer row and superseded it.
        let row: Option<(Option<i32>, Option<String>)> = sqlx::query_as(
            r#"SELECT latest.turn_number, latest.agent_type
               FROM (
                 SELECT sub_type, turn_number, agent_type
                 FROM task_events
                 WHERE task_id = $1 AND type = 'taskTurn'
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1
               ) latest
               WHERE latest.sub_type = 'active'"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;

        Ok(row.map(|(turn_number, agent_type)| UnclosedTurn {
            task_id: task_id.to_string(),
            turn_number,
            agent_type,
        }))
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
