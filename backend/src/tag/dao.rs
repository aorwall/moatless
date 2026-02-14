//! Tag DAO — queries for tag data access.

use super::models::{Tag, TaskTag};
use crate::db::DbPool;

/// List all tags.
pub async fn list_all(pool: &DbPool) -> Result<Vec<Tag>, sqlx::Error> {
    sqlx::query_as::<_, Tag>(
        "SELECT id, tag_id, name, description, created_at FROM tags ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await
}

/// Add tags to a task (additive — does not remove existing tags).
pub async fn add_task_tags(
    pool: &DbPool,
    task_id: &str,
    tag_ids: &[String],
) -> Result<(), sqlx::Error> {
    for tag_id in tag_ids {
        sqlx::query(
            "INSERT INTO task_tags (task_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(task_id)
        .bind(tag_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Find or create tags by name. Returns all matching tag rows.
pub async fn find_or_create_tags(pool: &DbPool, names: &[String]) -> Result<Vec<Tag>, sqlx::Error> {
    if names.is_empty() {
        return Ok(vec![]);
    }

    let mut tags = Vec::with_capacity(names.len());
    for name in names {
        let tag_id = uuid::Uuid::new_v4().to_string();
        // INSERT on conflict DO NOTHING, then SELECT
        sqlx::query(
            r#"INSERT INTO tags (tag_id, name)
               VALUES ($1, $2)
               ON CONFLICT (name) DO NOTHING"#,
        )
        .bind(&tag_id)
        .bind(name)
        .execute(pool)
        .await?;

        let row = sqlx::query_as::<_, Tag>(
            "SELECT id, tag_id, name, description, created_at FROM tags WHERE name = $1",
        )
        .bind(name)
        .fetch_one(pool)
        .await?;

        tags.push(row);
    }

    Ok(tags)
}

/// Set tags for a task (replaces all existing tag associations).
pub async fn set_task_tags(
    pool: &DbPool,
    task_id: &str,
    tag_ids: &[String],
) -> Result<(), sqlx::Error> {
    // Delete existing associations
    sqlx::query("DELETE FROM task_tags WHERE task_id = $1")
        .bind(task_id)
        .execute(pool)
        .await?;

    // Insert new associations
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO task_tags (task_id, tag_id) VALUES ($1, $2)")
            .bind(task_id)
            .bind(tag_id)
            .execute(pool)
            .await?;
    }

    Ok(())
}

/// Get tags for a single task.
pub async fn get_tags_for_task(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<TaskTag>, sqlx::Error> {
    sqlx::query_as::<_, TaskTag>(
        r#"SELECT tt.task_id, tg.tag_id, tg.name
           FROM task_tags tt
           JOIN tags tg ON tt.tag_id = tg.tag_id
           WHERE tt.task_id = $1"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// Get tags for multiple tasks in a batch.
pub async fn get_tags_for_tasks(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<Vec<TaskTag>, sqlx::Error> {
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
    .fetch_all(pool)
    .await
}
