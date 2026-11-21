//! The source rows a conversation is replayed from: a Task's own for a
//! rebuild, and each ancestor's up to its cut for a fork (4.9a).

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::inbox::InboxItem;
use crate::inbox::models::InboxRow;
use crate::trajectory::models::TrajectorySession;

/// A Trajectory line on the replayed lineage, or in a subagent session hung
/// under it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ReplayLine {
    pub id: Uuid,
    pub session_id: Uuid,
    /// The Task whose session holds the line.
    pub task_id: String,
    pub seq: i64,
    pub harness_item_id: Option<String>,
    pub content: Value,
    pub turn_number: Option<i32>,
    pub created_at: DateTime<Utc>,
    /// In a subagent session rather than on the lineage itself.
    pub subagent: bool,
}

/// A `task_events` row the Projector reads.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ReplayEvent {
    pub event_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    pub turn_number: Option<i32>,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

/// Every line on `tip`'s lineage, skipping switch prefixes, and every line of
/// the subagent sessions hung under one, in `ord` order.
///
/// Two statements deliberately: one query matching `id IN lineage OR
/// session_id IN subagents` seq-scans every `trajectory_events` row, which took
/// 20-27 s on staging, rather than the two index lookups here.
pub(crate) async fn lineage_lines(
    tx: &mut PgConnection,
    tip: Uuid,
) -> Result<Vec<ReplayLine>, sqlx::Error> {
    let (ids, subagents): (Vec<Uuid>, Vec<Uuid>) = sqlx::query_as(
        r#"WITH RECURSIVE lineage(id, parent_event_id, jump) AS (
               SELECT e.id, e.parent_event_id,
                      CASE WHEN e.seq <= s.own_from_seq THEN s.transcoded_from_event_id END
                 FROM trajectory_events e JOIN trajectory_sessions s ON s.id = e.session_id
                WHERE e.id = $1
               UNION
               SELECT e.id, e.parent_event_id,
                      CASE WHEN e.seq <= s.own_from_seq THEN s.transcoded_from_event_id END
                 FROM lineage l
                 JOIN trajectory_events e ON e.id = l.parent_event_id OR e.id = l.jump
                 JOIN trajectory_sessions s ON s.id = e.session_id
           ),
           subagents(id) AS (
               SELECT s.id FROM trajectory_sessions s JOIN lineage l ON l.id = s.parent_event_id
               UNION
               SELECT s.id FROM trajectory_sessions s
                 JOIN trajectory_events e ON e.id = s.parent_event_id
                 JOIN subagents p ON p.id = e.session_id
           )
           SELECT COALESCE((SELECT array_agg(id) FROM lineage), '{}'),
                  COALESCE((SELECT array_agg(id) FROM subagents), '{}')"#,
    )
    .bind(tip)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query_as(
        r#"SELECT e.id, e.session_id, s.task_id, e.seq, e.harness_item_id, e.content,
                  e.turn_number, e.created_at, e.session_id = ANY($2) AS subagent
             FROM (SELECT id FROM trajectory_events WHERE id = ANY($1)
                   UNION
                   SELECT id FROM trajectory_events WHERE session_id = ANY($2)) picked
             JOIN trajectory_events e USING (id)
             JOIN trajectory_sessions s ON s.id = e.session_id
            WHERE s.own_from_seq IS NULL OR e.seq >= s.own_from_seq
            ORDER BY e.ord"#,
    )
    .bind(&ids)
    .bind(&subagents)
    .fetch_all(tx)
    .await
}

/// A frozen row a fork copies: a Message, or an activity and the Message it
/// is on.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct FrozenRow {
    pub id: Uuid,
    pub message: bool,
    pub message_uuid: Option<Uuid>,
    pub parent_tool_use_id: Option<String>,
    pub position: i64,
}

