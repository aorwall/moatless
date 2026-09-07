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

pub async fn changes_after(
    pool: &UiRpcDbPool,
    after: i64,
    limit: i64,
) -> Result<Vec<UiRpcListingChange>, sqlx::Error> {
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
