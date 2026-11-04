//! The `adapter_connections` table, as each of its two authorities reaches it.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::{AdapterConnection, AdapterKind};
use moatless_types::store::StoreError;

use super::dao::PgAdapterConnectionDao;
use super::ports::{ConnectionEvent, ConnectionInstaller, ConnectionLookup, ConnectionReader};
use crate::error::AdapterError;

/// Both connection handles over one store.
#[derive(Clone)]
pub struct ConnectionGraph {
    store: Arc<PgAdapterConnectionDao>,
}

impl ConnectionGraph {
    pub(crate) fn over(store: Arc<PgAdapterConnectionDao>) -> Self {
        Self { store }
    }

    /// The connections a viewer administers.
    pub fn administration(&self) -> Arc<AdapterConnections> {
        Arc::new(AdapterConnections {
            store: self.store.clone(),
        })
    }

    /// The connections an Adapter routes against.
    pub fn routing(&self) -> Arc<ConnectionRouting> {
        Arc::new(ConnectionRouting {
            store: self.store.clone(),
        })
    }
}

/// Adapter connections as a caller reaches them: every operation runs under the
/// viewer's `ScopeContext`, so it reads and writes only what that viewer may.
pub struct AdapterConnections {
    store: Arc<PgAdapterConnectionDao>,
}

impl AdapterConnections {
    /// The connection with this id, when `ctx` may see it.
    pub async fn get(
        &self,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        self.store.find_by_id(id, ctx).await
    }

    /// The connections `ctx` may see, oldest first, for one Adapter or all.
    pub async fn list(
        &self,
        ctx: &ScopeContext,
        kind: Option<AdapterKind>,
    ) -> Result<Vec<AdapterConnection>, StoreError> {
        self.store.list(ctx, kind.map(AdapterKind::as_str)).await
    }

    /// Write a connection, keyed by its Adapter and external account. A write
    /// `ctx` may not make matches no row, and surfaces as a missing row.
    pub async fn save(
        &self,
        ctx: &ScopeContext,
        connection: &AdapterConnection,
    ) -> Result<AdapterConnection, StoreError> {
        self.store.upsert(connection, ctx).await
    }

    /// Delete a connection, returning how many rows went.
    pub async fn delete(&self, ctx: &ScopeContext, id: &str) -> Result<u64, StoreError> {
        self.store.delete(id, ctx).await
    }

    /// See [`super::resolve_connection`].
    pub async fn resolve(
        &self,
        ctx: &ScopeContext,
        kind: AdapterKind,
        connection_id: Option<&str>,
    ) -> Result<AdapterConnection, AdapterError> {
        super::resolve_connection(self, ctx, kind, connection_id).await
    }

    /// See [`super::connection_for_auth`].
    pub async fn for_auth(
        &self,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<AdapterConnection, AdapterError> {
        super::connection_for_auth(self, ctx, id).await
    }
}

#[async_trait]
impl ConnectionReader for AdapterConnections {
    async fn get(
        &self,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        AdapterConnections::get(self, ctx, id).await
    }

    async fn list(
        &self,
        ctx: &ScopeContext,
        kind: Option<AdapterKind>,
    ) -> Result<Vec<AdapterConnection>, StoreError> {
        AdapterConnections::list(self, ctx, kind).await
    }
}

/// Adapter connections as an Adapter reaches them: the caller is the provider
/// itself, so there is no viewer to filter by and every read here is unscoped.
pub struct ConnectionRouting {
    store: Arc<PgAdapterConnectionDao>,
}

impl ConnectionRouting {
    /// See [`ConnectionLookup::by_scope_value`].
    pub async fn by_scope_value(
        &self,
        provider: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        self.store.find_by_scope_value(provider, scope_value).await
    }

    /// See [`ConnectionLookup::by_external_account`].
    pub async fn by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        self.store
            .find_by_external_account(provider, external_account_id)
            .await
    }

    /// See [`ConnectionLookup::by_id_trusted`].
    pub async fn by_id_trusted(&self, id: &str) -> Result<Option<AdapterConnection>, StoreError> {
        // reason: routing reads the connection a stored row already names, so
        // there is no caller identity to scope by.
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        self.store.find_by_id(id, &sys).await
    }

    /// See [`ConnectionLookup::by_provider`].
    pub async fn by_provider(&self, provider: &str) -> Result<Vec<AdapterConnection>, StoreError> {
        self.store.list_by_provider(provider).await
    }

    /// See [`ConnectionLookup::record_event`].
    pub async fn record_event(
        &self,
        connection_id: &str,
        event_type: &str,
        subject_key: &str,
        external_id: &str,
    ) -> Result<(), StoreError> {
        self.store
            .record_event(connection_id, event_type, subject_key, external_id)
            .await
    }

    /// Every `event_type` a `provider` connection delivered at or after
    /// `since`.
    pub async fn events_since(
        &self,
        provider: &str,
        event_type: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<ConnectionEvent>, StoreError> {
        self.store.events_since(provider, event_type, since).await
    }

    /// See [`ConnectionInstaller::install`].
    pub async fn install(
        &self,
        connection: &AdapterConnection,
    ) -> Result<AdapterConnection, StoreError> {
        // reason: an OAuth install writes the connection the grant just
        // authorized, and no session backs it.
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        self.store.upsert(connection, &sys).await
    }
}

#[async_trait]
impl ConnectionLookup for ConnectionRouting {
    async fn by_scope_value(
        &self,
        provider: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        ConnectionRouting::by_scope_value(self, provider, scope_value).await
    }

    async fn by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError> {
        ConnectionRouting::by_external_account(self, provider, external_account_id).await
    }

    async fn by_id_trusted(&self, id: &str) -> Result<Option<AdapterConnection>, StoreError> {
        ConnectionRouting::by_id_trusted(self, id).await
    }

    async fn by_provider(&self, provider: &str) -> Result<Vec<AdapterConnection>, StoreError> {
        ConnectionRouting::by_provider(self, provider).await
    }

    async fn record_event(
        &self,
        connection_id: &str,
        event_type: &str,
        subject_key: &str,
        external_id: &str,
    ) -> Result<(), StoreError> {
        ConnectionRouting::record_event(self, connection_id, event_type, subject_key, external_id)
            .await
    }
}

#[async_trait]
impl ConnectionInstaller for ConnectionRouting {
    async fn install(
        &self,
        connection: &AdapterConnection,
    ) -> Result<AdapterConnection, StoreError> {
        ConnectionRouting::install(self, connection).await
    }
}
