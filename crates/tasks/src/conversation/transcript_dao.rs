//! The SQL behind [`super::transcript`]: a Task's top-level Messages in the
//! columns a transcript reads, and the Turn rows it is rendered with.

use moatless_core::db::DbPool;
use sqlx::AssertSqlSafe;

use super::transcript::{RewoundTurn, TranscriptEntry, TurnOutcome};

/// A Task's top-level Messages in the columns a transcript reads, aliased
/// `m`: a user Message's text, origin and uploads are its inbox item's, a
/// frozen row's the columns it was frozen with. `$1` is the Task.
macro_rules! transcript_rows {
    () => {
        r#"SELECT m.position::int AS id, m.uuid::text AS uuid, m.task_id, m.position,
                  CASE WHEN m.frozen THEN m.legacy ->> 'message_type'
                       WHEN m.author = 'user' THEN 'user'
                       WHEN m.author = 'agent' THEN 'assistant'
                       WHEN m.kind = 'error' THEN 'error'
                       ELSE 'result' END AS message_type,
                  CASE WHEN m.frozen THEN m.legacy ->> 'message_role'
                       WHEN m.author = 'user' THEN 'user'
                       ELSE 'assistant' END AS message_role,
                  COALESCE(m.text, i.text) AS message_text,
                  CASE WHEN i.delivery_options ? 'toolResponse' THEN 'tool_response'
                       WHEN m.frozen THEN COALESCE(m.legacy ->> 'user_type', 'system')
                       WHEN m.author = 'user' THEN 'human'
                       ELSE 'system' END AS user_type,
                  COALESCE(i.origin = 'sandbox', false) AS sandbox_wake,
                  m.turn_number,
                  COALESCE((i.delivery_options ->> 'queue')::boolean, false) AS held,
                  i.queue_order,
                  COALESCE(m.legacy ->> 'timestamp',
                           to_char(m.created_at AT TIME ZONE 'UTC',
                                   'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')) AS "timestamp",
                  m.created_at,
                  COALESCE(i.adapter, m.legacy ->> 'external_adapter') AS external_adapter,
                  COALESCE(i.external_source_name, m.legacy ->> 'external_source_name')
                      AS external_source_name,
                  COALESCE(i.external_subject_key, m.legacy ->> 'external_subject_key')
                      AS external_subject_key,
                  -- An inherited Message claims no event of its ancestor's.
                  CASE WHEN i.task_id <> m.task_id THEN NULL
                       ELSE COALESCE(i.external_message_id, m.legacy ->> 'external_event_id')
                  END AS external_event_id,
                  COALESCE(i.external_author, m.legacy ->> 'external_author') AS external_author,
                  CASE WHEN i.task_id <> m.task_id THEN i.task_id
                       ELSE COALESCE(i.source_task_id, m.legacy ->> 'source_task_id')
                  END AS source_task_id,
                  COALESCE(i.sent_by, m.legacy ->> 'sent_by') AS sent_by,
                  i.delivery_options -> 'contexts' AS contexts,
                  i.delivery_options ->> 'childOutcome' AS child_outcome
             FROM conversation_messages m
             LEFT JOIN task_inbox i ON i.id = m.inbox_id
            WHERE m.task_id = $1 AND m.parent_tool_use_id IS NULL
              -- A Message removed from the queue before any turn took it.
              AND i.status IS DISTINCT FROM 'cancelled'"#
    };
}

/// The columns a transcript read projects, over the rows aliased `m` and the
/// uploads joined onto them.
///
/// Kept in one place: the snapshot and live-delta reads must select the same
/// shape into [`TranscriptEntry`], or a column present on reload goes missing
/// from the frame that delivered it.
const TRANSCRIPT_COLUMNS: &str = r#"m.id, m.uuid, m.message_type, m.message_role,
                      m.message_text, m.user_type, m.sandbox_wake, m.turn_number,
                      m.held, m.queue_order, m."timestamp",
                      m.created_at, m.external_adapter, m.external_source_name,
                      m.external_subject_key, m.external_event_id,
                      m.external_author, m.source_task_id, m.sent_by,
                      files.file_contexts, m.child_outcome"#;

/// The uploads a Message carries, from its inbox item's `file` contexts.
const TRANSCRIPT_FILES_JOIN: &str = r#"LEFT JOIN LATERAL (
                          SELECT jsonb_agg(
                                     jsonb_build_object(
                                         'referenceId', c ->> 'referenceId',
                                         'data', c -> 'data'
                                     )
                                     ORDER BY n
                                 ) AS file_contexts
                          FROM jsonb_array_elements(COALESCE(m.contexts, '[]'::jsonb))
                               WITH ORDINALITY AS e(c, n)
                          WHERE c ->> 'type' = 'file'
                      ) files ON true"#;

