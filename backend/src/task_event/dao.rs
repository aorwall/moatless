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

/// A task whose most recent `taskTurn` event is `active` — i.e. a turn that
/// was started but never observed a terminal (`completed`/`error`/`stopped`)
/// event. Surfaced by [`TaskEventRepository::list_tasks_with_unclosed_active_turn`]
/// so the reaper can reconcile a turn left stuck after a lost terminal event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnclosedTurn {
    pub task_id: String,
    pub turn_number: Option<i32>,
    pub agent_type: Option<String>,
}

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

    /// List tasks whose most recent `taskTurn` event is `active` and older than
    /// `older_than`. A task appears here only when no terminal `taskTurn` event
    /// (`completed`/`error`/`stopped`) followed its latest `active` — meaning
    /// the turn is stuck. The age floor keeps a slow-but-live turn out of the
    /// result.
    async fn list_tasks_with_unclosed_active_turn(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error>;

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

    async fn list_tasks_with_unclosed_active_turn(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
        // For each task, take its single most-recent taskTurn event (DISTINCT ON
        // ordered by created_at desc). A task is stuck when that latest event is
        // `active` and predates the cutoff — a terminal event would have
        // produced a newer row and superseded it. Matching on "latest row"
        // rather than per-turn-number is null-safe and naturally picks the most
        // recent turn.
        let rows: Vec<(String, Option<i32>, Option<String>)> = sqlx::query_as(
            r#"SELECT latest.task_id, latest.turn_number, latest.agent_type
               FROM (
                 SELECT DISTINCT ON (task_id)
                        task_id, sub_type, turn_number, agent_type, created_at
                 FROM task_events
                 WHERE type = 'taskTurn'
                 ORDER BY task_id, created_at DESC, id DESC
               ) latest
               WHERE latest.sub_type = 'active'
                 AND latest.created_at < $1"#,
        )
        .bind(older_than)
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(task_id, turn_number, agent_type)| UnclosedTurn {
                task_id,
                turn_number,
                agent_type,
            })
            .collect())
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
}
