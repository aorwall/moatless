//! Persistence helpers for `provider_connections`.

use crate::db::DbPool;
use crate::scope::{ScopeContext, ScopedEntity};

use super::super::models::ProviderConnection;

#[async_trait::async_trait]
pub trait ProviderConnectionRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<ProviderConnection>, sqlx::Error>;

    async fn find_by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<ProviderConnection>, sqlx::Error>;

    async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderConnection>, sqlx::Error>;

    async fn upsert(&self, row: &ProviderConnection) -> Result<ProviderConnection, sqlx::Error>;

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error>;
}

pub struct PgProviderConnectionRepository(pub DbPool);

const PROVIDER_CONNECTION_SELECT: &str = r#"
    SELECT id,
           provider,
           connection_kind,
           external_account_id AS scope_value,
           credential_ref,
           webhook_secret,
           transport_config,
           created_by,
           scope,
           team_id,
           owner_user_id,
           created_at,
           updated_at
    FROM provider_connections
"#;

impl PgProviderConnectionRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<ProviderConnection>, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>(&format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_by_scope_value(
        &self,
        provider: &str,
        scope_value: &str,
    ) -> Result<Option<ProviderConnection>, sqlx::Error> {
        self.find_by_external_account(provider, scope_value).await
    }

    pub async fn find_by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<ProviderConnection>, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>(&format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE provider = $1 AND external_account_id = $2 LIMIT 1"
        ))
        .bind(provider)
        .bind(external_account_id)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn list_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<ProviderConnection>, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>(&format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE provider = $1 ORDER BY created_at ASC"
        ))
        .bind(provider)
        .fetch_all(&self.0)
        .await
    }

    pub async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderConnection>, sqlx::Error> {
        if let Some(provider) = provider {
            let access = ctx.access_filter(ScopedEntity::Integration, "", 2);
            let sql = format!(
                "{PROVIDER_CONNECTION_SELECT} \
                 WHERE provider = $1 AND {} \
                 ORDER BY created_at ASC",
                access.sql()
            );
            return sqlx::query_as::<_, ProviderConnection>(&sql)
                .bind(provider)
                .bind(ctx.is_admin)
                .bind(&ctx.team_ids)
                .bind(&ctx.user_id)
                .fetch_all(&self.0)
                .await;
        }
        let access = ctx.access_filter(ScopedEntity::Integration, "", 1);
        let sql = format!(
            "{PROVIDER_CONNECTION_SELECT} \
             WHERE {} \
             ORDER BY created_at ASC",
            access.sql()
        );
        sqlx::query_as::<_, ProviderConnection>(&sql)
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await
    }

    pub async fn upsert(
        &self,
        row: &ProviderConnection,
    ) -> Result<ProviderConnection, sqlx::Error> {
        sqlx::query_as::<_, ProviderConnection>(
            r#"INSERT INTO provider_connections
               (id, provider, connection_kind, external_account_id, credential_ref, webhook_secret,
                transport_config, created_by, scope, team_id, owner_user_id,
                created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               ON CONFLICT (provider, external_account_id) DO UPDATE
               SET connection_kind  = EXCLUDED.connection_kind,
                   credential_ref   = EXCLUDED.credential_ref,
                   webhook_secret   = EXCLUDED.webhook_secret,
                   transport_config = EXCLUDED.transport_config,
                   created_by       = EXCLUDED.created_by,
                   scope            = EXCLUDED.scope,
                   team_id          = EXCLUDED.team_id,
                   owner_user_id    = EXCLUDED.owner_user_id,
                   updated_at       = EXCLUDED.updated_at
               RETURNING id,
                         provider,
                         connection_kind,
                         external_account_id AS scope_value,
                         credential_ref,
                         webhook_secret,
                         transport_config,
                         created_by,
                         scope,
                         team_id,
                         owner_user_id,
                         created_at,
                         updated_at"#,
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
        .bind(&row.owner_user_id)
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

#[async_trait::async_trait]
impl ProviderConnectionRepository for PgProviderConnectionRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<ProviderConnection>, sqlx::Error> {
        PgProviderConnectionRepository::find_by_id(self, id).await
    }

    async fn find_by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<ProviderConnection>, sqlx::Error> {
        PgProviderConnectionRepository::find_by_external_account(
            self,
            provider,
            external_account_id,
        )
        .await
    }

    async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderConnection>, sqlx::Error> {
        PgProviderConnectionRepository::list(self, ctx, provider).await
    }

    async fn upsert(&self, row: &ProviderConnection) -> Result<ProviderConnection, sqlx::Error> {
        PgProviderConnectionRepository::upsert(self, row).await
    }

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        PgProviderConnectionRepository::delete(self, id).await
    }
}
