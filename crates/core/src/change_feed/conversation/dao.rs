//! Transactional change-feed persistence.

use super::position::{
    ConversationChange, ConversationHead, RETAINED_CHANGES,
};
use sqlx::{PgConnection, PgPool};

// ── The conversation's position counter and change log ──────────────────────
//
// Activities and Messages are ordered by one counter per Task, and a reader
// learns the conversation moved from the log that counter writes.

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
    allocate(tx, task_id, Some(message_uuid), None, None).await
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
    allocate(tx, task_id, None, None, None).await
}

/// Record that an activity happened on no Message — a Sandbox lifecycle change,
/// a context-window reading. Names the activity so a live reader re-reads that
/// one row rather than the conversation's own session, which nothing about the
/// activity moved.
///
/// Same transaction rule and same ordering against the shell's allocator as
/// [`record_message_change`].
pub async fn record_activity_change(
    tx: &mut PgConnection,
    task_id: &str,
    activity_id: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None, Some(activity_id), None).await
}

/// Record that the conversation was rewound to `reverted_to_turn`, dropping
/// every turn above it. `0` keeps none.
///
/// Names no Message and no activity, unlike every other record here, because
/// the rows it is about are gone by the time a reader looks: a name would
/// re-read nothing and project no event, leaving the client showing a
/// transcript this server no longer holds.
///
/// Same transaction rule and same ordering against the shell's allocator as
/// [`record_message_change`].
pub async fn record_revert_change(
    tx: &mut PgConnection,
    task_id: &str,
    reverted_to_turn: i32,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None, None, Some(reverted_to_turn)).await
}

/// Take the next position without recording a change.
///
/// Activities are ordered by this counter and notified through the Message they
/// belong to, so each one takes a position but leaves no record: a record per
/// activity would multiply the feed's row count by the number of blocks in a
/// turn to say something the Message's own record already said.
///
/// One position per activity rather than one shared by all of a Message's is
/// what keeps `(task_id, position)` a total order. Sharing one would leave
/// activities within a Message tied, and the only tiebreaker left is the id —
/// where `msg:10` sorts before `msg:2`.
///
/// Same transaction rule as [`record_message_change`], and no ordering hazard
/// against it: the first allocation in a transaction takes the counter row and
/// holds it until commit, so every later one on the same connection is free.
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
/// change, and 0 for a Task that has never had one.
///
/// The counter is the authority, never `MAX(task_conversation_changes.position)`:
/// [`prune_changes`] keeps the records for a bounded window, so on a
/// conversation older than that window a `MAX` drops back to a position long
/// since passed and hands a client a stale place to resume from, silently.
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
/// At `from` rather than after it: a client disconnected partway through one
/// change's events holds a cursor inside that position, and the events it
/// missed are the rest of that same change. Filtering the *events* by sequence
/// is what makes this exact — see
/// [`position_of`](super::position::position_of).
///
/// Safe as a polling cursor only because commits happen in position order, so
/// nothing can appear below a position this has already returned.
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
/// window, returning how many went. The cut comes from each Task's own counter,
/// per [`current_head`]. Safe from every replica — one bounded `DELETE` over
/// durable state, so two racing delete the same rows and the second finds
/// nothing. No lease, no leader.
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
