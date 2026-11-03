//! Access-control vocabulary a provider consults before processing an inbound
//! message.
//!
//! States whether a source may reach the agent, when an allowed source triggers
//! a run, and what a single check decided. Used by the chat providers
//! (Slack, and future ones) that gate inbound traffic per source.

use crate::error::AdapterError;
use moatless_types::access_control::{
    ResolvedScope, Scope, ScopeContext, resolve_scope_for_create,
};
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
