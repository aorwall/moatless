//! Persistence helpers for `integration_connections`.

use sqlx::AssertSqlSafe;

use crate::db::DbPool;
use moatless_backend_core::scope::{ScopeContext, ScopedEntity};

use crate::adapters::models::AdapterConnection;

#[async_trait::async_trait]
pub trait AdapterConnectionRepository: Send + Sync {
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<AdapterConnection>, sqlx::Error>;

    /// Look up a connection by provider + external account. Used for webhook
    /// routing where we identify the tenant from the inbound event, not from
    /// an authenticated caller — no scope filter applied.
    async fn find_by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error>;

    async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error>;

    async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, sqlx::Error>;

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, sqlx::Error>;
}

pub struct PgAdapterConnectionRepository(pub DbPool);

const PROVIDER_CONNECTION_SELECT: &str = r#"
    SELECT id,
           adapter_kind AS provider,
           connection_kind,
           external_account_id AS scope_value,
           credential_ref,
           webhook_secret,
           transport_config,
           created_by,
           scope,
           owner_user_id,
           created_at,
           updated_at
    FROM integration_connections
"#;

impl PgAdapterConnectionRepository {
    pub async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE id = $1 AND {}",
            access.sql()
        );
        sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    pub async fn find_by_scope_value(
        &self,
        provider: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        self.find_by_external_account(provider, scope_value).await
    }

    /// Look up a connection by provider + external account. Used for webhook
    /// routing (identifies tenant from inbound event) — no scope filter applied.
    pub async fn find_by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 AND external_account_id = $2 LIMIT 1"
        )))
        .bind(provider)
        .bind(external_account_id)
        .fetch_optional(&self.0)
        .await
    }

    /// List all connections for a provider, unscoped. Used by webhook routing
    /// to enumerate all tenants for a provider — no authenticated caller.
    pub async fn list_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 ORDER BY created_at ASC"
        )))
        .bind(provider)
        .fetch_all(&self.0)
        .await
    }

    pub async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        if let Some(provider) = provider {
            let access = ctx.access_filter(ScopedEntity::Resource, "", 2);
            let sql = format!(
                "{PROVIDER_CONNECTION_SELECT} \
                 WHERE adapter_kind = $1 AND {} \
                 ORDER BY created_at ASC",
                access.sql()
            );
            return sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(sql))
                .bind(provider)
                .bind(ctx.is_admin)
                .bind(&ctx.user_id)
                .fetch_all(&self.0)
                .await;
        }
        let access = ctx.access_filter(ScopedEntity::Resource, "", 1);
        let sql = format!(
            "{PROVIDER_CONNECTION_SELECT} \
             WHERE {} \
             ORDER BY created_at ASC",
            access.sql()
        );
        sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await
    }

    /// Upsert an adapter connection. On conflict (adapter_kind, external_account_id),
    /// the DO UPDATE arm includes a `control_filter` guard so cross-user updates
    /// affect 0 rows — the caller maps rows_affected == 0 to NotFound.
    pub async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, sqlx::Error> {
        // In the DO UPDATE arm both the target row and EXCLUDED are in scope,
        // so the filter's columns must be table-qualified.
        let ctrl = ctx.control_filter("integration_connections.", 13);
        let sql = format!(
            r#"INSERT INTO integration_connections
               (id, adapter_kind, connection_kind, external_account_id, credential_ref, webhook_secret,
                transport_config, created_by, scope, owner_user_id,
                created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               ON CONFLICT (adapter_kind, external_account_id) DO UPDATE
               SET connection_kind  = EXCLUDED.connection_kind,
                   credential_ref   = EXCLUDED.credential_ref,
                   webhook_secret   = EXCLUDED.webhook_secret,
                   transport_config = EXCLUDED.transport_config,
                   created_by       = EXCLUDED.created_by,
                   scope            = EXCLUDED.scope,
                   owner_user_id    = EXCLUDED.owner_user_id,
                   updated_at       = EXCLUDED.updated_at
               WHERE {}
               RETURNING id,
                         adapter_kind AS provider,
                         connection_kind,
                         external_account_id AS scope_value,
                         credential_ref,
                         webhook_secret,
                         transport_config,
                         created_by,
                         scope,
                         owner_user_id,
                         created_at,
                         updated_at"#,
            ctrl.sql()
        );
        sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(sql))
            .bind(&row.id)
            .bind(&row.provider)
            .bind(&row.connection_kind)
            .bind(&row.scope_value)
            .bind(&row.credential_ref)
            .bind(&row.webhook_secret)
            .bind(&row.transport_config)
            .bind(&row.created_by)
            .bind(row.scope)
            .bind(&row.owner_user_id)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, sqlx::Error> {
        let ctrl = ctx.control_filter("", 2);
        let sql = format!(
            "DELETE FROM integration_connections WHERE id = $1 AND {}",
            ctrl.sql()
        );
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }
}

#[async_trait::async_trait]
impl AdapterConnectionRepository for PgAdapterConnectionRepository {
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionRepository::find_by_id(self, id, ctx).await
    }

    async fn find_by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionRepository::find_by_external_account(self, provider, external_account_id)
            .await
    }

    async fn list(
        &self,
        ctx: &ScopeContext,
        provider: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionRepository::list(self, ctx, provider).await
    }

    async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, sqlx::Error> {
        PgAdapterConnectionRepository::upsert(self, row, ctx).await
    }

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, sqlx::Error> {
        PgAdapterConnectionRepository::delete(self, id, ctx).await
    }
}
