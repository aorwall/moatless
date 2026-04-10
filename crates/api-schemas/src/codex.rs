//! Typed schemas for Codex agent tool calls.
//!
//! The Codex CLI does not expose a Claude-style tool-use/tool-result pair
//! natively. The sandbox translates Codex `ThreadItem`s into Claude-shaped
//! `tool_use` + `tool_result` blocks and uses these structs as the typed
//! contract for the `input` and `tool_use_result` payloads.
//!
//! Tool name convention: `codex.<kind>` (e.g. `codex.shell`, `codex.edit`).

use serde::{Deserialize, Serialize};

/// Canonical codex tool names used as `tool_use.name` in stored messages.
pub const CODEX_TOOL_SHELL: &str = "codex.shell";
pub const CODEX_TOOL_EDIT: &str = "codex.edit";
pub const CODEX_TOOL_TODO_LIST: &str = "codex.todo_list";

/// Status reported by codex for a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CodexToolStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

/// A high-level categorization of a shell command action (codex parsed form).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub struct CommandAction {
    #[serde(rename = "type")]
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Input for `codex.shell` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexShellInput {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_actions: Vec<CommandAction>,
}

/// Result for `codex.shell` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexShellResult {
    pub status: CodexToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregated_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

/// Kind of file change operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FileUpdateKind {
    Add,
    Update,
    Delete,
}

/// A single file-change entry within an edit tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileUpdateChange {
    pub path: String,
    pub kind: FileUpdateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

/// Input for `codex.edit` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexEditInput {
    pub changes: Vec<FileUpdateChange>,
}

/// Result for `codex.edit` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexEditResult {
    pub status: CodexToolStatus,
    #[serde(default)]
    pub changes: Vec<FileUpdateChange>,
}

/// A single entry in a codex `todo_list` plan tracker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexTodoItem {
    pub text: String,
    pub completed: bool,
}

/// Input for `codex.todo_list` tool calls. Codex emits this as a running plan
/// tracker: the same `item_id` is re-emitted on every progress update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexTodoListInput {
    #[serde(default)]
    pub items: Vec<CodexTodoItem>,
}

/// Typed union of codex tool inputs, keyed by canonical tool name.
///
/// Used by the backend enricher to deserialize stored `tool_input` JSON
/// into a strongly-typed value based on the content block's `tool_name`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum CodexToolInput {
    Shell(CodexShellInput),
    Edit(CodexEditInput),
    TodoList(CodexTodoListInput),
}

impl CodexToolInput {
    /// Deserialize a stored `tool_input` value by tool name.
    ///
    /// Returns `None` for unknown/unsupported codex tool names.
    pub fn from_tool(
        tool_name: &str,
        value: &serde_json::Value,
    ) -> Option<Result<Self, serde_json::Error>> {
        match tool_name {
            CODEX_TOOL_SHELL => Some(
                serde_json::from_value::<CodexShellInput>(value.clone()).map(CodexToolInput::Shell),
            ),
            CODEX_TOOL_EDIT => Some(
                serde_json::from_value::<CodexEditInput>(value.clone()).map(CodexToolInput::Edit),
            ),
            CODEX_TOOL_TODO_LIST => Some(
                serde_json::from_value::<CodexTodoListInput>(value.clone())
                    .map(CodexToolInput::TodoList),
            ),
            _ => None,
        }
    }
}

/// Typed union of codex tool results, keyed by canonical tool name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum CodexToolResult {
    Shell(CodexShellResult),
    Edit(CodexEditResult),
}

impl CodexToolResult {
    /// Deserialize a stored `tool_use_result` value by tool name.
    pub fn from_tool(
        tool_name: &str,
        value: &serde_json::Value,
    ) -> Option<Result<Self, serde_json::Error>> {
        match tool_name {
            CODEX_TOOL_SHELL => Some(
                serde_json::from_value::<CodexShellResult>(value.clone())
                    .map(CodexToolResult::Shell),
            ),
            CODEX_TOOL_EDIT => Some(
                serde_json::from_value::<CodexEditResult>(value.clone()).map(CodexToolResult::Edit),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shell_input_roundtrip() {
        let input = CodexShellInput {
            command: "/bin/sh -c 'ls -la'".to_string(),
            cwd: Some("/workspace".to_string()),
            command_actions: vec![],
        };
        let value = serde_json::to_value(&input).unwrap();
        let back: CodexShellInput = serde_json::from_value(value).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn shell_result_roundtrip_with_nonzero_exit() {
        let result = CodexShellResult {
            status: CodexToolStatus::Failed,
            exit_code: Some(1),
            aggregated_output: Some("oops".to_string()),
            duration_ms: Some(42),
        };
        let value = serde_json::to_value(&result).unwrap();
        let back: CodexShellResult = serde_json::from_value(value).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn edit_input_roundtrip_multi_file() {
        let input = CodexEditInput {
            changes: vec![
                FileUpdateChange {
                    path: "src/lib.rs".to_string(),
                    kind: FileUpdateKind::Update,
                    diff: Some("@@ -1 +1 @@\n-a\n+b".to_string()),
                },
                FileUpdateChange {
                    path: "src/new.rs".to_string(),
                    kind: FileUpdateKind::Add,
                    diff: None,
                },
            ],
        };
        let value = serde_json::to_value(&input).unwrap();
        let back: CodexEditInput = serde_json::from_value(value).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn from_tool_dispatches_by_name() {
        let shell = json!({ "command": "echo hi" });
        assert!(matches!(
            CodexToolInput::from_tool(CODEX_TOOL_SHELL, &shell),
            Some(Ok(CodexToolInput::Shell(_)))
        ));

        let edit = json!({ "changes": [{ "path": "a.txt", "kind": "add" }] });
        assert!(matches!(
            CodexToolInput::from_tool(CODEX_TOOL_EDIT, &edit),
            Some(Ok(CodexToolInput::Edit(_)))
        ));

        let todo = json!({ "items": [{ "text": "step 1", "completed": false }] });
        assert!(matches!(
            CodexToolInput::from_tool(CODEX_TOOL_TODO_LIST, &todo),
            Some(Ok(CodexToolInput::TodoList(_)))
        ));

        assert!(CodexToolInput::from_tool("unknown", &json!({})).is_none());
    }

    #[test]
    fn todo_list_input_roundtrip() {
        let input = CodexTodoListInput {
            items: vec![
                CodexTodoItem {
                    text: "first step".to_string(),
                    completed: true,
                },
                CodexTodoItem {
                    text: "second step".to_string(),
                    completed: false,
                },
            ],
        };
        let value = serde_json::to_value(&input).unwrap();
        let back: CodexTodoListInput = serde_json::from_value(value).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn todo_list_input_accepts_empty_items() {
        let value = json!({ "items": [] });
        let parsed: CodexTodoListInput = serde_json::from_value(value).unwrap();
        assert!(parsed.items.is_empty());
    }

    #[test]
    fn shell_result_from_tool_parses_status() {
        let value = json!({
            "status": "completed",
            "exitCode": 0,
            "aggregatedOutput": "done\n",
        });
        let parsed = CodexToolResult::from_tool(CODEX_TOOL_SHELL, &value)
            .expect("handled")
            .expect("ok");
        match parsed {
            CodexToolResult::Shell(r) => {
                assert_eq!(r.status, CodexToolStatus::Completed);
                assert_eq!(r.exit_code, Some(0));
                assert_eq!(r.aggregated_output.as_deref(), Some("done\n"));
            }
            CodexToolResult::Edit(_) => panic!("wrong variant"),
        }
    }
}
