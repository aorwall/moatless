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
pub const CODEX_TOOL_MCP: &str = "codex.mcp_tool_call";
pub const CODEX_TOOL_COLLAB: &str = "codex.collab_tool_call";
pub const CODEX_TOOL_WEB_SEARCH: &str = "codex.web_search";

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

/// Input for `codex.mcp_tool_call` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexMcpInput {
    pub server: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub arguments: serde_json::Value,
}

/// Result for `codex.mcp_tool_call` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexMcpResult {
    pub status: CodexToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// The collab tool being invoked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CodexCollabTool {
    SpawnAgent,
    SendInput,
    Wait,
    CloseAgent,
    #[serde(other)]
    Unknown,
}

/// Collab agent status for one of the agents involved in a collab tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CodexCollabAgentStatus {
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    NotFound,
    #[serde(other)]
    Unknown,
}

/// Last known state of a collab agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexCollabAgentState {
    pub status: CodexCollabAgentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Input for `codex.collab_tool_call` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexCollabInput {
    pub tool: CodexCollabTool,
    pub sender_thread_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receiver_thread_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default)]
    pub agents_states: std::collections::BTreeMap<String, CodexCollabAgentState>,
}

/// Result for `codex.collab_tool_call` tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexCollabResult {
    pub status: CodexToolStatus,
    #[serde(default)]
    pub agents_states: std::collections::BTreeMap<String, CodexCollabAgentState>,
}

/// Input for `codex.web_search` tool calls.
///
/// Upstream's `WebSearchAction` is itself an enum whose variants depend on
/// the underlying provider; we keep it as a raw JSON value since we do not
/// introspect it today.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexWebSearchInput {
    pub query: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub action: serde_json::Value,
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
    Mcp(CodexMcpInput),
    Collab(CodexCollabInput),
    WebSearch(CodexWebSearchInput),
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
            CODEX_TOOL_MCP => Some(
                serde_json::from_value::<CodexMcpInput>(value.clone()).map(CodexToolInput::Mcp),
            ),
            CODEX_TOOL_COLLAB => Some(
                serde_json::from_value::<CodexCollabInput>(value.clone())
                    .map(CodexToolInput::Collab),
            ),
            CODEX_TOOL_WEB_SEARCH => Some(
                serde_json::from_value::<CodexWebSearchInput>(value.clone())
                    .map(CodexToolInput::WebSearch),
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
    Mcp(CodexMcpResult),
    Collab(CodexCollabResult),
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
            CODEX_TOOL_MCP => Some(
                serde_json::from_value::<CodexMcpResult>(value.clone()).map(CodexToolResult::Mcp),
            ),
            CODEX_TOOL_COLLAB => Some(
                serde_json::from_value::<CodexCollabResult>(value.clone())
                    .map(CodexToolResult::Collab),
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
    fn mcp_input_from_tool_parses() {
        let value = json!({ "server": "atlassian", "tool": "jira_search", "arguments": { "q": "foo" } });
        let parsed = CodexToolInput::from_tool(CODEX_TOOL_MCP, &value)
            .expect("handled")
            .expect("ok");
        match parsed {
            CodexToolInput::Mcp(m) => {
                assert_eq!(m.server, "atlassian");
                assert_eq!(m.tool, "jira_search");
                assert_eq!(m.arguments["q"], "foo");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mcp_result_from_tool_roundtrips_error() {
        let value = json!({
            "status": "failed",
            "errorMessage": "connection refused",
        });
        let parsed = CodexToolResult::from_tool(CODEX_TOOL_MCP, &value)
            .expect("handled")
            .expect("ok");
        match parsed {
            CodexToolResult::Mcp(m) => {
                assert_eq!(m.status, CodexToolStatus::Failed);
                assert_eq!(m.error_message.as_deref(), Some("connection refused"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn collab_input_from_tool_parses_agents() {
        let value = json!({
            "tool": "spawn_agent",
            "senderThreadId": "main",
            "receiverThreadIds": ["sub_a"],
            "prompt": "help",
            "agentsStates": {
                "sub_a": { "status": "running" }
            }
        });
        let parsed = CodexToolInput::from_tool(CODEX_TOOL_COLLAB, &value)
            .expect("handled")
            .expect("ok");
        match parsed {
            CodexToolInput::Collab(c) => {
                assert_eq!(c.tool, CodexCollabTool::SpawnAgent);
                assert_eq!(c.sender_thread_id, "main");
                assert_eq!(c.receiver_thread_ids, vec!["sub_a"]);
                let state = c.agents_states.get("sub_a").expect("agent state");
                assert_eq!(state.status, CodexCollabAgentStatus::Running);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn web_search_input_from_tool_parses() {
        let value = json!({ "query": "rust release", "action": { "type": "search" } });
        let parsed = CodexToolInput::from_tool(CODEX_TOOL_WEB_SEARCH, &value)
            .expect("handled")
            .expect("ok");
        match parsed {
            CodexToolInput::WebSearch(w) => {
                assert_eq!(w.query, "rust release");
                assert_eq!(w.action["type"], "search");
            }
            _ => panic!("wrong variant"),
        }
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
            _ => panic!("wrong variant"),
        }
    }
}
