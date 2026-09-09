//! What a provider crate names to reach an Adapter connection, and the policy
//! every consumer of one shares.
//!
//! A provider compiles no database driver, so it cannot name either handle in
//! [`crate::connections`]. Each trait below is one authority: the reads a
//! provider makes as itself ([`ConnectionLookup`]), the reads it makes on
//! behalf of a caller ([`ConnectionReader`]), and the one write an OAuth
//! install performs ([`ConnectionInstaller`]). The handles implement them under
//! `postgres`, and `crate::test_support::InMemoryConnections` implements them
//! for a test.

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::error::{AdapterError, SharedAdapterError};
use crate::loops::LoopReader;
use crate::metrics::{inbound_reason, record_inbound};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::{AdapterConnection, AdapterKind};
use moatless_types::loops::SubscriptionMatch;
use moatless_types::store::StoreError;

/// The connection reads a provider makes as itself, where there is no viewer to
/// filter by: an inbound event names its tenant, and a stored Binding names the
/// connection it was created against.
#[async_trait]
pub trait ConnectionLookup: Send + Sync {
    /// The Adapter's connection for this scope value, or `None`.
    async fn by_scope_value(
        &self,
        provider: &str,
        scope_value: &str,
    ) -> Result<Option<AdapterConnection>, StoreError>;

    /// The Adapter's connection for this external account, or `None`.
    async fn by_external_account(
        &self,
        provider: &str,
        external_account_id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError>;

    /// The connection with this id, whoever owns it. For a caller that must be
    /// held to what it may see, take [`ConnectionReader::get`] instead.
    async fn by_id_trusted(&self, id: &str) -> Result<Option<AdapterConnection>, StoreError>;

    /// Every connection installed for this Adapter, oldest first.
    async fn by_provider(&self, provider: &str) -> Result<Vec<AdapterConnection>, StoreError>;
}

/// The connection reads a provider makes on behalf of a caller. Both are
/// filtered by what `ctx` may see.
#[async_trait]
pub trait ConnectionReader: Send + Sync {
    /// The connection with this id, when `ctx` may see it.
    async fn get(
        &self,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<Option<AdapterConnection>, StoreError>;

    /// The connections `ctx` may see, oldest first, for one Adapter or all.
    async fn list(
        &self,
        ctx: &ScopeContext,
        kind: Option<AdapterKind>,
    ) -> Result<Vec<AdapterConnection>, StoreError>;
}

/// The connection an OAuth install writes for the account it just granted,
/// over the lookup that carries the existing row's ownership forward.
#[async_trait]
pub trait ConnectionInstaller: ConnectionLookup {
    /// Write the connection, keyed by its Adapter and external account.
    async fn install(
        &self,
        connection: &AdapterConnection,
    ) -> Result<AdapterConnection, StoreError>;
}

/// The connection `id` names, as an error when the caller may not see it.
pub async fn connection_for_auth(
    connections: &dyn ConnectionReader,
    ctx: &ScopeContext,
    id: &str,
) -> Result<AdapterConnection, AdapterError> {
    connections.get(ctx, id).await?.ok_or_else(|| {
        SharedAdapterError::NotFound(format!("Provider connection {id} not found")).into()
    })
}

/// The connection a caller's request runs against: the one it named, or the
/// single one of that Adapter it can see.
///
/// Every error here is a CLI hint, so each names what to pass next.
pub async fn resolve_connection(
    connections: &dyn ConnectionReader,
    ctx: &ScopeContext,
    kind: AdapterKind,
    connection_id: Option<&str>,
) -> Result<AdapterConnection, AdapterError> {
    if let Some(id) = connection_id {
        let conn = connection_for_auth(connections, ctx, id).await?;
        if conn.adapter_kind != kind {
            return Err(SharedAdapterError::BadRequest(format!(
                "Connection {id} is for adapter {}, not {}",
                conn.adapter_kind,
                kind.as_str()
            ))
            .into());
        }
        return Ok(conn);
    }
    let rows = connections.list(ctx, Some(kind)).await?;
    let mut iter = rows.into_iter();
    match (iter.next(), iter.next()) {
        (None, _) => Err(SharedAdapterError::BadRequest(format!(
            "No {} connection configured. Create one with `moat adapters connections create --adapter {} ...` first.",
            kind.as_str(),
            kind.as_str()
        )).into()),
        (Some(only), None) => Ok(only),
        (Some(first), Some(second)) => {
            let mut ids: Vec<String> = vec![first.id, second.id];
            for row in iter {
                ids.push(row.id);
            }
            Err(SharedAdapterError::BadRequest(format!(
                "Multiple {} connections accessible: [{}]. Pass --connection <id> to choose.",
                kind.as_str(),
                ids.join(", ")
            )).into())
        }
    }
}

/// Resolve one inbound chat event to its connection and Subscription.
///
/// Every chat Adapter answers the same question before it can route: which
/// Adapter connection owns this event's scope, and which Subscription matches
/// its source? The answer is contract, not policy — the tenant-isolation rule
/// below is one every Adapter has to obey identically — so it is spelled once
/// here rather than per provider.
///
/// `Ok(None)` is an ignore that has already been metered: each early return
/// records its own `inbound_reason` before it leaves, so a caller that drops
/// the `None` still leaves the operator a reason to query.
pub async fn resolve_chat_provider_event(
    connections: &dyn ConnectionLookup,
    loops: &dyn LoopReader,
    provider: &str,
    scope_value: &str,
    source_id: &str,
) -> Result<Option<(AdapterConnection, SubscriptionMatch)>, AdapterError> {
    // Look up exactly the requested scope_value. We deliberately do not fall
    // back to the "default" connection when scope_value is unknown — that would
    // silently route a tenant A event to tenant B's connection. Callers that
    // want the default behavior pass "default" explicitly.
    let Some(connection) = connections.by_scope_value(provider, scope_value).await? else {
        warn!(
            provider,
            scope_value, "No provider connection matched incoming event"
        );
        record_inbound(provider, "ignored", inbound_reason::NO_CONNECTION, "", "");
        return Ok(None);
    };

    let subscription = loops
        .subscription_for_source(provider, source_id)
        .await?
        .or(loops.wildcard_subscription(provider).await?);
    let Some(subscription) = subscription else {
        record_inbound(
            provider,
            "ignored",
            inbound_reason::NO_RULE_MATCH,
            "",
            &connection.id,
        );
        return Ok(None);
    };

    if subscription.connection_id != connection.id {
        debug!(
            provider,
            source_id,
            subscription_id = %subscription.id,
            connection_id = %connection.id,
            "Task trigger belongs to a different connection"
        );
        record_inbound(
            provider,
            "ignored",
            inbound_reason::WRONG_CONNECTION,
            &subscription.id,
            &connection.id,
        );
        return Ok(None);
    }

    Ok(Some((connection, subscription)))
}
