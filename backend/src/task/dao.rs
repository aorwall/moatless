//! Task DAO — queries for task data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{Task, TaskStatus, Visibility};
use crate::db::DbPool;

/// Filter struct for task listing queries.
#[derive(Default)]
pub struct TaskFilter {
    pub user_id: String,
    pub limit: i64,
    pub offset: i64,
    pub status: Option<String>,
    pub repository_id: Option<String>,
    pub created_by: Option<String>,
    pub tag: Option<String>,
}

/// Repository trait for task persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Find by task_id (no access control — internal/system use).
    async fn find_by_id_internal(&self, task_id: &str) -> Result<Option<Task>, sqlx::Error>;
    /// Find by task_id, scoped to user visibility.
    async fn find_by_id(&self, task_id: &str, user_id: &str) -> Result<Option<Task>, sqlx::Error>;
    async fn insert_task(&self, row: &Task) -> Result<(), sqlx::Error>;
    async fn list_tasks(
        &self,
        filter: &TaskFilter,
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
        filter: &TaskFilter,
    ) -> Result<(Vec<Task>, i64), sqlx::Error> {
        // Use a CTE with window function to get total count in one query
        #[derive(sqlx::FromRow)]
        struct TaskWithCount {
            task_id: String,
            short_id: Option<String>,
            created_at: String,
            last_message_at: String,
            agent_type: String,
            repository_id: String,
            status: TaskStatus,
            total_cost_usd: f64,
            site_url: Option<String>,
            template_id: Option<String>,
            vibe: Option<bool>,
            name: Option<String>,
            description: Option<String>,
            site_id: Option<String>,
            site_provider: Option<String>,
            branch: Option<String>,
            created_by: Option<String>,
            visibility: Visibility,
            created_at_db: Option<DateTime<Utc>>,
            total_count: i64,
        }

        let rows = sqlx::query_as::<_, TaskWithCount>(
            r#"SELECT DISTINCT
                t.task_id, t.short_id, t.created_at, t.last_message_at, t.agent_type,
                t.repository_id, t.status, t.total_cost_usd, t.site_url, t.template_id,
                t.vibe, t.name, t.description, t.site_id, t.site_provider, t.branch,
                t.created_by, t.visibility, t.created_at_db,
                COUNT(*) OVER() AS total_count
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
        .bind(&filter.user_id)
        .bind(&filter.status)
        .bind(&filter.repository_id)
        .bind(&filter.created_by)
        .bind(&filter.tag)
        .bind(filter.limit)
        .bind(filter.offset)
        .fetch_all(&self.0)
        .await?;

        let total = rows.first().map_or(0, |r| r.total_count);

        let tasks: Vec<Task> = rows
            .into_iter()
            .map(|r| Task {
                task_id: r.task_id,
                short_id: r.short_id,
                created_at: r.created_at,
                last_message_at: r.last_message_at,
                agent_type: r.agent_type,
                repository_id: r.repository_id,
                status: r.status,
                total_cost_usd: r.total_cost_usd,
                site_url: r.site_url,
                template_id: r.template_id,
                vibe: r.vibe,
                name: r.name,
                description: r.description,
                site_id: r.site_id,
                site_provider: r.site_provider,
                branch: r.branch,
                created_by: r.created_by,
                visibility: r.visibility,
                created_at_db: r.created_at_db,
            })
            .collect();

        Ok((tasks, total))
    }

    async fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error> {
        // Use individual UPDATE statements per field for type safety
        // rather than dynamic SQL construction
        if let Some(ref status) = patch.status {
            sqlx::query("UPDATE tasks SET status = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(status)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref name) = patch.name {
            sqlx::query("UPDATE tasks SET name = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(name)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref description) = patch.description {
            sqlx::query("UPDATE tasks SET description = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(description)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref last_message_at) = patch.last_message_at {
            sqlx::query("UPDATE tasks SET last_message_at = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(last_message_at)
                .execute(&self.0)
                .await?;
        }
        if let Some(total_cost_usd) = patch.total_cost_usd {
            sqlx::query("UPDATE tasks SET total_cost_usd = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(total_cost_usd)
                .execute(&self.0)
                .await?;
        }
        if let Some(increment) = patch.total_cost_usd_increment {
            sqlx::query("UPDATE tasks SET total_cost_usd = total_cost_usd + $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(increment)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref site_url) = patch.site_url {
            sqlx::query("UPDATE tasks SET site_url = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(site_url)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref visibility) = patch.visibility {
            sqlx::query("UPDATE tasks SET visibility = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(visibility)
                .execute(&self.0)
                .await?;
        }
        if let Some(ref short_id) = patch.short_id {
            sqlx::query("UPDATE tasks SET short_id = $2 WHERE task_id = $1")
                .bind(task_id)
                .bind(short_id)
                .execute(&self.0)
                .await?;
        }
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
