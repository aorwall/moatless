//! Persistence helpers for canonical trigger+route views.

use crate::db::DbPool;

use super::super::models::TaskTriggerRoute;

#[async_trait::async_trait]
pub trait TaskTriggerRouteRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskTriggerRoute>, sqlx::Error>;

    async fn find_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Option<TaskTriggerRoute>, sqlx::Error>;

    async fn list_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error>;

    async fn find_wildcard(&self, provider: &str) -> Result<Option<TaskTriggerRoute>, sqlx::Error>;

    async fn find_by_connection_id(
        &self,
        connection_id: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error>;

    async fn list_by_provider(&self, provider: &str) -> Result<Vec<TaskTriggerRoute>, sqlx::Error>;

    async fn upsert(&self, row: &TaskTriggerRoute) -> Result<TaskTriggerRoute, sqlx::Error>;

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error>;
}

pub struct PgTaskTriggerRouteRepository(pub DbPool);

const TASK_TRIGGER_ROUTE_SELECT: &str = r#"
    SELECT tt.id,
           tt.provider,
           tt.connection_id,
           tt.source_matcher AS source_matcher,
           tt.source_name,
           tr.routing_mode,
           tt.event_filter,
           tr.repository_id,
           tr.prompt_template,
           tr.task_name,
           tr.tag_ids,
           tr.run_as_user_id,
           tr.agent_type,
           tt.allow_direct_send,
           tt.trigger_config AS provider_config,
           tr.ongoing_task_id,
           tt.close_task_on_close,
           tt.notify_on_success,
           tt.active,
           tt.created_by,
           tt.scope,
           tt.team_id,
           tt.created_at,
           tt.updated_at
    FROM task_triggers tt
    JOIN task_routes tr ON tr.id = tt.route_id
"#;

impl PgTaskTriggerRouteRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskTriggerRoute>(&format!(
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE tt.id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskTriggerRoute>(&format!(
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE tt.provider = $1 AND tt.source_matcher = $2 ORDER BY tt.created_at ASC LIMIT 1"
        ))
        .bind(provider)
        .bind(source_matcher)
        .fetch_optional(&self.0)
        .await
    }

    /// Return ALL rules for a given (provider, source_matcher) — used by
    /// Linear, where multiple listeners can share a team UUID.
    pub async fn list_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskTriggerRoute>(&format!(
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE tt.provider = $1 AND tt.source_matcher = $2 ORDER BY tt.created_at ASC"
        ))
        .bind(provider)
        .bind(source_matcher)
        .fetch_all(&self.0)
        .await
    }

    pub async fn find_wildcard(
        &self,
        provider: &str,
    ) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskTriggerRoute>(&format!(
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE tt.provider = $1 AND tt.source_matcher = '*' LIMIT 1"
        ))
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_by_connection_id(
        &self,
        connection_id: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskTriggerRoute>(&format!(
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE tt.connection_id = $1 ORDER BY tt.created_at ASC"
        ))
        .bind(connection_id)
        .fetch_all(&self.0)
        .await
    }

    pub async fn list_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskTriggerRoute>(&format!(
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE tt.provider = $1 ORDER BY tt.created_at ASC"
        ))
        .bind(provider)
        .fetch_all(&self.0)
        .await
    }

    pub async fn upsert(&self, row: &TaskTriggerRoute) -> Result<TaskTriggerRoute, sqlx::Error> {
        let route_id = format!("route_{}", row.id);

        sqlx::query(
            r#"INSERT INTO task_routes
               (id, repository_id, routing_mode, ongoing_task_id, prompt_template,
                task_name, tag_ids, run_as_user_id, agent_type, scope, team_id,
                created_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
               ON CONFLICT (id) DO UPDATE
               SET repository_id   = EXCLUDED.repository_id,
                   routing_mode    = EXCLUDED.routing_mode,
                   ongoing_task_id = EXCLUDED.ongoing_task_id,
                   prompt_template = EXCLUDED.prompt_template,
                   task_name       = EXCLUDED.task_name,
                   tag_ids         = EXCLUDED.tag_ids,
                   run_as_user_id  = EXCLUDED.run_as_user_id,
                   agent_type      = EXCLUDED.agent_type,
                   scope           = EXCLUDED.scope,
                   team_id         = EXCLUDED.team_id,
                   updated_at      = EXCLUDED.updated_at"#,
        )
        .bind(&route_id)
        .bind(&row.repository_id)
        .bind(row.routing_mode)
        .bind(&row.ongoing_task_id)
        .bind(&row.prompt_template)
        .bind(&row.task_name)
        .bind(&row.tag_ids)
        .bind(&row.run_as_user_id)
        .bind(&row.agent_type)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(&row.created_by)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.0)
        .await?;

        sqlx::query(
            r#"INSERT INTO task_triggers
               (id, provider, connection_id, source_matcher, source_name,
                event_filter, trigger_config, route_id,
                allow_direct_send, close_task_on_close, notify_on_success,
                active, created_by, scope, team_id, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                       $10, $11, $12, $13, $14, $15, $16, $17)
               ON CONFLICT (id) DO UPDATE
               SET provider             = EXCLUDED.provider,
                   connection_id        = EXCLUDED.connection_id,
                   source_matcher       = EXCLUDED.source_matcher,
                   source_name          = EXCLUDED.source_name,
                   event_filter         = EXCLUDED.event_filter,
                   trigger_config       = EXCLUDED.trigger_config,
                   route_id             = EXCLUDED.route_id,
                   allow_direct_send    = EXCLUDED.allow_direct_send,
                   close_task_on_close  = EXCLUDED.close_task_on_close,
                   notify_on_success    = EXCLUDED.notify_on_success,
                   active               = EXCLUDED.active,
                   created_by           = EXCLUDED.created_by,
                   scope                = EXCLUDED.scope,
                   team_id              = EXCLUDED.team_id,
                   updated_at           = EXCLUDED.updated_at"#,
        )
        .bind(&row.id)
        .bind(&row.provider)
        .bind(&row.connection_id)
        .bind(&row.source_matcher)
        .bind(&row.source_name)
        .bind(&row.event_filter)
        .bind(&row.provider_config)
        .bind(&route_id)
        .bind(row.allow_direct_send)
        .bind(row.close_task_on_close)
        .bind(row.notify_on_success)
        .bind(row.active)
        .bind(&row.created_by)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.0)
        .await?;

        self.find_by_id(&row.id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
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
impl TaskTriggerRouteRepository for PgTaskTriggerRouteRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        PgTaskTriggerRouteRepository::find_by_id(self, id).await
    }

    async fn find_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        PgTaskTriggerRouteRepository::find_by_source_matcher(self, provider, source_matcher).await
    }

    async fn list_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        PgTaskTriggerRouteRepository::list_by_source_matcher(self, provider, source_matcher).await
    }

    async fn find_wildcard(&self, provider: &str) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        PgTaskTriggerRouteRepository::find_wildcard(self, provider).await
    }

    async fn find_by_connection_id(
        &self,
        connection_id: &str,
    ) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        PgTaskTriggerRouteRepository::find_by_connection_id(self, connection_id).await
    }

    async fn list_by_provider(&self, provider: &str) -> Result<Vec<TaskTriggerRoute>, sqlx::Error> {
        PgTaskTriggerRouteRepository::list_by_provider(self, provider).await
    }

    async fn upsert(&self, row: &TaskTriggerRoute) -> Result<TaskTriggerRoute, sqlx::Error> {
        PgTaskTriggerRouteRepository::upsert(self, row).await
    }

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        PgTaskTriggerRouteRepository::delete(self, id).await
    }
}
