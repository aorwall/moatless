//! The canonical, fully-typed content block.
//!
//! Both agent harnesses (Claude Code and Codex) normalize their native output
//! into this one type, matching the Anthropic content-block wire format the
//! frontend consumes — so serializing a [`CanonicalContentBlock`] is
//! wire-compatible downstream.

use claude_agent_sdk::api::content::ImageSource;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One canonical content block. Tagged by `type`, matching the Anthropic
/// content-block wire format the UI consumes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CanonicalContentBlock {
    /// Assistant or user text.
    Text { text: String },

    /// An assistant tool invocation. `input` is the tool's own argument object,
    /// which is heterogeneous across tools and stays untyped by design.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },

    /// The result of a tool invocation, linked to its `ToolUse` by
    /// `tool_use_id`.
    ToolResult {
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },

    /// Model reasoning with its verification signature.
    Thinking { thinking: String, signature: String },

    /// An inline image.
    Image { source: ImageSource },

    /// A server-side tool invocation (e.g. web search), Claude-only.
    ServerToolUse {
        id: String,
        name: String,
        input: Value,
    },

    /// The result of a server-side tool invocation, Claude-only.
    WebSearchToolResult {
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Value>,
    },

    /// Reasoning the provider redacted; only its opaque payload survives.
    RedactedThinking { data: String },

    /// Codex's per-turn marker: the turn finished. Carries no content of its
    /// own — the turn's spend rides the terminal `Outcome`'s `RunTotals`
    /// instead.
    TurnResult,

    /// A native block whose kind the harness does not map. `raw` is the
    /// original native JSON, preserved verbatim so an upstream CLI addition is
    /// surfaced rather than silently discarded.
    Unknown { raw: Value },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Each variant serializes to the Anthropic-shaped tag the UI consumes,
    /// so adopting the type is wire-compatible.
    #[test]
    fn variants_serialize_to_anthropic_tags() {
        assert_eq!(
            serde_json::to_value(CanonicalContentBlock::Text {
                text: "hi".to_string()
            })
            .unwrap(),
            json!({ "type": "text", "text": "hi" })
        );
        assert_eq!(
            serde_json::to_value(CanonicalContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "Bash".to_string(),
                input: json!({ "command": "ls" }),
            })
            .unwrap(),
            json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } })
        );
        assert_eq!(
            serde_json::to_value(CanonicalContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                content: Some(json!("ok")),
                is_error: Some(false),
            })
            .unwrap(),
            json!({ "type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": false })
        );
        assert_eq!(
            serde_json::to_value(CanonicalContentBlock::TurnResult).unwrap(),
            json!({ "type": "turn_result" })
        );
    }

    /// `ToolResult`'s optional fields are omitted when absent, matching the
    /// Anthropic wire format's sparse encoding.
    #[test]
    fn tool_result_omits_absent_optionals() {
        assert_eq!(
            serde_json::to_value(CanonicalContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                content: None,
                is_error: None,
            })
            .unwrap(),
            json!({ "type": "tool_result", "tool_use_id": "t1" })
        );
    }

    /// The wire form round-trips back into the typed enum.
    #[test]
    fn round_trips_through_json() {
        let block = CanonicalContentBlock::ToolUse {
            id: "t1".to_string(),
            name: "Bash".to_string(),
            input: json!({ "command": "ls" }),
        };
        let value = serde_json::to_value(&block).unwrap();
        let back: CanonicalContentBlock = serde_json::from_value(value).unwrap();
        assert_eq!(block, back);
    }
}