/// `task_id`'s frozen rows through `turn` and at or before `cut_at`, in
/// position order: its Messages, and the activities on them or on none. An
/// activity sorts after the Message sharing its position.
pub(crate) async fn frozen_rows(
    tx: &mut PgConnection,
    task_id: &str,
    turn: Option<i32>,
    cut_at: DateTime<Utc>,
) -> Result<Vec<FrozenRow>, sqlx::Error> {
    sqlx::query_as(
        r#"WITH m AS (
               SELECT uuid, parent_tool_use_id, position FROM conversation_messages
                WHERE task_id = $1 AND frozen AND created_at <= $3
                  AND (turn_number IS NULL OR $2::int IS NULL OR turn_number <= $2)
           )
           SELECT id, message, message_uuid, parent_tool_use_id, position FROM (
               SELECT uuid AS id, true AS message, NULL::uuid AS message_uuid,
                      parent_tool_use_id, position, 0 AS ordinal
                 FROM m
               UNION ALL
               SELECT a.activity_id, false, a.message_uuid, a.parent_tool_use_id, a.position,
                      a.ordinal
                 FROM conversation_activities a
                WHERE a.task_id = $1 AND a.frozen AND a.created_at <= $3
                  AND (a.message_uuid IN (SELECT uuid FROM m) OR NOT EXISTS (
                          SELECT 1 FROM conversation_messages x WHERE x.uuid = a.message_uuid))
           ) rows
           ORDER BY position, message DESC, ordinal, id"#,
    )
    .bind(task_id)
    .bind(turn)
    .bind(cut_at)
    .fetch_all(tx)
    .await
}

/// The Message ids `task_id`'s frozen usage rows name that no Message holds.
pub(crate) async fn orphan_frozen_usage(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT u.message_uuid FROM conversation_usage u
            WHERE u.task_id = $1 AND u.message_uuid IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM conversation_messages m WHERE m.uuid = u.message_uuid)"#,
    )
    .bind(task_id)
    .fetch_all(tx)
    .await
}

/// The sessions `ids` name.
pub(crate) async fn sessions(
    tx: &mut PgConnection,
    ids: &[Uuid],
) -> Result<Vec<TrajectorySession>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT id, task_id, harness, harness_session_id, forked_from_event_id,
                  parent_event_id, transcoded_from_event_id, own_from_seq, last_event_id, last_seq
             FROM trajectory_sessions WHERE id = ANY($1)"#,
    )
    .bind(ids)
    .fetch_all(tx)
    .await
}

/// `task_id`'s inbox items a conversation shows: all of the Task's own since
/// `after`, or an ancestor's delivered through `through`'s turn and at or
/// before its cut. An ancestor's item a later rewind cut is still one it held
/// at the cut; one rewound before the cut is not.
pub(crate) async fn inbox_items(
    tx: &mut PgConnection,
    task_id: &str,
    through: Option<(i32, DateTime<Utc>)>,
    after: Option<DateTime<Utc>>,
) -> Result<Vec<InboxItem>, sqlx::Error> {
    let rows: Vec<InboxRow> = sqlx::query_as(
        r#"SELECT * FROM task_inbox
            WHERE task_id = $1 AND origin <> 'sandbox'
              AND CASE WHEN $2::int IS NULL THEN status IN ('pending', 'delivered')
                  ELSE turn_number <= $2 AND created_at <= $3
                       AND (status = 'delivered' OR (status = 'rewound' AND rewound_at > $3))
                  END
              AND ($4::timestamptz IS NULL OR created_at > $4)
            ORDER BY created_at, id"#,
    )
    .bind(task_id)
    .bind(through.map(|(turn, _)| turn))
    .bind(through.map(|(_, at)| at))
    .bind(after)
    .fetch_all(tx)
    .await?;
    rows.into_iter().map(InboxItem::try_from).collect()
}

