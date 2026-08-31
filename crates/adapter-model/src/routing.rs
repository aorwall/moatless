//! Resolving an inbound chat event to the connection and Subscription it
//! belongs to.
//!
//! Every chat Adapter answers the same question before it can route: which
//! Adapter connection owns this event's scope, and which Subscription matches
//! its source? The answer is contract, not policy — the tenant-isolation rule
//! below is one every Adapter has to obey identically — so it is spelled once
//! here rather than per provider.

use tracing::{debug, warn};

use crate::dao::AdapterConnectionDao;
use crate::error::AdapterError;
use crate::loops::LoopReader;
use crate::metrics::{inbound_reason, record_inbound};
use crate::models::{AdapterConnection, SubscriptionMatch};

/// Resolve one inbound chat event to its connection and Subscription.
///
/// `Ok(None)` is an ignore that has already been metered: each early return
/// records its own `inbound_reason` before it leaves, so a caller that drops
/// the `None` still leaves the operator a reason to query.
pub async fn resolve_chat_provider_event(
    adapter_connections: &dyn AdapterConnectionDao,
    loops: &dyn LoopReader,
    provider: &str,
    scope_value: &str,
    source_id: &str,
) -> Result<Option<(AdapterConnection, SubscriptionMatch)>, AdapterError> {
    let connection_repo = adapter_connections;
    // Look up exactly the requested scope_value. We deliberately do not fall
    // back to the "default" connection when scope_value is unknown — that would
    // silently route a tenant A event to tenant B's connection. Callers that
    // want the default behavior pass "default" explicitly.
    let Some(connection) = connection_repo
        .find_by_scope_value(provider, scope_value)
        .await?
    else {
        warn!(
            provider,
            scope_value, "No provider connection matched incoming event"
        );
        record_inbound(provider, "ignored", inbound_reason::NO_CONNECTION, "", "");
        return Ok(None);
    };

    let subscription_repo = loops;
    let subscription = subscription_repo
        .subscription_for_source(provider, source_id)
        .await?
        .or(subscription_repo
            .wildcard_subscription(provider)
            .await?);
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
