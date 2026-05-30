//! Persistence helpers for canonical trigger+route views.

use crate::db::DbPool;

use crate::adapters::models::TaskTriggerRoute;

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
    SELECT t.id,
           aes.adapter_kind AS provider,
           aes.connection_id,
           aes.source_matcher AS source_matcher,
           aes.source_name,
           r.routing_mode,
           aes.event_filter,
           r.repository_id,
           r.prompt AS prompt_template,
           r.task_name,
           r.tag_ids,
           r.run_as_user_id,
           r.agent_type,
           r.inactive_ttl_days,
           aes.allow_direct_send,
           aes.adapter_config AS provider_config,
           r.ongoing_task_id,
           aes.close_task_on_close,
           aes.notify_on_success,
           t.active,
           t.created_by,
           t.scope,
           t.team_id,
           t.owner_user_id,
           t.created_at,
           t.updated_at
    FROM triggers t
    JOIN trigger_rules r ON r.trigger_id = t.id
    JOIN adapter_event_trigger_sources aes ON aes.trigger_id = t.id
"#;

impl PgTaskTriggerRouteRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<TaskTriggerRoute>, sqlx::Error> {
        sqlx::query_as::<_, TaskTriggerRoute>(&format!(
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE t.id = $1"
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
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = $2 AND t.active = true ORDER BY t.created_at ASC LIMIT 1"
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
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = $2 AND t.active = true ORDER BY t.created_at ASC"
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
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = '*' AND t.active = true LIMIT 1"
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
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE aes.connection_id = $1 ORDER BY t.created_at ASC"
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
            "{TASK_TRIGGER_ROUTE_SELECT} WHERE aes.adapter_kind = $1 AND t.active = true ORDER BY t.created_at ASC"
        ))
        .bind(provider)
        .fetch_all(&self.0)
        .await
    }

    pub async fn upsert(&self, row: &TaskTriggerRoute) -> Result<TaskTriggerRoute, sqlx::Error> {
        let mut tx = self.0.begin().await?;

        sqlx::query(
            r#"INSERT INTO triggers
               (id, name, kind, active, scope, team_id, owner_user_id,
                created_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (id) DO UPDATE
               SET name            = EXCLUDED.name,
                   kind            = EXCLUDED.kind,
                   active          = EXCLUDED.active,
                   scope           = EXCLUDED.scope,
                   team_id         = EXCLUDED.team_id,
                   owner_user_id   = EXCLUDED.owner_user_id,
                   updated_at      = EXCLUDED.updated_at"#,
        )
        .bind(&row.id)
        .bind(&row.source_name)
        .bind("adapter_event")
        .bind(row.active)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(&row.owner_user_id)
        .bind(&row.created_by)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO trigger_rules
               (trigger_id, repository_id, routing_mode, ongoing_task_id, prompt,
                task_name, tag_ids, run_as_user_id, agent_type, inactive_ttl_days)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (trigger_id) DO UPDATE
               SET repository_id     = EXCLUDED.repository_id,
                   routing_mode      = EXCLUDED.routing_mode,
                   ongoing_task_id   = EXCLUDED.ongoing_task_id,
                   prompt            = EXCLUDED.prompt,
                   task_name         = EXCLUDED.task_name,
                   tag_ids           = EXCLUDED.tag_ids,
                   run_as_user_id    = EXCLUDED.run_as_user_id,
                   agent_type        = EXCLUDED.agent_type,
                   inactive_ttl_days = EXCLUDED.inactive_ttl_days"#,
        )
        .bind(&row.id)
        .bind(&row.repository_id)
        .bind(row.routing_mode)
        .bind(&row.ongoing_task_id)
        .bind(&row.prompt_template)
        .bind(&row.task_name)
        .bind(&row.tag_ids)
        .bind(&row.run_as_user_id)
        .bind(&row.agent_type)
        .bind(row.inactive_ttl_days)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO adapter_event_trigger_sources
               (trigger_id, adapter_kind, connection_id, source_matcher, source_name,
                event_filter, adapter_config,
                allow_direct_send, close_task_on_close, notify_on_success)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (trigger_id) DO UPDATE
               SET adapter_kind         = EXCLUDED.adapter_kind,
                   connection_id        = EXCLUDED.connection_id,
                   source_matcher       = EXCLUDED.source_matcher,
                   source_name          = EXCLUDED.source_name,
                   event_filter         = EXCLUDED.event_filter,
                   adapter_config       = EXCLUDED.adapter_config,
                   allow_direct_send    = EXCLUDED.allow_direct_send,
                   close_task_on_close  = EXCLUDED.close_task_on_close,
                   notify_on_success    = EXCLUDED.notify_on_success"#,
        )
        .bind(&row.id)
        .bind(&row.provider)
        .bind(&row.connection_id)
        .bind(&row.source_matcher)
        .bind(&row.source_name)
        .bind(&row.event_filter)
        .bind(&row.provider_config)
        .bind(row.allow_direct_send)
        .bind(row.close_task_on_close)
        .bind(row.notify_on_success)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        self.find_by_id(&row.id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM triggers WHERE id = $1")
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
