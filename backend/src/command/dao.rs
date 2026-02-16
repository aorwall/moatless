//! Command DAO — queries for command data access.

use async_trait::async_trait;

use super::models::Command;
use crate::db::DbPool;

/// Repository trait for command persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CommandRepository: Send + Sync {
    async fn insert(&self, cmd: &Command) -> Result<(), sqlx::Error>;
    async fn insert_batch(&self, commands: &[Command]) -> Result<(), sqlx::Error>;
    async fn find_by_name(&self, name: &str) -> Result<Option<Command>, sqlx::Error>;
    async fn list_all(&self) -> Result<Vec<Command>, sqlx::Error>;
    #[allow(clippy::too_many_arguments)]
    async fn update_by_name(
        &self,
        name: &str,
        description: Option<&str>,
        prompt: Option<&str>,
        conditions: Option<&serde_json::Value>,
        icon: Option<&str>,
        variant: Option<&str>,
        active: Option<bool>,
        will_not_resume: Option<bool>,
        agent_mode: Option<&str>,
    ) -> Result<bool, sqlx::Error>;
    async fn delete_by_name(&self, name: &str) -> Result<bool, sqlx::Error>;
    async fn count(&self) -> Result<i64, sqlx::Error>;
}

/// PostgreSQL implementation of [`CommandRepository`].
pub struct PgCommandRepository(pub DbPool);

#[async_trait]
impl CommandRepository for PgCommandRepository {
    async fn insert(&self, cmd: &Command) -> Result<(), sqlx::Error> {
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
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn insert_batch(&self, commands: &[Command]) -> Result<(), sqlx::Error> {
        for cmd in commands {
            self.insert(cmd).await?;
        }
        Ok(())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Command>, sqlx::Error> {
        sqlx::query_as::<_, Command>(
            r#"SELECT name, description, prompt, conditions, icon, variant, active, will_not_resume, agent_mode
               FROM commands WHERE name = $1"#,
        )
        .bind(name)
        .fetch_optional(&self.0)
        .await
    }

    async fn list_all(&self) -> Result<Vec<Command>, sqlx::Error> {
        sqlx::query_as::<_, Command>(
            r#"SELECT name, description, prompt, conditions, icon, variant, active, will_not_resume, agent_mode
               FROM commands ORDER BY name"#,
        )
        .fetch_all(&self.0)
        .await
    }

    async fn update_by_name(
        &self,
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
        .execute(&self.0)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_by_name(&self, name: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM commands WHERE name = $1")
            .bind(name)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn count(&self) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM commands")
            .fetch_one(&self.0)
            .await?;
        Ok(row.0)
    }
}
