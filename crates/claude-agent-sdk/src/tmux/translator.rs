//! JSONL transcript record → `SdkMessage` translation.
//!
//! The interactive Claude TUI writes a structured JSONL transcript to
//! `~/.claude/projects/<dashed-cwd>/<session-id>.jsonl`. This module maps each
//! line into the SDK's `SdkMessage` shape so consumers cannot tell which
//! backend produced the events.
//!
//! Differences from the `--print` stream-json output:
//! - JSONL uses camelCase at the top level (`sessionId`, `requestId`,
//!   `parentUuid`); we patch `sessionId` → `session_id` before deserializing.
//! - There is no `Result` record. `turn_duration` system records mark the end
//!   of an assistant turn; we synthesize a `Result::Success` from the
//!   accumulated assistant usage.
//! - There is no `System(init)` record either; the tailer synthesizes one on
//!   startup from the JSONL filename + first assistant model.
//! - MCP tool results carry `content` as an array of text blocks. We normalize
//!   that to a single string for parity with the subprocess path.

use serde_json::Value;
use tracing::{debug, warn};

use crate::types::content::Usage;
use crate::types::messages::{
    CompactBoundaryMessage, CompactMetadata, CompactTrigger, ResultMessage, ResultSuccess,
    SdkMessage,
};

/// Accumulates per-turn state across multiple JSONL records.
///
/// Reset at each `turn_duration` (which we synthesize into `Result::Success`).
#[derive(Debug)]
pub(crate) struct TurnAccumulator {
    pub(crate) session_id: String,
    pub(crate) turn_number: u32,
    pub(crate) last_text: String,
    pub(crate) last_assistant_uuid: Option<String>,
    pub(crate) accumulated_usage: Usage,
    /// Set when the turn contained an API-error assistant record (e.g. a 404
    /// `model_not_found`). Makes the synthesized `Result` a loud failure
    /// instead of a phantom empty success.
    pub(crate) api_error: bool,
}

fn zero_usage() -> Usage {
    Usage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        service_tier: None,
        cache_creation: None,
        server_tool_use: None,
    }
}

impl TurnAccumulator {
    pub(crate) fn new(session_id: String) -> Self {
        Self {
            session_id,
            turn_number: 0,
            last_text: String::new(),
            last_assistant_uuid: None,
            accumulated_usage: zero_usage(),
            api_error: false,
        }
    }

    fn accumulate(&mut self, usage: &Usage, last_text: Option<String>, assistant_uuid: String) {
        self.accumulated_usage.input_tokens += usage.input_tokens;
        self.accumulated_usage.output_tokens += usage.output_tokens;
        self.accumulated_usage.cache_read_input_tokens = Some(
            self.accumulated_usage.cache_read_input_tokens.unwrap_or(0)
                + usage.cache_read_input_tokens.unwrap_or(0),
        );
        self.accumulated_usage.cache_creation_input_tokens = Some(
            self.accumulated_usage
                .cache_creation_input_tokens
                .unwrap_or(0)
                + usage.cache_creation_input_tokens.unwrap_or(0),
        );
        if usage.service_tier.is_some() {
            self.accumulated_usage
                .service_tier
                .clone_from(&usage.service_tier);
        }
        if usage.cache_creation.is_some() {
            self.accumulated_usage
                .cache_creation
                .clone_from(&usage.cache_creation);
        }
        if usage.server_tool_use.is_some() {
            self.accumulated_usage
                .server_tool_use
                .clone_from(&usage.server_tool_use);
        }
        if let Some(text) = last_text {
            if !text.is_empty() {
                self.last_text = text;
            }
        }
        self.last_assistant_uuid = Some(assistant_uuid);
    }

    fn reset(&mut self) {
        self.turn_number = self.turn_number.saturating_add(1);
        self.last_text.clear();
        self.last_assistant_uuid = None;
        self.accumulated_usage = zero_usage();
        self.api_error = false;
    }
}

/// Outcome of translating a single JSONL record.
pub(crate) enum Translated {
    /// Nothing to emit (record was metadata or unknown).
    Skip,
    /// Emit a single `SdkMessage` (boxed — `SdkMessage` is large).
    One(Box<SdkMessage>),
}

