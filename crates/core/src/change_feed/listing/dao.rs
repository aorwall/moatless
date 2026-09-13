//! Transactional change-feed persistence.

use super::position::{EnvironmentIdentity, ListingChange, ListingHead, RETAINED_CHANGES};
use sqlx::{PgConnection, PgPool};

/// Record that `task_id`'s listing-visible state changed, returning the
/// allocated position.
///
/// `tx` must be `&mut *tx`, never a pool, which would commit immediately and
/// expose the record early. Take every other lock this transaction needs before
/// calling this, none after — the counter is one row for the whole deployment,
/// so calling it out of order deadlocks under load.
pub async fn record_change(tx: &mut PgConnection, task_id: &str) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO task_listing_position (id, position) VALUES (TRUE, 1)
               ON CONFLICT (id) DO UPDATE
                 SET position = task_listing_position.position + 1
               RETURNING position
           )
           INSERT INTO task_listing_changes (position, task_id)
           SELECT position, $1 FROM allocated
           RETURNING position"#,
    )
    .bind(task_id)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The highest position this deployment has reached, and the database identity
/// it reached it in.
///
/// 0 before the first change; both values come from **one** statement (see
/// [`ListingHead`]). The counter is the authority, never
/// `MAX(task_listing_changes.position)`: [`prune_changes`] bounds that window,
/// so `MAX` can silently go stale on a deployment older than it.
pub async fn current_head(pool: &PgPool) -> Result<ListingHead, sqlx::Error> {
    // `pg_control_system()` carries no ACL, so an ordinary application role can
    // call it. Verified rather than assumed — `router_from_env` fails startup
    // rather than inventing an identity, so the reverse would be a deployment
    // blocker. Decision 7, and
    // `the_identity_is_readable_by_a_least_privilege_application_role`.
    let (position, identity): (i64, String) = sqlx::query_as(
        r#"SELECT
               COALESCE((SELECT position FROM task_listing_position), 0),
               (SELECT system_identifier::text FROM pg_control_system())
                   || ':'
                   || (SELECT oid::text FROM pg_database
                       WHERE datname = current_database())"#,
    )
    .fetch_one(pool)
    .await?;
    Ok(ListingHead {
        identity: EnvironmentIdentity::new(identity),
        position,
    })
}

/// The counter alone, for callers with no use for the identity beside it.
pub async fn current_position(pool: &PgPool) -> Result<i64, sqlx::Error> {
    Ok(current_head(pool).await?.position)
}

/// Every change recorded after `after`, oldest first, capped at `limit`.
///
/// Safe as a polling cursor only because commits happen in position order, so
/// nothing can appear below a position already returned.
pub async fn changes_after(
    pool: &PgPool,
    after: i64,
    limit: i64,
) -> Result<Vec<ListingChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, task_id
           FROM task_listing_changes
           WHERE position > $1
           ORDER BY position
           LIMIT $2"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Delete the changes that have fallen out of the retention window; returns how
/// many went.
///
/// Safe from every replica: one bounded `DELETE` over durable state, so two
/// racing prunes target the same rows and the second finds nothing. No lease,
/// no leader.
pub async fn prune_changes(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM task_listing_changes
           WHERE position <= (SELECT position FROM task_listing_position) - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
}
