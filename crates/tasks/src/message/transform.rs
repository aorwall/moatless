//! Transform DB rows → UI message format.
//!
//! This is the most complex transformation — converts raw DB rows into the UI
//! message format with merged tool calls, content blocks, and pending user
//! input detection.

use std::collections::{HashMap, HashSet};

use moatless_api_schemas::codex::{CodexToolInput, CodexToolResult};

use super::contexts::task_message_context_to_input;
use super::models::{HARNESS_USER_TYPE, TOOL_RESPONSE_USER_TYPE, TaskMessage, TaskMessageContext};
use crate::activities::dao::Activity;
use crate::activities::mapping::{ActivityKind, ActivityStatus};
use crate::activities::view::ActivityView;
use crate::message::models::MessageType;
use crate::task::schemas::{FileContextData, MessageContextInput};
use moatless_core::file_storage::FileInfo;

use super::schemas::*;

/// The tool calls that hand a conversation back to a person.
///
/// Keep in sync with `handlers::INTERACTIVE_TOOL_NAMES` and the same pair in
/// `message::dao` and `moatless_t3code::dao`.
const INTERACTIVE_TOOLS: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

/// Which slice of rows a transform pass renders.
///
/// `TopLevel` drops subagent rows (those carrying a `parent_uuid`, the spawning
/// `Agent` tool-use id); `Subagent` renders exactly the rows whose
/// `parent_uuid` equals the given tool-use id.
#[derive(Clone, Copy)]
enum RenderScope<'a> {
    TopLevel,
    Subagent(&'a str),
}

impl RenderScope<'_> {
    fn includes(&self, parent_uuid: Option<&str>) -> bool {
        match self {
            RenderScope::TopLevel => parent_uuid.is_none(),
            RenderScope::Subagent(tool_use_id) => parent_uuid == Some(*tool_use_id),
        }
    }
}

/// One turn: what was said, and what happened on the way to saying it.
///
/// Both halves are optional: a tool-only turn has no Message row, a person's
/// turn has no activities. `position` orders them: the earliest conversation
/// position of the Message and its activities.
struct Turn<'a> {
    position: i64,
    uuid: String,
    message: Option<&'a TaskMessage>,
    activities: Vec<&'a Activity>,
}

/// Transform raw DB rows into UI-facing (top-level) messages.
pub fn transform_to_ui(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    activities: &[Activity],
) -> Vec<UiMessage> {
    transform_to_ui_with_files(messages, contexts, activities, &HashMap::new())
}

/// Like [`transform_to_ui`] but with a pre-fetched `file_metadata` map.
///
/// Synthesises File contexts for legacy messages (recorded without contexts)
/// from their `attachments.file_ids`. Top-level only — subagent rows are
/// assembled on demand via [`transform_subagent_transcript`].
pub fn transform_to_ui_with_files(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    activities: &[Activity],
    file_metadata: &HashMap<String, FileInfo>,
) -> Vec<UiMessage> {
    transform_scoped(
        messages,
        contexts,
        activities,
        file_metadata,
        RenderScope::TopLevel,
    )
}

/// Assemble a single subagent's transcript: run the same row-transform over the
/// rows whose `parent_uuid` equals `tool_use_id`.
pub fn transform_subagent_transcript(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    activities: &[Activity],
    tool_use_id: &str,
) -> Vec<UiMessage> {
    transform_scoped(
        messages,
        contexts,
        activities,
        &HashMap::new(),
        RenderScope::Subagent(tool_use_id),
    )
}

