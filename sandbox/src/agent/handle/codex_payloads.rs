//! Pure builders that map Codex CLI thread items to event JSON payloads
//! (assistant `tool_use` / user `tool_result` envelopes) and publish them.

use std::collections::HashMap;

use moatless_events::AgentSubType;

use moatless_api_schemas::codex::{
    CodexCollabAgentState, CodexCollabAgentStatus, CodexCollabInput, CodexCollabResult,
    CodexCollabTool, CodexEditInput, CodexEditResult, CodexMcpInput, CodexMcpResult,
    CodexShellInput, CodexShellResult, CodexTodoItem, CodexTodoListInput, CodexToolStatus,
    CodexWebSearchInput, FileUpdateChange, FileUpdateKind, CODEX_TOOL_COLLAB, CODEX_TOOL_EDIT,
    CODEX_TOOL_MCP, CODEX_TOOL_SHELL, CODEX_TOOL_TODO_LIST, CODEX_TOOL_WEB_SEARCH,
};

use crate::agent::codex_exec::{
    self, CollabAgentStatus as CodexExecCollabAgentStatus, CollabTool as CodexExecCollabTool,
    CollabToolCallItem, CollabToolCallStatus, CommandExecutionItem, CommandExecutionStatus,
    FileChangeItem, McpToolCallItem, McpToolCallStatus, PatchApplyStatus, TodoListItem, Usage,
    WebSearchItem,
};
use crate::events::GrpcEventPublisher;

pub(super) fn codex_message_payload(
    uuid: &str,
    session_id: &str,
    text: &str,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{ "type": "text", "text": text }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
    })
}

/// Assistant text payload for a codex reasoning item, tagged so the UI
/// can render it collapsed.
pub(super) fn codex_reasoning_payload(
    uuid: &str,
    session_id: &str,
    text: &str,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{ "type": "text", "text": text }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
        "sdk_metadata": { "codex_reasoning": true },
    })
}

/// Build an assistant message envelope containing a single `tool_use` block.
fn codex_tool_use_payload(
    uuid: &str,
    session_id: &str,
    tool_use_id: &str,
    tool_name: &str,
    tool_input: serde_json::Value,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{
            "type": "tool_use",
            "id": tool_use_id,
            "name": tool_name,
            "input": tool_input,
        }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
    })
}

/// Build a user message envelope containing a single `tool_result` block
/// referencing a prior `tool_use`. The `tool_use_result` field carries the
/// typed codex result for backend enrichment.
fn codex_tool_result_payload(
    uuid: &str,
    session_id: &str,
    tool_use_id: &str,
    content: &str,
    is_error: bool,
    tool_use_result: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }],
        },
        "tool_use_result": tool_use_result,
    })
}

