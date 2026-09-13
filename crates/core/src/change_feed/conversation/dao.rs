//! Transactional change-feed persistence.

use super::position::{
    ConversationChange, ConversationHead, RETAINED_CHANGES,
};
use sqlx::{PgConnection, PgPool};

// ── The conversation's position counter and change log ──────────────────────
//
// Activities and Messages are ordered by one counter per Task, and a reader
// learns the conversation moved from the log that counter writes.

/// Record that `message_uuid` was written or rewritten, returning the allocated
/// position.
///
/// `tx` must be `&mut *tx`, never a pool, to avoid exposing the record early;
/// call before `task_listing::record_change` since both allocators lock a
/// counter row until commit.
pub async fn record_message_change(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, Some(message_uuid), None, None).await
}

/// Record that the conversation itself moved — its Turn, or the Sandbox its
/// session status is read from.
///
/// Names no Message; the reader re-reads the conversation's own row. Same `tx`
/// and ordering rules as [`record_message_change`].
pub async fn record_session_change(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None, None, None).await
}

/// Record that an activity happened on no Message — a Sandbox lifecycle change
/// or a context-window reading.
///
/// Names the activity so a live reader re-reads that row instead of the
/// conversation's own session. Same `tx` and ordering rules as
/// [`record_message_change`].
pub async fn record_activity_change(
    tx: &mut PgConnection,
    task_id: &str,
    activity_id: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None, Some(activity_id), None).await
}

/// Record that the conversation was rewound to `reverted_to_turn` (`0` keeps
/// none), dropping every turn above it.
///
/// Names no Message and no activity, since those rows are gone by the time a
/// reader looks. Same `tx` and ordering rules as [`record_message_change`].
pub async fn record_revert_change(
    tx: &mut PgConnection,
    task_id: &str,
    reverted_to_turn: i32,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None, None, Some(reverted_to_turn)).await
}

/// Take the next position without recording a change.
///
/// One position per activity keeps `(task_id, position)` a total order —
/// sharing one with a Message ties activities via lexicographic order (`msg:10`
/// before `msg:2`).
pub async fn allocate_position(tx: &mut PgConnection, task_id: &str) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"INSERT INTO task_conversation_position (task_id, position) VALUES ($1, 1)
           ON CONFLICT (task_id) DO UPDATE
             SET position = task_conversation_position.position + 1
           RETURNING position"#,
    )
    .bind(task_id)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The one allocating statement, shared so the two subjects cannot drift into
/// two different orderings of the same counter row. The `ON CONFLICT DO UPDATE`
/// is the allocation, and it holds that row for the rest of the transaction.
async fn allocate(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: Option<&str>,
    activity_id: Option<&str>,
    reverted_to_turn: Option<i32>,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO task_conversation_position (task_id, position) VALUES ($1, 1)
               ON CONFLICT (task_id) DO UPDATE
                 SET position = task_conversation_position.position + 1
               RETURNING position
           )
           INSERT INTO task_conversation_changes
               (task_id, position, message_uuid, activity_id, reverted_to_turn)
           SELECT $1, position, $2, $3, $4 FROM allocated
           RETURNING position"#,
    )
    .bind(task_id)
    .bind(message_uuid)
    .bind(activity_id)
    .bind(reverted_to_turn)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The highest position this conversation has reached — 0 before its first
/// change.
///
/// The counter is the authority, never
/// `MAX(task_conversation_changes.position)`: [`prune_changes`] bounds that
/// table, so `MAX` can silently drop back on an old conversation.
pub async fn current_head(pool: &PgPool, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
    let position: Option<(i64,)> =
        sqlx::query_as(r#"SELECT position FROM task_conversation_position WHERE task_id = $1"#)
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(ConversationHead {
        position: position.map_or(0, |(position,)| position),
    })
}

/// Every change to this conversation recorded at or after `from`, oldest first,
/// capped at `limit`.
///
/// At `from`, not after: a disconnected client's cursor can sit inside a change
/// it partly missed; filtering *events* by sequence
/// ([`position_of`](super::position::position_of)) keeps this a safe polling
/// cursor.
pub async fn changes_from(
    pool: &PgPool,
    task_id: &str,
    from: i64,
    limit: i64,
) -> Result<Vec<ConversationChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, message_uuid, activity_id, reverted_to_turn
           FROM task_conversation_changes
           WHERE task_id = $1 AND position >= $2
           ORDER BY position
           LIMIT $3"#,
    )
    .bind(task_id)
    .bind(from)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Delete the changes that have fallen out of each conversation's retention
/// window; returns how many went.
///
/// Safe from every replica: one bounded `DELETE` over durable state, so two
/// racing prunes target the same rows and the second finds nothing. No lease,
/// no leader.
pub async fn prune_changes(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM task_conversation_changes c
           USING task_conversation_position p
           WHERE c.task_id = p.task_id
             AND c.position <= p.position - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
}
