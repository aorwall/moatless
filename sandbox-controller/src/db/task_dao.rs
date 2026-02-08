//! Task DAO — queries for task data access.

use super::models::TaskRow;
use super::DbPool;

/// Find a task by its task_id (no access control — for internal/system use).
pub async fn find_by_id_internal(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>(
        r#"SELECT
            task_id, short_id, created_at, last_message_at, agent_type,
            repository_id, status, total_cost_usd, site_url, template_id,
            vibe, name, description, site_id, site_provider, branch,
            created_by, visibility, created_at_db
        FROM tasks WHERE task_id = $1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
}

/// Find a task by its task_id, scoped to user visibility.
pub async fn find_by_id(
    pool: &DbPool,
    task_id: &str,
    user_id: &str,
) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>(
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
    .fetch_optional(pool)
    .await
}

/// Insert a new task.
pub async fn insert_task(pool: &DbPool, row: &TaskRow) -> Result<(), sqlx::Error> {
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
    .bind(&row.status)
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
    .bind(&row.visibility)
    .execute(pool)
    .await?;
    Ok(())
}

/// List tasks with pagination and filters, scoped to user visibility.
pub async fn list_tasks(
    pool: &DbPool,
    user_id: &str,
    limit: i64,
    offset: i64,
    status: Option<&str>,
    repository_id: Option<&str>,
    created_by: Option<&str>,
    tag: Option<&str>,
) -> Result<(Vec<TaskRow>, i64), sqlx::Error> {
    // Build dynamic query — using a single parameterised query with optional filters.
    // We use COALESCE-style tricks to keep it as a single prepared statement.
    let rows = sqlx::query_as::<_, TaskRow>(
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
    .fetch_all(pool)
    .await?;

    // Count query
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
    .fetch_one(pool)
    .await?;

    Ok((rows, total.0))
}

/// Update task fields dynamically. Only non-None fields are updated.
#[allow(unused_assignments)]
pub async fn update_task(pool: &DbPool, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error> {
    // Build SET clauses dynamically
    let mut sets = Vec::new();
    let mut idx = 2u32; // $1 = task_id

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

    query.execute(pool).await?;
    Ok(())
}

/// Delete a task by its task_id.
pub async fn delete_task(pool: &DbPool, task_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM tasks WHERE task_id = $1")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Check if a short_id already exists.
pub async fn exists_by_short_id(pool: &DbPool, short_id: &str) -> Result<bool, sqlx::Error> {
    let row: (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM tasks WHERE short_id = $1)")
            .bind(short_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}

/// Patch struct for partial task updates.
#[derive(Default)]
pub struct TaskPatch {
    pub status: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub last_message_at: Option<String>,
    pub total_cost_usd: Option<f64>,
    pub site_url: Option<String>,
    pub visibility: Option<String>,
    pub short_id: Option<String>,
}