/// `task_id`'s task events the Projector reads: an ancestor's before turn
/// `before_turn` (or, with no turn, at or before `cut_at`), or all of the
/// Task's own since `after`. An event of a turn a rewind deleted and a later
/// turn reused predates that turn's start, and is skipped.
pub(crate) async fn task_events(
    tx: &mut PgConnection,
    task_id: &str,
    before_turn: Option<(i32, DateTime<Utc>)>,
    after: Option<DateTime<Utc>>,
) -> Result<Vec<ReplayEvent>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT t.event_id, t.type AS event_type, t.sub_type, t.is_error, t.turn_number,
                  t.payload::jsonb AS payload, t.created_at
             FROM task_events t
            WHERE t.task_id = $1
              AND ((t.type = 'task' AND t.sub_type IN ('error', 'binding_reply_sent', 'agent_switched',
                                                            'artifact_shown', 'secret_requested',
                                                            'secret_request_answered'))
                   OR (t.type = 'agent' AND t.sub_type IN ('completed', 'context_window', 'recording_stopped'))
                   OR (t.type = 'sandbox' AND t.sub_type IN ('initializing', 'ready', 'stopped', 'error')))
              AND ($2::int IS NULL
                   OR t.turn_number < $2
                   OR (t.turn_number IS NULL AND t.created_at <= $3))
              AND ($4::timestamptz IS NULL OR t.created_at > $4)
              AND (t.type = 'sandbox' OR t.turn_number IS NULL OR EXISTS (
                      SELECT 1 FROM task_turns tt
                       WHERE tt.task_id = t.task_id AND tt.turn_number = t.turn_number
                         AND tt.started_at <= t.created_at))
            ORDER BY t.created_at, t.id"#,
    )
    .bind(task_id)
    .bind(before_turn.map(|(turn, _)| turn))
    .bind(before_turn.map(|(_, at)| at))
    .bind(after)
    .fetch_all(tx)
    .await
}

/// When `task_id`'s history froze, if it did.
pub(crate) async fn frozen_at(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar("SELECT frozen_at FROM conversation_frozen WHERE task_id = $1")
        .bind(task_id)
        .fetch_optional(tx)
        .await
}

/// The event `task_id`'s conversation ends on: its fork or rewind cut until
/// it records past it, else its latest top-level session's tip.
pub(crate) async fn conversation_tip(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT COALESCE(
               (SELECT f.source_event_id FROM task_forks f
                 WHERE f.task_id = $1 AND NOT EXISTS (
                       SELECT 1 FROM trajectory_sessions s
                         JOIN trajectory_events e ON e.session_id = s.id
                        WHERE s.task_id = $1 AND s.parent_event_id IS NULL
                          AND e.created_at > f.created_at)),
               (SELECT s.last_event_id FROM trajectory_sessions s
                  JOIN trajectory_events e ON e.id = s.last_event_id
                 WHERE s.task_id = $1 AND s.parent_event_id IS NULL
                 ORDER BY e.ord DESC LIMIT 1))"#,
    )
    .bind(task_id)
    .fetch_one(tx)
    .await
}

/// Writes the fork's closed cut turn: `turn_number` as the source recorded it,
/// an open one as `stopped`, and decided, since nothing awaits a fork's
/// trigger. Without it the running rule reads the inherited user rows as a
/// turn still owed.
pub(crate) async fn close_cut_turn(
    tx: &mut PgConnection,
    fork_task_id: &str,
    source_task_id: &str,
    turn_number: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO task_turns
               (task_id, turn_number, status, agent_profile, started_at, ended_at, trigger_decided)
           SELECT $2, $3,
                  CASE WHEN t.status IS NULL THEN 'completed'
                       WHEN t.status IN ('active', 'interrupted') THEN 'stopped'
                       ELSE t.status END,
                  t.agent_profile, NOW(), NOW(), true
             FROM (SELECT 1) one
             LEFT JOIN task_turns t ON t.task_id = $1 AND t.turn_number = $3
           ON CONFLICT DO NOTHING"#,
    )
    .bind(source_task_id)
    .bind(fork_task_id)
    .bind(turn_number)
    .execute(tx)
    .await?;
    Ok(())
}

/// Deletes a fork's inherited conversation and closed cut turn, for a fork
/// whose create failed.
pub(crate) async fn discard(tx: &mut PgConnection, task_id: &str) -> Result<(), sqlx::Error> {
    super::dao::delete_projected(&mut *tx, task_id, None).await?;
    sqlx::query("DELETE FROM task_turns WHERE task_id = $1")
        .bind(task_id)
        .execute(tx)
        .await?;
    Ok(())
}
