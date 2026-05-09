//! Persistence for `task_triggers`.

use crate::db::DbPool;
use crate::scope::{ScopeContext, ScopedEntity};

use super::super::models::TaskTrigger;

pub struct PgTaskTriggerRepository(pub DbPool);

#[async_trait::async_trait]
pub trait TaskTriggerRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskTrigger>, sqlx::Error>;

    async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
        route_id: Option<&str>,
    ) -> Result<Vec<TaskTrigger>, sqlx::Error>;

    async fn insert(&self, row: NewTaskTrigger) -> Result<TaskTrigger, sqlx::Error>;

    async fn update(&self, row: UpdateTaskTriggerRow) -> Result<TaskTrigger, sqlx::Error>;

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error>;
}

pub struct NewTaskTrigger {
    pub id: String,
    pub provider: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub trigger_config: Option<serde_json::Value>,
    pub route_id: String,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub scope: crate::scope::Scope,
    pub team_id: Option<String>,
    pub created_by: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct UpdateTaskTriggerRow {
    pub id: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub trigger_config: Option<serde_json::Value>,
    pub route_id: String,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub scope: crate::scope::Scope,
    pub team_id: Option<String>,
}

impl PgTaskTriggerRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<TaskTrigger>, sqlx::Error> {
        sqlx::query_as::<_, TaskTrigger>("SELECT * FROM task_triggers WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    pub async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
        route_id: Option<&str>,
    ) -> Result<Vec<TaskTrigger>, sqlx::Error> {
        match (provider, route_id) {
            (Some(provider), Some(route_id)) => {
                let access = ctx.access_filter(ScopedEntity::Integration, "", 3);
                let sql = format!(
                    "SELECT * FROM task_triggers \
                     WHERE provider = $1 AND route_id = $2 AND {} \
                     ORDER BY created_at DESC",
                    access.sql()
                );
                sqlx::query_as::<_, TaskTrigger>(&sql)
                    .bind(provider)
                    .bind(route_id)
                    .bind(ctx.is_admin)
                    .bind(&ctx.team_ids)
                    .bind(&ctx.user_id)
                    .fetch_all(&self.0)
                    .await
            }
            (Some(provider), None) => {
                let access = ctx.access_filter(ScopedEntity::Integration, "", 2);
                let sql = format!(
                    "SELECT * FROM task_triggers \
                     WHERE provider = $1 AND {} \
                     ORDER BY created_at DESC",
                    access.sql()
                );
                sqlx::query_as::<_, TaskTrigger>(&sql)
                    .bind(provider)
                    .bind(ctx.is_admin)
                    .bind(&ctx.team_ids)
                    .bind(&ctx.user_id)
                    .fetch_all(&self.0)
                    .await
            }
            (None, Some(route_id)) => {
                let access = ctx.access_filter(ScopedEntity::Integration, "", 2);
                let sql = format!(
                    "SELECT * FROM task_triggers \
                     WHERE route_id = $1 AND {} \
                     ORDER BY created_at DESC",
                    access.sql()
                );
                sqlx::query_as::<_, TaskTrigger>(&sql)
                    .bind(route_id)
                    .bind(ctx.is_admin)
                    .bind(&ctx.team_ids)
                    .bind(&ctx.user_id)
                    .fetch_all(&self.0)
                    .await
            }
            (None, None) => {
                let access = ctx.access_filter(ScopedEntity::Integration, "", 1);
                let sql = format!(
                    "SELECT * FROM task_triggers \
                     WHERE {} \
                     ORDER BY created_at DESC",
                    access.sql()
                );
                sqlx::query_as::<_, TaskTrigger>(&sql)
                    .bind(ctx.is_admin)
                    .bind(&ctx.team_ids)
                    .bind(&ctx.user_id)
                    .fetch_all(&self.0)
                    .await
            }
        }
    }

    pub async fn insert(&self, row: NewTaskTrigger) -> Result<TaskTrigger, sqlx::Error> {
        sqlx::query_as::<_, TaskTrigger>(
            r#"INSERT INTO task_triggers
               (id, provider, connection_id, source_matcher, source_name, event_filter,
                trigger_config, route_id, allow_direct_send,
                close_task_on_close, notify_on_success, active, scope, team_id,
                created_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                       $10, $11, $12, $13, $14, $15, $16, $16)
               RETURNING *"#,
        )
        .bind(row.id)
        .bind(row.provider)
        .bind(row.connection_id)
        .bind(row.source_matcher)
        .bind(row.source_name)
        .bind(row.event_filter)
        .bind(row.trigger_config)
        .bind(row.route_id)
        .bind(row.allow_direct_send)
        .bind(row.close_task_on_close)
        .bind(row.notify_on_success)
        .bind(row.active)
        .bind(row.scope)
        .bind(row.team_id)
        .bind(row.created_by)
        .bind(row.created_at)
        .fetch_one(&self.0)
        .await
    }

    pub async fn update(&self, row: UpdateTaskTriggerRow) -> Result<TaskTrigger, sqlx::Error> {
        sqlx::query_as::<_, TaskTrigger>(
            r#"UPDATE task_triggers
               SET connection_id = $2,
                   source_matcher = $3,
                   source_name = $4,
                   event_filter = $5,
                   trigger_config = $6,
                   route_id = $7,
                   allow_direct_send = $8,
                   close_task_on_close = $9,
                   notify_on_success = $10,
                   active = $11,
                   scope = $12,
                   team_id = $13,
                   updated_at = now()
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(row.id)
        .bind(row.connection_id)
        .bind(row.source_matcher)
        .bind(row.source_name)
        .bind(row.event_filter)
        .bind(row.trigger_config)
        .bind(row.route_id)
        .bind(row.allow_direct_send)
        .bind(row.close_task_on_close)
        .bind(row.notify_on_success)
        .bind(row.active)
        .bind(row.scope)
        .bind(row.team_id)
        .fetch_one(&self.0)
        .await
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM task_triggers WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }
}

#[async_trait::async_trait]
impl TaskTriggerRepository for PgTaskTriggerRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskTrigger>, sqlx::Error> {
        PgTaskTriggerRepository::find_by_id(self, id).await
    }

    async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
        route_id: Option<&str>,
    ) -> Result<Vec<TaskTrigger>, sqlx::Error> {
        PgTaskTriggerRepository::list(self, ctx, provider, route_id).await
    }

    async fn insert(&self, row: NewTaskTrigger) -> Result<TaskTrigger, sqlx::Error> {
        PgTaskTriggerRepository::insert(self, row).await
    }

    async fn update(&self, row: UpdateTaskTriggerRow) -> Result<TaskTrigger, sqlx::Error> {
        PgTaskTriggerRepository::update(self, row).await
    }

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        PgTaskTriggerRepository::delete(self, id).await
    }
}
