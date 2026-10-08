use super::position::{UiRpcEnvironmentIdentity, UiRpcListingChange, UiRpcListingHead};
use crate::UiRpcDbPool;

pub async fn current_head(pool: &UiRpcDbPool) -> Result<UiRpcListingHead, sqlx::Error> {
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
    Ok(UiRpcListingHead {
        identity: UiRpcEnvironmentIdentity::new(identity),
        position,
    })
}

/// The changes after `after`, each followed at its own position by the changed
/// Task's `created_by` or `forked_from` parent: a parent's row counts its
/// children and reads Waiting while one works. `limit` caps the changes read,
/// not the rows returned.
pub async fn changes_after(
    pool: &UiRpcDbPool,
    after: i64,
    limit: i64,
) -> Result<Vec<UiRpcListingChange>, sqlx::Error> {
    sqlx::query_as(
        r#"WITH changed AS (
               SELECT position, task_id
               FROM task_listing_changes
               WHERE position > $1
               ORDER BY position
               LIMIT $2
           )
           SELECT position, task_id FROM changed
           UNION
           SELECT c.position, r.to_task_id
           FROM changed c
           JOIN task_relations r ON r.from_task_id = c.task_id
           WHERE r.relation_type IN ('created_by', 'forked_from')
           ORDER BY position, task_id"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
