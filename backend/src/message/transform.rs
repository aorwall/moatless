//! Transform DB rows → UI message format.
//!
//! This is the most complex transformation — converts raw DB rows into
//! the UI message format with merged tool calls, content blocks, and
//! pending user input detection.

use std::collections::HashMap;

use moatless_api_schemas::codex::{CodexToolInput, CodexToolResult};

use super::models::{TaskMessage, TaskMessageContentBlock, TaskMessageContext};
use crate::message::models::MessageType;

use super::schemas::*;

/// Transform raw DB rows into UI-facing messages.
pub fn transform_to_ui(
    messages: &[TaskMessage],
    _contexts: &[TaskMessageContext],
    content_blocks: &[TaskMessageContentBlock],
) -> Vec<UiMessage> {
    // Group content blocks by message ID
    let blocks_by_msg: HashMap<i32, Vec<&TaskMessageContentBlock>> = {
        let mut map: HashMap<i32, Vec<&TaskMessageContentBlock>> = HashMap::new();
        for block in content_blocks {
            map.entry(block.task_message_id).or_default().push(block);
        }
        map
    };

    // Build tool_use_result_map: tool_use_id from parent → tool_use_result JSON
    let tool_use_result_map: HashMap<String, &serde_json::Value> = messages
        .iter()
        .filter(|m| m.message_type == MessageType::System && m.tool_use_result.is_some())
        .filter_map(|m| {
            // Find the tool_use_id this result corresponds to (from tool_result content blocks)
            if let Some(content) = m.message_content.as_array() {
                for block in content {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                        if let Some(tool_use_id) = block.get("tool_use_id").and_then(|v| v.as_str())
                        {
                            if let Some(ref tur) = m.tool_use_result {
                                return Some((tool_use_id.to_string(), tur));
                            }
                        }
                    }
                }
            }
            None
        })
        .collect();

    // Build tool_results_map: tool_use_id → tool_result message
    let tool_results_map: HashMap<String, &TaskMessage> = messages
        .iter()
        .filter(|m| {
            m.message_type == MessageType::Result
                || m.message_role == crate::message::models::MessageRole::User
        })
        .filter_map(|m| {
            // Check if message_content has tool_use_id (tool_result messages)
            if let Some(content) = m.message_content.as_array() {
                for block in content {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                        if let Some(tool_use_id) = block.get("tool_use_id").and_then(|v| v.as_str())
                        {
                            return Some((tool_use_id.to_string(), m));
                        }
                    }
                }
            }
            None
        })
        .collect();

    let mut ui_messages = Vec::new();
    let total = messages.len();

    for (idx, msg) in messages.iter().enumerate() {
        let is_last = idx == total - 1;
        let msg_blocks = blocks_by_msg.get(&msg.id).cloned().unwrap_or_default();

        match msg.message_type {
            MessageType::Assistant => {
                let ui_msg = transform_assistant(
                    msg,
                    &msg_blocks,
                    &tool_results_map,
                    &tool_use_result_map,
                    is_last,
                    messages,
                );
                ui_messages.push(ui_msg);
            }
            MessageType::User => {
                // Skip tool_result user messages (inlined into assistant)
                if msg.message_content.as_array().is_some_and(|arr| {
                    arr.iter()
                        .any(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
                }) {
                    continue;
                }

                let ui_msg = transform_user(msg, &msg_blocks);
                ui_messages.push(ui_msg);
            }
            MessageType::System => {
                // System rows wrap tool_result content; they feed
                // `tool_results_map` for the assistant transform above and are
                // never rendered as independent bubbles.
                continue;
            }
            MessageType::Result | MessageType::Error => {
                let ui_msg = transform_result(msg, &msg_blocks);
                ui_messages.push(ui_msg);
            }
        }
    }

    ui_messages
}

