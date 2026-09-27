//! Native `SdkMessage` content → canonical content block normalization.
//!
//! One canonical block per native content block, in native order. This is the
//! only place that maps Claude-native payloads; the shared harness/driver
//! never sees an `SdkMessage`. Both the one-shot subprocess adapter and the
//! persistent tmux adapter share this normalizer, so neither owns a second
//! copy of the native mapping.

use crate::agent::harness::content::CanonicalContentBlock;
use claude_agent_sdk::api::content::ContentBlock;
use claude_agent_sdk::api::messages::{AssistantMessage, ToolProgressMessage, UserMessage};

use super::super::signal::{Activity, SessionId};

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

/// Tracks the Session as currently known from the native stream.
pub(super) struct Normalizer {
    session: Option<SessionId>,
}

impl Normalizer {
    pub(super) fn new() -> Self {
        Self { session: None }
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

    /// Liveness activity for a tool-progress envelope.
    pub(super) fn progress_activity(&self, progress: &ToolProgressMessage) -> Activity {
        Activity {
            session: self.session_or(&progress.session_id),
            detail: progress.tool_name.clone(),
        }
    }
}

/// Canonical blocks for one assistant envelope, in native order.
pub(super) fn assistant_blocks(assistant: &AssistantMessage) -> Vec<CanonicalContentBlock> {
    assistant
        .message
        .content
        .iter()
        .map(canonical_from_claude)
        .collect()
}

/// Canonical blocks for one user envelope, in native order.
pub(super) fn user_blocks(user: &UserMessage) -> Vec<CanonicalContentBlock> {
    user.message
        .content
        .iter()
        .map(canonical_from_claude)
        .collect()
}