/// Gather the Messages and activities of one scope into turns, in order.
fn turns<'a>(
    messages: &'a [TaskMessage],
    activities: &'a [Activity],
    scope: RenderScope<'_>,
) -> Vec<Turn<'a>> {
    let mut by_uuid: HashMap<&str, Vec<&Activity>> = HashMap::new();
    for activity in activities {
        if !scope.includes(activity.parent_uuid.as_deref()) {
            continue;
        }
        // A Sandbox activity happened on no Message and belongs to no turn. The
        // Moatless frontend has never rendered one; the T3 surface does, from
        // its own projection.
        let Some(message_uuid) = activity.message_uuid.as_deref() else {
            continue;
        };
        by_uuid.entry(message_uuid).or_default().push(activity);
    }
    for group in by_uuid.values_mut() {
        group.sort_by_key(|activity| (activity.position, activity.activity_id.clone()));
    }

    let mut turns: Vec<Turn<'a>> = Vec::new();
    let mut claimed: HashSet<&str> = HashSet::new();
    for message in messages {
        if !scope.includes(message.parent_uuid.as_deref()) {
            continue;
        }
        claimed.insert(message.uuid.as_str());
        let activities = by_uuid
            .get(message.uuid.as_str())
            .cloned()
            .unwrap_or_default();
        let first_activity = activities
            .first()
            .map_or(i64::MAX, |activity| activity.position);
        turns.push(Turn {
            position: i64::from(message.id).min(first_activity),
            uuid: message.uuid.clone(),
            message: Some(message),
            activities,
        });
    }

    // Turns that said nothing: no Message row, so their first activity places them.
    for (message_uuid, group) in &by_uuid {
        if claimed.contains(message_uuid) {
            continue;
        }
        let Some(first) = group.first() else { continue };
        turns.push(Turn {
            position: first.position,
            uuid: (*message_uuid).to_string(),
            message: None,
            activities: group.clone(),
        });
    }

    // Deliberately position, never timestamp: an inherited user Message keeps
    // its ancestor's inbox time while the fork's replayed rows carry the fork's.
    turns.sort_by_key(|turn| turn.position);
    turns
}

fn transform_scoped(
    messages: &[TaskMessage],
    contexts: &[TaskMessageContext],
    activities: &[Activity],
    file_metadata: &HashMap<String, FileInfo>,
    scope: RenderScope<'_>,
) -> Vec<UiMessage> {
    // Group contexts by message ID
    let contexts_by_msg: HashMap<i32, Vec<&TaskMessageContext>> = {
        let mut map: HashMap<i32, Vec<&TaskMessageContext>> = HashMap::new();
        for ctx in contexts {
            map.entry(ctx.task_message_id).or_default().push(ctx);
        }
        map
    };

    let turns = turns(messages, activities, scope);
    let total = turns.len();
    let mut ui_messages = Vec::new();

    for (idx, turn) in turns.iter().enumerate() {
        let is_last = idx + 1 == total;
        let Some(msg) = turn.message else {
            // A turn that only called tools. It said nothing, so there is no row
            // to carry its role, timestamp or model — but the calls it made are
            // exactly what the conversation has to show.
            ui_messages.push(transform_tool_only_turn(
                &turn.uuid,
                &turn.activities,
                is_last,
            ));
            continue;
        };
        let msg_context_rows = contexts_by_msg.get(&msg.id).cloned().unwrap_or_default();

        match msg.message_type {
            MessageType::Assistant => {
                ui_messages.push(transform_assistant(msg, &turn.activities, is_last));
            }
            MessageType::User => {
                // The carrier for a person's answer to `AskUserQuestion` or
                // `ExitPlanMode`. The answer is already on screen as the outcome
                // of the call that asked; rendering the carrier shows it twice.
                if msg.user_type == TOOL_RESPONSE_USER_TYPE {
                    continue;
                }
                // The sandbox waking its own agent; nobody said it.
                if msg.user_type == HARNESS_USER_TYPE {
                    continue;
                }
                ui_messages.push(transform_user(msg, &msg_context_rows, file_metadata));
            }
            // Carrier rows for tool results. Their content is attached to the
            // call it answers, and rendering them as their own bubbles would put
            // every tool's raw output in the conversation. No longer written;
            // still present on Tasks that predate the change.
            MessageType::System => continue,
            // Also no longer written — a run outcome is not something somebody
            // said. Rendered where history still holds one.
            MessageType::Result | MessageType::Error => {
                ui_messages.push(transform_result(msg));
            }
        }
    }

    ui_messages
}