fn transform_assistant(
    msg: &TaskMessage,
    blocks: &[&TaskMessageContentBlock],
    tool_results_map: &HashMap<String, &TaskMessage>,
    tool_use_result_map: &HashMap<String, &serde_json::Value>,
    is_last: bool,
    all_messages: &[TaskMessage],
) -> UiMessage {
    let content = build_content_from_message(msg, blocks);

    // Collect inline tool results
    let mut tool_results = Vec::new();
    if let Some(arr) = msg.message_content.as_array() {
        for block in arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                if let Some(tool_use_id) = block.get("id").and_then(|v| v.as_str()) {
                    if let Some(result_msg) = tool_results_map.get(tool_use_id) {
                        if let Some(result_content) = result_msg.message_content.as_array() {
                            for rb in result_content {
                                if rb.get("tool_use_id").and_then(|v| v.as_str())
                                    == Some(tool_use_id)
                                {
                                    let raw_typed = tool_use_result_map
                                        .get(tool_use_id)
                                        .copied()
                                        .cloned();
                                    let tool_name =
                                        block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                    let typed_result = enrich_codex_typed_result(
                                        tool_name,
                                        block.get("input"),
                                        raw_typed,
                                    );

                                    tool_results.push(UiToolResult {
                                        tool_use_id: tool_use_id.to_string(),
                                        content: rb
                                            .get("content")
                                            .and_then(|v| v.as_str())
                                            .map(std::string::ToString::to_string),
                                        is_error: rb
                                            .get("is_error")
                                            .and_then(serde_json::Value::as_bool),
                                        typed_result,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Check for pending user input (AskUserQuestion / ExitPlanMode without response)
    let has_pending = if is_last {
        check_pending_user_input(msg, all_messages)
    } else {
        false
    };

    UiMessage {
        uuid: msg.uuid.clone(),
        message_type: msg.message_type,
        role: msg.message_role,
        timestamp: msg.timestamp.clone(),
        session_id: msg.session_id.clone(),
        turn_number: msg.turn_number,
        agent_mode: msg.agent_mode.clone(),
        model: msg.model.clone(),
        is_queued: None,
        content,
        tool_results,
        has_pending_user_input: if has_pending { Some(true) } else { None },
        assistant: Some(UiAssistantMeta {
            message_id: msg.assistant_message_id.clone(),
            model: msg.assistant_message_model.clone(),
            stop_reason: msg.assistant_stop_reason.clone(),
            stop_sequence: msg.assistant_stop_sequence.clone(),
            usage: Some(UiUsage {
                input_tokens: msg.assistant_usage_input_tokens,
                output_tokens: msg.assistant_usage_output_tokens,
                cache_creation_input_tokens: msg.assistant_usage_cache_creation_input_tokens,
                cache_read_input_tokens: msg.assistant_usage_cache_read_input_tokens,
            }),
        }),
        result: None,
        sdk_metadata: msg.sdk_metadata.clone(),
        channel: None,
        command_name: msg.command_name.clone(),
        parent_uuid: msg.parent_uuid.clone(),
    }
}

fn transform_user(msg: &TaskMessage, blocks: &[&TaskMessageContentBlock]) -> UiMessage {
    // A message is "queued" until the agent confirms receipt via session_init.
    // sent_at is set together with session_id when the agent session starts.
    let is_queued = msg.sent_at.is_none();

    // Use content blocks if available, filtering to showInUI=true
    let content = if blocks.is_empty() {
        // Fall back to message_content
        extract_text_from_content(&msg.message_content)
    } else {
        blocks
            .iter()
            .filter(|b| b.show_in_ui)
            .map(|b| UiContentBlock {
                block_type: b.block_type.to_string(),
                text: b.text_content.clone(),
                tool_use_id: None,
                tool_name: None,
                tool_input: None,
                show_in_ui: Some(true),
            })
            .collect()
    };

    let channel = msg.channel_provider.as_ref().map(|provider| {
        let external_id = msg.channel_external_id.clone();
        let external_label = external_id.as_deref().map(format_external_label);
        UiMessageChannel {
            provider: provider.clone(),
            source_name: msg.channel_source_name.clone(),
            external_id,
            external_label,
        }
    });

    UiMessage {
        uuid: msg.uuid.clone(),
        message_type: msg.message_type,
        role: msg.message_role,
        timestamp: msg.timestamp.clone(),
        session_id: msg.session_id.clone(),
        turn_number: msg.turn_number,
        agent_mode: msg.agent_mode.clone(),
        model: msg.model.clone(),
        is_queued: if is_queued { Some(true) } else { None },
        content,
        tool_results: vec![],
        has_pending_user_input: None,
        assistant: None,
        result: None,
        sdk_metadata: msg.sdk_metadata.clone(),
        channel,
        command_name: msg.command_name.clone(),
        parent_uuid: msg.parent_uuid.clone(),
    }
}

fn transform_result(msg: &TaskMessage, blocks: &[&TaskMessageContentBlock]) -> UiMessage {
    let content = if blocks.is_empty() {
        extract_text_from_content(&msg.message_content)
    } else {
        blocks
            .iter()
            .filter(|b| b.show_in_ui)
            .map(|b| UiContentBlock {
                block_type: b.block_type.to_string(),
                text: b.text_content.clone(),
                tool_use_id: None,
                tool_name: None,
                tool_input: None,
                show_in_ui: Some(true),
            })
            .collect()
    };

    UiMessage {
        uuid: msg.uuid.clone(),
        message_type: msg.message_type,
        role: msg.message_role,
        timestamp: msg.timestamp.clone(),
        session_id: msg.session_id.clone(),
        turn_number: msg.turn_number,
        agent_mode: msg.agent_mode.clone(),
        model: msg.model.clone(),
        is_queued: None,
        content,
        tool_results: vec![],
        has_pending_user_input: None,
        assistant: None,
        result: Some(UiResultMeta {
            will_not_resume: msg.will_not_resume,
            run_metrics: msg.run_metrics.clone(),
        }),
        sdk_metadata: msg.sdk_metadata.clone(),
        channel: None,
        command_name: msg.command_name.clone(),
        parent_uuid: msg.parent_uuid.clone(),
    }
}

/// Build content blocks from message_content JSON + DB content blocks.
fn build_content_from_message(
    msg: &TaskMessage,
    _blocks: &[&TaskMessageContentBlock],
) -> Vec<UiContentBlock> {
    // Use the message_content JSON array directly
    if let Some(arr) = msg.message_content.as_array() {
        arr.iter()
            .map(|block| {
                let block_type = block
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("text")
                    .to_string();

                UiContentBlock {
                    block_type: block_type.clone(),
                    text: block
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string),
                    tool_use_id: block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string),
                    tool_name: block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string),
                    tool_input: block.get("input").filter(|v| !v.is_null()).cloned(),
                    show_in_ui: None,
                }
            })
            .collect()
    } else {
        vec![]
    }
}

/// Extract text content blocks from message_content JSON.
fn extract_text_from_content(content: &serde_json::Value) -> Vec<UiContentBlock> {
    if let Some(arr) = content.as_array() {
        arr.iter()
            .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("text"))
            .map(|b| UiContentBlock {
                block_type: "text".to_string(),
                text: b
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                tool_use_id: None,
                tool_name: None,
                tool_input: None,
                show_in_ui: Some(true),
            })
            .collect()
    } else {
        vec![]
    }
}

/// For codex tool calls, re-validate the stored `tool_use_result` against
/// the shared typed schemas so the frontend receives a known-shape payload.
///
/// Falls back to the raw value when the tool name is not a codex tool or
/// when deserialization fails — no message is ever dropped.
pub(crate) fn enrich_codex_typed_result(
    tool_name: &str,
    tool_input: Option<&serde_json::Value>,
    raw_typed: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    if !tool_name.starts_with("codex.") {
        return raw_typed;
    }

    if let Some(input) = tool_input {
        match CodexToolInput::from_tool(tool_name, input) {
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                tracing::warn!(
                    tool_name,
                    error = %e,
                    "codex tool_input failed typed validation; keeping raw",
                );
            }
            None => {}
        }
    }

    let raw = raw_typed?;
    match CodexToolResult::from_tool(tool_name, &raw) {
        Some(Ok(typed)) => Some(serde_json::to_value(typed).unwrap_or(raw)),
        Some(Err(e)) => {
            tracing::warn!(
                tool_name,
                error = %e,
                "codex tool_use_result failed typed validation; keeping raw",
            );
            Some(raw)
        }
        None => Some(raw),
    }
}

/// Render a stable channel external id as a short human-readable label.
///
/// Mirrors the prefix scheme produced by inbound channel providers:
/// `github:ic:N`, `github:rc:N`, `github:rv:N`, `github:cr:N`, `github:cs:N`,
/// `github:pr:<action>:<number>[:<sha>]`, `slack:ts:<ts>`, `tg:m:<id>`.
/// Falls back to the raw value for legacy bare-int rows or unknown shapes.
pub(crate) fn format_external_label(raw: &str) -> String {
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() < 2 {
        return raw.to_string();
    }

    let namespace = parts[0];
    let kind = parts[1];
    let rest = &parts[2..];

    match namespace {
        "github" => format_github_label(kind, rest, raw),
        "slack" if kind == "ts" && rest.len() == 1 => format!("ts {}", rest[0]),
        "tg" if kind == "m" && rest.len() == 1 => format!("msg #{}", rest[0]),
        _ => raw.to_string(),
    }
}

fn format_github_label(kind: &str, rest: &[&str], raw: &str) -> String {
    let single = if rest.len() == 1 { Some(rest[0]) } else { None };
    match kind {
        "ic" => single.map_or_else(|| raw.to_string(), |n| format!("Comment #{n}")),
        "rc" | "rv" => single.map_or_else(|| raw.to_string(), |n| format!("Review #{n}")),
        "cr" => single.map_or_else(|| raw.to_string(), |n| format!("Check #{n}")),
        "cs" => single.map_or_else(|| raw.to_string(), |n| format!("Suite #{n}")),
        "pr" => match rest.len() {
            2 => format!("PR #{} {}", rest[1], rest[0]),
            3 => format!("PR #{} {} {}", rest[1], rest[0], rest[2]),
            _ => raw.to_string(),
        },
        _ => raw.to_string(),
    }
}

/// Check if the last assistant message has a pending interactive tool call
/// (AskUserQuestion or ExitPlanMode) without a corresponding tool result.
fn check_pending_user_input(msg: &TaskMessage, all_messages: &[TaskMessage]) -> bool {
    if let Some(arr) = msg.message_content.as_array() {
        for block in arr {
            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                let tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if tool_name == "AskUserQuestion" || tool_name == "ExitPlanMode" {
                    let tool_use_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    // Check if there's a tool_result for this tool_use_id
                    let has_result = all_messages.iter().any(|m| {
                        if let Some(content) = m.message_content.as_array() {
                            content.iter().any(|b| {
                                b.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                                    && b.get("tool_use_id").and_then(|v| v.as_str())
                                        == Some(tool_use_id)
                            })
                        } else {
                            false
                        }
                    });

                    if !has_result {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enrich_codex_shell_normalizes_typed_result() {
        let input = json!({ "command": "false" });
        let raw = json!({
            "status": "failed",
            "exitCode": 1,
            "aggregatedOutput": "",
        });
        let enriched =
            enrich_codex_typed_result("codex.shell", Some(&input), Some(raw)).expect("typed");
        assert_eq!(enriched["status"], "failed");
        assert_eq!(enriched["exitCode"], 1);
    }

    #[test]
    fn enrich_codex_malformed_result_falls_back_to_raw() {
        let input = json!({ "command": "ls" });
        let raw = json!({ "garbage": true });
        let enriched =
            enrich_codex_typed_result("codex.shell", Some(&input), Some(raw.clone()))
                .expect("keeps raw");
        assert_eq!(enriched, raw);
    }

    #[test]
    fn enrich_non_codex_tool_passes_through_untouched() {
        let raw = json!({ "stdout": "ok" });
        let enriched =
            enrich_codex_typed_result("Bash", None, Some(raw.clone())).expect("untouched");
        assert_eq!(enriched, raw);
    }

    #[test]
    fn enrich_codex_edit_normalizes_changes() {
        let input = json!({ "changes": [{ "path": "a.rs", "kind": "update" }] });
        let raw = json!({
            "status": "completed",
            "changes": [{ "path": "a.rs", "kind": "update" }],
        });
        let enriched =
            enrich_codex_typed_result("codex.edit", Some(&input), Some(raw)).expect("typed");
        assert_eq!(enriched["status"], "completed");
        assert_eq!(enriched["changes"][0]["kind"], "update");
    }

    #[test]
    fn enrich_codex_mcp_normalizes_typed_result() {
        let input = json!({ "server": "atlassian", "tool": "jira_search", "arguments": {} });
        let raw = json!({
            "status": "failed",
            "errorMessage": "timeout",
        });
        let enriched =
            enrich_codex_typed_result("codex.mcp_tool_call", Some(&input), Some(raw))
                .expect("typed");
        assert_eq!(enriched["status"], "failed");
        assert_eq!(enriched["errorMessage"], "timeout");
    }

    #[test]
    fn enrich_codex_collab_normalizes_typed_result() {
        let input = json!({
            "tool": "spawn_agent",
            "senderThreadId": "main",
            "receiverThreadIds": ["sub"],
            "agentsStates": {"sub": {"status": "running"}}
        });
        let raw = json!({
            "status": "completed",
            "agentsStates": {"sub": {"status": "completed"}}
        });
        let enriched =
            enrich_codex_typed_result("codex.collab_tool_call", Some(&input), Some(raw))
                .expect("typed");
        assert_eq!(enriched["status"], "completed");
        assert_eq!(enriched["agentsStates"]["sub"]["status"], "completed");
    }

    #[test]
    fn enrich_codex_web_search_passes_through_without_typed_result() {
        let input = json!({ "query": "rust release" });
        let enriched = enrich_codex_typed_result("codex.web_search", Some(&input), None);
        // web_search has no tool_result → None in/out.
        assert!(enriched.is_none());
    }

    // ── transform: chronological render of all rows ─────────────────

    use crate::message::models::MessageRole;

    fn make_msg(
        id: i32,
        uuid: &str,
        message_type: MessageType,
        role: MessageRole,
        turn: Option<i32>,
        text: &str,
    ) -> TaskMessage {
        TaskMessage {
            id,
            task_id: "task-1".into(),
            uuid: uuid.into(),
            message_type,
            user_type: "human".into(),
            parent_uuid: None,
            is_sidechain: None,
            cwd: None,
            session_id: None,
            turn_number: turn,
            version: None,
            git_branch: None,
            request_id: None,
            timestamp: format!("2025-01-01T00:00:0{id}.000Z"),
            message_role: role,
            message_content: json!([{"type": "text", "text": text}]),
            assistant_message_id: None,
            assistant_message_type: None,
            assistant_message_model: None,
            assistant_stop_reason: None,
            assistant_stop_sequence: None,
            assistant_usage_input_tokens: None,
            assistant_usage_output_tokens: None,
            assistant_usage_cache_creation_input_tokens: None,
            assistant_usage_cache_read_input_tokens: None,
            tool_use_result: None,
            sdk_metadata: None,
            attachments: None,
            agent_session_id: None,
            will_not_resume: None,
            run_metrics: None,
            agent_mode: None,
            model: None,
            resume: true,
            sent_at: None,
            created_at: None,
            channel_external_id: None,
            channel_provider: None,
            channel_source_name: None,
            command_name: None,
        }
    }

    #[test]
    fn renders_all_rows_in_order() {
        let messages = vec![
            make_msg(1, "u1", MessageType::User, MessageRole::User, Some(1), "hi"),
            make_msg(
                2,
                "a1",
                MessageType::Assistant,
                MessageRole::Assistant,
                Some(1),
                "step 1",
            ),
            make_msg(
                3,
                "a2",
                MessageType::Assistant,
                MessageRole::Assistant,
                Some(1),
                "step 2",
            ),
            make_msg(
                4,
                "r1",
                MessageType::Result,
                MessageRole::Assistant,
                Some(1),
                "done",
            ),
        ];

        let ui = transform_to_ui(&messages, &[], &[]);
        let types: Vec<_> = ui.iter().map(|m| m.message_type).collect();
        assert_eq!(
            types,
            vec![
                MessageType::User,
                MessageType::Assistant,
                MessageType::Assistant,
                MessageType::Result,
            ]
        );
    }

    #[test]
    fn format_external_label_github_ic() {
        assert_eq!(format_external_label("github:ic:12345"), "Comment #12345");
    }

    #[test]
    fn format_external_label_github_rc_and_rv() {
        assert_eq!(format_external_label("github:rc:99"), "Review #99");
        assert_eq!(format_external_label("github:rv:7"), "Review #7");
    }

    #[test]
    fn format_external_label_github_check_and_suite() {
        assert_eq!(format_external_label("github:cr:55"), "Check #55");
        assert_eq!(format_external_label("github:cs:66"), "Suite #66");
    }

    #[test]
    fn format_external_label_github_pr_action() {
        assert_eq!(
            format_external_label("github:pr:opened:42"),
            "PR #42 opened",
        );
    }

    #[test]
    fn format_external_label_github_pr_action_with_sha() {
        assert_eq!(
            format_external_label("github:pr:synchronize:42:abc1234"),
            "PR #42 synchronize abc1234",
        );
    }

    #[test]
    fn format_external_label_slack() {
        assert_eq!(
            format_external_label("slack:ts:1700000.000123"),
            "ts 1700000.000123",
        );
    }

    #[test]
    fn format_external_label_telegram() {
        assert_eq!(format_external_label("tg:m:789"), "msg #789");
    }

    #[test]
    fn format_external_label_legacy_and_unknown_fallback() {
        assert_eq!(format_external_label("12345"), "12345");
        assert_eq!(format_external_label("discord:msg:42"), "discord:msg:42");
        assert_eq!(
            format_external_label("github:unknown:42"),
            "github:unknown:42",
        );
    }

    #[test]
    fn renders_error_terminal_marker() {
        let messages = vec![
            make_msg(1, "u1", MessageType::User, MessageRole::User, Some(1), "hi"),
            make_msg(
                2,
                "a1",
                MessageType::Assistant,
                MessageRole::Assistant,
                Some(1),
                "tried",
            ),
            make_msg(
                3,
                "e1",
                MessageType::Error,
                MessageRole::Assistant,
                Some(1),
                "boom",
            ),
        ];

        let ui = transform_to_ui(&messages, &[], &[]);
        let types: Vec<_> = ui.iter().map(|m| m.message_type).collect();
        assert_eq!(
            types,
            vec![
                MessageType::User,
                MessageType::Assistant,
                MessageType::Error,
            ]
        );
    }
}
