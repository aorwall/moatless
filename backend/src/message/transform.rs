//! Transform DB rows → UI message format.
//!
//! This is the most complex transformation — converts raw DB rows into
//! the UI message format with merged tool calls, content blocks, and
//! pending user input detection.

use std::collections::HashMap;

use moatless_api_schemas::codex::{CodexToolInput, CodexToolResult};

use super::contexts::task_message_context_to_input;
use super::models::{TaskMessage, TaskMessageContentBlock, TaskMessageContext};
use crate::file_storage::models::FileMetadata;
use crate::message::models::MessageType;
use crate::task::schemas::{FileContextData, MessageContextInput};

use super::schemas::*;

/// Which slice of rows a transform pass renders.
///
/// `TopLevel` renders the task's own conversation and drops subagent rows
/// (those carrying a `parent_uuid` — the spawning `Agent` tool-use id).
/// `Subagent` renders exactly one subagent's transcript: the rows whose
/// `parent_uuid` equals the given tool-use id.
#[derive(Clone, Copy)]
enum RenderScope<'a> {
    TopLevel,
    Subagent(&'a str),
}

impl RenderScope<'_> {
    fn includes(&self, msg: &TaskMessage) -> bool {
        match self {
            RenderScope::TopLevel => msg.parent_uuid.is_none(),
            RenderScope::Subagent(tool_use_id) => msg.parent_uuid.as_deref() == Some(*tool_use_id),
        }
    }
}

/// Transform raw DB rows into UI-facing (top-level) messages.
pub fn transform_to_ui(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    content_blocks: &[TaskMessageContentBlock],
) -> Vec<UiMessage> {
    transform_to_ui_with_files(messages, contexts, content_blocks, &HashMap::new())
}

/// Like [`transform_to_ui`] but with a pre-fetched `file_metadata` map used to
/// synthesise File contexts for legacy messages persisted before the
/// `task_message_contexts` write path was wired up. Such messages still have
/// `attachments.file_ids` populated by the previous PR.
///
/// Top-level only: subagent rows (those carrying a `parent_uuid`) are excluded.
/// Their transcripts are assembled on demand via [`transform_subagent_transcript`].
pub fn transform_to_ui_with_files(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    content_blocks: &[TaskMessageContentBlock],
    file_metadata: &HashMap<String, FileMetadata>,
) -> Vec<UiMessage> {
    transform_scoped(
        messages,
        contexts,
        content_blocks,
        file_metadata,
        RenderScope::TopLevel,
    )
}

/// Assemble a single subagent's transcript: run the same fold-resolving
/// row-transform over the rows whose `parent_uuid` equals `tool_use_id`.
/// Returns the subagent's UI-facing Messages (with its tool results inlined).
pub fn transform_subagent_transcript(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    content_blocks: &[TaskMessageContentBlock],
    tool_use_id: &str,
) -> Vec<UiMessage> {
    transform_scoped(
        messages,
        contexts,
        content_blocks,
        &HashMap::new(),
        RenderScope::Subagent(tool_use_id),
    )
}

