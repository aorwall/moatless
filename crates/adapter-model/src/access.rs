//! The access-control vocabulary a provider consults before it processes an
//! inbound message: whether a source is allowed to reach the agent at all, when
//! an allowed source triggers a run, and what a single check decided.
//!
//! Used by the chat providers (Telegram, Slack, and future ones) that gate
//! inbound traffic per source.

use crate::dao::AdapterConnectionDao;
use crate::error::{AdapterError, SharedAdapterError};
use moatless_types::access_control::{
    ResolvedScope, Scope, ScopeContext, resolve_scope_for_create,
};
use moatless_types::adapter::{AdapterConnection, AdapterKind};
use serde::{Deserialize, Serialize};

/// Access policy for an external source (DM, group, or Slack channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceAccessPolicy {
    /// Accept messages from any source.
    #[default]
    Open,
    /// Reject all messages.
    Disabled,
    /// Accept only from sources in the allowlist.
    Allowlist,
    /// Allowlist + prompt unknown users with a pairing code (DM only).
    Pairing,
}

/// When to trigger processing of an inbound message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageTrigger {
    /// Only process messages that mention the bot.
    #[default]
    MentionOnly,
    /// Process all messages in the source.
    AllMessages,
}

/// Result of an access control check.
#[derive(Debug)]
pub enum AccessDecision {
    /// Message is allowed through.
    Allowed,
    /// Message is blocked (silently dropped).
    Blocked,
    /// A pairing code was sent to the user; message is not processed.
    PairingSent,
}

pub fn resolve_scope_for_adapter_create(
    ctx: &ScopeContext,
    requested_scope: Option<Scope>,
) -> Result<ResolvedScope, AdapterError> {
    Ok(resolve_scope_for_create(ctx, requested_scope)?)
}

pub async fn load_adapter_connection_for_auth_with(
    adapter_connections: &dyn AdapterConnectionDao,
    ctx: &ScopeContext,
    id: &str,
) -> Result<AdapterConnection, AdapterError> {
    adapter_connections
        .find_by_id(id, ctx)
        .await?
        .ok_or_else(|| {
            SharedAdapterError::NotFound(format!("Provider connection {id} not found")).into()
        })
}

pub async fn resolve_connection(
    adapter_connections: &dyn AdapterConnectionDao,
    ctx: &ScopeContext,
    kind: AdapterKind,
    connection_id: Option<&str>,
) -> Result<AdapterConnection, AdapterError> {
    if let Some(id) = connection_id {
        let conn = load_adapter_connection_for_auth_with(adapter_connections, ctx, id).await?;
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
    let rows = adapter_connections.list(ctx, Some(kind.as_str())).await?;
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