/// Translate a parsed JSONL value into zero, one, or two `SdkMessage`s.
pub(crate) fn translate_record(value: Value, acc: &mut TurnAccumulator) -> Translated {
    let Some(obj) = value.as_object() else {
        debug!("Skipping non-object JSONL record");
        return Translated::Skip;
    };

    let record_type = obj.get("type").and_then(Value::as_str).unwrap_or("");

    match record_type {
        "assistant" => translate_assistant(value, acc),
        "user" => translate_user(value),
        "system" => translate_system(value, acc),
        // Pure metadata / TUI bookkeeping — no SDK consumer cares.
        "queue-operation"
        | "permission-mode"
        | "file-history-snapshot"
        | "attachment"
        | "last-prompt"
        | "ai-title"
        | "custom-title"
        | "agent-name"
        | "pr-link" => Translated::Skip,
        other => {
            debug!(record_type = other, "Unknown JSONL record type, skipping");
            Translated::Skip
        }
    }
}

fn translate_assistant(mut value: Value, acc: &mut TurnAccumulator) -> Translated {
    rename_top_level_key(&mut value, "sessionId", "session_id");

    match serde_json::from_value::<crate::types::messages::AssistantMessage>(value.clone()) {
        Ok(msg) => {
            // An API error (e.g. 404 model_not_found) must surface as a failed
            // turn, not a silent empty success. Flag it so the synthesized
            // Result is marked is_error.
            if msg.is_api_error_message == Some(true) || msg.error.is_some() {
                acc.api_error = true;
            }
            // Pull out the most recent text block for the synthesized Result
            // and accumulate usage.
            let last_text = msg.message.content.iter().rev().find_map(|b| match b {
                crate::types::content::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            });
            if let Some(usage) = &msg.message.usage {
                acc.accumulate(usage, last_text, msg.uuid.clone());
            } else {
                // Some assistant records (e.g. error fallbacks) have no usage —
                // still update last_text / last_uuid.
                if let Some(text) = last_text {
                    if !text.is_empty() {
                        acc.last_text = text;
                    }
                }
                acc.last_assistant_uuid = Some(msg.uuid.clone());
            }
            Translated::One(Box::new(SdkMessage::Assistant(msg)))
        }
        Err(e) => {
            // Last-resort guard: even if the record shape is unrecognized, an
            // API-error record must never be silently dropped. Mark the turn as
            // errored and salvage the error text so the run fails loudly.
            if value.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
                acc.api_error = true;
                if let Some(text) = first_text_block(&value) {
                    if !text.is_empty() {
                        acc.last_text = text;
                    }
                }
                warn!(
                    error = %e,
                    raw = %value,
                    "API-error assistant record failed to deserialize — marking turn as error"
                );
            } else {
                warn!(error = %e, raw = %value, "Failed to deserialize JSONL assistant record");
            }
            Translated::Skip
        }
    }
}

/// Extract the first text block from a raw JSONL assistant record's
/// `message.content` array. Used to salvage error text when full
/// deserialization fails.
fn first_text_block(value: &Value) -> Option<String> {
    value
        .get("message")?
        .get("content")?
        .as_array()?
        .iter()
        .find_map(|block| {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            } else {
                None
            }
        })
}

fn translate_user(mut value: Value) -> Translated {
    // Skip plain user messages (we sent them); only forward tool_result content
    // so the sandbox can publish it as a `Message` event.
    let has_tool_result = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .is_some_and(|arr| {
            arr.iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        });

    if !has_tool_result {
        return Translated::Skip;
    }

    rename_top_level_key(&mut value, "sessionId", "session_id");
    normalize_tool_result_content(&mut value);

    match serde_json::from_value::<crate::types::messages::UserMessage>(value.clone()) {
        Ok(msg) => Translated::One(Box::new(SdkMessage::User(msg))),
        Err(e) => {
            warn!(error = %e, raw = %value, "Failed to deserialize JSONL user record");
            Translated::Skip
        }
    }
}

