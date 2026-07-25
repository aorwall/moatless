//! Native Codex thread item → canonical Anthropic-shaped content block(s).
//!
//! Codex reports native item families (`agent_message`, `command_execution`, …)
//! whose type strings the backend assembler and UI do not understand, so a
//! forwarded native block renders as nothing. This maps each family into the
//! same `text` / `tool_use` / `tool_result` blocks the legacy actor path emits,
//! so a Codex turn renders identically through the harness. Tool families expand
//! to a `tool_use` block plus, when the item carries output, a paired
//! `tool_result` block carrying the typed Codex result for backend enrichment.
//!
//! The block-shaping itself is shared with the legacy path via the pure
//! `handle::codex_payloads` builders — this module only wires those shapes into
//! canonical blocks with the right tool name, role, and shared tool-use id.
//!
//! Blocks are built through the same [`claude_agent_sdk::ContentBlock`] enum the
//! Claude harness normalizes into, so both agents converge on one type-checked
//! Anthropic-shaped wire format rather than parallel hand-rolled JSON.

use claude_agent_sdk::ContentBlock;
use serde_json::{json, Value};

use moatless_api_schemas::codex::{
    CodexWebSearchInput, CODEX_TOOL_COLLAB, CODEX_TOOL_EDIT, CODEX_TOOL_MCP, CODEX_TOOL_SHELL,
    CODEX_TOOL_TODO_LIST, CODEX_TOOL_WEB_SEARCH,
};

use crate::agent::codex_exec::ThreadItemDetails;
use crate::agent::handle::codex_payloads::{
    build_codex_todo_list_input, build_collab_tool_result, build_collab_tool_use,
    build_edit_tool_result, build_edit_tool_use, build_mcp_tool_result, build_mcp_tool_use,
    build_shell_tool_result, build_shell_tool_use,
};

use super::super::source::Role;

/// One canonical content block derived from a native Codex item.
pub(super) struct CanonicalBlock {
    pub role: Role,
    /// Suffix distinguishing this block's Message identity from the item's
    /// qualified id — empty for the primary block, `_result` for a paired
    /// tool_result so it lands in its own user Message row (mirroring the legacy
    /// `{id}` / `{id}_result` uuid split).
    pub id_suffix: &'static str,
    pub block: Value,
    /// Typed Codex result, surfaced at the payload top level (`toolUseResult`)
    /// for backend enrichment. `None` for every non-result block.
    pub tool_use_result: Option<Value>,
}

/// Serialize a canonical [`ContentBlock`] to its Anthropic-shaped JSON. The
/// serialization is infallible for these variants; `Null` is a defensive floor
/// mirroring the Claude normalizer.
fn to_value(block: ContentBlock) -> Value {
    serde_json::to_value(block).unwrap_or(Value::Null)
}

fn text(role: Role, text: &str) -> CanonicalBlock {
    CanonicalBlock {
        role,
        id_suffix: "",
        block: to_value(ContentBlock::Text {
            text: text.to_string(),
        }),
        tool_use_result: None,
    }
}

fn tool_use(tool_use_id: &str, name: &str, input: Value) -> CanonicalBlock {
    CanonicalBlock {
        role: Role::Assistant,
        id_suffix: "",
        block: to_value(ContentBlock::ToolUse {
            id: tool_use_id.to_string(),
            name: name.to_string(),
            input,
        }),
        tool_use_result: None,
    }
}

fn tool_result(tool_use_id: &str, content: String, is_error: bool, typed: Value) -> CanonicalBlock {
    CanonicalBlock {
        role: Role::User,
        id_suffix: "_result",
        block: to_value(ContentBlock::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: Some(Value::String(content)),
            is_error: Some(is_error),
        }),
        tool_use_result: Some(typed),
    }
}

/// A tool_use block and its paired tool_result, sharing `tool_use_id`.
fn tool_call(
    tool_use_id: &str,
    name: &str,
    input: Value,
    result: (Value, String, bool),
) -> Vec<CanonicalBlock> {
    let (typed, summary, is_error) = result;
    vec![
        tool_use(tool_use_id, name, input),
        tool_result(tool_use_id, summary, is_error, typed),
    ]
}

/// Map one native item's typed details into canonical blocks. `tool_use_id` is
/// the item's qualified id, shared by a tool_use and its paired tool_result.
/// Returns `None` for an unknown (`Other`) kind, which the caller forwards as its
/// raw native block with bounded diagnostics instead.
pub(super) fn item_blocks(
    details: &ThreadItemDetails,
    tool_use_id: &str,
) -> Option<Vec<CanonicalBlock>> {
    let blocks = match details {
        // Assistant prose, model reasoning, and non-fatal item errors all render
        // as assistant text (matching the legacy path; the turn is not failed).
        ThreadItemDetails::AgentMessage(m) => vec![text(Role::Assistant, &m.text)],
        ThreadItemDetails::Reasoning(r) => vec![text(Role::Assistant, &r.text)],
        ThreadItemDetails::Error(e) => vec![text(Role::Assistant, &e.message)],
        ThreadItemDetails::CommandExecution(cmd) => tool_call(
            tool_use_id,
            CODEX_TOOL_SHELL,
            build_shell_tool_use(cmd),
            build_shell_tool_result(cmd),
        ),
        ThreadItemDetails::FileChange(edit) => tool_call(
            tool_use_id,
            CODEX_TOOL_EDIT,
            build_edit_tool_use(edit),
            build_edit_tool_result(edit),
        ),
        ThreadItemDetails::McpToolCall(mcp) => tool_call(
            tool_use_id,
            CODEX_TOOL_MCP,
            build_mcp_tool_use(mcp),
            build_mcp_tool_result(mcp),
        ),
        ThreadItemDetails::CollabToolCall(collab) => tool_call(
            tool_use_id,
            CODEX_TOOL_COLLAB,
            build_collab_tool_use(collab),
            build_collab_tool_result(collab),
        ),
        // Web search and the running plan tracker have no separate result concept,
        // so each is a lone tool_use (todo_list re-emits on start/update/complete
        // under a stable id so the backend upserts one live card).
        ThreadItemDetails::WebSearch(web) => {
            let input = serde_json::to_value(CodexWebSearchInput {
                query: web.query.clone(),
                action: web.action.clone(),
            })
            .unwrap_or_else(|_| json!({}));
            vec![tool_use(tool_use_id, CODEX_TOOL_WEB_SEARCH, input)]
        }
        ThreadItemDetails::TodoList(todo) => {
            let input = serde_json::to_value(build_codex_todo_list_input(todo))
                .unwrap_or_else(|_| json!({}));
            vec![tool_use(tool_use_id, CODEX_TOOL_TODO_LIST, input)]
        }
        ThreadItemDetails::Other { .. } => return None,
    };
    Some(blocks)
}
