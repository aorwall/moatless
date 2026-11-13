//! Transactional change-feed persistence.

use super::position::{
    EnvironmentIdentity, ListingChange, ListingHead, ListingSubject, RETAINED_CHANGES,
};
use moatless_types::task::TaskRelationType;
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
                     AND relation_type = ANY($2)
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
    .bind(&TaskRelationType::PARENT_WIRE[..])
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// Record that `workspace_id`'s project row changed, returning its position.
///
/// Same counter and same rule as [`record_change`]: pass `&mut *tx`, and call
/// it last.
pub async fn record_workspace_change(
    tx: &mut PgConnection,
    workspace_id: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        r#"WITH allocated AS (
               INSERT INTO task_listing_position (id, position)
               VALUES (TRUE, 1)
               ON CONFLICT (id) DO UPDATE
                 SET position = task_listing_position.position + 1
               RETURNING position
           )
           INSERT INTO workspace_listing_changes (position, workspace_id)
           SELECT position, $1 FROM allocated
           RETURNING position"#,
    )
    .bind(workspace_id)
    .fetch_one(tx)
    .await
}

/// The highest position this deployment has reached, and the database
/// identity it reached it in.
///
/// 0 before the first change; both values come from **one** statement. The
/// counter is authoritative, never `MAX(task_listing_changes.position)`:
/// [`prune_changes`] bounds that window, so `MAX` can go stale.
pub async fn current_head(pool: &PgPool) -> Result<ListingHead, sqlx::Error> {
    // `pg_control_system()` needs no special ACL. Verified rather than
    // assumed: `router_if_enabled` fails startup if an ordinary role can't
    // read this identity (see test
    // `the_identity_is_readable_by_a_least_privilege_application_role`).
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

/// Every change recorded after `after`, Task and Workspace alike, oldest
/// first, capped at `limit`.
///
/// Safe as a polling cursor only because commits happen in position order, so
/// nothing can appear below a position already returned.
pub async fn changes_after(
    pool: &PgPool,
    after: i64,
    limit: i64,
) -> Result<Vec<ListingChange>, sqlx::Error> {
    let rows: Vec<(i64, Option<String>, Option<String>)> = sqlx::query_as(
        r#"(SELECT position, task_id, NULL::text AS workspace_id
            FROM task_listing_changes
            WHERE position > $1
            ORDER BY position
            LIMIT $2)
           UNION ALL
           (SELECT position, NULL::text, workspace_id
            FROM workspace_listing_changes
            WHERE position > $1
            ORDER BY position
            LIMIT $2)
           ORDER BY position
           LIMIT $2"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(position, task_id, workspace_id)| {
            let subject = match (task_id, workspace_id) {
                (Some(task_id), _) => ListingSubject::Task(task_id),
                (None, Some(workspace_id)) => ListingSubject::Workspace(workspace_id),
                (None, None) => return None,
            };
            Some(ListingChange { position, subject })
        })
        .collect())
}

/// Delete the changes that have fallen out of the retention window; returns how
/// many went.
///
/// Safe from every replica: one bounded `DELETE` over durable state, so two
/// racing prunes target the same rows and the second finds nothing. No lease,
/// no leader.
pub async fn prune_changes(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let tasks = sqlx::query(
        r#"DELETE FROM task_listing_changes
           WHERE position <= (SELECT position FROM task_listing_position) - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    let workspaces = sqlx::query(
        r#"DELETE FROM workspace_listing_changes
           WHERE position <= (SELECT position FROM task_listing_position) - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(tasks + workspaces)
}
