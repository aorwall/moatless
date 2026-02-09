//! Command DAO — queries for command data access.

use super::models::CommandRow;
use super::DbPool;

/// Insert a new command.
pub async fn insert(pool: &DbPool, cmd: &CommandRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO commands (name, description, prompt, conditions, icon, variant, active, will_not_resume, agent_mode)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (name) DO NOTHING"#,
    )
    .bind(&cmd.name)
    .bind(&cmd.description)
    .bind(&cmd.prompt)
    .bind(&cmd.conditions)
    .bind(&cmd.icon)
    .bind(&cmd.variant)
    .bind(cmd.active)
    .bind(cmd.will_not_resume)
    .bind(&cmd.agent_mode)
    .execute(pool)
    .await?;
    Ok(())
}

/// Insert a batch of commands (ignores conflicts).
pub async fn insert_batch(pool: &DbPool, commands: &[CommandRow]) -> Result<(), sqlx::Error> {
    for cmd in commands {
        insert(pool, cmd).await?;
    }
    Ok(())
}

/// Find a command by name.
pub async fn find_by_name(pool: &DbPool, name: &str) -> Result<Option<CommandRow>, sqlx::Error> {
    sqlx::query_as::<_, CommandRow>(
        r#"SELECT name, description, prompt, conditions, icon, variant, active, will_not_resume, agent_mode
           FROM commands WHERE name = $1"#,
    )
    .bind(name)
    .fetch_optional(pool)
    .await
}

/// List all commands.
pub async fn list_all(pool: &DbPool) -> Result<Vec<CommandRow>, sqlx::Error> {
    sqlx::query_as::<_, CommandRow>(
        r#"SELECT name, description, prompt, conditions, icon, variant, active, will_not_resume, agent_mode
           FROM commands ORDER BY name"#,
    )
    .fetch_all(pool)
    .await
}

/// Update a command by name.
pub async fn update_by_name(
    pool: &DbPool,
    name: &str,
    description: Option<&str>,
    prompt: Option<&str>,
    conditions: Option<&serde_json::Value>,
    icon: Option<&str>,
    variant: Option<&str>,
    active: Option<bool>,
    will_not_resume: Option<bool>,
    agent_mode: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"UPDATE commands SET
            description = COALESCE($2, description),
            prompt = COALESCE($3, prompt),
            conditions = COALESCE($4, conditions),
            icon = COALESCE($5, icon),
            variant = COALESCE($6, variant),
            active = COALESCE($7, active),
            will_not_resume = COALESCE($8, will_not_resume),
            agent_mode = COALESCE($9, agent_mode)
           WHERE name = $1"#,
    )
    .bind(name)
    .bind(description)
    .bind(prompt)
    .bind(conditions)
    .bind(icon)
    .bind(variant)
    .bind(active)
    .bind(will_not_resume)
    .bind(agent_mode)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Delete a command by name.
pub async fn delete_by_name(pool: &DbPool, name: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM commands WHERE name = $1")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Count total commands.
pub async fn count(pool: &DbPool) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM commands")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}