fn translate_system(value: Value, acc: &mut TurnAccumulator) -> Translated {
    let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");

    match subtype {
        "compact_boundary" => translate_compact_boundary(value),
        "turn_duration" => translate_turn_duration(&value, acc),
        // Diagnostic / informational — the sandbox doesn't consume these in
        // subprocess mode either (CLI emits them as in-band records and the
        // sandbox skips them).
        "api_error"
        | "stop_hook_summary"
        | "away_summary"
        | "scheduled_task_fire"
        | "bridge_status"
        | "informational"
        | "local_command" => {
            debug!(subtype, "Skipping system subtype");
            Translated::Skip
        }
        other => {
            debug!(subtype = other, "Skipping unknown system subtype");
            Translated::Skip
        }
    }
}

fn translate_compact_boundary(value: Value) -> Translated {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Translated::Skip,
    };
    let uuid = obj
        .get("uuid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let session_id = obj
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let meta = obj.get("compactMetadata").and_then(Value::as_object);
    let trigger = meta
        .and_then(|m| m.get("trigger"))
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_value::<CompactTrigger>(Value::String(s.to_string())).ok())
        .unwrap_or(CompactTrigger::Unknown);
    let pre_tokens = meta
        .and_then(|m| m.get("preTokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    Translated::One(Box::new(SdkMessage::CompactBoundary(
        CompactBoundaryMessage {
            uuid,
            session_id,
            compact_metadata: CompactMetadata {
                trigger,
                pre_tokens,
            },
        },
    )))
}

/// Synthesize a [`SdkMessage::Result`] from a `Stop` hook payload when the
/// JSONL transcript hasn't yielded a `turn_duration` system record within
/// the supervisor's retry budget.
///
/// Returns `None` if the payload has no usable `last_assistant_message`.
///
/// `total_cost_usd` is `0.0` (already a documented tmux limitation). The
/// usage block is zeroed since we can't recover token counts from the
/// hook payload.
pub(crate) fn translate_stop_payload_fallback(
    payload_json: &str,
    fallback_session_id: &str,
) -> Option<SdkMessage> {
    let last_text = super::hooks::extract_last_assistant_message(payload_json)?;
    if last_text.is_empty() {
        return None;
    }
    let session_id = super::hooks::extract_session_id(payload_json)
        .unwrap_or_else(|| fallback_session_id.to_string());

    let result = ResultSuccess {
        uuid: format!("tmux-fallback-{}", uuid::Uuid::new_v4().simple()),
        session_id,
        duration_ms: 0,
        duration_api_ms: 0,
        is_error: false,
        num_turns: 1,
        result: last_text,
        total_cost_usd: 0.0,
        usage: zero_usage(),
        model_usage: std::collections::HashMap::new(),
        permission_denials: Vec::new(),
        structured_output: None,
    };
    Some(SdkMessage::Result(ResultMessage::Success(result)))
}

fn translate_turn_duration(value: &Value, acc: &mut TurnAccumulator) -> Translated {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Translated::Skip,
    };
    let uuid = obj.get("uuid").and_then(Value::as_str).map_or_else(
        || {
            acc.last_assistant_uuid
                .clone()
                .unwrap_or_else(|| format!("tmux-turn-{}", acc.turn_number + 1))
        },
        std::string::ToString::to_string,
    );
    let session_id = obj
        .get("sessionId")
        .and_then(Value::as_str)
        .map_or_else(|| acc.session_id.clone(), std::string::ToString::to_string);
    let duration_ms = obj.get("durationMs").and_then(Value::as_u64).unwrap_or(0);

    let result = ResultSuccess {
        uuid,
        session_id,
        duration_ms,
        duration_api_ms: 0,
        is_error: acc.api_error,
        num_turns: acc.turn_number.saturating_add(1),
        result: acc.last_text.clone(),
        total_cost_usd: 0.0,
        usage: acc.accumulated_usage.clone(),
        model_usage: std::collections::HashMap::new(),
        permission_denials: Vec::new(),
        structured_output: None,
    };

    acc.reset();
    Translated::One(Box::new(SdkMessage::Result(ResultMessage::Success(result))))
}

/// Rename a key on the top-level object in-place.
fn rename_top_level_key(value: &mut Value, from: &str, to: &str) {
    if let Some(obj) = value.as_object_mut() {
        if let Some(v) = obj.remove(from) {
            obj.insert(to.to_string(), v);
        }
    }
}

