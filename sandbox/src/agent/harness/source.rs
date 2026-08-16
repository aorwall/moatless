//! Canonical source events — one per native content block.
//!
//! Every source event represents exactly one native content block. The type
//! deliberately carries a single `block`, never a vector, so a harness cannot
//! return an already-assembled Message: Message assembly is exclusively the
//! backend's job (server-owned message assembly, the agent.message skeleton, the harness-outcome boundary).

use crate::agent::harness::content::CanonicalContentBlock;

use super::signal::SessionId;

/// The producer role of a native content block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Assistant,
    User,
    System,
}

/// Stable, collision-safe identity for one source event.
///
/// When a CLI reuses native identifiers after process replacement or Sandbox
/// recreation, the harness adds a collision-avoidance element (without exposing
/// provider details) so two blocks never share an identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(pub String);

/// Native Message grouping identity — the backend folds blocks that share this
/// identity into one assembled Message, preserving block order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeMessageId(pub String);

/// One native content block, normalized into the canonical shape the backend
/// assembles. Harnesses preserve native ordering and repeated usage; they do
/// not coalesce content, fold tool results, or deduplicate usage.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceEvent {
    /// Stable, collision-safe identity for this block.
    pub source_id: SourceId,
    /// Native Message identity used by the backend to group blocks.
    pub message_id: NativeMessageId,
    /// The Session that produced the block.
    pub session: SessionId,
    /// Parent tool identity for subagent attribution, when present.
    pub parent_tool_use_id: Option<String>,
    /// Producer role.
    pub role: Role,
    /// The single canonical content block. Both agent harnesses transform their
    /// native output into this one type-checked shape.
    pub block: CanonicalContentBlock,
    /// Typed tool result surfaced at the payload top level (`toolUseResult`) for
    /// backend enrichment of a `tool_result` block — e.g. a Codex `CodexShellResult`
    /// the UI renders as command output. `None` for every non-result block.
    pub tool_use_result: Option<serde_json::Value>,
    /// Usage exactly as reported on this block (repeated usage is preserved for
    /// backend admission, never coalesced in the harness).
    pub usage: Option<serde_json::Value>,
    /// Model that produced the block, where available.
    pub model: Option<String>,
    /// Bounded native diagnostics preserved for unknown or new block types so an
    /// upstream CLI update does not silently discard output.
    pub diagnostics: Option<serde_json::Value>,
}

impl SourceEvent {
    /// The block serialized to its Anthropic-shaped wire JSON — the form the
    /// coordinator forwards to the backend assembler. Serialization is infallible
    /// for these variants; `Null` is a defensive floor.
    pub fn block_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.block).unwrap_or(serde_json::Value::Null)
    }
}
