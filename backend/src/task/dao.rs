//! Task DAO — queries for task data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{Task, TaskStatus, Visibility};
use crate::db::DbPool;

/// Repository trait for task persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Find by task_id (no access control — internal/system use).
    async fn find_by_id_internal(&self, task_id: &str) -> Result<Option<Task>, sqlx::Error>;
    /// Find by task_id, scoped to user visibility.
    async fn find_by_id(&self, task_id: &str, user_id: &str) -> Result<Option<Task>, sqlx::Error>;
    async fn insert_task(&self, row: &Task) -> Result<(), sqlx::Error>;
    #[allow(clippy::too_many_arguments)]
    async fn list_tasks(
        &self,
        user_id: &str,
        limit: i64,
        offset: i64,
        status: Option<&str>,
        repository_id: Option<&str>,
        created_by: Option<&str>,
        tag: Option<&str>,
    ) -> Result<(Vec<Task>, i64), sqlx::Error>;
    async fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error>;
    async fn delete_task(&self, task_id: &str) -> Result<(), sqlx::Error>;
    async fn exists_by_short_id(&self, short_id: &str) -> Result<bool, sqlx::Error>;
    async fn get_created_times(&self, task_ids: &[String]) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error>;
}

/// PostgreSQL implementation of [`TaskRepository`].
pub struct PgTaskRepository(pub DbPool);

#[async_trait]
impl TaskRepository for PgTaskRepository {
    async fn find_by_id_internal(&self, task_id: &str) -> Result<Option<Task>, sqlx::Error> {
        sqlx::query_as::<_, Task>(
            r#"SELECT
                task_id, short_id, created_at, last_message_at, agent_type,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility, created_at_db
            FROM tasks WHERE task_id = $1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_id(&self, task_id: &str, user_id: &str) -> Result<Option<Task>, sqlx::Error> {
        sqlx::query_as::<_, Task>(
            r#"SELECT
                task_id, short_id, created_at, last_message_at, agent_type,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility, created_at_db
            FROM tasks
            WHERE task_id = $1
              AND (visibility = 'public' OR created_by = $2)"#,
        )
        .bind(task_id)
        .bind(user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert_task(&self, row: &Task) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO tasks (
                task_id, short_id, created_at, last_message_at, agent_type,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18
            )"#,
        )
        .bind(&row.task_id)
        .bind(&row.short_id)
        .bind(&row.created_at)
        .bind(&row.last_message_at)
        .bind(&row.agent_type)
        .bind(&row.repository_id)
        .bind(row.status)
        .bind(row.total_cost_usd)
        .bind(&row.site_url)
        .bind(&row.template_id)
        .bind(row.vibe)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.site_id)
        .bind(&row.site_provider)
        .bind(&row.branch)
        .bind(&row.created_by)
        .bind(row.visibility)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list_tasks(
        &self,
        user_id: &str,
        limit: i64,
        offset: i64,
        status: Option<&str>,
        repository_id: Option<&str>,
        created_by: Option<&str>,
        tag: Option<&str>,
    ) -> Result<(Vec<Task>, i64), sqlx::Error> {
        let rows = sqlx::query_as::<_, Task>(
            r#"SELECT DISTINCT
                t.task_id, t.short_id, t.created_at, t.last_message_at, t.agent_type,
                t.repository_id, t.status, t.total_cost_usd, t.site_url, t.template_id,
                t.vibe, t.name, t.description, t.site_id, t.site_provider, t.branch,
                t.created_by, t.visibility, t.created_at_db
            FROM tasks t
            LEFT JOIN task_tags tt ON t.task_id = tt.task_id
            LEFT JOIN tags tg ON tt.tag_id = tg.tag_id
            WHERE (t.visibility = 'public' OR t.created_by = $1)
              AND ($2::text IS NULL OR t.status = $2)
              AND ($3::text IS NULL OR t.repository_id = $3)
              AND ($4::text IS NULL OR t.created_by = $4)
              AND ($5::text IS NULL OR tg.name = $5)
            ORDER BY t.last_message_at DESC
            LIMIT $6 OFFSET $7"#,
        )
        .bind(user_id)
        .bind(status)
        .bind(repository_id)
        .bind(created_by)
        .bind(tag)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.0)
        .await?;

        let total: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(DISTINCT t.task_id) as count
            FROM tasks t
            LEFT JOIN task_tags tt ON t.task_id = tt.task_id
            LEFT JOIN tags tg ON tt.tag_id = tg.tag_id
            WHERE (t.visibility = 'public' OR t.created_by = $1)
              AND ($2::text IS NULL OR t.status = $2)
              AND ($3::text IS NULL OR t.repository_id = $3)
              AND ($4::text IS NULL OR t.created_by = $4)
              AND ($5::text IS NULL OR tg.name = $5)"#,
        )
        .bind(user_id)
        .bind(status)
        .bind(repository_id)
        .bind(created_by)
        .bind(tag)
        .fetch_one(&self.0)
        .await?;

        Ok((rows, total.0))
    }