/// Map a codex shell status + exit code to the shared typed status enum.
///
/// A `Completed` status with a non-zero exit code is surfaced as `Failed` so
/// the UI can flag it — codex itself only flips to `Failed` for catastrophic
/// failures, not for commands that ran successfully but returned non-zero.
fn command_execution_to_tool_status(
    status: CommandExecutionStatus,
    exit_code: Option<i32>,
) -> CodexToolStatus {
    match status {
        CommandExecutionStatus::InProgress => CodexToolStatus::InProgress,
        CommandExecutionStatus::Completed => match exit_code {
            Some(code) if code != 0 => CodexToolStatus::Failed,
            _ => CodexToolStatus::Completed,
        },
        CommandExecutionStatus::Failed => CodexToolStatus::Failed,
        CommandExecutionStatus::Declined => CodexToolStatus::Declined,
        // Forward-compat fallback: unknown status → treat as completed so the
        // turn is not stalled; the aggregated output will still render.
        CommandExecutionStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn patch_apply_to_tool_status(status: PatchApplyStatus) -> CodexToolStatus {
    match status {
        PatchApplyStatus::InProgress => CodexToolStatus::InProgress,
        PatchApplyStatus::Completed => CodexToolStatus::Completed,
        PatchApplyStatus::Failed => CodexToolStatus::Failed,
        PatchApplyStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn mcp_to_tool_status(status: McpToolCallStatus) -> CodexToolStatus {
    match status {
        McpToolCallStatus::InProgress => CodexToolStatus::InProgress,
        McpToolCallStatus::Completed => CodexToolStatus::Completed,
        McpToolCallStatus::Failed => CodexToolStatus::Failed,
        McpToolCallStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn collab_to_tool_status(status: CollabToolCallStatus) -> CodexToolStatus {
    match status {
        CollabToolCallStatus::InProgress => CodexToolStatus::InProgress,
        CollabToolCallStatus::Completed => CodexToolStatus::Completed,
        CollabToolCallStatus::Failed => CodexToolStatus::Failed,
        CollabToolCallStatus::Unknown => CodexToolStatus::Completed,
    }
}

fn map_collab_tool(tool: CodexExecCollabTool) -> CodexCollabTool {
    match tool {
        CodexExecCollabTool::SpawnAgent => CodexCollabTool::SpawnAgent,
        CodexExecCollabTool::SendInput => CodexCollabTool::SendInput,
        CodexExecCollabTool::Wait => CodexCollabTool::Wait,
        CodexExecCollabTool::CloseAgent => CodexCollabTool::CloseAgent,
        CodexExecCollabTool::Unknown => CodexCollabTool::Unknown,
    }
}

fn map_collab_agent_status(status: CodexExecCollabAgentStatus) -> CodexCollabAgentStatus {
    match status {
        CodexExecCollabAgentStatus::PendingInit => CodexCollabAgentStatus::PendingInit,
        CodexExecCollabAgentStatus::Running => CodexCollabAgentStatus::Running,
        CodexExecCollabAgentStatus::Interrupted => CodexCollabAgentStatus::Interrupted,
        CodexExecCollabAgentStatus::Completed => CodexCollabAgentStatus::Completed,
        CodexExecCollabAgentStatus::Errored => CodexCollabAgentStatus::Errored,
        CodexExecCollabAgentStatus::Shutdown => CodexCollabAgentStatus::Shutdown,
        CodexExecCollabAgentStatus::NotFound => CodexCollabAgentStatus::NotFound,
        CodexExecCollabAgentStatus::Unknown => CodexCollabAgentStatus::Unknown,
    }
}

fn map_collab_agents_states(
    states: &HashMap<String, codex_exec::CollabAgentState>,
) -> std::collections::BTreeMap<String, CodexCollabAgentState> {
    states
        .iter()
        .map(|(id, state)| {
            (
                id.clone(),
                CodexCollabAgentState {
                    status: map_collab_agent_status(state.status),
                    message: state.message.clone(),
                },
            )
        })
        .collect()
}

fn parse_file_update_kind(raw: &str) -> FileUpdateKind {
    match raw {
        "delete" => FileUpdateKind::Delete,
        "update" => FileUpdateKind::Update,
        // Treat unknown kinds as `add` — the worst case is a misleading
        // badge rather than dropping the change.
        _ => FileUpdateKind::Add,
    }
}

fn build_shell_tool_use(cmd: &CommandExecutionItem) -> serde_json::Value {
    let input = CodexShellInput {
        command: cmd.command.clone(),
        cwd: cmd.cwd.clone(),
        command_actions: vec![],
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_shell_tool_result(cmd: &CommandExecutionItem) -> (serde_json::Value, String, bool) {
    let status = command_execution_to_tool_status(cmd.status, cmd.exit_code);
    let is_error = matches!(status, CodexToolStatus::Failed | CodexToolStatus::Declined);
    let summary_text = cmd.aggregated_output.clone().unwrap_or_default();

    let result = CodexShellResult {
        status,
        exit_code: cmd.exit_code,
        aggregated_output: cmd.aggregated_output.clone(),
        duration_ms: cmd.duration_ms,
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

fn build_edit_tool_use(edit: &FileChangeItem) -> serde_json::Value {
    let input = CodexEditInput {
        changes: edit
            .changes
            .iter()
            .map(|c| FileUpdateChange {
                path: c.path.clone(),
                kind: parse_file_update_kind(&c.kind),
                diff: c.diff.clone(),
            })
            .collect(),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_edit_tool_result(edit: &FileChangeItem) -> (serde_json::Value, String, bool) {
    let status = patch_apply_to_tool_status(edit.status);
    let is_error = matches!(status, CodexToolStatus::Failed | CodexToolStatus::Declined);

    let summary_text = if edit.changes.is_empty() {
        String::new()
    } else {
        edit.changes
            .iter()
            .map(|c| format!("{} {}", c.kind, c.path))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let result = CodexEditResult {
        status,
        changes: edit
            .changes
            .iter()
            .map(|c| FileUpdateChange {
                path: c.path.clone(),
                kind: parse_file_update_kind(&c.kind),
                diff: c.diff.clone(),
            })
            .collect(),
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

pub(super) async fn publish_codex_shell_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    spawn_nonce: &str,
    item_id: &str,
    cmd: &CommandExecutionItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{spawn_nonce}_{item_id}");
    let assistant_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_call");
    let tool_input = build_shell_tool_use(cmd);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_SHELL,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_shell_tool_result(cmd);
    let result_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

pub(super) async fn publish_codex_edit_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    spawn_nonce: &str,
    item_id: &str,
    edit: &FileChangeItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{spawn_nonce}_{item_id}");
    let assistant_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_call");
    let tool_input = build_edit_tool_use(edit);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_EDIT,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_edit_tool_result(edit);
    let result_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

fn build_codex_todo_list_input(todo: &TodoListItem) -> CodexTodoListInput {
    CodexTodoListInput {
        items: todo
            .items
            .iter()
            .map(|entry| CodexTodoItem {
                text: entry.text.clone(),
                completed: entry.completed,
            })
            .collect(),
    }
}

/// Publish a codex `todo_list` item as a tool_use block. Unlike shell/edit,
/// this is called on every `item.started` / `item.updated` / `item.completed`
/// for the same `item_id` — the stable `assistant_uuid`
/// (session + spawn_nonce + item_id) causes the backend DAO to upsert in
/// place, so the UI sees one card that mutates live as codex flips items to
/// done. No `tool_result` is emitted; codex's todo_list has no separate
/// result concept.
pub(super) async fn publish_codex_todo_list_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    spawn_nonce: &str,
    item_id: &str,
    todo: &TodoListItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{spawn_nonce}_{item_id}");
    let assistant_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_call");
    let input = build_codex_todo_list_input(todo);
    let tool_input = serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}));
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_TODO_LIST,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;
}

fn build_mcp_tool_use(item: &McpToolCallItem) -> serde_json::Value {
    let input = CodexMcpInput {
        server: item.server.clone(),
        tool: item.tool.clone(),
        arguments: item.arguments.clone(),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_mcp_tool_result(item: &McpToolCallItem) -> (serde_json::Value, String, bool) {
    let status = mcp_to_tool_status(item.status);
    let is_error = matches!(status, CodexToolStatus::Failed) || item.error.is_some();

    let (content, structured_content) = match &item.result {
        Some(result) => (
            Some(result.content.clone()),
            result.structured_content.clone(),
        ),
        None => (None, None),
    };
    let error_message = item.error.as_ref().map(|e| e.message.clone());

    let summary_text = if let Some(ref msg) = error_message {
        msg.clone()
    } else if let Some(ref content) = content {
        content
            .iter()
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    let result = CodexMcpResult {
        status,
        content,
        structured_content,
        error_message,
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

pub(super) async fn publish_codex_mcp_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    spawn_nonce: &str,
    item_id: &str,
    mcp: &McpToolCallItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{spawn_nonce}_{item_id}");
    let assistant_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_call");
    let tool_input = build_mcp_tool_use(mcp);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_MCP,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_mcp_tool_result(mcp);
    let result_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

fn build_collab_tool_use(item: &CollabToolCallItem) -> serde_json::Value {
    let input = CodexCollabInput {
        tool: map_collab_tool(item.tool),
        sender_thread_id: item.sender_thread_id.clone(),
        receiver_thread_ids: item.receiver_thread_ids.clone(),
        prompt: item.prompt.clone(),
        agents_states: map_collab_agents_states(&item.agents_states),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

fn build_collab_tool_result(item: &CollabToolCallItem) -> (serde_json::Value, String, bool) {
    let status = collab_to_tool_status(item.status);
    let agents_states = map_collab_agents_states(&item.agents_states);
    let any_errored = agents_states
        .values()
        .any(|state| state.status == CodexCollabAgentStatus::Errored);
    let is_error = matches!(status, CodexToolStatus::Failed) || any_errored;

    let summary_text = agents_states
        .iter()
        .map(|(id, state)| {
            let status_str = serde_json::to_value(&state.status)
                .ok()
                .and_then(|v| v.as_str().map(std::string::ToString::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            format!("{id}: {status_str}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let result = CodexCollabResult {
        status,
        agents_states,
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

pub(super) async fn publish_codex_collab_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    spawn_nonce: &str,
    item_id: &str,
    collab: &CollabToolCallItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{spawn_nonce}_{item_id}");
    let assistant_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_call");
    let tool_input = build_collab_tool_use(collab);
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_COLLAB,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;

    let (result_value, summary_text, is_error) = build_collab_tool_result(collab);
    let result_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_result");
    let result_payload = codex_tool_result_payload(
        &result_uuid,
        session_id,
        &tool_use_id,
        &summary_text,
        is_error,
        result_value,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, result_payload)
        .await;
}

/// Publish a `web_search` item as a single tool_use block — no result is
/// emitted because upstream has no status or payload on completion.
pub(super) async fn publish_codex_web_search_call(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    spawn_nonce: &str,
    item_id: &str,
    web: &WebSearchItem,
    requested_model: Option<&str>,
) {
    let tool_use_id = format!("{session_id}_{spawn_nonce}_{item_id}");
    let assistant_uuid = format!("{session_id}_{spawn_nonce}_{item_id}_call");
    let input = CodexWebSearchInput {
        query: web.query.clone(),
        action: web.action.clone(),
    };
    let tool_input = serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}));
    let use_payload = codex_tool_use_payload(
        &assistant_uuid,
        session_id,
        &tool_use_id,
        CODEX_TOOL_WEB_SEARCH,
        tool_input,
        requested_model,
    );
    events
        .publish_agent_event(task_id, AgentSubType::Message, use_payload)
        .await;
}

/// Publish a non-fatal codex `error` item as an assistant text message,
/// flagged via `sdk_metadata.codex_error` so the UI can render it distinctly.
/// Note: this intentionally does NOT touch `agent_state.set_error()` — the
/// turn is still running; only the item itself reports a non-fatal error.
pub(super) async fn publish_codex_error_item(
    events: &GrpcEventPublisher,
    task_id: &str,
    session_id: &str,
    spawn_nonce: &str,
    item_id: &str,
    text: &str,
    requested_model: Option<&str>,
) {
    let uuid = format!("{session_id}_{spawn_nonce}_{item_id}");
    let payload = codex_error_message_payload(&uuid, session_id, text, requested_model);
    events
        .publish_agent_event(task_id, AgentSubType::Message, payload)
        .await;
}

/// Assistant text payload for a non-fatal codex error item.
fn codex_error_message_payload(
    uuid: &str,
    session_id: &str,
    text: &str,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), serde_json::json!("assistant"));
    message.insert(
        "content".to_string(),
        serde_json::json!([{ "type": "text", "text": text }]),
    );
    if let Some(model) = requested_model {
        message.insert("model".to_string(), serde_json::json!(model));
    }

    serde_json::json!({
        "uuid": uuid,
        "session_id": session_id,
        "message": serde_json::Value::Object(message),
        "sdk_metadata": { "codex_error": true },
    })
}

pub(super) fn codex_result_payload(
    session_id: Option<&str>,
    is_error: bool,
    subtype: &str,
    result: String,
    usage: Option<&Usage>,
    requested_model: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    if let Some(session_id) = session_id.filter(|session_id| !session_id.is_empty()) {
        payload.insert("session_id".to_string(), serde_json::json!(session_id));
    }
    payload.insert("is_error".to_string(), serde_json::json!(is_error));
    payload.insert("subtype".to_string(), serde_json::json!(subtype));
    payload.insert("result".to_string(), serde_json::json!(result.clone()));
    // Mirror the Claude `extract_result_info` shape: when `is_error`, expose the
    // text under `error` so `handle_agent_error` can read it without knowing
    // SDK-specific layout (it falls back to "Unknown agent error" otherwise).
    if is_error {
        payload.insert("error".to_string(), serde_json::json!(result));
    }
    payload.insert("total_cost_usd".to_string(), serde_json::json!(0.0));
    payload.insert("num_turns".to_string(), serde_json::json!(1));
    if let Some(model) = requested_model {
        payload.insert("model".to_string(), serde_json::json!(model));
    }
    if let Some(usage) = usage {
        payload.insert(
            "usage".to_string(),
            serde_json::json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cached_input_tokens,
                "cache_creation_input_tokens": serde_json::Value::Null,
                "reasoning_output_tokens": usage.reasoning_output_tokens,
            }),
        );
    }

    serde_json::Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_tool_use_payload_shapes_assistant_block() {
        let input = serde_json::json!({ "command": "echo hi" });
        let payload = codex_tool_use_payload(
            "sess_NONCE_item_1_call",
            "sess",
            "sess_NONCE_item_1",
            "codex.shell",
            input,
            Some("gpt-5"),
        );

        assert_eq!(payload["uuid"], "sess_NONCE_item_1_call");
        assert_eq!(payload["session_id"], "sess");
        assert_eq!(payload["message"]["role"], "assistant");
        assert_eq!(payload["message"]["model"], "gpt-5");

        let block = &payload["message"]["content"][0];
        assert_eq!(block["type"], "tool_use");
        assert_eq!(block["id"], "sess_NONCE_item_1");
        assert_eq!(block["name"], "codex.shell");
        assert_eq!(block["input"]["command"], "echo hi");
    }

    #[test]
    fn codex_tool_result_payload_carries_typed_result() {
        let typed = serde_json::json!({ "status": "completed", "exitCode": 0 });
        let payload = codex_tool_result_payload(
            "sess_NONCE_item_1_result",
            "sess",
            "sess_NONCE_item_1",
            "hi\n",
            false,
            typed,
        );

        assert_eq!(payload["uuid"], "sess_NONCE_item_1_result");
        assert_eq!(payload["message"]["role"], "user");
        let block = &payload["message"]["content"][0];
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["tool_use_id"], "sess_NONCE_item_1");
        assert_eq!(block["content"], "hi\n");
        assert_eq!(block["is_error"], false);
        assert_eq!(payload["tool_use_result"]["exitCode"], 0);
    }

    #[test]
    fn command_execution_status_nonzero_exit_is_failed() {
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Completed, Some(1)),
            CodexToolStatus::Failed
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Completed, Some(0)),
            CodexToolStatus::Completed
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Failed, Some(1)),
            CodexToolStatus::Failed
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::InProgress, None),
            CodexToolStatus::InProgress
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Declined, None),
            CodexToolStatus::Declined
        );
        assert_eq!(
            command_execution_to_tool_status(CommandExecutionStatus::Unknown, None),
            CodexToolStatus::Completed
        );
    }

    #[test]
    fn build_codex_todo_list_input_preserves_items() {
        use crate::agent::codex_exec::TodoListEntry;
        let todo = TodoListItem {
            items: vec![
                TodoListEntry {
                    text: "first".to_string(),
                    completed: true,
                },
                TodoListEntry {
                    text: "second".to_string(),
                    completed: false,
                },
            ],
        };
        let input = build_codex_todo_list_input(&todo);
        assert_eq!(input.items.len(), 2);
        assert_eq!(input.items[0].text, "first");
        assert!(input.items[0].completed);
        assert_eq!(input.items[1].text, "second");
        assert!(!input.items[1].completed);
    }

    #[test]
    fn codex_todo_list_payload_uses_stable_uuid_per_item_id() {
        // Simulate two emissions for the same codex item_id: first a started
        // snapshot with all false, then an updated snapshot with one true.
        // Both should produce the same uuid so the backend upserts in place.
        let initial = serde_json::to_value(build_codex_todo_list_input(&TodoListItem {
            items: vec![
                codex_exec::TodoListEntry {
                    text: "a".to_string(),
                    completed: false,
                },
                codex_exec::TodoListEntry {
                    text: "b".to_string(),
                    completed: false,
                },
            ],
        }))
        .unwrap();
        let updated = serde_json::to_value(build_codex_todo_list_input(&TodoListItem {
            items: vec![
                codex_exec::TodoListEntry {
                    text: "a".to_string(),
                    completed: true,
                },
                codex_exec::TodoListEntry {
                    text: "b".to_string(),
                    completed: false,
                },
            ],
        }))
        .unwrap();

        let p1 = codex_tool_use_payload(
            &format!("{}_{}_{}_call", "sess", "NONCE", "item_1"),
            "sess",
            &format!("{}_{}_{}", "sess", "NONCE", "item_1"),
            CODEX_TOOL_TODO_LIST,
            initial,
            Some("gpt-5"),
        );
        let p2 = codex_tool_use_payload(
            &format!("{}_{}_{}_call", "sess", "NONCE", "item_1"),
            "sess",
            &format!("{}_{}_{}", "sess", "NONCE", "item_1"),
            CODEX_TOOL_TODO_LIST,
            updated,
            Some("gpt-5"),
        );

        assert_eq!(p1["uuid"], p2["uuid"]);
        assert_eq!(p1["uuid"], "sess_NONCE_item_1_call");
        assert_eq!(p1["message"]["content"][0]["name"], CODEX_TOOL_TODO_LIST);
        // Payloads differ only in the tool_input items array.
        assert_eq!(
            p1["message"]["content"][0]["input"]["items"][0]["completed"],
            false
        );
        assert_eq!(
            p2["message"]["content"][0]["input"]["items"][0]["completed"],
            true
        );
    }

    #[test]
    fn build_shell_tool_result_marks_error_on_nonzero_exit() {
        let cmd = CommandExecutionItem {
            command: "false".to_string(),
            cwd: None,
            aggregated_output: Some(String::new()),
            exit_code: Some(1),
            status: CommandExecutionStatus::Failed,
            duration_ms: None,
        };
        let (value, _, is_error) = build_shell_tool_result(&cmd);
        assert!(is_error);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["exitCode"], 1);
    }

    #[test]
    fn build_mcp_tool_result_marks_error_when_error_present() {
        let item = McpToolCallItem {
            server: "db".to_string(),
            tool: "query".to_string(),
            arguments: serde_json::json!({}),
            result: None,
            error: Some(codex_exec::McpToolCallItemError {
                message: "connection refused".to_string(),
            }),
            status: McpToolCallStatus::Failed,
        };
        let (value, summary, is_error) = build_mcp_tool_result(&item);
        assert!(is_error);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["errorMessage"], "connection refused");
        assert_eq!(summary, "connection refused");
    }

    #[test]
    fn build_mcp_tool_result_ok_when_status_completed() {
        let item = McpToolCallItem {
            server: "atlassian".to_string(),
            tool: "jira_search".to_string(),
            arguments: serde_json::json!({}),
            result: Some(codex_exec::McpToolCallItemResult {
                content: vec![serde_json::json!({"type":"text","text":"hit"})],
                structured_content: None,
            }),
            error: None,
            status: McpToolCallStatus::Completed,
        };
        let (value, summary, is_error) = build_mcp_tool_result(&item);
        assert!(!is_error);
        assert_eq!(value["status"], "completed");
        assert_eq!(summary, "hit");
    }

    #[test]
    fn build_collab_tool_result_marks_error_on_errored_agent() {
        let mut agents = HashMap::new();
        agents.insert(
            "sub_a".to_string(),
            codex_exec::CollabAgentState {
                status: CodexExecCollabAgentStatus::Errored,
                message: Some("crashed".to_string()),
            },
        );
        let item = CollabToolCallItem {
            tool: CodexExecCollabTool::SpawnAgent,
            sender_thread_id: "main".to_string(),
            receiver_thread_ids: vec!["sub_a".to_string()],
            prompt: None,
            agents_states: agents,
            status: CollabToolCallStatus::Completed,
        };
        let (_, _, is_error) = build_collab_tool_result(&item);
        assert!(is_error);
    }

    #[test]
    fn codex_error_message_payload_sets_metadata_flag() {
        let payload = codex_error_message_payload(
            "sess_item_8",
            "sess",
            "tool returned warning",
            Some("gpt-5"),
        );
        assert_eq!(payload["sdk_metadata"]["codex_error"], true);
        assert_eq!(
            payload["message"]["content"][0]["text"],
            "tool returned warning"
        );
        assert_eq!(payload["message"]["role"], "assistant");
    }

    #[test]
    fn codex_reasoning_payload_sets_metadata_flag() {
        let payload = codex_reasoning_payload("sess_item_0", "sess", "thinking...", Some("gpt-5"));
        assert_eq!(payload["sdk_metadata"]["codex_reasoning"], true);
        assert_eq!(payload["message"]["content"][0]["type"], "text");
        assert_eq!(payload["message"]["content"][0]["text"], "thinking...");
    }

    #[test]
    fn codex_result_payload_includes_usage_shape() {
        let payload = codex_result_payload(
            Some("thread-1"),
            false,
            "completed",
            "done".to_string(),
            Some(&Usage {
                input_tokens: 12,
                cached_input_tokens: 3,
                output_tokens: 4,
                reasoning_output_tokens: 7,
            }),
            Some("gpt-5"),
        );

        assert_eq!(payload["session_id"], "thread-1");
        assert_eq!(payload["is_error"], false);
        assert!(payload.get("error").is_none());
        assert_eq!(payload["usage"]["input_tokens"], 12);
        assert_eq!(payload["usage"]["cache_read_input_tokens"], 3);
        assert_eq!(payload["usage"]["reasoning_output_tokens"], 7);
        assert_eq!(payload["model"], "gpt-5");
    }

    #[test]
    fn codex_result_payload_injects_error_key_when_failed() {
        let payload = codex_result_payload(
            Some("thread-2"),
            true,
            "failed",
            "refresh_token_reused".to_string(),
            None,
            Some("gpt-5"),
        );

        assert_eq!(payload["is_error"], true);
        assert_eq!(payload["result"], "refresh_token_reused");
        // `error` mirrors `result` so backend `handle_agent_error` (which reads
        // payload["error"]) renders the failure text instead of "Unknown agent error".
        assert_eq!(payload["error"], "refresh_token_reused");
    }
}
