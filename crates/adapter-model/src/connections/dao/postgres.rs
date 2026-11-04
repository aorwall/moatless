//! PostgreSQL Adapter connection persistence.

use sqlx::{AssertSqlSafe, PgPool};

use moatless_types::access_control::{ScopeContext, ScopedEntity};
use moatless_types::adapter::AdapterConnection;
use moatless_types::store::StoreError;

use super::row::AdapterConnectionRow;
use crate::connections::ConnectionEvent;
use chrono::{DateTime, Utc};

pub(crate) struct PgAdapterConnectionDao(PgPool);

impl PgAdapterConnectionDao {
    pub(crate) fn new(pool: PgPool) -> Self {
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
    pub(crate) async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE id = $1 AND {}",
            access.sql()
        );
        let row: Option<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
            .map_err(|e| StoreError::with_source(format!("read adapter connection {id}"), e))?;
        Ok(row.map(AdapterConnection::from))
    }

    /// Look up a connection by Adapter + external account. Used for webhook
    /// routing (identifies tenant from inbound event) — no scope filter
    /// applied.
    pub(crate) async fn find_by_external_account(
        &self,
        adapter_kind: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        let row: Option<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(format!(
        "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 AND external_account_id = $2 LIMIT 1"
    )))
    .bind(adapter_kind)
    .bind(external_account_id)
    .fetch_optional(&self.0)
    .await
    .map_err(|e| {
        StoreError::with_source(
            format!("read the {adapter_kind} connection for account {external_account_id}"),
            e,
        )
    })?;
        Ok(row.map(AdapterConnection::from))
    }

    /// Same query as [`Self::find_by_external_account`], for a caller that keys
    /// connections by scope value rather than by external account.
    pub(crate) async fn find_by_scope_value(
        &self,
        adapter_kind: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        let row: Option<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(format!(
        "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 AND external_account_id = $2 LIMIT 1"
    )))
    .bind(adapter_kind)
    .bind(scope_value)
    .fetch_optional(&self.0)
    .await
    .map_err(|e| {
        StoreError::with_source(
            format!("read the {adapter_kind} connection for {scope_value}"),
            e,
        )
    })?;
        Ok(row.map(AdapterConnection::from))
    }

    /// List all connections for an Adapter, unscoped. Used by webhook routing
    /// to enumerate all tenants for an Adapter — no authenticated caller.
    pub(crate) async fn list_by_provider(
        &self,
        adapter_kind: &str,
    ) -> Result<Vec<AdapterConnection>, StoreError> {
        let rows: Vec<AdapterConnectionRow> = sqlx::query_as(AssertSqlSafe(format!(
            "{PROVIDER_CONNECTION_SELECT} WHERE adapter_kind = $1 ORDER BY created_at ASC"
        )))
        .bind(adapter_kind)
        .fetch_all(&self.0)
        .await
        .map_err(|e| StoreError::with_source(format!("list the {adapter_kind} connections"), e))?;
        Ok(rows.into_iter().map(AdapterConnection::from).collect())
    }

    pub(crate) async fn list(
        &self,
        ctx: &ScopeContext,
        adapter_kind: Option<&str>,
    ) -> Result<Vec<AdapterConnection>, StoreError> {
        if let Some(adapter_kind) = adapter_kind {
            let access =
                moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 2);
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
                .await
                .map_err(|e| StoreError::with_source("list adapter connections", e))?;
            return Ok(rows.into_iter().map(AdapterConnection::from).collect());
        }
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 1);
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
            .await
            .map_err(|e| StoreError::with_source("list adapter connections", e))?;
        Ok(rows.into_iter().map(AdapterConnection::from).collect())
    }

    /// Upsert an adapter connection. On conflict (`adapter_kind`,
    /// `external_account_id`), the DO UPDATE arm includes a `write_filter`
    /// guard so cross-user updates affect 0 rows, which surfaces as
    /// `RowNotFound`.
    pub(crate) async fn upsert(
        &self,
        row: &AdapterConnection,
        ctx: &ScopeContext,
    ) -> Result<AdapterConnection, StoreError> {
        // In the DO UPDATE arm both the target row and EXCLUDED are in scope,
        // so the filter's columns must be table-qualified.
        let ctrl = moatless_core::db::access_control::write_filter(
            ScopedEntity::Resource,
            "adapter_connections.",
            13,
        );
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
            .await
            .and_then(|found| found.ok_or(sqlx::Error::RowNotFound))
            .map_err(|e| {
                StoreError::with_source(format!("write adapter connection {}", row.id), e)
            })?;
        Ok(AdapterConnection::from(inserted))
    }

    /// Record that `connection_id` delivered `event_type` for `subject_key`,
    /// replacing the previous record of that pair.
    pub(crate) async fn record_event(
        &self,
        connection_id: &str,
        event_type: &str,
        subject_key: &str,
        external_id: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO adapter_connection_events \
               (connection_id, event_type, subject_key, external_id, received_at) \
             VALUES ($1, $2, $3, $4, now()) \
             ON CONFLICT (connection_id, event_type, subject_key) DO UPDATE \
               SET external_id = EXCLUDED.external_id, received_at = EXCLUDED.received_at",
        )
        .bind(connection_id)
        .bind(event_type)
        .bind(subject_key)
        .bind(external_id)
        .execute(&self.0)
        .await
        .map_err(|e| {
            StoreError::with_source(
                format!("record {event_type} for {subject_key} on connection {connection_id}"),
                e,
            )
        })?;
        Ok(())
    }

    /// Every `event_type` an `adapter_kind` connection delivered at or after
    /// `since`, with the scope value of the connection that delivered it.
    pub(crate) async fn events_since(
        &self,
        adapter_kind: &str,
        event_type: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<ConnectionEvent>, StoreError> {
        let rows: Vec<(String, String, String, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT e.connection_id, c.external_account_id, e.subject_key, e.external_id, \
                    e.received_at \
             FROM adapter_connection_events e \
             JOIN adapter_connections c ON c.id = e.connection_id \
             WHERE c.adapter_kind = $1 AND e.event_type = $2 AND e.received_at >= $3",
        )
        .bind(adapter_kind)
        .bind(event_type)
        .bind(since)
        .fetch_all(&self.0)
        .await
        .map_err(|e| {
            StoreError::with_source(format!("list {adapter_kind} {event_type} events"), e)
        })?;
        Ok(rows
            .into_iter()
            .map(
                |(connection_id, scope_value, subject_key, external_id, received_at)| {
                    ConnectionEvent {
                        connection_id,
                        scope_value,
                        subject_key,
                        external_id,
                        received_at,
                    }
                },
            )
            .collect())
    }

    pub(crate) async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, StoreError> {
        let ctrl = moatless_core::db::access_control::write_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "DELETE FROM adapter_connections WHERE id = $1 AND {}",
            ctrl.sql()
        );
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await
            .map_err(|e| StoreError::with_source(format!("delete adapter connection {id}"), e))?;
        Ok(result.rows_affected())
    }
}
