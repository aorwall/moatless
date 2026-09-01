//! The access-control vocabulary a provider consults before it processes an
//! inbound message: whether a source is allowed to reach the agent at all, when
//! an allowed source triggers a run, and what a single check decided.
//!
//! Used by the chat providers (Telegram, Slack, and future ones) that gate
//! inbound traffic per source.

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
