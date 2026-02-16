//! Tag DAO — queries for tag data access.

use async_trait::async_trait;

use super::models::{Tag, TaskTag};
use crate::db::DbPool;

/// Repository trait for tag persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TagRepository: Send + Sync {
    async fn list_all(&self) -> Result<Vec<Tag>, sqlx::Error>;
    async fn add_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error>;
    async fn find_or_create_tags(&self, names: &[String]) -> Result<Vec<Tag>, sqlx::Error>;
    async fn set_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error>;
    async fn get_tags_for_task(&self, task_id: &str) -> Result<Vec<TaskTag>, sqlx::Error>;
    async fn get_tags_for_tasks(&self, task_ids: &[String]) -> Result<Vec<TaskTag>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TagRepository`].
pub struct PgTagRepository(pub DbPool);

#[async_trait]
impl TagRepository for PgTagRepository {
    async fn list_all(&self) -> Result<Vec<Tag>, sqlx::Error> {
        sqlx::query_as::<_, Tag>(
            "SELECT id, tag_id, name, description, created_at FROM tags ORDER BY name ASC",
        )
        .fetch_all(&self.0)
        .await
    }

    async fn add_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error> {
        for tag_id in tag_ids {
            sqlx::query(
                "INSERT INTO task_tags (task_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(task_id)
            .bind(tag_id)
            .execute(&self.0)
            .await?;
        }
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

            let row = sqlx::query_as::<_, Tag>(
                "SELECT id, tag_id, name, description, created_at FROM tags WHERE name = $1",
            )
            .bind(name)
            .fetch_one(&self.0)
            .await?;

            tags.push(row);
        }

        Ok(tags)
    }

    async fn set_task_tags(&self, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM task_tags WHERE task_id = $1")
            .bind(task_id)
            .execute(&self.0)
            .await?;

        for tag_id in tag_ids {
            sqlx::query("INSERT INTO task_tags (task_id, tag_id) VALUES ($1, $2)")
                .bind(task_id)
                .bind(tag_id)
                .execute(&self.0)
                .await?;
        }

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
    PgTagRepository(pool.clone()).list_all().await
}

/// Find or create tags by name (convenience wrapper).
pub async fn find_or_create_tags(pool: &DbPool, names: &[String]) -> Result<Vec<Tag>, sqlx::Error> {
    PgTagRepository(pool.clone()).find_or_create_tags(names).await
}

/// Set tags for a task (replaces existing) (convenience wrapper).
pub async fn set_task_tags(pool: &DbPool, task_id: &str, tag_ids: &[String]) -> Result<(), sqlx::Error> {
    PgTagRepository(pool.clone()).set_task_tags(task_id, tag_ids).await
}

/// Get tags for a single task (convenience wrapper).
pub async fn get_tags_for_task(pool: &DbPool, task_id: &str) -> Result<Vec<TaskTag>, sqlx::Error> {
    PgTagRepository(pool.clone()).get_tags_for_task(task_id).await
}

/// Get tags for multiple tasks (convenience wrapper).
pub async fn get_tags_for_tasks(pool: &DbPool, task_ids: &[String]) -> Result<Vec<TaskTag>, sqlx::Error> {
    PgTagRepository(pool.clone()).get_tags_for_tasks(task_ids).await
}
