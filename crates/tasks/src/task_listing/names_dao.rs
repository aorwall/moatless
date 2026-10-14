//! The names of Tasks a viewer may read, by id.

use std::collections::HashMap;

use sqlx::AssertSqlSafe;

use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};

pub(crate) async fn readable_names(
    pool: &DbPool,
    viewer: &ScopeContext,
    task_ids: &[String],
) -> Result<HashMap<String, String>, sqlx::Error> {
    // `$1` is the id set; the access filter binds `is_admin` then `user_id`
    // from `$2`.
    let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "", 2);
    let sql = format!(
        "SELECT task_id, name FROM tasks WHERE task_id = ANY($1) AND {}",
        access.sql()
    );
    let rows: Vec<(String, String)> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(task_ids)
        .bind(viewer.is_admin)
        .bind(&viewer.user_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}
