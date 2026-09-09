//! Named `db-tests` fixtures over the `tags` and `task_tags` tables.
//!
//! A test that needs a tag, or a task tagged with one, reaches the store
//! through one of these rather than through the store itself.

use moatless_core::db::DbPool;

use super::dao::{PgTagDao, TagDao};
use super::models::Tag;

/// See [`TagDao::find_or_create_tags`].
pub async fn find_or_create_tags(pool: &DbPool, names: &[String]) -> Result<Vec<Tag>, sqlx::Error> {
    PgTagDao(pool.clone()).find_or_create_tags(names).await
}

/// See [`TagDao::add_task_tags`].
pub async fn add_task_tags(
    pool: &DbPool,
    task_id: &str,
    tag_ids: &[String],
) -> Result<(), sqlx::Error> {
    PgTagDao(pool.clone()).add_task_tags(task_id, tag_ids).await
}

/// See [`TagDao::set_task_tags`].
pub async fn set_task_tags(
    pool: &DbPool,
    task_id: &str,
    tag_ids: &[String],
) -> Result<(), sqlx::Error> {
    PgTagDao(pool.clone()).set_task_tags(task_id, tag_ids).await
}
