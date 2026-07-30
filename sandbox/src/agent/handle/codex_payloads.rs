//! Pure builders that map Codex CLI thread items to typed tool inputs/results.

use std::collections::HashMap;

use moatless_api_schemas::codex::{
    CodexCollabAgentState, CodexCollabAgentStatus, CodexCollabInput, CodexCollabResult,
    CodexCollabTool, CodexEditInput, CodexEditResult, CodexMcpInput, CodexMcpResult,
    CodexShellInput, CodexShellResult, CodexTodoItem, CodexTodoListInput, CodexToolStatus,
    FileUpdateChange, FileUpdateKind,
};

use crate::agent::codex_exec::{
    self, CollabAgentStatus as CodexExecCollabAgentStatus, CollabTool as CodexExecCollabTool,
    CollabToolCallItem, CollabToolCallStatus, CommandExecutionItem, CommandExecutionStatus,
    FileChangeItem, McpToolCallItem, McpToolCallStatus, PatchApplyStatus, TodoListItem,
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

pub(crate) fn build_shell_tool_use(cmd: &CommandExecutionItem) -> serde_json::Value {
    let input = CodexShellInput {
        command: cmd.command.clone(),
        cwd: cmd.cwd.clone(),
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

pub(crate) fn build_edit_tool_result(edit: &FileChangeItem) -> (serde_json::Value, String, bool) {
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

pub(crate) fn build_codex_todo_list_input(todo: &TodoListItem) -> CodexTodoListInput {
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

pub(crate) fn build_collab_tool_use(item: &CollabToolCallItem) -> serde_json::Value {
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
    item: &CollabToolCallItem,
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
}