/// Normalize array-of-text-blocks `tool_result.content` (MCP shape) to a single
/// string, matching what the subprocess path emits. Walks
/// `message.content[*].content` and collapses arrays into strings.
fn normalize_tool_result_content(value: &mut Value) {
    let Some(message) = value.get_mut("message").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(content_arr) = message.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    for block in content_arr.iter_mut() {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(block_obj) = block.as_object_mut() else {
            continue;
        };
        let Some(inner) = block_obj.get_mut("content") else {
            continue;
        };
        if let Some(arr) = inner.as_array() {
            let joined = collect_text_blocks(arr);
            *inner = Value::String(joined);
        }
    }
}

fn collect_text_blocks(arr: &[Value]) -> String {
    arr.iter()
        .filter_map(|b| {
            if b.get("type").and_then(Value::as_str) == Some("text") {
                b.get("text").and_then(Value::as_str).map(String::from)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn acc() -> TurnAccumulator {
        TurnAccumulator::new("sess-1".to_string())
    }

    #[test]
    fn translate_simple_assistant_text() {
        let mut a = acc();
        let value = json!({
            "type": "assistant",
            "uuid": "asst-1",
            "sessionId": "sess-1",
            "message": {
                "role": "assistant",
                "model": "claude-opus-4-7",
                "id": "msg_01",
                "content": [{"type": "text", "text": "Hello!"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 5}
            },
            "requestId": "req-1",
            "parentUuid": null
        });

        match translate_record(value, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::Assistant(m) => {
                    assert_eq!(m.uuid, "asst-1");
                    assert_eq!(m.session_id, "sess-1");
                    assert_eq!(m.message.id.as_deref(), Some("msg_01"));
                    assert_eq!(m.message.content.len(), 1);
                }
                other => panic!("Expected Assistant, got {other:?}"),
            },
            Translated::Skip => panic!("Expected Assistant, got Skip"),
        }
        assert_eq!(a.last_text, "Hello!");
        assert_eq!(a.accumulated_usage.input_tokens, 10);
        assert_eq!(a.accumulated_usage.output_tokens, 5);
    }

    #[test]
    fn api_error_assistant_record_marks_turn_errored_and_surfaces_text() {
        // Mirrors the real 404 model_not_found record: an unknown `error` code
        // plus `isApiErrorMessage: true`. Must parse (not get dropped) and flag
        // the turn as errored.
        let mut a = acc();
        let value = json!({
            "type": "assistant",
            "uuid": "asst-err",
            "sessionId": "sess-1",
            "isApiErrorMessage": true,
            "apiErrorStatus": 404,
            "error": "model_not_found",
            "message": {
                "role": "assistant",
                "model": "<synthetic>",
                "id": "msg_err",
                "content": [{"type": "text", "text": "There's an issue with the selected model (fable)."}],
                "stop_reason": "stop_sequence",
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }
        });

        match translate_record(value, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::Assistant(_) => {}
                other => panic!("Expected Assistant, got {other:?}"),
            },
            Translated::Skip => panic!("API-error record was dropped instead of surfaced"),
        }
        assert!(a.api_error, "api_error flag must be set");
        assert_eq!(
            a.last_text,
            "There's an issue with the selected model (fable)."
        );

        // The synthesized turn_duration Result must be a loud failure.
        let td = json!({"type": "system", "subtype": "turn_duration", "durationMs": 731});
        match translate_record(td, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::Result(ResultMessage::Success(s)) => {
                    assert!(s.is_error, "synthesized Result must have is_error=true");
                    assert!(s.result.contains("fable"));
                }
                other => panic!("Expected Result::Success, got {other:?}"),
            },
            Translated::Skip => panic!("Expected synthesized Result"),
        }
    }

    #[test]
    fn unparseable_api_error_record_still_marks_turn_errored() {
        // Even when the record can't be deserialized at all, an API-error record
        // must not be silently skipped — the turn is flagged and text salvaged.
        let mut a = acc();
        let value = json!({
            "type": "assistant",
            // Missing required fields (uuid/session_id) → deserialization fails.
            "isApiErrorMessage": true,
            "message": {
                "content": [{"type": "text", "text": "Overloaded, try again."}]
            }
        });

        // Record itself is skipped (unparseable) but the flag/text are captured.
        matches!(translate_record(value, &mut a), Translated::Skip);
        assert!(
            a.api_error,
            "api_error flag must be set even on parse failure"
        );
        assert_eq!(a.last_text, "Overloaded, try again.");
    }

    #[test]
    fn translate_assistant_tool_use_keeps_last_text_empty() {
        let mut a = acc();
        let value = json!({
            "type": "assistant",
            "uuid": "asst-2",
            "sessionId": "sess-1",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "tu-1",
                    "name": "Bash",
                    "input": {"command": "ls"}
                }],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 8, "output_tokens": 12}
            }
        });
        match translate_record(value, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::Assistant(_) => {}
                other => panic!("Expected Assistant, got {other:?}"),
            },
            Translated::Skip => panic!("Expected Assistant, got Skip"),
        }
        assert_eq!(a.last_text, ""); // no text block this turn
        assert_eq!(a.accumulated_usage.output_tokens, 12);
    }

    #[test]
    fn translate_user_tool_result_string_content() {
        let mut a = acc();
        let value = json!({
            "type": "user",
            "uuid": "u-1",
            "sessionId": "sess-1",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tu-1",
                    "content": "ls output here",
                    "is_error": false
                }]
            },
            "toolUseResult": {"stdout": "ls output here", "stderr": "", "interrupted": false, "isImage": false}
        });
        match translate_record(value, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::User(u) => {
                    assert_eq!(u.session_id, "sess-1");
                    assert_eq!(u.message.content.len(), 1);
                }
                other => panic!("Expected User, got {other:?}"),
            },
            Translated::Skip => panic!("Expected User, got Skip"),
        }
    }

    #[test]
    fn translate_user_tool_result_mcp_array_normalizes_to_string() {
        let mut a = acc();
        let value = json!({
            "type": "user",
            "uuid": "u-2",
            "sessionId": "sess-1",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tu-2",
                    "content": [
                        {"type": "text", "text": "Line one"},
                        {"type": "text", "text": "Line two"}
                    ]
                }]
            },
            "toolUseResult": [
                {"type": "text", "text": "Line one"},
                {"type": "text", "text": "Line two"}
            ]
        });
        match translate_record(value, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::User(u) => {
                    let block = &u.message.content[0];
                    if let crate::types::content::ContentBlock::ToolResult { content, .. } = block {
                        let s = content.as_ref().expect("content present");
                        let text = s.as_str().expect("content was normalized to a string");
                        assert_eq!(text, "Line one\nLine two");
                    } else {
                        panic!("expected ToolResult block");
                    }
                }
                other => panic!("Expected User, got {other:?}"),
            },
            Translated::Skip => panic!("Expected User, got Skip"),
        }
    }

    #[test]
    fn translate_plain_user_message_is_skipped() {
        let mut a = acc();
        let value = json!({
            "type": "user",
            "uuid": "u-3",
            "sessionId": "sess-1",
            "message": {
                "role": "user",
                "content": "hello there"
            }
        });
        assert!(matches!(translate_record(value, &mut a), Translated::Skip));
    }

    #[test]
    fn translate_compact_boundary_maps_metadata() {
        let mut a = acc();
        let value = json!({
            "type": "system",
            "subtype": "compact_boundary",
            "uuid": "cb-1",
            "sessionId": "sess-1",
            "compactMetadata": {
                "trigger": "auto",
                "preTokens": 12345
            }
        });
        match translate_record(value, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::CompactBoundary(b) => {
                    assert_eq!(b.uuid, "cb-1");
                    assert_eq!(b.session_id, "sess-1");
                    assert_eq!(b.compact_metadata.pre_tokens, 12345);
                    assert!(matches!(b.compact_metadata.trigger, CompactTrigger::Auto));
                }
                other => panic!("Expected CompactBoundary, got {other:?}"),
            },
            Translated::Skip => panic!("Expected CompactBoundary, got Skip"),
        }
    }

    #[test]
    fn translate_turn_duration_synthesizes_result() {
        let mut a = acc();

        // Feed an assistant first so the accumulator has state.
        let asst = json!({
            "type": "assistant",
            "uuid": "asst-1",
            "sessionId": "sess-1",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "final answer"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 100, "output_tokens": 25}
            }
        });
        let _ = translate_record(asst, &mut a);

        let td = json!({
            "type": "system",
            "subtype": "turn_duration",
            "uuid": "td-1",
            "sessionId": "sess-1",
            "durationMs": 8200,
            "messageCount": 3
        });
        match translate_record(td, &mut a) {
            Translated::One(boxed) => match *boxed {
                SdkMessage::Result(ResultMessage::Success(r)) => {
                    assert_eq!(r.session_id, "sess-1");
                    assert_eq!(r.duration_ms, 8200);
                    assert_eq!(r.num_turns, 1);
                    assert_eq!(r.result, "final answer");
                    assert_eq!(r.usage.input_tokens, 100);
                    assert_eq!(r.usage.output_tokens, 25);
                }
                other => panic!("Expected Result::Success, got {other:?}"),
            },
            Translated::Skip => panic!("Expected Result::Success, got Skip"),
        }
        // Accumulator should have reset
        assert_eq!(a.turn_number, 1);
        assert_eq!(a.last_text, "");
        assert_eq!(a.accumulated_usage.input_tokens, 0);
    }

    #[test]
    fn translate_skips_metadata_records() {
        let mut a = acc();
        for ty in [
            "queue-operation",
            "permission-mode",
            "file-history-snapshot",
            "attachment",
            "last-prompt",
            "ai-title",
            "custom-title",
            "agent-name",
            "pr-link",
        ] {
            let value = json!({ "type": ty, "sessionId": "sess-1" });
            assert!(
                matches!(translate_record(value, &mut a), Translated::Skip),
                "expected Skip for type {ty}"
            );
        }
    }

    #[test]
    fn translate_skips_diagnostic_system_subtypes() {
        let mut a = acc();
        for st in [
            "api_error",
            "stop_hook_summary",
            "away_summary",
            "scheduled_task_fire",
            "bridge_status",
            "informational",
            "local_command",
            "unknown_future_subtype",
        ] {
            let value = json!({ "type": "system", "subtype": st, "sessionId": "sess-1" });
            assert!(
                matches!(translate_record(value, &mut a), Translated::Skip),
                "expected Skip for system.{st}"
            );
        }
    }

    #[test]
    fn stop_payload_fallback_emits_result_from_last_assistant_message() {
        let payload = r#"{"last_assistant_message":"final answer","session_id":"sess-9"}"#;
        let msg = translate_stop_payload_fallback(payload, "default-sid").expect("expected Some");
        match msg {
            SdkMessage::Result(ResultMessage::Success(r)) => {
                assert_eq!(r.result, "final answer");
                assert_eq!(r.session_id, "sess-9");
                assert!((r.total_cost_usd - 0.0).abs() < f64::EPSILON);
                assert_eq!(r.num_turns, 1);
            }
            other => panic!("Expected Result::Success, got {other:?}"),
        }
    }

    #[test]
    fn stop_payload_fallback_returns_none_when_no_last_assistant_message() {
        assert!(translate_stop_payload_fallback(r#"{"session_id":"x"}"#, "x").is_none());
    }

    #[test]
    fn stop_payload_fallback_uses_fallback_session_id_when_missing() {
        let payload = r#"{"last_assistant_message":"hi"}"#;
        let msg = translate_stop_payload_fallback(payload, "fallback-sid").expect("expected Some");
        if let SdkMessage::Result(ResultMessage::Success(r)) = msg {
            assert_eq!(r.session_id, "fallback-sid");
        } else {
            panic!("Expected Result::Success");
        }
    }

    #[test]
    fn accumulate_sums_input_output_tokens_across_assistants() {
        let mut a = acc();
        for (uuid, in_t, out_t) in [("a1", 10u64, 5u64), ("a2", 20, 7), ("a3", 1, 0)] {
            let v = json!({
                "type": "assistant",
                "uuid": uuid,
                "sessionId": "sess-1",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": format!("turn {uuid}")}],
                    "usage": {"input_tokens": in_t, "output_tokens": out_t}
                }
            });
            let _ = translate_record(v, &mut a);
        }
        assert_eq!(a.accumulated_usage.input_tokens, 31);
        assert_eq!(a.accumulated_usage.output_tokens, 12);
        assert_eq!(a.last_text, "turn a3");
        assert_eq!(a.last_assistant_uuid.as_deref(), Some("a3"));
    }
}
