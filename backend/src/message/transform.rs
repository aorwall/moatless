//! Transform DB rows → UI message format.
//!
//! This is the most complex transformation — converts raw DB rows into
//! the UI message format with merged tool calls, content blocks, and
//! pending user input detection.

use std::collections::HashMap;

use super::models::{TaskMessageContentBlock, TaskMessageContext, TaskMessage};
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

    // Build tool_results_map: tool_use_id → tool_result message
    let tool_results_map: HashMap<String, &TaskMessage> = messages
        .iter()
        .filter(|m| m.message_type == MessageType::Result || m.message_role == crate::message::models::MessageRole::User)
        .filter_map(|m| {
            // Check if message_content has tool_use_id (tool_result messages)
            if let Some(content) = m.message_content.as_array() {
                for block in content {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                        if let Some(tool_use_id) = block.get("tool_use_id").and_then(|v| v.as_str()) {
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
                let ui_msg = transform_assistant(msg, &msg_blocks, &tool_results_map, is_last, messages);
                ui_messages.push(ui_msg);
            }
            MessageType::User => {
                // Skip tool_result user messages (inlined into assistant)
                if msg.message_content.as_array().map_or(false, |arr| {
                    arr.iter().any(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
                }) {
                    continue;
                }

                let ui_msg = transform_user(msg, &msg_blocks);
                ui_messages.push(ui_msg);
            }
            MessageType::System => {
                let ui_msg = transform_system(msg);
                ui_messages.push(ui_msg);
            }
            MessageType::Result => {
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
                                if rb.get("tool_use_id").and_then(|v| v.as_str()) == Some(tool_use_id) {
                                    tool_results.push(UiToolResult {
                                        tool_use_id: tool_use_id.to_string(),
                                        content: rb.get("content").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                        is_error: rb.get("is_error").and_then(|v| v.as_bool()),
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
            usage: Some(UiUsage {
                input_tokens: msg.assistant_usage_input_tokens,
                output_tokens: msg.assistant_usage_output_tokens,
                cache_creation_input_tokens: msg.assistant_usage_cache_creation_input_tokens,
                cache_read_input_tokens: msg.assistant_usage_cache_read_input_tokens,
            }),
        }),
        result: None,
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
    }
}

fn transform_system(msg: &TaskMessage) -> UiMessage {
    let content = extract_text_from_content(&msg.message_content);

    UiMessage {
        uuid: msg.uuid.clone(),
        message_type: msg.message_type,
        role: msg.message_role,
        timestamp: msg.timestamp.clone(),
        session_id: msg.session_id.clone(),
        turn_number: msg.turn_number,
        agent_mode: None,
        model: None,
        is_queued: None,
        content,
        tool_results: vec![],
        has_pending_user_input: None,
        assistant: None,
        result: None,
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
            turn_status: msg.turn_status,
            turn_result: msg.turn_result.clone(),
            will_not_resume: msg.will_not_resume,
            run_metrics: msg.run_metrics.clone(),
        }),
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
                    text: block.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    tool_use_id: block.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    tool_name: block.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    tool_input: block.get("input").cloned(),
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
                text: b.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()),
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
                                    && b.get("tool_use_id").and_then(|v| v.as_str()) == Some(tool_use_id)
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
