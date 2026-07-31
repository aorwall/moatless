//! Allocating a conversation's positions and reading the changes they order.

use sqlx::PgConnection;

use super::models::{ConversationChange, ConversationHead, RETAINED_CHANGES};
use crate::db::DbPool;

/// Record that `message_uuid` was written or rewritten, returning the position
/// allocated for it. `tx` is the connection the mutation's transaction is
/// running on — `&mut *tx`, never a pool, which would commit immediately and
/// make the record visible before the write it describes.
///
/// # Call this before `task_listing::record_change`
///
/// Both allocators hold a counter row until commit, so every transaction that
/// takes both must take them in one order or the pair deadlocks. This one goes
/// first and the shell's stays last, exactly as its own "call this last" says:
/// this counter is one row per Task and the shell's is one row for the whole
/// deployment, so this order also holds the contended one for the shorter time.
pub async fn record_message_change(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, Some(message_uuid)).await
}

/// Record that the conversation itself moved — its Turn, or the Sandbox its
/// session status is read from. Names no Message, because none changed: the
/// reader answers this by re-reading the conversation's own row.
///
/// Same transaction rule and same ordering against the shell's allocator as
/// [`record_message_change`].
pub async fn record_session_change(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None).await
}

/// The one allocating statement, shared so the two subjects cannot drift into
/// two different orderings of the same counter row. The `ON CONFLICT DO UPDATE`
/// is the allocation, and it holds that row for the rest of the transaction.
async fn allocate(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO task_conversation_position (task_id, position) VALUES ($1, 1)
               ON CONFLICT (task_id) DO UPDATE
                 SET position = task_conversation_position.position + 1
               RETURNING position
           )
           INSERT INTO task_conversation_changes (task_id, position, message_uuid)
           SELECT $1, position, $2 FROM allocated
           RETURNING position"#,
    )
    .bind(task_id)
    .bind(message_uuid)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The highest position this conversation has reached — 0 before its first
/// change, and 0 for a Task that has never had one.
///
/// The counter is the authority, never `MAX(task_conversation_changes.position)`:
/// [`prune_changes`] keeps the records for a bounded window, so on a
/// conversation older than that window a `MAX` drops back to a position long
/// since passed and hands a client a stale place to resume from, silently.
pub async fn current_head(pool: &DbPool, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
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
/// At `from` rather than after it: a client disconnected partway through one
/// change's events holds a cursor inside that position, and the events it
/// missed are the rest of that same change. Filtering the *events* by sequence
/// is what makes this exact — see
/// [`position_of`](super::models::position_of).
///
/// Safe as a polling cursor only because commits happen in position order, so
/// nothing can appear below a position this has already returned.
pub async fn changes_from(
    pool: &DbPool,
    task_id: &str,
    from: i64,
    limit: i64,
) -> Result<Vec<ConversationChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, message_uuid
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
/// window, returning how many went. The cut comes from each Task's own counter,
/// per [`current_head`]. Safe from every replica — one bounded `DELETE` over
/// durable state, so two racing delete the same rows and the second finds
/// nothing. No lease, no leader.
pub async fn prune_changes(pool: &DbPool) -> Result<u64, sqlx::Error> {
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
