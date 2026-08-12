//! Tag DAO — queries for tag data access.

use async_trait::async_trait;

use super::models::{Tag, TaskTag};
use crate::task_listing;
use moatless_core::db::DbPool;

/// DAO trait for tag persistence.
// `MockTagDao` is consumed by task-creation tests and downstream backend test
// tiers, so the mock is generated off `test-support` to cross that crate seam.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait]
pub trait TagDao: Send + Sync {
    async fn list_all(&self) -> Result<Vec<Tag>, sqlx::Error>;
    async fn add_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error>;
    async fn find_or_create_tags(&self, names: &[String]) -> Result<Vec<Tag>, sqlx::Error>;
    async fn set_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error>;
    async fn get_tags_for_task(&self, task_id: &str) -> Result<Vec<TaskTag>, sqlx::Error>;
    async fn get_tags_for_tasks(&self, task_ids: &[String]) -> Result<Vec<TaskTag>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TagDao`].
pub struct PgTagDao(pub DbPool);

#[async_trait]
impl TagDao for PgTagDao {
    async fn list_all(&self) -> Result<Vec<Tag>, sqlx::Error> {
        sqlx::query_as::<_, Tag>("SELECT * FROM tags ORDER BY name ASC")
            .fetch_all(&self.0)
            .await
    }

    async fn add_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error> {
        if tag_ids.is_empty() {
            return Ok(());
        }
        let mut tx = self.0.begin().await?;
        let mut added = 0;
        for tag_id in tag_ids {
            let res = sqlx::query(
                "INSERT INTO task_tags (task_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(task_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
            added += res.rows_affected();
        }
        // Only when a tag was actually added — re-adding changes nothing and
        // must not advance the position. Last, per "call this last".
        if added > 0 {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn find_or_create_tags(&self, names: &[String]) -> Result<Vec<Tag>, sqlx::Error> {
        if names.is_empty() {
            return Ok(vec![]);
        }

        let mut tags = Vec::with_capacity(names.len());
        for name in names {
            let tag_id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                r#"INSERT INTO tags (tag_id, name)
                   VALUES ($1, $2)
                   ON CONFLICT (name) DO NOTHING"#,
            )
            .bind(&tag_id)
            .bind(name)
            .execute(&self.0)
            .await?;

            let row = sqlx::query_as::<_, Tag>("SELECT * FROM tags WHERE name = $1")
                .bind(name)
                .fetch_one(&self.0)
                .await?;

            tags.push(row);
        }

        Ok(tags)
    }

    async fn set_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let removed = sqlx::query("DELETE FROM task_tags WHERE task_id = $1")
            .bind(task_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();

        for tag_id in tag_ids {
            sqlx::query("INSERT INTO task_tags (task_id, tag_id) VALUES ($1, $2)")
                .bind(task_id)
                .bind(tag_id)
                .execute(&mut *tx)
                .await?;
        }

        // A replace that touched no row either way is a no-op. Anything else
        // moved the tag set, and the whole set is replaced here rather than
        // diffed, so "removed nothing and added nothing" is the only silence.
        if removed > 0 || !tag_ids.is_empty() {
            task_listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn get_tags_for_task(&self, task_id: &str) -> Result<Vec<TaskTag>, sqlx::Error> {
        sqlx::query_as::<_, TaskTag>(
            r#"SELECT tt.task_id, tg.tag_id, tg.name
               FROM task_tags tt
               JOIN tags tg ON tt.tag_id = tg.tag_id
               WHERE tt.task_id = $1"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    async fn get_tags_for_tasks(&self, task_ids: &[String]) -> Result<Vec<TaskTag>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as::<_, TaskTag>(
            r#"SELECT tt.task_id, tg.tag_id, tg.name
               FROM task_tags tt
               JOIN tags tg ON tt.tag_id = tg.tag_id
               WHERE tt.task_id = ANY($1)"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await
    }
}

// =========================================================================
// Free-function wrappers for integration tests
// =========================================================================

/// List all tags (convenience wrapper).
pub async fn list_all(pool: &DbPool) -> Result<Vec<Tag>, sqlx::Error> {
    PgTagDao(pool.clone()).list_all().await
}

/// Find or create tags by name (convenience wrapper).
pub async fn find_or_create_tags(pool: &DbPool, names: &[String]) -> Result<Vec<Tag>, sqlx::Error> {
    PgTagDao(pool.clone()).find_or_create_tags(names).await
}

/// Set tags for a task (replaces existing) (convenience wrapper).
pub async fn set_task_tags(
    pool: &DbPool,
    task_id: &str,
    tag_ids: &[String],
) -> Result<(), sqlx::Error> {
    PgTagDao(pool.clone()).set_task_tags(task_id, tag_ids).await
}

/// Get tags for a single task (convenience wrapper).
pub async fn get_tags_for_task(pool: &DbPool, task_id: &str) -> Result<Vec<TaskTag>, sqlx::Error> {
    PgTagDao(pool.clone()).get_tags_for_task(task_id).await
}

/// Get tags for multiple tasks (convenience wrapper).
pub async fn get_tags_for_tasks(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<Vec<TaskTag>, sqlx::Error> {
    PgTagDao(pool.clone()).get_tags_for_tasks(task_ids).await
}