/// The tool calls one turn made, as the content blocks the frontend renders.
fn call_blocks(activities: &[&Activity]) -> Vec<UiContentBlock> {
    activities
        .iter()
        .map(|activity| ActivityView::new(activity))
        .filter(|activity| activity.kind() == ActivityKind::ToolCall)
        .map(|activity| UiContentBlock {
            block_type: "tool_use".to_string(),
            text: None,
            tool_use_id: activity.tool_use_id().map(str::to_string),
            tool_name: Some(activity.summary().to_string()),
            tool_input: Some(activity.input().clone()).filter(|v| !v.is_null()),
            show_in_ui: None,
            subagent_summary: None,
        })
        .collect()
}

/// The outcomes of the calls that have come back.
fn call_results(activities: &[&Activity]) -> Vec<UiToolResult> {
    activities
        .iter()
        .map(|activity| ActivityView::new(activity))
        .filter(|activity| activity.kind() == ActivityKind::ToolCall && activity.is_settled())
        .filter_map(|activity| {
            let tool_use_id = activity.tool_use_id()?.to_string();
            Some(UiToolResult {
                tool_use_id,
                content: activity.result_text().map(str::to_string),
                is_error: Some(activity.is_error()),
                typed_result: enrich_codex_typed_result(
                    activity.summary(),
                    Some(activity.input()),
                    activity.typed_result().cloned(),
                ),
            })
        })
        .collect()
}

/// Whether this turn is still waiting on a person.
///
/// An interactive call that has not come back is the whole condition: the
/// person's answer settles the call's activity, so `in_progress` on one of
/// these two tools *is* "waiting".
fn has_unanswered_question(activities: &[&Activity]) -> bool {
    activities.iter().any(|activity| {
        let activity = ActivityView::new(activity);
        activity.kind() == ActivityKind::ToolCall
            && activity.status() == ActivityStatus::InProgress
            && INTERACTIVE_TOOLS.contains(&activity.summary())
    })
}

