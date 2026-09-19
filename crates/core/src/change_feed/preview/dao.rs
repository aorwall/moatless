//! Preview-relay persistence.

use super::position::{PreviewTabOpen, RETAINED_CHANGES};
use sqlx::PgPool;

/// Record that `origin` opened `tab_id` on `task_id` at `url`, returning the
/// allocated position.
///
/// A pool rather than a transaction, unlike the Task feeds: the tab this
/// announces is already stored and published on the origin, so there is no
/// earlier state a reader could observe out of order.
pub async fn record_open(
    pool: &PgPool,
    origin: &str,
    task_id: &str,
    tab_id: &str,
    url: &str,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO preview_tab_position (id, position) VALUES (TRUE, 1)
               ON CONFLICT (id) DO UPDATE
                 SET position = preview_tab_position.position + 1
               RETURNING position
           )
           INSERT INTO preview_tab_changes (position, origin, task_id, tab_id, url)
           SELECT position, $1, $2, $3, $4 FROM allocated
           RETURNING position"#,
    )
    .bind(origin)
    .bind(task_id)
    .bind(tab_id)
    .bind(url)
    .fetch_one(pool)
    .await?;
    Ok(position)
}

/// The highest position this deployment has reached; 0 before the first open.
///
/// The counter is the authority, never `MAX(preview_tab_changes.position)`:
/// [`prune_opens`] bounds that window, so `MAX` would read as a rewind on a
/// deployment quiet for longer than the retention.
pub async fn current_position(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) =
        sqlx::query_as(r#"SELECT COALESCE((SELECT position FROM preview_tab_position), 0)"#)
            .fetch_one(pool)
            .await?;
    Ok(position)
}

/// Every open recorded after `after`, oldest first, capped at `limit`.
///
/// Safe as a polling cursor: [`record_open`] holds the counter row's lock
/// through its own insert, so commits land in position order.
pub async fn opens_after(
    pool: &PgPool,
    after: i64,
    limit: i64,
) -> Result<Vec<PreviewTabOpen>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, origin, task_id, tab_id, url
           FROM preview_tab_changes
           WHERE position > $1
           ORDER BY position
           LIMIT $2"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Delete the opens that have fallen out of the retention window; returns how
/// many went.
///
/// Safe from every replica: one bounded `DELETE` over durable state, so two
/// racing prunes target the same rows and the second finds nothing. No lease,
/// no leader.
pub async fn prune_opens(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM preview_tab_changes
           WHERE position <= (SELECT position FROM preview_tab_position) - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
}
