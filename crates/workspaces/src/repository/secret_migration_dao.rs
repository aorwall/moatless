//! Repository environment storage for secret migration.

use moatless_core::db::DbPool;

#[derive(Debug, sqlx::FromRow)]
pub(super) struct RepoEnvRow {
    pub id: String,
    pub created_by: Option<String>,
    pub env_vars: Option<serde_json::Value>,
}

pub(super) async fn list_repository_env(pool: &DbPool) -> Result<Vec<RepoEnvRow>, sqlx::Error> {
    let rows = sqlx::query_as(
        r#"SELECT id, created_by, env_vars
           FROM repositories
           WHERE env_vars IS NOT NULL"#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub(super) async fn strip_keys(pool: &DbPool, key: &str, ids: &[&str]) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE repositories
           SET env_vars = env_vars - $1
           WHERE id = ANY($2)"#,
    )
    .bind(key)
    .bind(ids)
    .execute(pool)
    .await?;
    Ok(())
}