/// The newest `limit` top-level Messages, oldest first.
pub(super) async fn newest(
    pool: &DbPool,
    task_id: &str,
    limit: i64,
) -> Result<Vec<TranscriptEntry>, sqlx::Error> {
    // Newest first, reversed below: `position`'s unique index read
    // backwards makes the cap a bounded scan, not a sort. Keep the outer
    // `ORDER BY`: the subquery's order does not survive the joins.
    //
    // Subagent rows are excluded before the cap: they're bulk but never
    // rendered inline, so counting them would evict the real conversation.
    let sql = format!(
        r#"SELECT {TRANSCRIPT_COLUMNS}
           FROM ({} ORDER BY m.position DESC LIMIT $2) m
           {TRANSCRIPT_FILES_JOIN}
           ORDER BY m.position DESC"#,
        transcript_rows!(),
    );
    let mut entries = sqlx::query_as::<_, TranscriptEntry>(AssertSqlSafe(sql))
        .bind(task_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    entries.reverse();
    Ok(entries)
}

/// How many top-level Messages the Task has, cancelled queue entries included.
pub(super) async fn top_level_count(pool: &DbPool, task_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT count(*)
           FROM conversation_messages
           WHERE task_id = $1
             AND parent_tool_use_id IS NULL"#,
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
}

/// Every Turn's ending, ascending.
pub(super) async fn turn_outcomes(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<TurnOutcome>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT tt.turn_number, tt.status, tt.started_at, tt.ended_at,
                  failure.error, failure.reason, failure.resets_at
             FROM task_turns tt
             LEFT JOIN LATERAL (
                 SELECT (e.payload::jsonb)->>'error' AS error,
                        (e.payload::jsonb)->>'reason' AS reason,
                        (e.payload::jsonb)->>'resetsAt' AS resets_at
                 FROM task_events e
                 WHERE tt.status = 'error'
                   AND e.task_id = tt.task_id
                   AND e.turn_number = tt.turn_number
                   AND e.type IN ('agent', 'taskTurn')
                   AND e.sub_type = 'error'
                 ORDER BY e.id DESC
                 LIMIT 1
             ) failure ON TRUE
            WHERE tt.task_id = $1
            ORDER BY tt.turn_number"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// The turns a rewind cut, ascending.
pub(super) async fn rewound_turns(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<RewoundTurn>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT turn_number, started_at, rewound_at FROM task_rewound_turns
           WHERE task_id = $1 ORDER BY turn_number"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// The named top-level Messages, oldest first.
pub(super) async fn by_uuids(
    pool: &DbPool,
    task_id: &str,
    uuids: &[String],
) -> Result<Vec<TranscriptEntry>, sqlx::Error> {
    // A result settles the activity the call already wrote rather than landing
    // on its own Message; the change naming it points to the Message that
    // *made* the call. Top-level only: subagent rows never render inline.
    let sql = format!(
        r#"SELECT {TRANSCRIPT_COLUMNS}
           FROM ({} AND m.uuid::text = ANY($2)) m
           {TRANSCRIPT_FILES_JOIN}
           ORDER BY m.position"#,
        transcript_rows!(),
    );
    sqlx::query_as::<_, TranscriptEntry>(AssertSqlSafe(sql))
        .bind(task_id)
        .bind(uuids)
        .fetch_all(pool)
        .await
}

/// The oldest `limit` settled turn numbers above `above`.
pub(super) async fn settled_turn_numbers(
    pool: &DbPool,
    task_id: &str,
    above: Option<i32>,
    limit: i64,
) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT turn_number FROM task_turns
           WHERE task_id = $1 AND status NOT IN ('active', 'interrupted')
             AND turn_number > COALESCE($2, -1)
           ORDER BY turn_number
           LIMIT $3"#,
    )
    .bind(task_id)
    .bind(above)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// The top-level Messages acknowledged into `turns`, oldest first.
pub(super) async fn in_turns(
    pool: &DbPool,
    task_id: &str,
    turns: &[i32],
) -> Result<Vec<TranscriptEntry>, sqlx::Error> {
    let sql = format!(
        r#"SELECT {TRANSCRIPT_COLUMNS}
           FROM ({} AND m.turn_number = ANY($2)) m
           {TRANSCRIPT_FILES_JOIN}
           ORDER BY m.position"#,
        transcript_rows!(),
    );
    sqlx::query_as::<_, TranscriptEntry>(AssertSqlSafe(sql))
        .bind(task_id)
        .bind(turns)
        .fetch_all(pool)
        .await
}
