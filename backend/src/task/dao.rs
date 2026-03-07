//! Task DAO — queries for task data access.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::{Task, TaskStatus, Visibility};
use crate::db::DbPool;
use crate::scope::{Scope, ScopeContext};
use crate::shared_models::AgentType;

/// Filter struct for task listing queries.
#[derive(Default)]
pub struct TaskFilter {
    pub user_id: String,
    pub team_ids: Vec<String>,
    pub is_admin: bool,
    pub limit: i64,
    pub offset: i64,
    pub status: Option<TaskStatus>,
    pub repository_id: Option<String>,
    pub created_by: Option<String>,
    pub tag: Option<String>,
    pub parent_task_id: Option<String>,
}

/// Repository trait for task persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Find by task_id (no access control — internal/system use).
    async fn find_by_id_internal(&self, task_id: &str) -> Result<Option<Task>, sqlx::Error>;
    /// Find by task_id, scoped to access control.
    async fn find_by_id(&self, task_id: &str, ctx: &ScopeContext) -> Result<Option<Task>, sqlx::Error>;
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
                created_by, visibility, scope, team_id, created_at_db,
                parent_task_id
            FROM tasks WHERE task_id = $1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_id(&self, task_id: &str, ctx: &ScopeContext) -> Result<Option<Task>, sqlx::Error> {
        sqlx::query_as::<_, Task>(
            r#"SELECT
                task_id, short_id, created_at, last_message_at, agent_type,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility, scope, team_id, created_at_db,
                parent_task_id
            FROM tasks
            WHERE task_id = $1
              AND ($2 OR scope = 'global'
                   OR (scope = 'team' AND team_id = ANY($3))
                   OR (scope = 'user' AND created_by = $4))"#,
        )
        .bind(task_id)
        .bind(ctx.is_admin)
        .bind(&ctx.team_ids)
        .bind(&ctx.user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert_task(&self, row: &Task) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO tasks (
                task_id, short_id, created_at, last_message_at, agent_type,
                repository_id, status, total_cost_usd, site_url, template_id,
                vibe, name, description, site_id, site_provider, branch,
                created_by, visibility, scope, team_id, parent_task_id
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21
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
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(&row.parent_task_id)
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
            agent_type: AgentType,
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
            scope: Scope,
            team_id: Option<String>,
            created_at_db: Option<DateTime<Utc>>,
            parent_task_id: Option<String>,
            total_count: i64,
        }

        let rows = sqlx::query_as::<_, TaskWithCount>(
            r#"SELECT DISTINCT
                t.task_id, t.short_id, t.created_at, t.last_message_at, t.agent_type,
                t.repository_id, t.status, t.total_cost_usd, t.site_url, t.template_id,
                t.vibe, t.name, t.description, t.site_id, t.site_provider, t.branch,
                t.created_by, t.visibility, t.scope, t.team_id, t.created_at_db,
                t.parent_task_id,
                COUNT(*) OVER() AS total_count
            FROM tasks t
            LEFT JOIN task_tags tt ON t.task_id = tt.task_id
            LEFT JOIN tags tg ON tt.tag_id = tg.tag_id
            WHERE ($1 OR t.scope = 'global'
                   OR (t.scope = 'team' AND t.team_id = ANY($2))
                   OR (t.scope = 'user' AND t.created_by = $3))
              AND ($4::text IS NULL OR t.status = $4)
              AND ($5::text IS NULL OR t.repository_id = $5)
              AND ($6::text IS NULL OR t.created_by = $6)
              AND ($7::text IS NULL OR tg.name = $7)
              AND ($8::text IS NULL OR t.parent_task_id = $8)
            ORDER BY t.last_message_at DESC
            LIMIT $9 OFFSET $10"#,
        )
        .bind(filter.is_admin)
        .bind(&filter.team_ids)
        .bind(&filter.user_id)
        .bind(filter.status)
        .bind(&filter.repository_id)
        .bind(&filter.created_by)
        .bind(&filter.tag)
        .bind(&filter.parent_task_id)
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
                scope: r.scope,
                team_id: r.team_id,
                created_at_db: r.created_at_db,
                parent_task_id: r.parent_task_id,
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

// =========================================================================
// Free-function wrappers for integration tests
// =========================================================================
//
// Integration tests in `tests/` call these as `task_dao::insert_task(&pool, &row)`.
// Each wrapper constructs a `PgTaskRepository` and delegates to the trait method.

/// Insert a task (convenience wrapper).
pub async fn insert_task(pool: &DbPool, row: &Task) -> Result<(), sqlx::Error> {
    PgTaskRepository(pool.clone()).insert_task(row).await
}

/// Find by task_id scoped to access control (convenience wrapper).
pub async fn find_by_id(pool: &DbPool, task_id: &str, ctx: &ScopeContext) -> Result<Option<Task>, sqlx::Error> {
    PgTaskRepository(pool.clone()).find_by_id(task_id, ctx).await
}

/// Find by task_id without access control (convenience wrapper).
pub async fn find_by_id_internal(pool: &DbPool, task_id: &str) -> Result<Option<Task>, sqlx::Error> {
    PgTaskRepository(pool.clone()).find_by_id_internal(task_id).await
}

/// List tasks with individual filter arguments (convenience wrapper).
#[allow(clippy::too_many_arguments)]
pub async fn list_tasks(
    pool: &DbPool,
    ctx: &ScopeContext,
    limit: i64,
    offset: i64,
    status: Option<TaskStatus>,
    repository_id: Option<&str>,
    created_by: Option<&str>,
    tag: Option<&str>,
    parent_task_id: Option<&str>,
) -> Result<(Vec<Task>, i64), sqlx::Error> {
    let filter = TaskFilter {
        user_id: ctx.user_id.clone(),
        team_ids: ctx.team_ids.clone(),
        is_admin: ctx.is_admin,
        limit,
        offset,
        status,
        repository_id: repository_id.map(String::from),
        created_by: created_by.map(String::from),
        tag: tag.map(String::from),
        parent_task_id: parent_task_id.map(String::from),
    };
    PgTaskRepository(pool.clone()).list_tasks(&filter).await
}

/// Update task fields (convenience wrapper).
pub async fn update_task(pool: &DbPool, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error> {
    PgTaskRepository(pool.clone()).update_task(task_id, patch).await
}

/// Delete a task (convenience wrapper).
pub async fn delete_task(pool: &DbPool, task_id: &str) -> Result<(), sqlx::Error> {
    PgTaskRepository(pool.clone()).delete_task(task_id).await
}

/// Check if a task with the given short_id exists (convenience wrapper).
pub async fn exists_by_short_id(pool: &DbPool, short_id: &str) -> Result<bool, sqlx::Error> {
    PgTaskRepository(pool.clone()).exists_by_short_id(short_id).await
}
