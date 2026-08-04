//! Persistence helpers for `adapter_connections`.

use sqlx::AssertSqlSafe;

use crate::db::DbPool;
use moatless_backend_core::scope::{ScopeContext, ScopedEntity};

use crate::adapters::models::AdapterConnection;

#[async_trait::async_trait]
pub trait AdapterConnectionDao: Send + Sync {
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<AdapterConnection>, sqlx::Error>;

    /// Look up a connection by Adapter + external account. Used for webhook
    /// routing where we identify the tenant from the inbound event, not from
    /// an authenticated caller — no scope filter applied.
    async fn find_by_external_account(
        &self,
        adapter_kind: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error>;

    /// Look up a connection by Adapter + external account. Used for webhook
    /// routing where we identify the tenant from the inbound event, not from
    /// an authenticated caller — no scope filter applied.
    async fn find_by_scope_value(
        &self,
        adapter_kind: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error>;

    /// List all connections for an Adapter, unscoped. Used by webhook routing
    /// to enumerate all tenants for an Adapter — no authenticated caller.
    async fn list_by_provider(
        &self,
        adapter_kind: &str,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error>;

    async fn list(
        &self,
        ctx: &ScopeContext,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error>;

    async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, sqlx::Error>;

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, sqlx::Error>;
}

pub struct PgAdapterConnectionDao(DbPool);

impl PgAdapterConnectionDao {
    /// Construct over a pool. The only way to build one — ad-hoc construction
    /// at use sites is what the composition-root invariant bans, so the field
    /// is private and callers take an `Arc<dyn AdapterConnectionDao>` port
    /// instead.
    pub fn new(pool: DbPool) -> Self {
        Self(pool)
    }
}

const PROVIDER_CONNECTION_SELECT: &str = r#"
    SELECT id,
           adapter_kind,
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
    FROM adapter_connections
"#;

impl PgAdapterConnectionDao {
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
        adapter_kind: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        self.find_by_external_account(adapter_kind, scope_value)
            .await
    }

    /// Look up a connection by Adapter + external account. Used for webhook
    /// routing (identifies tenant from inbound event) — no scope filter applied.
    pub async fn find_by_external_account(
        &self,
        adapter_kind: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 AND external_account_id = $2 LIMIT 1"
        )))
        .bind(adapter_kind)
        .bind(external_account_id)
        .fetch_optional(&self.0)
        .await
    }

    /// List all connections for an Adapter, unscoped. Used by webhook routing
    /// to enumerate all tenants for an Adapter — no authenticated caller.
    pub async fn list_by_provider(
        &self,
        adapter_kind: &str,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 ORDER BY created_at ASC"
        )))
        .bind(adapter_kind)
        .fetch_all(&self.0)
        .await
    }

    pub async fn list(
        &self,
        ctx: &ScopeContext,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        if let Some(adapter_kind) = adapter_kind {
            let access = ctx.access_filter(ScopedEntity::Resource, "", 2);
            let sql = format!(
                "{PROVIDER_CONNECTION_SELECT} \
                 WHERE adapter_kind = $1 AND {} \
                 ORDER BY created_at ASC",
                access.sql()
            );
            return sqlx::query_as::<_, AdapterConnection>(AssertSqlSafe(sql))
                .bind(adapter_kind)
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
        let ctrl = ctx.control_filter("adapter_connections.", 13);
        let sql = format!(
            r#"INSERT INTO adapter_connections
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
                         adapter_kind,
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
            .bind(&row.adapter_kind)
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
            "DELETE FROM adapter_connections WHERE id = $1 AND {}",
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
impl AdapterConnectionDao for PgAdapterConnectionDao {
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionDao::find_by_id(self, id, ctx).await
    }

    async fn find_by_external_account(
        &self,
        adapter_kind: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionDao::find_by_external_account(self, adapter_kind, external_account_id)
            .await
    }

    async fn find_by_scope_value(
        &self,
        adapter_kind: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionDao::find_by_scope_value(self, adapter_kind, scope_value).await
    }

    async fn list_by_provider(
        &self,
        adapter_kind: &str,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionDao::list_by_provider(self, adapter_kind).await
    }

    async fn list(
        &self,
        ctx: &ScopeContext,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        PgAdapterConnectionDao::list(self, ctx, adapter_kind).await
    }

    async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, sqlx::Error> {
        PgAdapterConnectionDao::upsert(self, row, ctx).await
    }

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, sqlx::Error> {
        PgAdapterConnectionDao::delete(self, id, ctx).await
    }
}
