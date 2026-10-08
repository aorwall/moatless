//! Transactional change-feed persistence.

use super::position::{EnvironmentIdentity, ListingChange, ListingHead, RETAINED_CHANGES};
use sqlx::{PgConnection, PgPool};

/// Record that `task_id`'s listing-visible state changed, then each
/// `created_by`/`forked_from` parent at its own next position (the shell stream
/// drops a repeated one), returning `task_id`'s position.
///
/// Pass `&mut *tx`, never a pool. Call it last: the counter is one row, so a
/// lock taken after it deadlocks under load.
pub async fn record_change(tx: &mut PgConnection, task_id: &str) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH targets AS (
               SELECT $1::text AS task_id, 0::bigint AS ord
               UNION ALL
               SELECT to_task_id, row_number() OVER (ORDER BY to_task_id)
               FROM (
                   SELECT DISTINCT to_task_id
                   FROM task_relations
                   WHERE from_task_id = $1
                     -- `moatless_tasks`'s `PARENT_RELATION_TYPES`.
                     AND relation_type IN ('created_by', 'forked_from')
               ) parents
           ),
           allocated AS (
               INSERT INTO task_listing_position (id, position)
               VALUES (TRUE, (SELECT count(*) FROM targets))
               ON CONFLICT (id) DO UPDATE
                 SET position = task_listing_position.position
                              + (SELECT count(*) FROM targets)
               RETURNING position
           ),
           recorded AS (
               INSERT INTO task_listing_changes (position, task_id)
               SELECT a.position - (SELECT count(*) FROM targets) + 1 + t.ord, t.task_id
               FROM allocated a CROSS JOIN targets t
               RETURNING position, task_id
           )
           SELECT min(position) FROM recorded"#,
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
    // call it. Verified rather than assumed — `router_if_enabled` fails startup
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
