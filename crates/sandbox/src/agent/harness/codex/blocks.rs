//! Native Codex thread item → canonical Anthropic-shaped content block(s).
//!
//! Each family maps into the shared [`CanonicalContentBlock`] enum, which the
//! Claude harness also normalizes into. Tool families expand to a `tool_use`
//! plus, when the item carries output, a paired `tool_result` under one id.

use crate::agent::harness::content::CanonicalContentBlock;
use serde_json::{Value, json};

use moatless_types::codex::{
    CODEX_TOOL_COLLAB, CODEX_TOOL_EDIT, CODEX_TOOL_MCP, CODEX_TOOL_SHELL, CODEX_TOOL_TODO_LIST,
    CODEX_TOOL_WEB_SEARCH, CodexWebSearchInput,
};

use crate::agent::codex_app_server::protocol::{ReasoningItem, ThreadItem, TurnPlanStep};
use crate::agent::harness::codex::payloads::{
    build_codex_todo_list_input, build_collab_tool_result, build_collab_tool_use,
    build_dynamic_tool_result, build_dynamic_tool_use, build_edit_tool_result, build_edit_tool_use,
    build_mcp_tool_result, build_mcp_tool_use, build_shell_tool_result, build_shell_tool_use,
};

fn text(text: &str) -> CanonicalContentBlock {
    CanonicalContentBlock::Text {
        text: text.to_string(),
    }
}

fn tool_use(tool_use_id: &str, name: &str, input: Value) -> CanonicalContentBlock {
    CanonicalContentBlock::ToolUse {
        id: tool_use_id.to_string(),
        name: name.to_string(),
        input,
    }
}

fn tool_result(tool_use_id: &str, content: String, is_error: bool) -> CanonicalContentBlock {
    CanonicalContentBlock::ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content: Some(Value::String(content)),
        is_error: Some(is_error),
    }
}

/// A `tool_use` block and its paired `tool_result`, sharing `tool_use_id`.
fn tool_call(
    tool_use_id: &str,
    name: &str,
    input: Value,
    result: (Value, String, bool),
) -> Vec<CanonicalContentBlock> {
    let (_typed, summary, is_error) = result;
    vec![
        tool_use(tool_use_id, name, input),
        tool_result(tool_use_id, summary, is_error),
    ]
}

/// The reasoning text, from whichever of the item's two lists carries it.
///
/// `summary` is redacted and `content` is the raw chain of thought; observed
/// items always populate `summary`, but both are read since an item with
/// neither is normal and must yield no block.
fn reasoning_text(item: &ReasoningItem) -> Option<String> {
    let parts = if item.summary.is_empty() {
        &item.content
    } else {
        &item.summary
    };
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("\n\n"))
}

/// The running plan tracker, which arrives as the `turn/plan/updated`
/// notification rather than as an item. Re-emitted under a stable id on every
/// update so the backend upserts one live card, exactly as `todo_list` did.
pub(super) fn plan_blocks(plan: &[TurnPlanStep], tool_use_id: &str) -> Vec<CanonicalContentBlock> {
    let input =
        serde_json::to_value(build_codex_todo_list_input(plan)).unwrap_or_else(|_| json!({}));
    vec![tool_use(tool_use_id, CODEX_TOOL_TODO_LIST, input)]
}

/// Map one native item into canonical blocks. `tool_use_id` is the item's
/// qualified id, shared by a `tool_use` and its paired `tool_result`.
///
/// Returns `None` for an unknown family (forwarded raw with bounded
/// diagnostics) or an empty list when the family is known and deliberately
/// renders nothing.
pub(super) fn item_blocks(
    item: &ThreadItem,
    tool_use_id: &str,
) -> Option<Vec<CanonicalContentBlock>> {
    let blocks = match item {
        // Assistant text and model reasoning both render as assistant text.
        // There is no `error` item on this transport — a failed Turn reports its
        // error on the terminal instead.
        ThreadItem::AgentMessage(m) => vec![text(&m.text)],
        ThreadItem::Reasoning(r) => match reasoning_text(r) {
            Some(reasoning) => vec![text(&reasoning)],
            None => vec![],
        },
        ThreadItem::CommandExecution(cmd) => tool_call(
            tool_use_id,
            CODEX_TOOL_SHELL,
            build_shell_tool_use(cmd),
            build_shell_tool_result(cmd),
        ),
        ThreadItem::FileChange(edit) => tool_call(
            tool_use_id,
            CODEX_TOOL_EDIT,
            build_edit_tool_use(edit),
            build_edit_tool_result(edit),
        ),
        ThreadItem::McpToolCall(mcp) => tool_call(
            tool_use_id,
            CODEX_TOOL_MCP,
            build_mcp_tool_use(mcp),
            build_mcp_tool_result(mcp),
        ),
        // A namespaced tool with JSON arguments and content items back is what
        // `codex.mcp_tool_call` already renders, so it reuses that card rather
        // than being invisible — the one new family with an honest home.
        ThreadItem::DynamicToolCall(dynamic) => tool_call(
            tool_use_id,
            CODEX_TOOL_MCP,
            build_dynamic_tool_use(dynamic),
            build_dynamic_tool_result(dynamic),
        ),
        ThreadItem::CollabAgentToolCall(collab) => tool_call(
            tool_use_id,
            CODEX_TOOL_COLLAB,
            build_collab_tool_use(collab),
            build_collab_tool_result(collab),
        ),
        // Web search has no separate result concept, so it is a lone tool_use.
        ThreadItem::WebSearch(web) => {
            let input = serde_json::to_value(CodexWebSearchInput {
                query: web.query.clone(),
                action: web.action.clone(),
            })
            .unwrap_or_else(|_| json!({}));
            vec![tool_use(tool_use_id, CODEX_TOOL_WEB_SEARCH, input)]
        }

        // ── Deliberately dropped ────────────────────────────────────────
        //
        // The Turn's own prompt is echoed back as a `userMessage`, and so is a
        // steered one. Both are already in the conversation as the Message that
        // produced them, so emitting a block would show the text twice.
        ThreadItem::UserMessage(_)
        // Prompt material a Codex hook injected, not agent output. Rendering it
        // as assistant text would attribute someone else's words to the agent.
        | ThreadItem::HookPrompt(_)
        // A written plan, distinct from the running plan tracker (which is the
        // `turn/plan/updated` notification). No probe has produced one, and its
        // single `text` field would be indistinguishable from an agent message.
        | ThreadItem::Plan(_)
        // Status notices with no content to render: a sub-agent lifecycle tick,
        // a file the agent looked at, a wait, a generated image, review-mode
        // boundaries the sandbox never enters, and an automatic compaction.
        // Each would need a canonical block type that does not exist yet.
        | ThreadItem::SubAgentActivity(_)
        | ThreadItem::ImageView(_)
        | ThreadItem::Sleep(_)
        | ThreadItem::ImageGeneration(_)
        | ThreadItem::EnteredReviewMode(_)
        | ThreadItem::ExitedReviewMode(_)
        | ThreadItem::ContextCompaction(_) => vec![],

        ThreadItem::Other { .. } => return None,
    };
    Some(blocks)
}
