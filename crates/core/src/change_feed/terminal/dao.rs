//! Terminal-relay persistence.
//!
//! Built to the shape `change_feed::preview`'s dao states in full: why the
//! counter rather than `MAX` answers for the head, why the cursor is safe to
//! poll on, and why any replica may prune.

use super::position::{RETAINED_CHANGES, TerminalSessionStart};
use serde_json::Value;
use sqlx::PgPool;

/// Record that `origin` started `terminal_id` on `task_id`, returning the
/// allocated position.
///
/// Takes a pool, not a transaction: the session is running on the origin
/// before this row exists, so it publishes nothing a reader could reach early.
pub async fn record_start(
    pool: &PgPool,
    origin: &str,
    task_id: &str,
    terminal_id: &str,
    summary: &Value,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO terminal_session_position (id, position) VALUES (TRUE, 1)
               ON CONFLICT (id) DO UPDATE
                 SET position = terminal_session_position.position + 1
               RETURNING position
           )
           INSERT INTO terminal_session_changes (position, origin, task_id, terminal_id, summary)
           SELECT position, $1, $2, $3, $4 FROM allocated
           RETURNING position"#,
    )
    .bind(origin)
    .bind(task_id)
    .bind(terminal_id)
    .bind(summary)
    .fetch_one(pool)
    .await?;
    Ok(position)
}

/// The highest position this deployment has reached; 0 before the first start.
///
/// Read off the counter. `MAX(terminal_session_changes.position)` is not the
/// same number once [`prune_starts`] has run.
pub async fn current_position(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) =
        sqlx::query_as(r#"SELECT COALESCE((SELECT position FROM terminal_session_position), 0)"#)
            .fetch_one(pool)
            .await?;
    Ok(position)
}

/// Every start recorded after `after`, oldest first, capped at `limit`.
///
/// `after` works as a polling cursor because [`record_start`] does not release
/// the counter row until its own insert has committed.
pub async fn starts_after(
    pool: &PgPool,
    after: i64,
    limit: i64,
) -> Result<Vec<TerminalSessionStart>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, origin, task_id, terminal_id, summary
           FROM terminal_session_changes
           WHERE position > $1
           ORDER BY position
           LIMIT $2"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Delete the starts that have fallen out of the retention window; returns how
/// many went.
///
/// Every replica calls this. The `DELETE` names the same rows whoever runs it,
/// so a loser of the race deletes nothing and needs no lease to find out.
pub async fn prune_starts(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM terminal_session_changes
           WHERE position <= (SELECT position FROM terminal_session_position) - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
}