fn transform_tool_only_turn(uuid: &str, activities: &[&Activity], is_last: bool) -> UiMessage {
    let timestamp = activities.first().map_or_else(String::new, |activity| {
        activity
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    });
    let has_pending = is_last && has_unanswered_question(activities);

    UiMessage {
        uuid: uuid.to_string(),
        message_type: MessageType::Assistant,
        role: crate::message::models::MessageRole::Assistant,
        timestamp,
        session_id: None,
        turn_number: None,
        agent_mode: None,
        model: None,
        is_queued: None,
        content: call_blocks(activities),
        contexts: vec![],
        tool_results: call_results(activities),
        has_pending_user_input: if has_pending { Some(true) } else { None },
        message_id: None,
        stop_reason: None,
        stop_sequence: None,
        usage: None,
        result: None,
        sdk_metadata: None,
        external_source: None,
        parent_uuid: activities
            .first()
            .and_then(|activity| activity.parent_uuid.clone()),
        source_task_id: None,
        sent_by: None,
    }
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
/// `(message_count, tokens)` aggregate keyed by tool-use id.
///
/// `status` derives from the outer `tool_result` inlined on the Message:
/// present + `is_error` → `Error`; present → `Completed`; absent → `Running`. A
/// missing aggregate entry yields a zeroed count (a just-spawned subagent).
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

/// Build the `UiMessage.contexts` list for a user message.
///
/// Returns the Message's recorded contexts if present; otherwise
/// synthesises File contexts from `attachments.file_ids` via `file_metadata`,
/// covering legacy messages predating that write path.
fn build_user_contexts(
    msg: &TaskMessage,
    rows: &[&TaskMessageContext],
    file_metadata: &HashMap<String, FileInfo>,
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

fn transform_assistant(msg: &TaskMessage, activities: &[&Activity], is_last: bool) -> UiMessage {
    // Text first, then the calls in the order they were made. The text is one
    // folded string rather than a run of blocks, so there is no native order to
    // interleave them by — and nothing renders them interleaved.
    let mut content = Vec::new();
    if let Some(text) = msg.message_text.as_deref().filter(|t| !t.is_empty()) {
        content.push(UiContentBlock {
            block_type: "text".to_string(),
            text: Some(text.to_string()),
            tool_use_id: None,
            tool_name: None,
            tool_input: None,
            show_in_ui: None,
            subagent_summary: None,
        });
    }
    content.extend(call_blocks(activities));

    let has_pending = is_last && has_unanswered_question(activities);

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
        agent_mode: msg.agent_mode(),
        model: msg
            .assistant_message_model
            .clone()
            .or_else(|| msg.model.clone()),
        is_queued: None,
        content,
        contexts: vec![],
        tool_results: call_results(activities),
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
    context_rows: &[&TaskMessageContext],
    file_metadata: &HashMap<String, FileInfo>,
) -> UiMessage {
    // A message is "queued" until the agent confirms receipt via session_init.
    // sent_at is set together with session_id when the agent session starts.
    let is_queued = msg.sent_at.is_none();

    // What the person wrote, and only that. The enrichment the agent sees — the
    // preview URL, the selected element, the attribution, the attachments — is
    // rebuilt at delivery time and was never what this rendered.
    let content = text_blocks(msg.message_text.as_deref());

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
        agent_mode: msg.agent_mode(),
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

fn transform_result(msg: &TaskMessage) -> UiMessage {
    let content = text_blocks(msg.message_text.as_deref());

    UiMessage {
        uuid: msg.uuid.clone(),
        message_type: msg.message_type,
        role: msg.message_role,
        timestamp: msg.timestamp.clone(),
        session_id: msg.session_id.clone(),
        turn_number: msg.turn_number,
        agent_mode: msg.agent_mode(),
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

/// A Message's text, as the one content block the frontend renders it from.
///
/// Empty where there is none — a caller has already decided this row is worth
/// rendering, and a Message with no text is only ever a row predating that
/// write path.
fn text_blocks(text: Option<&str>) -> Vec<UiContentBlock> {
    text.filter(|text| !text.is_empty())
        .map(|text| {
            vec![UiContentBlock {
                block_type: "text".to_string(),
                text: Some(text.to_string()),
                tool_use_id: None,
                tool_name: None,
                tool_input: None,
                show_in_ui: Some(true),
                subagent_summary: None,
            }]
        })
        .unwrap_or_default()
}

/// For codex tool calls, re-validate the stored `tool_use_result` against the
/// shared typed schemas so the frontend receives a known-shape payload.
///
/// Falls back to the raw value when the tool isn't a codex tool or
/// deserialization fails — no message is ever dropped.
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
/// Mirrors the prefix scheme produced by inbound adapters (`github:*`,
/// `slack:ts:*`, `tg:m:*`); falls back to the raw value for legacy bare-int
/// rows or unknown shapes.
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

/// Render the pull request an id names, whichever shape it carries.
fn pull_request(value: &str) -> String {
    if value.contains('#') {
        value.to_string()
    } else {
        format!("#{value}")
    }
}

fn format_github_label(kind: &str, rest: &[&str], raw: &str) -> String {
    let single = if rest.len() == 1 { Some(rest[0]) } else { None };
    match kind {
        "ic" => single.map_or_else(|| raw.to_string(), |n| format!("Comment #{n}")),
        "rc" | "rv" => single.map_or_else(|| raw.to_string(), |n| format!("Review #{n}")),
        "cr" => single.map_or_else(|| raw.to_string(), |n| format!("Check #{n}")),
        "cs" => single.map_or_else(|| raw.to_string(), |n| format!("Suite #{n}")),
        // `rest[1]` is the pull request: `owner/repo#607` since the ids began
        // naming their repository, a bare `607` in a row written before that.
        // Only the bare form needs a `#` supplied.
        "pr" => match rest.len() {
            2 => format!("PR {} {}", pull_request(rest[1]), rest[0]),
            3 => format!("PR {} {} {}", pull_request(rest[1]), rest[0], rest[2]),
            _ => raw.to_string(),
        },
        _ => raw.to_string(),
    }
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

    use crate::activities::mapping::ActivityStatus;
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
            message_text: Some(text.to_string()),
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
            tool_response: None,
            sdk_metadata: None,
            attachments: None,
            agent_session_id: None,
            will_not_resume: None,
            run_metrics: None,
            agent_mode_raw: None,
            effort: None,
            model: None,
            skills: Vec::new(),
            resume: true,
            sent_at: None,
            created_at: None,
            external_subject_key: None,
            external_author: None,
            external_sender_id: None,
            source_task_id: None,
            start_fresh: false,
            sent_by: None,
            external_event_id: None,
            external_adapter: None,
            external_source_name: None,
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

    /// A row written before the ids named their repository still renders.
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

    /// An id that names its repository shows it, rather than a bare number the
    /// reader would have to guess the repository of.
    #[test]
    fn format_external_label_github_pr_names_its_repository() {
        assert_eq!(
            format_external_label("github:pr:opened:acme/widgets#42"),
            "PR acme/widgets#42 opened",
        );
        assert_eq!(
            format_external_label("github:pr:synchronize:acme/widgets#42:abc1234"),
            "PR acme/widgets#42 synchronize abc1234",
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
            FileInfo {
                file_id: "abc".to_string(),
                filename: "legacy.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size: 7,
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

    // ── Seam 2: turns assembled from activities + lazy subagents ────────

    /// One settled tool call on `uuid`, at `position`.
    fn call(uuid: &str, position: i64, tool_use_id: &str, name: &str) -> Activity {
        Activity {
            activity_id: format!("{uuid}:{position}"),
            task_id: "task-1".into(),
            position,
            ordinal: 0,
            message_uuid: Some(uuid.into()),
            parent_uuid: None,
            kind: ActivityKind::ToolCall,
            status: ActivityStatus::Completed,
            tool_use_id: Some(tool_use_id.into()),
            summary: name.into(),
            payload: json!({
                "title": name,
                "input": {},
                "result": "ok",
            }),
            detail: Some("ok".into()),
            is_error: false,
            created_at: chrono::DateTime::from_timestamp(1_735_689_600 + position, 0)
                .expect("valid ts"),
        }
    }

    #[test]
    fn a_forks_inherited_turns_render_in_position_order() {
        let mut messages = vec![
            make_msg(
                1,
                "u1",
                MessageType::User,
                MessageRole::User,
                Some(1),
                "first",
            ),
            make_msg(
                3,
                "a1",
                MessageType::Assistant,
                MessageRole::Assistant,
                Some(1),
                "OK",
            ),
            make_msg(
                4,
                "u2",
                MessageType::User,
                MessageRole::User,
                Some(2),
                "second",
            ),
            make_msg(
                6,
                "a2",
                MessageType::Assistant,
                MessageRole::Assistant,
                Some(2),
                "OK 2",
            ),
        ];
        // As `assemble` stamps a fork: an inherited prompt keeps its ancestor's
        // inbox time, a replayed agent row and activity carry the fork's.
        for message in &mut messages {
            message.timestamp = if message.message_type == MessageType::User {
                "2025-01-01T00:00:00.000Z".into()
            } else {
                "2025-06-01T00:00:00.000Z".into()
            };
        }
        let mut activity = call("a2", 5, "t1", "Bash");
        activity.created_at = chrono::DateTime::from_timestamp(1_748_736_000, 0).expect("valid ts");

        let ui = transform_to_ui(&messages, &[], &[activity]);
        let uuids: Vec<_> = ui.iter().map(|m| m.uuid.as_str()).collect();
        assert_eq!(uuids, vec!["u1", "a1", "u2", "a2"]);
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
    fn a_tool_result_reaches_the_calling_turn_without_a_row_of_its_own() {
        // The call and its outcome are one activity. Nothing carries the result
        // as a Message any more, so this is the whole of what a reader needs.
        let assistant = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "running it",
        );
        let ui = transform_to_ui(&[assistant], &[], &[call("a1", 1, "toolu_1", "Bash")]);

        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].uuid, "a1");
        assert_eq!(ui[0].tool_results.len(), 1);
        assert_eq!(ui[0].tool_results[0].tool_use_id, "toolu_1");
    }

    /// The spec's own behaviour clause: a turn whose agent only called tools
    /// writes no Message, and its calls must still be rendered.
    #[test]
    fn a_turn_that_said_nothing_is_rendered_from_its_activities() {
        let spoken = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "first",
        );
        // No Message row for `a2` at all.
        let ui = transform_to_ui(
            &[spoken],
            &[],
            &[
                call("a1", 1, "toolu_1", "Bash"),
                call("a2", 2, "toolu_2", "Read"),
            ],
        );

        assert_eq!(ui.len(), 2);
        assert_eq!(ui[1].uuid, "a2");
        assert_eq!(ui[1].tool_results[0].tool_use_id, "toolu_2");
    }

    #[test]
    fn a_call_still_waiting_contributes_no_result() {
        let assistant = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "running it",
        );
        let mut waiting = call("a1", 1, "toolu_1", "Bash");
        waiting.status = ActivityStatus::InProgress;
        waiting.payload = json!({ "title": "Bash", "input": {} });

        let ui = transform_to_ui(&[assistant], &[], &[waiting]);

        assert_eq!(ui.len(), 1);
        assert!(
            ui[0].tool_results.is_empty(),
            "a call nothing has answered yet has no outcome to show: {:?}",
            ui[0].tool_results
        );
    }

    #[test]
    fn an_unanswered_question_on_the_last_turn_is_pending_user_input() {
        let assistant = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "which one?",
        );
        let mut asking = call("a1", 1, "toolu_1", "AskUserQuestion");
        asking.status = ActivityStatus::InProgress;

        let ui = transform_to_ui(
            std::slice::from_ref(&assistant),
            &[],
            std::slice::from_ref(&asking),
        );
        assert_eq!(ui[0].has_pending_user_input, Some(true));

        // The person answered: the call settled, and the turn is no longer waiting.
        let answered = call("a1", 1, "toolu_1", "AskUserQuestion");
        let ui = transform_to_ui(&[assistant], &[], &[answered]);
        assert_eq!(ui[0].has_pending_user_input, None);
    }

    #[test]
    fn subagent_transcript_assembles_folded_messages() {
        let mut sub_asst = make_msg(
            1,
            "s1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "working",
        );
        sub_asst.parent_uuid = Some("toolu_1".to_string());
        let mut inner = call("s1", 1, "toolu_inner", "Read");
        inner.parent_uuid = Some("toolu_1".to_string());
        // A top-level row that the transcript must ignore.
        let top = make_msg(
            3,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "top",
        );

        let ui = transform_subagent_transcript(&[sub_asst, top], &[], &[inner], "toolu_1");
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].uuid, "s1");
        assert_eq!(ui[0].tool_results.len(), 1);
        assert_eq!(ui[0].tool_results[0].tool_use_id, "toolu_inner");
    }

    #[test]
    fn inject_subagent_summary_derives_count_tokens_and_status() {
        let assistant = make_msg(
            1,
            "a1",
            MessageType::Assistant,
            MessageRole::Assistant,
            Some(1),
            "delegating",
        );

        // Running: the Agent call has not come back.
        let mut running = call("a1", 1, "toolu_agent", "Agent");
        running.status = ActivityStatus::InProgress;
        running.payload = json!({ "title": "Agent", "input": { "description": "do work" } });

        let mut ui = transform_to_ui(std::slice::from_ref(&assistant), &[], &[running]);
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

        // Completed: the call settled without an error.
        let completed = call("a1", 1, "toolu_agent", "Agent");
        let mut ui2 = transform_to_ui(std::slice::from_ref(&assistant), &[], &[completed]);
        inject_subagent_summaries(&mut ui2, &agg);
        let status2 = ui2[0]
            .content
            .iter()
            .find(|b| b.tool_name.as_deref() == Some("Agent"))
            .and_then(|b| b.subagent_summary.as_ref())
            .map(|s| s.status)
            .unwrap();
        assert_eq!(status2, SubagentStatus::Completed);

        // Error: the call settled as failed.
        let mut failed = call("a1", 1, "toolu_agent", "Agent");
        failed.status = ActivityStatus::Failed;
        failed.is_error = true;
        let mut ui3 = transform_to_ui(&[assistant], &[], &[failed]);
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
