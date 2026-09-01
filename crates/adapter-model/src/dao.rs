//! The `adapter_connections` store, and the Postgres implementation of it.
//!
//! The trait returns [`StoreError`], so a crate that reads connections needs no
//! database driver. [`PgAdapterConnectionDao`] sits behind the `postgres`
//! feature with everything else that names `sqlx`.

use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;

use moatless_types::adapter::AdapterConnection;

#[async_trait::async_trait]
pub trait AdapterConnectionDao: Send + Sync {
    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<AdapterConnection>, StoreError>;

    /// Look up a connection by Adapter + external account. Used for webhook
    /// routing where we identify the tenant from the inbound event, not from
    /// an authenticated caller — no scope filter applied.
    async fn find_by_external_account(
        &self,
        adapter_kind: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError>;

    /// Look up a connection by Adapter + external account. Used for webhook
    /// routing where we identify the tenant from the inbound event, not from
    /// an authenticated caller — no scope filter applied.
    async fn find_by_scope_value(
        &self,
        adapter_kind: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, StoreError>;

    /// List all connections for an Adapter, unscoped. Used by webhook routing
    /// to enumerate all tenants for an Adapter — no authenticated caller.
    async fn list_by_provider(
        &self,
        adapter_kind: &str,
    ) -> Result<Vec<AdapterConnection>, StoreError>;

    async fn list(
        &self,
        ctx: &ScopeContext,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, StoreError>;

    async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, StoreError>;

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, StoreError>;
}

#[cfg(feature = "postgres")]
mod postgres {
    use sqlx::{AssertSqlSafe, PgPool};

    use moatless_types::access_control::{ScopeContext, ScopedEntity};
    use moatless_types::store::StoreError;

    use moatless_types::adapter::AdapterConnection;
    use crate::persistence::AdapterConnectionRow;

    use super::AdapterConnectionDao;

pub struct PgAdapterConnectionDao(PgPool);

impl PgAdapterConnectionDao {
    /// Construct over a pool. The only way to build one — ad-hoc construction
    /// at use sites is what the composition-root invariant bans, so the field
    /// is private and callers take an `Arc<dyn AdapterConnectionDao>` port
    /// instead.
    pub fn new(pool: PgPool) -> Self {
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
        let row: Option<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await?;
        Ok(row.map(AdapterConnection::from))
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
        let row: Option<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 AND external_account_id = $2 LIMIT 1"
        )))
        .bind(adapter_kind)
        .bind(external_account_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(AdapterConnection::from))
    }

    /// List all connections for an Adapter, unscoped. Used by webhook routing
    /// to enumerate all tenants for an Adapter — no authenticated caller.
    pub async fn list_by_provider(
        &self,
        adapter_kind: &str,
    ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
        let rows: Vec<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 ORDER BY created_at ASC"
        )))
        .bind(adapter_kind)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().map(AdapterConnection::from).collect())
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
            let rows: Vec<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(sql))
                .bind(adapter_kind)
                .bind(ctx.is_admin)
                .bind(&ctx.user_id)
                .fetch_all(&self.0)
                .await?;
            return Ok(rows.into_iter().map(AdapterConnection::from).collect());
        }
        let access = ctx.access_filter(ScopedEntity::Resource, "", 1);
        let sql = format!(
            "{PROVIDER_CONNECTION_SELECT} \
             WHERE {} \
             ORDER BY created_at ASC",
            access.sql()
        );
        let rows: Vec<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await?;
        Ok(rows.into_iter().map(AdapterConnection::from).collect())
    }

    /// Upsert an adapter connection. On conflict (adapter_kind, external_account_id),
    /// the DO UPDATE arm includes a `write_filter` guard so cross-user updates
    /// affect 0 rows — the caller maps rows_affected == 0 to NotFound.
    pub async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, sqlx::Error> {
        // In the DO UPDATE arm both the target row and EXCLUDED are in scope,
        // so the filter's columns must be table-qualified.
        let ctrl = ctx.write_filter("adapter_connections.", 13);
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
        let inserted: AdapterConnectionRow = sqlx::query_as(AssertSqlSafe(sql))
            .bind(&row.id)
            .bind(row.adapter_kind.as_str())
            .bind(&row.connection_kind)
            .bind(&row.scope_value)
            .bind(&row.credential_ref)
            .bind(&row.webhook_secret)
            .bind(&row.transport_config)
            .bind(&row.created_by)
            .bind(row.scope.as_str())
            .bind(&row.owner_user_id)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        Ok(AdapterConnection::from(inserted))
    }

    pub async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, sqlx::Error> {
        let ctrl = ctx.write_filter("", 2);
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
    ) -> Result<Option<AdapterConnection>, StoreError> {
        PgAdapterConnectionDao::find_by_id(self, id, ctx)
            .await
            .map_err(|e| StoreError::with_source(format!("read adapter connection {id}"), e))
    }

    async fn find_by_external_account(
        &self,
        adapter_kind: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        PgAdapterConnectionDao::find_by_external_account(self, adapter_kind, external_account_id)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the {adapter_kind} connection for account {external_account_id}"),
                    e,
                )
            })
    }

    async fn find_by_scope_value(
        &self,
        adapter_kind: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        PgAdapterConnectionDao::find_by_scope_value(self, adapter_kind, scope_value)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the {adapter_kind} connection for {scope_value}"),
                    e,
                )
            })
    }

    async fn list_by_provider(
        &self,
        adapter_kind: &str,
    ) -> Result<Vec<AdapterConnection>, StoreError> {
        PgAdapterConnectionDao::list_by_provider(self, adapter_kind)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("list the {adapter_kind} connections"), e)
            })
    }

    async fn list(
        &self,
        ctx: &ScopeContext,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, StoreError> {
        PgAdapterConnectionDao::list(self, ctx, adapter_kind)
            .await
            .map_err(|e| StoreError::with_source("list adapter connections", e))
    }

    async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, StoreError> {
        PgAdapterConnectionDao::upsert(self, row, ctx)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("write adapter connection {}", row.id), e)
            })
    }

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, StoreError> {
        PgAdapterConnectionDao::delete(self, id, ctx)
            .await
            .map_err(|e| StoreError::with_source(format!("delete adapter connection {id}"), e))
    }
}
}

#[cfg(feature = "postgres")]
pub use postgres::PgAdapterConnectionDao;