fn transform_scoped(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    content_blocks: &[TaskMessageContentBlock],
    file_metadata: &HashMap<String, FileMetadata>,
    scope: RenderScope<'_>,
) -> Vec<UiMessage> {
    // Restrict to the rows this scope renders before building any maps, so
    // is_last / pending detection is computed over the scoped slice only.
    let scoped: Vec<&TaskMessage> = messages.iter().filter(|m| scope.includes(m)).collect();

    // Group content blocks by message ID
    let blocks_by_msg: HashMap<i32, Vec<&TaskMessageContentBlock>> = {
        let mut map: HashMap<i32, Vec<&TaskMessageContentBlock>> = HashMap::new();
        for block in content_blocks {
            map.entry(block.task_message_id).or_default().push(block);
        }
        map
    };

    // Group contexts by message ID
    let contexts_by_msg: HashMap<i32, Vec<&TaskMessageContext>> = {
        let mut map: HashMap<i32, Vec<&TaskMessageContext>> = HashMap::new();
        for ctx in contexts {
            map.entry(ctx.task_message_id).or_default().push(ctx);
        }
        map
    };

    // Build tool_use_result_map: tool_use_id from parent → tool_use_result JSON
    let tool_use_result_map: HashMap<String, &serde_json::Value> = scoped
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
    let tool_results_map: HashMap<String, &TaskMessage> = scoped
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
                            return Some((tool_use_id.to_string(), *m));
                        }
                    }
                }
            }
            None
        })
        .collect();

    let mut ui_messages = Vec::new();
    let total = scoped.len();

    for (idx, &msg) in scoped.iter().enumerate() {
        let is_last = idx == total - 1;
        let msg_blocks = blocks_by_msg.get(&msg.id).cloned().unwrap_or_default();
        let msg_context_rows = contexts_by_msg.get(&msg.id).cloned().unwrap_or_default();

        match msg.message_type {
            MessageType::Assistant => {
                let ui_msg = transform_assistant(
                    msg,
                    &msg_blocks,
                    &tool_results_map,
                    &tool_use_result_map,
                    is_last,
                    &scoped,
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

                let ui_msg = transform_user(msg, &msg_blocks, &msg_context_rows, file_metadata);
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

/// tool_use ids declared by an assistant message's content array.
pub(crate) fn tool_use_ids(content: &serde_json::Value) -> Vec<String> {
    content
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_use"))
                .filter_map(|b| {
                    b.get("id")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// tool_use ids referenced by a row's `tool_result` blocks (i.e. the ids of the
/// calls this row is the result for).
pub(crate) fn tool_result_ids(content: &serde_json::Value) -> Vec<String> {
    content
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
                .filter_map(|b| {
                    b.get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Collect the `Agent` tool-use ids referenced by these top-level Messages —
/// the keys for the batched subagent-summary aggregate.
pub(crate) fn agent_tool_use_ids(messages: &[UiMessage]) -> Vec<String> {
    let mut ids = Vec::new();
    for m in messages {
        for b in &m.content {
            if b.tool_name.as_deref() == Some("Agent") {
                if let Some(id) = &b.tool_use_id {
                    ids.push(id.clone());
                }
            }
        }
    }
    ids
}

/// Inject `subagentSummary` onto each `Agent` tool-use block from the batched
/// `(message_count, tokens)` aggregate keyed by tool-use id. `status` is derived
/// from the outer `Agent` tool_result inlined on the owning Message: present +
/// `is_error` → `Error`; present → `Completed`; absent → `Running`. A missing
/// aggregate entry yields a zeroed count (a just-spawned subagent).
pub(crate) fn inject_subagent_summaries(
    messages: &mut [UiMessage],
    aggregates: &HashMap<String, (u32, u32)>,
) {
    for m in messages.iter_mut() {
        // Snapshot each inlined tool_result's error state by tool_use_id before
        // the mutable walk over content blocks.
        let result_status: HashMap<String, bool> = m
            .tool_results
            .iter()
            .map(|r| (r.tool_use_id.clone(), r.is_error.unwrap_or(false)))
            .collect();
        for b in &mut m.content {
            if b.tool_name.as_deref() != Some("Agent") {
                continue;
            }
            let Some(id) = b.tool_use_id.clone() else {
                continue;
            };
            let (message_count, tokens) = aggregates.get(&id).copied().unwrap_or((0, 0));
            let outer_result = result_status.get(&id).copied();
            let status = SubagentStatus::from_outer_result(
                outer_result.is_some(),
                outer_result == Some(true),
            );
            b.subagent_summary = Some(SubagentSummary {
                message_count,
                tokens,
                status,
            });
        }
    }
}

/// Resolve the uuid of the top-level Message a change to `changed_uuid`
/// affects — the seam's **affected-parent resolution**.
///
/// - A folded row (a `tool_result`/system row inlined into the assistant
///   Message that called it) resolves to that **parent** assistant Message's
///   uuid, so a top-level tool result renders live under its call.
/// - A standalone Message resolves to its own uuid.
/// - A subagent row (carrying `parent_uuid`) resolves to `None` — those never
///   upsert a top-level Message (their activity rides `subagentProgress`).
/// - `None` when the row is unknown or its parent is not present in `rows`.
pub(crate) fn resolve_affected_parent_uuid(
    rows: &[TaskMessage],
    changed_uuid: &str,
) -> Option<String> {
    let changed = rows.iter().find(|m| m.uuid == changed_uuid)?;
    if changed.parent_uuid.is_some() {
        return None;
    }

    let folded = tool_result_ids(&changed.message_content);
    if folded.is_empty() {
        return Some(changed.uuid.clone());
    }

    for tool_use_id in &folded {
        if let Some(parent) = rows.iter().find(|m| {
            m.parent_uuid.is_none()
                && tool_use_ids(&m.message_content)
                    .iter()
                    .any(|id| id == tool_use_id)
        }) {
            return Some(parent.uuid.clone());
        }
    }
    None
}

/// Build the `UiMessage.contexts` list for a user message.
///
/// If the message has persisted `task_message_contexts` rows, return them.
/// Otherwise fall back to synthesising File contexts from `attachments.file_ids`
/// using the supplied `file_metadata` map — covers legacy messages persisted
/// before `task_message_contexts` was actually written to.
fn build_user_contexts(
    msg: &TaskMessage,
    rows: &[&TaskMessageContext],
    file_metadata: &HashMap<String, FileMetadata>,
) -> Vec<MessageContextInput> {
    if !rows.is_empty() {
        return rows
            .iter()
            .filter_map(|row| task_message_context_to_input(row))
            .collect();
    }

    // Legacy fallback: synthesise File contexts from attachments.file_ids.
    let file_ids = msg.file_ids();
    if file_ids.is_empty() {
        return Vec::new();
    }

    file_ids
        .into_iter()
        .map(|file_id| {
            let data = file_metadata
                .get(&file_id)
                .map(|m| FileContextData {
                    filename: m.filename.clone(),
                    mime_type: m.mime_type.clone(),
                    size: m.size,
                })
                .unwrap_or_default();
            MessageContextInput::File {
                reference_id: file_id,
                data,
            }
        })
        .collect()
}

fn transform_assistant(
    msg: &TaskMessage,
    blocks: &[&TaskMessageContentBlock],
    tool_results_map: &HashMap<String, &TaskMessage>,
    tool_use_result_map: &HashMap<String, &serde_json::Value>,
    is_last: bool,
    all_messages: &[&TaskMessage],
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
                                    let raw_typed =
                                        tool_use_result_map.get(tool_use_id).copied().cloned();
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

    let external_source = msg.external_adapter.as_ref().map(|adapter_kind| {
        let external_id = msg.external_event_id.clone();
        let external_label = external_id.as_deref().map(format_external_label);
        UiMessageExternalSource {
            adapter_kind: adapter_kind.clone(),
            source_name: msg.external_source_name.clone(),
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
        model: msg
            .assistant_message_model
            .clone()
            .or_else(|| msg.model.clone()),
        is_queued: None,
        content,
        contexts: vec![],
        tool_results,
        has_pending_user_input: if has_pending { Some(true) } else { None },
        message_id: msg.assistant_message_id.clone(),
        stop_reason: msg.assistant_stop_reason.clone(),
        stop_sequence: msg.assistant_stop_sequence.clone(),
        usage: Some(UiUsage {
            input_tokens: msg.assistant_usage_input_tokens,
            output_tokens: msg.assistant_usage_output_tokens,
            cache_creation_input_tokens: msg.assistant_usage_cache_creation_input_tokens,
            cache_read_input_tokens: msg.assistant_usage_cache_read_input_tokens,
        }),
        result: None,
        sdk_metadata: msg.sdk_metadata.clone(),
        external_source,
        parent_uuid: msg.parent_uuid.clone(),
        source_task_id: None,
        sent_by: msg.sent_by.clone(),
    }
}

fn transform_user(
    msg: &TaskMessage,
    blocks: &[&TaskMessageContentBlock],
    context_rows: &[&TaskMessageContext],
    file_metadata: &HashMap<String, FileMetadata>,
) -> UiMessage {
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
                subagent_summary: None,
            })
            .collect()
    };

    let external_source = msg.external_adapter.as_ref().map(|adapter_kind| {
        let external_id = msg.external_event_id.clone();
        let external_label = external_id.as_deref().map(format_external_label);
        UiMessageExternalSource {
            adapter_kind: adapter_kind.clone(),
            source_name: msg.external_source_name.clone(),
            external_id,
            external_label,
        }
    });

    let contexts = build_user_contexts(msg, context_rows, file_metadata);

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
        contexts,
        tool_results: vec![],
        has_pending_user_input: None,
        message_id: None,
        stop_reason: None,
        stop_sequence: None,
        usage: None,
        result: None,
        sdk_metadata: msg.sdk_metadata.clone(),
        external_source,
        parent_uuid: msg.parent_uuid.clone(),
        source_task_id: msg.source_task_id.clone(),
        sent_by: msg.sent_by.clone(),
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
                subagent_summary: None,
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
        contexts: vec![],
        tool_results: vec![],
        has_pending_user_input: None,
        message_id: None,
        stop_reason: None,
        stop_sequence: None,
        usage: None,
        result: Some(UiResultMeta {
            will_not_resume: msg.will_not_resume,
            run_metrics: msg.run_metrics.clone(),
        }),
        sdk_metadata: msg.sdk_metadata.clone(),
        external_source: None,
        parent_uuid: msg.parent_uuid.clone(),
        source_task_id: None,
        sent_by: msg.sent_by.clone(),
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
                    subagent_summary: None,
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
                subagent_summary: None,
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

/// Render a stable external event id as a short human-readable label.
///
/// Mirrors the prefix scheme produced by inbound adapters:
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
fn check_pending_user_input(msg: &TaskMessage, all_messages: &[&TaskMessage]) -> bool {
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
        let enriched = enrich_codex_typed_result("codex.shell", Some(&input), Some(raw.clone()))
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
        let enriched = enrich_codex_typed_result("codex.mcp_tool_call", Some(&input), Some(raw))
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
        let enriched = enrich_codex_typed_result("codex.collab_tool_call", Some(&input), Some(raw))
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
            assistant_usage: None,
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
            external_event_id: None,
            external_adapter: None,
            external_subject_key: None,
            external_source_name: None,
            source_task_id: None,
            start_fresh: false,
            sent_by: None,
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
    fn transform_assistant_surfaces_tagged_external_source() {
        let mut assistant = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "opening pr",
        );
        assistant.external_adapter = Some("github_pr".to_string());
        assistant.external_source_name = Some("foo/bar".to_string());
        assistant.external_event_id = Some("github:pr:opened:42".to_string());

        let ui = transform_to_ui(&[assistant], &[], &[]);
        assert_eq!(ui.len(), 1);
        let external_source = ui[0]
            .external_source
            .as_ref()
            .expect("external source populated");
        assert_eq!(external_source.adapter_kind, "github_pr");
        assert_eq!(external_source.source_name.as_deref(), Some("foo/bar"));
        assert_eq!(
            external_source.external_id.as_deref(),
            Some("github:pr:opened:42")
        );
        assert_eq!(
            external_source.external_label.as_deref(),
            Some("PR #42 opened")
        );
    }

    #[test]
    fn transform_assistant_has_no_external_source_when_provider_null() {
        let assistant = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "plain reply",
        );
        let ui = transform_to_ui(&[assistant], &[], &[]);
        assert_eq!(ui.len(), 1);
        assert!(ui[0].external_source.is_none());
    }

    #[test]
    fn transform_user_attaches_file_context_from_rows() {
        let msg = make_msg(
            1,
            "u1",
            MessageType::User,
            MessageRole::User,
            Some(1),
            "with attachment",
        );
        let row = TaskMessageContext {
            id: 100,
            task_message_id: 1,
            context_type: "file".to_string(),
            reference_id: "upload-1".to_string(),
            data: json!({
                "filename": "report.pdf",
                "mimeType": "application/pdf",
                "size": 1234
            }),
            created_at: None,
        };

        let ui = transform_to_ui(&[msg], &[row], &[]);
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].contexts.len(), 1);
        match &ui[0].contexts[0] {
            MessageContextInput::File { reference_id, data } => {
                assert_eq!(reference_id, "upload-1");
                assert_eq!(data.filename, "report.pdf");
                assert_eq!(data.mime_type, "application/pdf");
                assert_eq!(data.size, 1234);
            }
            other => panic!("Expected File variant, got {other:?}"),
        }
    }

    #[test]
    fn transform_user_legacy_fallback_synthesizes_file_context() {
        let mut msg = make_msg(
            1,
            "u1",
            MessageType::User,
            MessageRole::User,
            Some(1),
            "legacy",
        );
        msg.attachments = Some(json!({ "file_ids": ["abc"] }));

        let mut file_metadata = HashMap::new();
        file_metadata.insert(
            "abc".to_string(),
            FileMetadata {
                file_id: "abc".to_string(),
                filename: "legacy.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size: 7,
                scope: moatless_backend_core::scope::Scope::User,
                team_id: "personal-user-1".to_string(),
                owner_user_id: Some("user-1".to_string()),
                created_by: Some("user-1".to_string()),
                uploaded_at: chrono::Utc::now(),
            },
        );

        let ui = transform_to_ui_with_files(&[msg], &[], &[], &file_metadata);
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].contexts.len(), 1);
        match &ui[0].contexts[0] {
            MessageContextInput::File { reference_id, data } => {
                assert_eq!(reference_id, "abc");
                assert_eq!(data.filename, "legacy.txt");
                assert_eq!(data.mime_type, "text/plain");
                assert_eq!(data.size, 7);
            }
            other => panic!("Expected File variant, got {other:?}"),
        }
    }

    #[test]
    fn transform_user_legacy_fallback_yields_empty_data_when_metadata_missing() {
        let mut msg = make_msg(
            1,
            "u1",
            MessageType::User,
            MessageRole::User,
            Some(1),
            "legacy",
        );
        msg.attachments = Some(json!({ "file_ids": ["xyz"] }));

        let ui = transform_to_ui(&[msg], &[], &[]);
        assert_eq!(ui[0].contexts.len(), 1);
        match &ui[0].contexts[0] {
            MessageContextInput::File { reference_id, data } => {
                assert_eq!(reference_id, "xyz");
                assert_eq!(data.filename, "");
            }
            other => panic!("Expected File variant, got {other:?}"),
        }
    }

    #[test]
    fn transform_user_round_trips_element_and_preview_contexts() {
        let msg = make_msg(
            1,
            "u1",
            MessageType::User,
            MessageRole::User,
            Some(1),
            "with contexts",
        );
        let element_row = TaskMessageContext {
            id: 100,
            task_message_id: 1,
            context_type: "element".to_string(),
            reference_id: "el-1".to_string(),
            data: json!({
                "componentId": "cmp-1",
                "path": "src/app.tsx",
                "line": 12,
                "column": 4,
                "componentName": "Button"
            }),
            created_at: None,
        };
        let preview_row = TaskMessageContext {
            id: 101,
            task_message_id: 1,
            context_type: "previewUrl".to_string(),
            reference_id: "preview-1".to_string(),
            data: json!({
                "path": "/dashboard",
                "url": "https://example.com/dashboard"
            }),
            created_at: None,
        };

        let ui = transform_to_ui(&[msg], &[element_row, preview_row], &[]);
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].contexts.len(), 2);
        assert!(matches!(
            &ui[0].contexts[0],
            MessageContextInput::Element { reference_id, .. } if reference_id == "el-1"
        ));
        assert!(matches!(
            &ui[0].contexts[1],
            MessageContextInput::PreviewUrl { reference_id, .. } if reference_id == "preview-1"
        ));
    }

    /// Rows persisted by removed features (e.g. the old Figma import context)
    /// are dropped rather than failing the whole transform.
    #[test]
    fn transform_user_drops_unrecognised_contexts() {
        let msg = make_msg(1, "u1", MessageType::User, MessageRole::User, Some(1), "hi");
        let legacy_row = TaskMessageContext {
            id: 100,
            task_message_id: 1,
            context_type: "figma".to_string(),
            reference_id: "imp-1".to_string(),
            data: json!({ "importId": "imp-1" }),
            created_at: None,
        };

        let ui = transform_to_ui(&[msg], &[legacy_row], &[]);
        assert_eq!(ui.len(), 1);
        assert!(ui[0].contexts.is_empty());
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

    // ── Seam 2: affected-parent resolution + lazy subagents ─────────────

    fn assistant_with_content(id: i32, uuid: &str, content: serde_json::Value) -> TaskMessage {
        let mut m = make_msg(
            id,
            uuid,
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "",
        );
        m.message_content = content;
        m
    }

    fn tool_result_row(id: i32, uuid: &str, content: serde_json::Value) -> TaskMessage {
        let mut m = make_msg(
            id,
            uuid,
            MessageType::System,
            MessageRole::User,
            Some(1),
            "",
        );
        m.message_content = content;
        m
    }

    #[test]
    fn resolve_affected_parent_folds_tool_result_into_parent() {
        let assistant = assistant_with_content(
            1,
            "a1",
            json!([{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}}]),
        );
        let result = tool_result_row(
            2,
            "sys1",
            json!([{"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"}]),
        );
        let rows = vec![assistant, result];

        // A folded tool_result resolves to its parent assistant Message.
        assert_eq!(
            resolve_affected_parent_uuid(&rows, "sys1"),
            Some("a1".to_string())
        );
        // A standalone Message resolves to itself.
        assert_eq!(
            resolve_affected_parent_uuid(&rows, "a1"),
            Some("a1".to_string())
        );
        // An unknown uuid resolves to None.
        assert_eq!(resolve_affected_parent_uuid(&rows, "nope"), None);
    }

    #[test]
    fn resolve_affected_parent_skips_subagent_rows() {
        let mut sub = make_msg(
            1,
            "sub1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "hi",
        );
        sub.parent_uuid = Some("toolu_agent".to_string());
        assert_eq!(resolve_affected_parent_uuid(&[sub], "sub1"), None);
    }

    #[test]
    fn top_level_transform_excludes_subagent_rows() {
        let top = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "top",
        );
        let mut sub = make_msg(
            2,
            "s1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "subagent step",
        );
        sub.parent_uuid = Some("toolu_1".to_string());

        let ui = transform_to_ui(&[top, sub], &[], &[]);
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].uuid, "a1");
    }

    #[test]
    fn subagent_transcript_assembles_folded_messages() {
        let mut sub_asst = assistant_with_content(
            1,
            "s1",
            json!([
                {"type": "text", "text": "working"},
                {"type": "tool_use", "id": "toolu_inner", "name": "Read", "input": {}}
            ]),
        );
        sub_asst.parent_uuid = Some("toolu_1".to_string());
        let mut sub_res = tool_result_row(
            2,
            "s2",
            json!([{"type": "tool_result", "tool_use_id": "toolu_inner", "content": "file"}]),
        );
        sub_res.parent_uuid = Some("toolu_1".to_string());
        // A top-level row that the transcript must ignore.
        let top = make_msg(
            3,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "top",
        );

        let ui = transform_subagent_transcript(&[sub_asst, sub_res, top], &[], &[], "toolu_1");
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].uuid, "s1");
        assert_eq!(ui[0].tool_results.len(), 1);
        assert_eq!(ui[0].tool_results[0].tool_use_id, "toolu_inner");
    }

    #[test]
    fn inject_subagent_summary_derives_count_tokens_and_status() {
        let agent_block = json!([{"type": "tool_use", "id": "toolu_agent", "name": "Agent", "input": {"description": "do work"}}]);

        // Running: no outer Agent tool_result present.
        let mut ui = transform_to_ui(
            &[assistant_with_content(1, "a1", agent_block.clone())],
            &[],
            &[],
        );
        let mut agg = HashMap::new();
        agg.insert("toolu_agent".to_string(), (5u32, 1234u32));
        inject_subagent_summaries(&mut ui, &agg);
        let summary = ui[0]
            .content
            .iter()
            .find(|b| b.tool_name.as_deref() == Some("Agent"))
            .and_then(|b| b.subagent_summary.as_ref())
            .expect("summary injected");
        assert_eq!(summary.message_count, 5);
        assert_eq!(summary.tokens, 1234);
        assert_eq!(summary.status, SubagentStatus::Running);

        // Completed: outer Agent tool_result present, not an error.
        let result = tool_result_row(
            2,
            "sys1",
            json!([{"type": "tool_result", "tool_use_id": "toolu_agent", "content": "done"}]),
        );
        let mut ui2 = transform_to_ui(
            &[assistant_with_content(1, "a1", agent_block.clone()), result],
            &[],
            &[],
        );
        inject_subagent_summaries(&mut ui2, &agg);
        let status2 = ui2[0]
            .content
            .iter()
            .find(|b| b.tool_name.as_deref() == Some("Agent"))
            .and_then(|b| b.subagent_summary.as_ref())
            .map(|s| s.status)
            .unwrap();
        assert_eq!(status2, SubagentStatus::Completed);

        // Error: outer Agent tool_result carries is_error.
        let err = tool_result_row(
            2,
            "sys1",
            json!([{"type": "tool_result", "tool_use_id": "toolu_agent", "content": "boom", "is_error": true}]),
        );
        let mut ui3 = transform_to_ui(
            &[assistant_with_content(1, "a1", agent_block), err],
            &[],
            &[],
        );
        // Missing aggregate entry → zeroed count.
        inject_subagent_summaries(&mut ui3, &HashMap::new());
        let s3 = ui3[0]
            .content
            .iter()
            .find(|b| b.tool_name.as_deref() == Some("Agent"))
            .and_then(|b| b.subagent_summary.as_ref())
            .expect("summary injected");
        assert_eq!(s3.status, SubagentStatus::Error);
        assert_eq!(s3.message_count, 0);
    }
}
