//! Pure builders that map Codex app-server thread items to typed tool
//! inputs/results.

use std::collections::HashMap;

use moatless_types::codex::{
    CodexCollabAgentState, CodexCollabAgentStatus, CodexCollabInput, CodexCollabResult,
    CodexCollabTool, CodexEditInput, CodexEditResult, CodexMcpInput, CodexMcpResult,
    CodexShellInput, CodexShellResult, CodexTodoItem, CodexTodoListInput, CodexTodoStatus,
    CodexToolStatus, FileUpdateChange, FileUpdateKind,
};

use crate::agent::codex_app_server::protocol::{
    CollabAgentState, CollabAgentStatus, CollabAgentTool, CollabAgentToolCallItem,
    CollabAgentToolCallStatus, CommandExecutionItem, CommandExecutionStatus, DynamicToolCallItem,
    FileChangeItem, McpToolCallItem, McpToolCallStatus, PatchApplyStatus, PatchChangeKind,
    TurnPlanStep, TurnPlanStepStatus,
};

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
        PatchApplyStatus::Declined => CodexToolStatus::Declined,
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

fn collab_to_tool_status(status: CollabAgentToolCallStatus) -> CodexToolStatus {
    match status {
        CollabAgentToolCallStatus::InProgress => CodexToolStatus::InProgress,
        CollabAgentToolCallStatus::Completed => CodexToolStatus::Completed,
        CollabAgentToolCallStatus::Failed => CodexToolStatus::Failed,
        CollabAgentToolCallStatus::Unknown => CodexToolStatus::Completed,
    }
}

/// `resumeAgent` has no canonical counterpart — the shared schema was written
/// against the four collab tools `codex exec` reported — so it lands in
/// `Unknown` rather than being mislabelled as one of them.
fn map_collab_tool(tool: CollabAgentTool) -> CodexCollabTool {
    match tool {
        CollabAgentTool::SpawnAgent => CodexCollabTool::SpawnAgent,
        CollabAgentTool::SendInput => CodexCollabTool::SendInput,
        CollabAgentTool::Wait => CodexCollabTool::Wait,
        CollabAgentTool::CloseAgent => CodexCollabTool::CloseAgent,
        CollabAgentTool::ResumeAgent | CollabAgentTool::Unknown => CodexCollabTool::Unknown,
    }
}

fn map_collab_agent_status(status: CollabAgentStatus) -> CodexCollabAgentStatus {
    match status {
        CollabAgentStatus::PendingInit => CodexCollabAgentStatus::PendingInit,
        CollabAgentStatus::Running => CodexCollabAgentStatus::Running,
        CollabAgentStatus::Interrupted => CodexCollabAgentStatus::Interrupted,
        CollabAgentStatus::Completed => CodexCollabAgentStatus::Completed,
        CollabAgentStatus::Errored => CodexCollabAgentStatus::Errored,
        CollabAgentStatus::Shutdown => CodexCollabAgentStatus::Shutdown,
        CollabAgentStatus::NotFound => CodexCollabAgentStatus::NotFound,
        CollabAgentStatus::Unknown => CodexCollabAgentStatus::Unknown,
    }
}

fn map_collab_agents_states(
    states: &HashMap<String, CollabAgentState>,
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

/// The app-server tags the kind internally (`{"type": "update", "move_path":
/// …}`) where `codex exec` sent a bare `"update"` string. `move_path` has no
/// home in the canonical change and is dropped.
fn map_file_update_kind(kind: &PatchChangeKind) -> FileUpdateKind {
    match kind {
        PatchChangeKind::Delete => FileUpdateKind::Delete,
        PatchChangeKind::Update { .. } => FileUpdateKind::Update,
        // Treat unknown kinds as `add` — the worst case is a misleading
        // badge rather than dropping the change.
        PatchChangeKind::Add | PatchChangeKind::Unknown => FileUpdateKind::Add,
    }
}

/// The wire name for a change kind, as it appeared in the `codex exec` summary
/// line (`"add /a.py"`), which this keeps byte-identical.
fn file_update_kind_label(kind: &PatchChangeKind) -> &'static str {
    match kind {
        PatchChangeKind::Add => "add",
        PatchChangeKind::Delete => "delete",
        PatchChangeKind::Update { .. } => "update",
        PatchChangeKind::Unknown => "unknown",
    }
}

