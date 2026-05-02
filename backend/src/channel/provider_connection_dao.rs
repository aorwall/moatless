//! Persistence helpers for `provider_connections`.

use crate::db::DbPool;

use super::models::ProviderConnection;

pub struct PgProviderConnectionRepository(pub DbPool);

impl PgProviderConnectionRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<ProviderConnection>, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>("SELECT * FROM provider_connections WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    pub async fn find_by_scope_value(
        &self,
        provider: &str,
        scope_value: &str,
    ) -> Result<Option<ProviderConnection>, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>(
            r#"SELECT * FROM provider_connections
               WHERE provider = $1 AND scope_value = $2
               LIMIT 1"#,
        )
        .bind(provider)
        .bind(scope_value)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn list_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<ProviderConnection>, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>(
            r#"SELECT * FROM provider_connections
               WHERE provider = $1
               ORDER BY created_at ASC"#,
        )
        .bind(provider)
        .fetch_all(&self.0)
        .await
    }

    pub async fn upsert(
        &self,
        row: &ProviderConnection,
    ) -> Result<ProviderConnection, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>(
            r#"INSERT INTO provider_connections
               (id, provider, connection_kind, scope_value, credential_ref, webhook_secret,
                transport_config, created_by, scope, team_id,
                created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               ON CONFLICT (provider, scope_value) DO UPDATE
               SET connection_kind  = EXCLUDED.connection_kind,
                   credential_ref   = EXCLUDED.credential_ref,
                   webhook_secret   = EXCLUDED.webhook_secret,
                   transport_config = EXCLUDED.transport_config,
                   created_by       = EXCLUDED.created_by,
                   scope            = EXCLUDED.scope,
                   team_id          = EXCLUDED.team_id,
                   updated_at       = EXCLUDED.updated_at
               RETURNING *"#,
        )
        .bind(&row.id)
        .bind(&row.provider)
        .bind(&row.connection_kind)
        .bind(&row.scope_value)
        .bind(&row.credential_ref)
        .bind(&row.webhook_secret)
        .bind(&row.transport_config)
        .bind(&row.created_by)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(row.created_at)
        .bind(row.updated_at)
        .fetch_one(&self.0)
        .await
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM provider_connections WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }
}
