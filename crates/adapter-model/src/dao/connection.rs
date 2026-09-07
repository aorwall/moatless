//! The `adapter_connections` store, and the Postgres implementation of it.
//!
//! The trait returns [`StoreError`], so a crate that reads connections needs no
//! database driver. `PgAdapterConnectionDao` sits behind the `postgres`
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

    /// Look up a connection by Adapter + `scope_value` (the connection's stored
    /// `external_account_id`), for callers that key connections by scope rather
    /// than by external account. `None` when no connection matches that exact
    /// value — the caller decides whether to fall back to another scope value.
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
