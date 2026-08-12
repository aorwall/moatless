//! Allocating positions and reading the changes they order.

use sqlx::PgConnection;

use super::models::{EnvironmentIdentity, ListingChange, ListingHead, RETAINED_CHANGES};
use moatless_core::db::DbPool;

/// Record that `task_id`'s listing-visible state changed, returning the position
/// allocated for it. `tx` is the connection a transaction is running on — `&mut
/// *tx`, never a pool, which would commit immediately and make the record
/// visible before the mutation it describes.
///
/// # Call this last
///
/// **Take every other lock this transaction needs before calling this, and take
/// none afterwards.** The counter is one row for the whole deployment: taking it
/// *before* a `tasks` row puts this transaction on the opposite side of a lock
/// order from every writer that takes it after, and the pair deadlocks under
/// concurrent load. Last makes the counter the innermost lock of everything that
/// touches it — a total order by construction, needing no agreement between
/// callers — and it shortens the hold on the row the whole deployment serializes
/// on: `the_counter_is_measured_under_concurrent_mutation` prices the same
/// transaction both ways, and last runs about half again as fast.
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

/// The highest position this deployment has reached — 0 before the first change
/// — together with the identity of the database it reached it in, from **one**
/// statement, which is why this returns a pair. See [`ListingHead`].
///
/// The counter is the authority here and everywhere below it, never
/// `MAX(task_listing_changes.position)`: [`prune_changes`] keeps the records for
/// a bounded window, so on a deployment older than that window a `MAX` drops
/// back to a position long since passed and hands clients a stale place to
/// resume from, silently. Neither lookup touches a heap, which matters because
/// the subscription poll makes this call every interval.
pub async fn current_head(pool: &DbPool) -> Result<ListingHead, sqlx::Error> {
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
pub async fn current_position(pool: &DbPool) -> Result<i64, sqlx::Error> {
    Ok(current_head(pool).await?.position)
}

/// Every change recorded after `after`, oldest first, capped at `limit`. Safe as
/// a polling cursor only because commits happen in position order, so nothing
/// can appear below a position this has already returned.
pub async fn changes_after(
    pool: &DbPool,
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

/// Delete the changes that have fallen out of the retention window, returning
/// how many went. The cut comes from the counter, per [`current_head`]. Safe
/// from every replica — one bounded `DELETE` over durable state, so two racing
/// delete the same rows and the second finds nothing. No lease, no leader.
pub async fn prune_changes(pool: &DbPool) -> Result<u64, sqlx::Error> {
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
