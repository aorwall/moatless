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

/// Latest successful S3 workspace upload reported by a sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSyncUpload {
    pub bytes: i64,
    pub created_at: DateTime<Utc>,
}

/// A task with an unclosed turn. The authoritative reaper query now lives on
/// the `task_turns` table ([`crate::task_turn::dao::TaskTurnDao::list_unclosed_active_turns`]);
/// this module keeps the event-log-derived [`TaskEventDao::active_turn_for_task`]
/// for the single-task lookup the pod watcher needs. Both share this type.
pub use crate::task_turn::models::UnclosedTurn;

/// DAO trait for task event persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskEventDao: Send + Sync {
    /// Insert a single event.
    async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error>;

    /// List events for a task from the OLDEST, ordered by id ascending.
    ///
    /// `limit` therefore truncates the *tail* — a task with more events than
    /// `limit` never surfaces its recent ones. Use this only where replaying
    /// from the start is the intent (the SSE history prelude); readers that
    /// want the current state of a task want [`Self::list_recent_events`].
    async fn list_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error>;

    /// List a task's most recent events, returned oldest-first.
    ///
    /// Selects descending so `limit` truncates the *head* (dropping old events)
    /// rather than the tail, then reverses so callers still read chronologically
    /// — the same shape [`Self::list_events`] returns. `offset` skips that many
    /// of the newest events, so paging walks backwards through history.
    async fn list_recent_events(
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
    /// [`crate::task_turn::dao::TaskTurnDao::list_unclosed_active_turns`].
    async fn active_turn_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<UnclosedTurn>, sqlx::Error>;

    /// Whether the task's agent run had already reached a terminal state
    /// (`completed` | `stopped` | `error`) by the time `event_id` was logged —
    /// i.e. the newest run-state event *older* than it is not `started`.
    ///
    /// The stop fan-out dedupe: one user stop publishes `agent.stopped` twice
    /// (the `/stop` handler, and the sandbox answering the graceful
    /// `StopAgent`), and only the first should count. Both rows can already be
    /// in the log by dispatch time — the consumer persists an event before
    /// dispatching it, and the sandbox's ingress commits its row before
    /// publishing — so the comparison is against the log's own order, not
    /// against "any event but mine".
    async fn agent_run_already_terminal(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<bool, sqlx::Error>;

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

    /// Return the task's most recent successful workspace upload marker.
    ///
    /// These markers are written from `ReportWorkspaceSync` after the sandbox
    /// completes a direct S3 upload. A present row means the backend has durable
    /// evidence that workspace data reached S3 before a later pod death.
    async fn latest_successful_workspace_upload(
        &self,
        task_id: &str,
    ) -> Result<Option<WorkspaceSyncUpload>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TaskEventDao`].
pub struct PgTaskEventDao(pub DbPool);

#[async_trait]
impl TaskEventDao for PgTaskEventDao {
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
        // `id::bigint`: the legacy Drizzle-created `task_events` table defines
        // `id` as int4, but `TaskEvent::id` is `i64` (INT8). Without this cast
        // sqlx fails to decode every row ("Rust type `i64` ... not compatible
        // with SQL type `INT4`"), which broke the events-history endpoints
        // entirely. Casting in the query fixes decoding on both the legacy int4
        // schema and a fresh bigint schema, without a locking ALTER on a table
        // that can hold a row per tool call / streamed chunk.
        sqlx::query_as::<_, TaskEvent>(
            r#"SELECT
                id::bigint AS id, event_id, task_id, type AS event_type, sub_type, is_error,
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

    async fn list_recent_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error> {
        let types_ref: Option<&[String]> = event_types.as_deref();
        // Same projection as `list_events` (see the `id::bigint` note there),
        // ordered DESC so LIMIT keeps the newest rows instead of the oldest.
        let mut rows = sqlx::query_as::<_, TaskEvent>(
            r#"SELECT
                id::bigint AS id, event_id, task_id, type AS event_type, sub_type, is_error,
                agent_type, turn_number, payload::jsonb AS payload, timestamp, created_at
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

    async fn latest_successful_workspace_upload(
        &self,
        task_id: &str,
    ) -> Result<Option<WorkspaceSyncUpload>, sqlx::Error> {
        let row: Option<(i64, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT COALESCE((payload::jsonb ->> 'bytes')::bigint, 0) AS bytes, created_at
               FROM task_events
               WHERE task_id = $1
                 AND type = 'sandbox'
                 AND sub_type = 'workspace_synced'
                 AND payload::jsonb ->> 'direction' = 'upload'
                 AND COALESCE((payload::jsonb ->> 'ok')::boolean, false)
               ORDER BY created_at DESC, id DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;

        Ok(row.map(|(bytes, created_at)| WorkspaceSyncUpload { bytes, created_at }))
    }
}
