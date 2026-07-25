//! Native Codex thread item → canonical `SourceEvent` normalization.
//!
//! One `SourceEvent` per native item, in native order, preserving the item's
//! exact JSON block (`ThreadItem::raw`) so every item family — agent messages,
//! reasoning, commands, file changes, MCP/collab tools, web search, todo lists,
//! errors, and unknown kinds — keeps its observable meaning without the harness
//! re-deriving native payloads. This is the only place that maps Codex-native
//! items; the shared harness/driver never sees a `CodexExecEvent`.
//!
//! Codex restarts its `item_id` numbering at `item_0` on every `codex exec
//! --resume`, so a per-spawn nonce is mixed into each `source_id` to keep two
//! blocks from ever colliding across process replacement.

use moatless_api_schemas::content::CanonicalContentBlock;

use crate::agent::codex_exec::{ThreadItem, Usage};

use super::super::signal::SessionId;
use super::super::source::{NativeMessageId, Role, SourceEvent, SourceId};
use super::blocks;

/// Upper bound on retained native diagnostics for an unknown item block.
const MAX_DIAGNOSTIC_BYTES: usize = 2000;

/// Collision-safe Codex normalizer. Holds the last-known Session (the Codex
/// thread), the requested model, and a per-spawn nonce so `source_id`s stay
/// unique even when the CLI reuses `item_0` after a resume.
pub(super) struct Normalizer {
    session: Option<SessionId>,
    model: Option<String>,
    /// Per-spawn collision-avoidance element mixed into every emitted id.
    nonce: String,
}

impl Normalizer {
    pub(super) fn new(model: Option<String>) -> Self {
        Self {
            session: None,
            model,
            nonce: uuid::Uuid::new_v4().simple().to_string()[..8].to_string(),
        }
    }

    /// Record the Codex thread as the Session and refresh the per-spawn nonce.
    /// A fresh nonce on every `thread.started` keeps ids disjoint across the
    /// resume-driven `item_0` restart.
    pub(super) fn set_session(&mut self, session: SessionId) {
        self.session = Some(session);
        self.nonce = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    }

    fn session_or(&self, native: &str) -> SessionId {
        self.session
            .clone()
            .unwrap_or_else(|| SessionId(native.to_string()))
    }

    /// Qualify a native id with the Session and per-spawn nonce so it survives
    /// Codex's per-resume `item_0` restart without colliding. Mirrors the legacy
    /// `{session}_{nonce}_{item}` UUID scheme so persisted reads line up.
    fn qualify(&self, session: &SessionId, native: &str) -> String {
        format!("{}_{}_{native}", session.0, self.nonce)
    }

    /// Canonical source events for one native Codex item, in native order. Each
    /// native family is mapped to the `text` / `tool_use` / `tool_result` blocks
    /// the backend assembler and UI understand (a tool item expands to a
    /// tool_use plus a paired tool_result in its own user Message). An unknown
    /// kind is forwarded as its raw native block with bounded diagnostics so an
    /// upstream CLI addition is never silently discarded.
    pub(super) fn item_sources(&self, item: &ThreadItem) -> Vec<SourceEvent> {
        let session = self.session_or(&item.id);
        let qualified = self.qualify(&session, &item.id);

        let Some(blocks) = blocks::item_blocks(&item.details, &qualified) else {
            // Unknown kind: preserve the raw native block, flagged with bounded
            // diagnostics.
            return vec![SourceEvent {
                source_id: SourceId(qualified.clone()),
                message_id: NativeMessageId(qualified),
                session,
                parent_tool_use_id: None,
                role: Role::Assistant,
                block: CanonicalContentBlock::Unknown {
                    raw: item.raw.clone(),
                },
                tool_use_result: None,
                usage: None,
                model: self.model.clone(),
                diagnostics: Some(bounded(&item.raw)),
            }];
        };

        blocks
            .into_iter()
            .map(|canonical| {
                // A tool_result lands in its own Message (`{id}_result`) so it is
                // grouped and role-assigned separately from the tool_use.
                let id = format!("{qualified}{}", canonical.id_suffix);
                SourceEvent {
                    source_id: SourceId(id.clone()),
                    message_id: NativeMessageId(id),
                    session: session.clone(),
                    // Codex items carry no per-item parent tool attribution; collab
                    // relationships ride inside the item block itself.
                    parent_tool_use_id: None,
                    role: canonical.role,
                    block: canonical.block,
                    tool_use_result: canonical.tool_use_result,
                    // Codex reports usage once per turn, not per item — carried on
                    // the turn-result source event instead.
                    usage: None,
                    model: self.model.clone(),
                    diagnostics: None,
                }
            })
            .collect()
    }

    /// Canonical source event for a completed turn, carrying the turn-level usage
    /// (Codex's only usage report) so it is retained through the seam rather than
    /// dropped at the terminal, which has no usage carrier.
    pub(super) fn turn_result_source(&self, usage: &Usage) -> SourceEvent {
        let session = self.session_or("");
        let id = format!("{}_{}_result", session.0, self.nonce);
        SourceEvent {
            source_id: SourceId(id.clone()),
            message_id: NativeMessageId(id),
            session,
            parent_tool_use_id: None,
            role: Role::System,
            block: CanonicalContentBlock::TurnResult,
            tool_use_result: None,
            usage: Some(serde_json::json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cached_input_tokens,
                "reasoning_output_tokens": usage.reasoning_output_tokens,
            })),
            model: self.model.clone(),
            diagnostics: None,
        }
    }
}

/// Bound a native diagnostic value to a fixed byte budget so an oversized
/// unknown item cannot blow up a retained diagnostic string.
fn bounded(raw: &serde_json::Value) -> serde_json::Value {
    let text = raw.to_string();
    if text.len() <= MAX_DIAGNOSTIC_BYTES {
        return raw.clone();
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::Value::String(format!("{}…", &text[..end]))
}