fn map_file_changes(item: &FileChangeItem) -> Vec<FileUpdateChange> {
    item.changes
        .iter()
        .map(|c| FileUpdateChange {
            path: c.path.clone(),
            kind: map_file_update_kind(&c.kind),
            diff: Some(c.diff.clone()),
        })
        .collect()
}

pub(crate) fn build_shell_tool_use(cmd: &CommandExecutionItem) -> serde_json::Value {
    let input = CodexShellInput {
        command: cmd.command.clone(),
        cwd: Some(cmd.cwd.clone()),
        // The app-server also reports `commandActions`, which `codex exec` never
        // did and no canonical consumer reads; left empty so the rendered input
        // is the one the previous transport produced.
        command_actions: vec![],
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

pub(crate) fn build_shell_tool_result(
    cmd: &CommandExecutionItem,
) -> (serde_json::Value, String, bool) {
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

pub(crate) fn build_edit_tool_use(edit: &FileChangeItem) -> serde_json::Value {
    let input = CodexEditInput {
        changes: map_file_changes(edit),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

pub(crate) fn build_edit_tool_result(edit: &FileChangeItem) -> (serde_json::Value, String, bool) {
    let status = patch_apply_to_tool_status(edit.status);
    let is_error = matches!(status, CodexToolStatus::Failed | CodexToolStatus::Declined);

    let summary_text = edit
        .changes
        .iter()
        .map(|c| format!("{} {}", file_update_kind_label(&c.kind), c.path))
        .collect::<Vec<_>>()
        .join("\n");

    let result = CodexEditResult {
        status,
        changes: map_file_changes(edit),
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

/// The running plan tracker. On this transport it arrives as the
/// `turn/plan/updated` notification rather than a `todo_list` item. Each step
/// keeps its own three-state status alongside the done/not-done boolean, because
/// *in progress* is the state a plan panel exists to show.
pub(crate) fn build_codex_todo_list_input(plan: &[TurnPlanStep]) -> CodexTodoListInput {
    CodexTodoListInput {
        items: plan
            .iter()
            .map(|step| CodexTodoItem {
                text: step.step.clone(),
                completed: step.status == TurnPlanStepStatus::Completed,
                status: todo_status(step.status),
            })
            .collect(),
    }
}

/// An unrecognised status is pending: the step exists and is not claimed done.
fn todo_status(status: TurnPlanStepStatus) -> CodexTodoStatus {
    match status {
        TurnPlanStepStatus::Completed => CodexTodoStatus::Completed,
        TurnPlanStepStatus::InProgress => CodexTodoStatus::InProgress,
        TurnPlanStepStatus::Pending | TurnPlanStepStatus::Unknown => CodexTodoStatus::Pending,
    }
}

pub(crate) fn build_mcp_tool_use(item: &McpToolCallItem) -> serde_json::Value {
    let input = CodexMcpInput {
        server: item.server.clone(),
        tool: item.tool.clone(),
        arguments: item.arguments.clone(),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

pub(crate) fn build_mcp_tool_result(item: &McpToolCallItem) -> (serde_json::Value, String, bool) {
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

    let summary_text = match (&error_message, &content) {
        (Some(msg), _) => msg.clone(),
        (None, Some(content)) => text_of(content),
        (None, None) => String::new(),
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

/// A dynamic tool call is a namespaced tool with JSON arguments and content
/// items back — the same shape `codex.mcp_tool_call` already renders, so it
/// reuses that tool rather than being invisible. The namespace stands in for the
/// server; a call with none is reported under `dynamic`.
pub(crate) fn build_dynamic_tool_use(item: &DynamicToolCallItem) -> serde_json::Value {
    let input = CodexMcpInput {
        server: item
            .namespace
            .clone()
            .unwrap_or_else(|| "dynamic".to_string()),
        tool: item.tool.clone(),
        arguments: item.arguments.clone(),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

pub(crate) fn build_dynamic_tool_result(
    item: &DynamicToolCallItem,
) -> (serde_json::Value, String, bool) {
    let status = mcp_to_tool_status(item.status);
    let is_error = matches!(status, CodexToolStatus::Failed) || item.success == Some(false);

    let content = item.content_items.clone();
    let summary_text = content
        .as_ref()
        .map(|items| text_of(items))
        .unwrap_or_default();

    let result = CodexMcpResult {
        status,
        content,
        structured_content: None,
        error_message: None,
    };
    let value = serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}));
    (value, summary_text, is_error)
}

/// The `text` fields of a content-block list, newline-joined — how both MCP and
/// dynamic tool output is summarized for the `tool_result` body.
fn text_of(content: &[serde_json::Value]) -> String {
    content
        .iter()
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn build_collab_tool_use(item: &CollabAgentToolCallItem) -> serde_json::Value {
    let input = CodexCollabInput {
        tool: map_collab_tool(item.tool),
        sender_thread_id: item.sender_thread_id.clone(),
        receiver_thread_ids: item.receiver_thread_ids.clone(),
        prompt: item.prompt.clone(),
        agents_states: map_collab_agents_states(&item.agents_states),
    };
    serde_json::to_value(input).unwrap_or_else(|_| serde_json::json!({}))
}

pub(crate) fn build_collab_tool_result(
    item: &CollabAgentToolCallItem,
) -> (serde_json::Value, String, bool) {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Deserializing from the wire keeps these tests honest about field names:
    /// the app-server protocol is camelCase throughout.
    fn item<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
        serde_json::from_value(value).expect("item")
    }

    #[test]
    fn build_codex_todo_list_input_maps_plan_steps() {
        let plan: Vec<TurnPlanStep> = item(serde_json::json!([
            {"step": "first", "status": "completed"},
            {"step": "second", "status": "inProgress"},
            {"step": "third", "status": "pending"},
        ]));
        let input = build_codex_todo_list_input(&plan);
        assert_eq!(input.items.len(), 3);
        assert_eq!(input.items[0].text, "first");
        assert!(input.items[0].completed);
        assert_eq!(input.items[0].status, CodexTodoStatus::Completed);
        // A step in flight is not yet completed, and says so as its own state.
        assert!(!input.items[1].completed);
        assert_eq!(input.items[1].status, CodexTodoStatus::InProgress);
        assert!(!input.items[2].completed);
        assert_eq!(input.items[2].status, CodexTodoStatus::Pending);
    }

    #[test]
    fn build_shell_tool_result_marks_error_on_nonzero_exit() {
        let cmd: CommandExecutionItem = item(serde_json::json!({
            "id": "call_0", "command": "false", "cwd": "/ws",
            "aggregatedOutput": "", "exitCode": 1, "status": "failed",
        }));
        let (value, _, is_error) = build_shell_tool_result(&cmd);
        assert!(is_error);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["exitCode"], 1);
    }

    #[test]
    fn build_shell_tool_use_carries_the_cwd_the_wire_now_always_sends() {
        let cmd: CommandExecutionItem = item(serde_json::json!({
            "id": "call_0", "command": "ls", "cwd": "/ws", "status": "completed",
        }));
        let value = build_shell_tool_use(&cmd);
        assert_eq!(value["cwd"], "/ws");
    }

    #[test]
    fn build_edit_tool_result_labels_internally_tagged_kinds() {
        let edit: FileChangeItem = item(serde_json::json!({
            "id": "call_0",
            "status": "completed",
            "changes": [
                {"path": "/a.py", "kind": {"type": "add"}, "diff": "+x"},
                {"path": "/b.py", "kind": {"type": "update", "move_path": null}, "diff": "@@"},
                {"path": "/c.py", "kind": {"type": "delete"}, "diff": "-y"},
            ],
        }));
        let (value, summary, is_error) = build_edit_tool_result(&edit);
        assert!(!is_error);
        // Byte-identical to the `codex exec` summary, which used the bare kind string.
        assert_eq!(summary, "add /a.py\nupdate /b.py\ndelete /c.py");
        assert_eq!(value["changes"][0]["kind"], "add");
        assert_eq!(value["changes"][1]["kind"], "update");
        assert_eq!(value["changes"][2]["kind"], "delete");
    }

    #[test]
    fn build_edit_tool_result_declined_patch_is_an_error() {
        let edit: FileChangeItem = item(serde_json::json!({
            "id": "call_0", "status": "declined", "changes": [],
        }));
        let (value, _, is_error) = build_edit_tool_result(&edit);
        assert!(is_error);
        assert_eq!(value["status"], "declined");
    }

    #[test]
    fn build_mcp_tool_result_marks_error_when_error_present() {
        let mcp: McpToolCallItem = item(serde_json::json!({
            "id": "call_0", "server": "db", "tool": "query", "arguments": {},
            "result": null, "error": {"message": "connection refused"}, "status": "failed",
        }));
        let (value, summary, is_error) = build_mcp_tool_result(&mcp);
        assert!(is_error);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["errorMessage"], "connection refused");
        assert_eq!(summary, "connection refused");
    }

    #[test]
    fn build_mcp_tool_result_ok_when_status_completed() {
        let mcp: McpToolCallItem = item(serde_json::json!({
            "id": "call_0", "server": "atlassian", "tool": "jira_search", "arguments": {},
            "result": {"content": [{"type": "text", "text": "hit"}], "structuredContent": null},
            "error": null, "status": "completed",
        }));
        let (value, summary, is_error) = build_mcp_tool_result(&mcp);
        assert!(!is_error);
        assert_eq!(value["status"], "completed");
        assert_eq!(summary, "hit");
    }

    #[test]
    fn build_dynamic_tool_reports_the_namespace_as_the_server() {
        let dynamic: DynamicToolCallItem = item(serde_json::json!({
            "id": "call_0", "tool": "lookup", "arguments": {"key": "value"},
            "status": "completed", "namespace": "acme", "success": true,
            "contentItems": [{"type": "inputText", "text": "result text"}],
        }));
        let use_value = build_dynamic_tool_use(&dynamic);
        assert_eq!(use_value["server"], "acme");
        assert_eq!(use_value["tool"], "lookup");
        assert_eq!(use_value["arguments"]["key"], "value");

        let (result, summary, is_error) = build_dynamic_tool_result(&dynamic);
        assert!(!is_error);
        assert_eq!(result["status"], "completed");
        assert_eq!(summary, "result text");
    }

    #[test]
    fn build_dynamic_tool_result_is_an_error_when_the_call_did_not_succeed() {
        let dynamic: DynamicToolCallItem = item(serde_json::json!({
            "id": "call_0", "tool": "lookup", "arguments": {},
            "status": "completed", "success": false,
        }));
        let use_value = build_dynamic_tool_use(&dynamic);
        assert_eq!(use_value["server"], "dynamic");
        let (_, _, is_error) = build_dynamic_tool_result(&dynamic);
        assert!(is_error);
    }

    #[test]
    fn build_collab_tool_result_marks_error_on_errored_agent() {
        let collab: CollabAgentToolCallItem = item(serde_json::json!({
            "id": "call_0", "tool": "spawnAgent", "senderThreadId": "main",
            "receiverThreadIds": ["sub_a"], "status": "completed",
            "agentsStates": {"sub_a": {"status": "errored", "message": "crashed"}},
        }));
        let (_, _, is_error) = build_collab_tool_result(&collab);
        assert!(is_error);
    }

    #[test]
    fn resume_agent_has_no_canonical_collab_tool() {
        let collab: CollabAgentToolCallItem = item(serde_json::json!({
            "id": "call_0", "tool": "resumeAgent", "senderThreadId": "main",
            "status": "inProgress", "agentsStates": {},
        }));
        assert_eq!(build_collab_tool_use(&collab)["tool"], "unknown");
    }
}