    #[allow(unused_assignments)]
    async fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error> {
        let mut sets = Vec::new();
        let mut idx = 2u32;

        macro_rules! maybe_set {
            ($field:ident, $col:expr) => {
                if patch.$field.is_some() {
                    sets.push(format!("{} = ${}", $col, idx));
                    idx += 1;
                }
            };
        }

        maybe_set!(status, "status");
        maybe_set!(name, "name");
        maybe_set!(description, "description");
        maybe_set!(last_message_at, "last_message_at");
        maybe_set!(total_cost_usd, "total_cost_usd");
        maybe_set!(site_url, "site_url");
        maybe_set!(visibility, "visibility");
        maybe_set!(short_id, "short_id");

        if patch.total_cost_usd_increment.is_some() {
            sets.push(format!("total_cost_usd = total_cost_usd + ${}", idx));
            #[allow(unused_assignments)]
            { idx += 1; }
        }

        if sets.is_empty() {
            return Ok(());
        }

        let sql = format!("UPDATE tasks SET {} WHERE task_id = $1", sets.join(", "));
        let mut query = sqlx::query(&sql).bind(task_id);

        macro_rules! maybe_bind {
            ($field:ident) => {
                if let Some(ref val) = patch.$field {
                    query = query.bind(val);
                }
            };
        }

        maybe_bind!(status);
        maybe_bind!(name);
        maybe_bind!(description);
        maybe_bind!(last_message_at);
        maybe_bind!(total_cost_usd);
        maybe_bind!(site_url);
        maybe_bind!(visibility);
        maybe_bind!(short_id);
        maybe_bind!(total_cost_usd_increment);

        query.execute(&self.0).await?;
        Ok(())
    }

    async fn delete_task(&self, task_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM tasks WHERE task_id = $1")
            .bind(task_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn exists_by_short_id(&self, short_id: &str) -> Result<bool, sqlx::Error> {
        let row: (bool,) =
            sqlx::query_as("SELECT EXISTS(SELECT 1 FROM tasks WHERE short_id = $1)")
                .bind(short_id)
                .fetch_one(&self.0)
                .await?;
        Ok(row.0)
    }

    async fn get_created_times(&self, task_ids: &[String]) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            r#"SELECT task_id, COALESCE(created_at_db, NOW()) AS created_at
               FROM tasks WHERE task_id = ANY($1)"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await?;

        Ok(rows.into_iter().collect())
    }
}
/// Patch struct for partial task updates.
#[derive(Default)]
pub struct TaskPatch {
    pub status: Option<TaskStatus>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub last_message_at: Option<String>,
    pub total_cost_usd: Option<f64>,
    /// Additive increment to total_cost_usd (uses SQL `total_cost_usd + $N`).
    pub total_cost_usd_increment: Option<f64>,
    pub site_url: Option<String>,
    pub visibility: Option<Visibility>,
    pub short_id: Option<String>,
}
