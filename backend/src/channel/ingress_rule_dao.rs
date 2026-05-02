//! Persistence helpers for `ingress_rules`.

use crate::db::DbPool;

use super::models::IngressRule;

pub struct PgIngressRuleRepository(pub DbPool);

impl PgIngressRuleRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<IngressRule>, sqlx::Error> {
        sqlx::query_as::<_, IngressRule>("SELECT * FROM ingress_rules WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    pub async fn find_by_scope_matcher(
        &self,
        provider: &str,
        scope_matcher: &str,
    ) -> Result<Option<IngressRule>, sqlx::Error> {
        sqlx::query_as::<_, IngressRule>(
            r#"SELECT * FROM ingress_rules
               WHERE provider = $1 AND scope_matcher = $2
               LIMIT 1"#,
        )
        .bind(provider)
        .bind(scope_matcher)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_wildcard(
        &self,
        provider: &str,
    ) -> Result<Option<IngressRule>, sqlx::Error> {
        sqlx::query_as::<_, IngressRule>(
            r#"SELECT * FROM ingress_rules
               WHERE provider = $1 AND scope_matcher = '*'
               LIMIT 1"#,
        )
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_by_connection_id(
        &self,
        connection_id: &str,
    ) -> Result<Vec<IngressRule>, sqlx::Error> {
        sqlx::query_as::<_, IngressRule>(
            r#"SELECT * FROM ingress_rules
               WHERE connection_id = $1
               ORDER BY created_at ASC"#,
        )
        .bind(connection_id)
        .fetch_all(&self.0)
        .await
    }

    pub async fn list_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<IngressRule>, sqlx::Error> {
        sqlx::query_as::<_, IngressRule>(
            r#"SELECT * FROM ingress_rules
               WHERE provider = $1
               ORDER BY created_at ASC"#,
        )
        .bind(provider)
        .fetch_all(&self.0)
        .await
    }

    pub async fn upsert(&self, row: &IngressRule) -> Result<IngressRule, sqlx::Error> {
        sqlx::query_as::<_, IngressRule>(
            r#"INSERT INTO ingress_rules
               (id, provider, connection_id, scope_matcher, event_filter, repository_id,
                prompt_template, task_name, tag_ids, run_as_user_id, agent_type,
                close_task_on_close, notify_on_success, active,
                created_by, scope, team_id, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                       $12, $13, $14, $15, $16, $17, $18, $19)
               ON CONFLICT (provider, scope_matcher) DO UPDATE
               SET connection_id       = EXCLUDED.connection_id,
                   event_filter        = EXCLUDED.event_filter,
                   repository_id       = EXCLUDED.repository_id,
                   prompt_template     = EXCLUDED.prompt_template,
                   task_name           = EXCLUDED.task_name,
                   tag_ids             = EXCLUDED.tag_ids,
                   run_as_user_id      = EXCLUDED.run_as_user_id,
                   agent_type          = EXCLUDED.agent_type,
                   close_task_on_close = EXCLUDED.close_task_on_close,
                   notify_on_success   = EXCLUDED.notify_on_success,
                   active              = EXCLUDED.active,
                   created_by          = EXCLUDED.created_by,
                   scope               = EXCLUDED.scope,
                   team_id             = EXCLUDED.team_id,
                   updated_at          = EXCLUDED.updated_at
               RETURNING *"#,
        )
        .bind(&row.id)
        .bind(&row.provider)
        .bind(&row.connection_id)
        .bind(&row.scope_matcher)
        .bind(&row.event_filter)
        .bind(&row.repository_id)
        .bind(&row.prompt_template)
        .bind(&row.task_name)
        .bind(&row.tag_ids)
        .bind(&row.run_as_user_id)
        .bind(&row.agent_type)
        .bind(row.close_task_on_close)
        .bind(row.notify_on_success)
        .bind(row.active)
        .bind(&row.created_by)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(row.created_at)
        .bind(row.updated_at)
        .fetch_one(&self.0)
        .await
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM ingress_rules WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }
}
