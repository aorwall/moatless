//! Native `SdkMessage` content → canonical `SourceEvent` normalization.
//!
//! One `SourceEvent` per native content block, in native order, with repeated
//! usage forwarded verbatim (never coalesced). This is the only place that maps
//! Claude-native payloads; the shared harness/driver never sees an
//! `SdkMessage`. Both the one-shot subprocess adapter and the persistent tmux
//! adapter share this normalizer, so neither owns a second copy of the native
//! mapping.

use crate::agent::harness::content::CanonicalContentBlock;
use claude_agent_sdk::api::content::ContentBlock;
use claude_agent_sdk::api::messages::{AssistantMessage, ToolProgressMessage, UserMessage};

use super::super::signal::{Activity, SessionId};
use super::super::source::{NativeMessageId, Role, SourceEvent, SourceId};

/// Map a native Claude `ContentBlock` into the shared
/// [`CanonicalContentBlock`]. The two enums share the Anthropic content-block
/// shape variant-for-variant, so this is a total, lossless projection.
fn canonical_from_claude(block: &ContentBlock) -> CanonicalContentBlock {
    match block {
        ContentBlock::Text { text } => CanonicalContentBlock::Text { text: text.clone() },
        ContentBlock::ToolUse { id, name, input } => CanonicalContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => CanonicalContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        ContentBlock::Thinking {
            thinking,
            signature,
        } => CanonicalContentBlock::Thinking {
            thinking: thinking.clone(),
            signature: signature.clone(),
        },
        ContentBlock::Image { source } => CanonicalContentBlock::Image {
            source: source.clone(),
        },
        ContentBlock::ServerToolUse { id, name, input } => CanonicalContentBlock::ServerToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentBlock::WebSearchToolResult {
            tool_use_id,
            content,
        } => CanonicalContentBlock::WebSearchToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
        },
        ContentBlock::RedactedThinking { data } => {
            CanonicalContentBlock::RedactedThinking { data: data.clone() }
        }
    }
}

/// The per-message metadata shared by every content block of one native
/// `SdkMessage`, so a whole assistant/user envelope's blocks are normalized
/// with one grouping id, parent, usage, and model.
struct BlockMeta {
    session: SessionId,
    message_id: Option<String>,
    parent_tool_use_id: Option<String>,
    role: Role,
    usage: Option<serde_json::Value>,
    model: Option<String>,
}

/// Collision-safe native-block normalizer.
///
/// Every emitted `source_id` stays unique across the whole process lifetime,
/// including a persistent session's many Turns, even when the CLI reuses or
/// omits a native `message.id` (see the fields for the mechanism).
pub(super) struct Normalizer {
    /// The Session as currently known from the native stream.
    session: Option<SessionId>,
    /// Monotonic block counter: keeps every `source_id` unique within this
    /// process even when the CLI reuses a native `message.id`.
    block_seq: u64,
    /// Per-process collision-avoidance element for blocks with no native id.
    nonce: String,
}

impl Normalizer {
    pub(super) fn new() -> Self {
        Self {
            session: None,
            block_seq: 0,
            nonce: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    pub(super) fn session(&self) -> Option<SessionId> {
        self.session.clone()
    }

    pub(super) fn set_session(&mut self, session: SessionId) {
        self.session = Some(session);
    }

    /// The Session as currently known, falling back to a native id when a block
    /// is normalized before its `System` init (defensive; the driver buffers
    /// early blocks regardless).
    fn session_or(&self, native: &str) -> SessionId {
        self.session
            .clone()
            .unwrap_or_else(|| SessionId(native.to_string()))
    }

    /// Build the canonical source event for one native content block, assigning
    /// a collision-safe id and preserving native grouping/usage/model.
    fn make_source(&mut self, meta: &BlockMeta, block: &ContentBlock) -> SourceEvent {
        let seq = self.block_seq;
        self.block_seq += 1;
        let native = meta
            .message_id
            .clone()
            .unwrap_or_else(|| format!("{}-anon-{seq}", self.nonce));
        SourceEvent {
            source_id: SourceId(format!("{native}#{seq}")),
            message_id: NativeMessageId(native),
            session: meta.session.clone(),
            parent_tool_use_id: meta.parent_tool_use_id.clone(),
            role: meta.role,
            block: canonical_from_claude(block),
            tool_use_result: None,
            usage: meta.usage.clone(),
            model: meta.model.clone(),
            diagnostics: None,
        }
    }

    /// Canonical per-block source events for one assistant envelope, in native
    /// order.
    pub(super) fn assistant_blocks(&mut self, assistant: &AssistantMessage) -> Vec<SourceEvent> {
        let meta = BlockMeta {
            session: self.session_or(&assistant.session_id),
            message_id: assistant.message.id.clone(),
            parent_tool_use_id: assistant.parent_tool_use_id.clone(),
            role: Role::Assistant,
            usage: assistant
                .message
                .usage
                .as_ref()
                .and_then(|u| serde_json::to_value(u).ok()),
            model: assistant.message.model.clone(),
        };
        assistant
            .message
            .content
            .iter()
            .map(|block| self.make_source(&meta, block))
            .collect()
    }

    /// Canonical per-block source events for one user envelope, in native
    /// order.
    pub(super) fn user_blocks(&mut self, user: &UserMessage) -> Vec<SourceEvent> {
        let meta = BlockMeta {
            session: self.session_or(&user.session_id),
            message_id: user.uuid.clone(),
            parent_tool_use_id: user.parent_tool_use_id.clone(),
            role: Role::User,
            usage: None,
            model: None,
        };
        user.message
            .content
            .iter()
            .map(|block| self.make_source(&meta, block))
            .collect()
    }

    /// Liveness activity for a tool-progress envelope.
    pub(super) fn progress_activity(&self, progress: &ToolProgressMessage) -> Activity {
        Activity {
            session: self.session_or(&progress.session_id),
            detail: progress.tool_name.clone(),
        }
    }
}
