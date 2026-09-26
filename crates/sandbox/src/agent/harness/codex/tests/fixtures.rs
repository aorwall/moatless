//! Wire-shaped `ServerEvent` builders and the transport fixtures every case
//! runs against.

use super::*;

pub(super) fn config() -> CodexConfig {
    CodexConfig {
        config: Arc::new(crate::config::test_support::minimal_config()),
        credential_failures: CredentialFailureDetector::default(),
        trajectory: None,
    }
}

pub(super) fn process() -> CodexProcess {
    CodexProcess::new(config())
}

pub(super) fn drain(p: &mut CodexProcess) -> Vec<ProcessEvent> {
    let mut out = Vec::new();
    while let Some(event) = p.pending.pop_front() {
        out.push(event);
    }
    out
}

pub(super) fn sources(events: &[ProcessEvent]) -> Vec<&SourceEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            ProcessEvent::Block(s) => Some(s),
            _ => None,
        })
        .collect()
}

// ── Server events, built from wire-shaped JSON ───────────────────

pub(super) fn parse<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap_or_else(|e| panic!("not wire-shaped: {e}"))
}

pub(super) fn thread_started(thread_id: &str) -> ServerEvent {
    ServerEvent::ThreadStarted(parse(json!({ "thread": { "id": thread_id } })))
}

pub(super) fn turn_started(turn_id: &str) -> ServerEvent {
    ServerEvent::TurnStarted(parse(json!({
        "turn": { "id": turn_id, "status": "inProgress" },
    })))
}

/// A `turn/completed` in the shape the server sends it: no items, and an error
/// only when it failed.
pub(super) fn turn_completed(status: &str, error: Option<Value>) -> ServerEvent {
    ServerEvent::TurnCompleted(parse(json!({
        "turn": {
            "id": "turn-1",
            "status": status,
            "error": error,
            "items": [],
            "itemsView": "notLoaded",
        },
    })))
}

pub(super) fn item_completed(item: Value) -> ServerEvent {
    ServerEvent::ItemCompleted(parse(json!({ "item": item })))
}

// ── The same notifications, attributed to a thread ───────────────
//
// A native sub-agent runs on its own thread and streams over the same
// connection, so the thread each notification names is the only thing that tells
// the two apart. The helpers above name none, which is the fail-open case.

pub(super) fn turn_started_on(thread_id: &str, turn_id: &str) -> ServerEvent {
    ServerEvent::TurnStarted(parse(json!({
        "threadId": thread_id,
        "turn": { "id": turn_id, "status": "inProgress" },
    })))
}

pub(super) fn turn_completed_on(thread_id: &str, status: &str) -> ServerEvent {
    ServerEvent::TurnCompleted(parse(json!({
        "threadId": thread_id,
        "turn": {
            "id": "turn-1",
            "status": status,
            "error": null,
            "items": [],
            "itemsView": "notLoaded",
        },
    })))
}

pub(super) fn item_completed_on(thread_id: &str, item: Value) -> ServerEvent {
    ServerEvent::ItemCompleted(parse(json!({ "threadId": thread_id, "item": item })))
}

pub(super) fn token_usage_on(thread_id: &str, input: i64, output: i64) -> ServerEvent {
    let breakdown = json!({
        "inputTokens": input,
        "cachedInputTokens": 0,
        "outputTokens": output,
        "reasoningOutputTokens": 0,
        "totalTokens": input + output,
    });
    ServerEvent::TokenUsage(parse(json!({
        "threadId": thread_id,
        "tokenUsage": { "last": breakdown, "total": breakdown },
    })))
}

pub(super) fn agent_message(id: &str, text: &str) -> Value {
    json!({ "type": "agentMessage", "id": id, "text": text })
}

pub(super) fn token_usage(input: i64, cached: i64, output: i64, reasoning: i64) -> ServerEvent {
    let breakdown = json!({
        "inputTokens": input,
        "cachedInputTokens": cached,
        "outputTokens": output,
        "reasoningOutputTokens": reasoning,
        "totalTokens": input + output,
    });
    ServerEvent::TokenUsage(parse(json!({
        "tokenUsage": { "last": breakdown, "total": breakdown },
    })))
}

/// A reading whose `last` and `total` differ, against a stated window — the
/// shape the meter is read from.
pub(super) fn token_usage_in_window(
    last: (i64, i64),
    total: (i64, i64),
    window: i64,
) -> ServerEvent {
    let breakdown = |(input, output): (i64, i64)| {
        json!({
            "inputTokens": input,
            "cachedInputTokens": 0,
            "outputTokens": output,
            "reasoningOutputTokens": 0,
            "totalTokens": input + output,
        })
    };
    ServerEvent::TokenUsage(parse(json!({
        "tokenUsage": {
            "last": breakdown(last),
            "total": breakdown(total),
            "modelContextWindow": window,
        },
    })))
}

pub(super) fn plan_updated(steps: &[(&str, &str)]) -> ServerEvent {
    let plan: Vec<Value> = steps
        .iter()
        .map(|(step, status)| json!({ "step": step, "status": status }))
        .collect();
    ServerEvent::PlanUpdated(parse(json!({ "plan": plan })))
}

pub(super) fn turn_error(value: Value) -> TurnError {
    parse(value)
}

// ── Item fixtures ────────────────────────────────────────────────

/// The `item` out of one of the transport's checked-in fixtures.
macro_rules! fixture_item {
    ($group:literal / $name:literal) => {{
        let message: Value = serde_json::from_str(include_str!(concat!(
            "../../../codex_app_server/fixtures/",
            $group,
            "/",
            $name,
            ".json"
        )))
        .expect("fixture is JSON");
        message["params"]["item"].clone()
    }};
}

pub(super) use fixture_item;

/// Every item family the wire can deliver, as `(family, fixture item)`.
pub(super) fn every_family() -> Vec<(&'static str, Value)> {
    vec![
        (
            "userMessage",
            fixture_item!("observed" / "item_user_message"),
        ),
        (
            "hookPrompt",
            fixture_item!("schema-derived" / "item_hook_prompt"),
        ),
        (
            "agentMessage",
            fixture_item!("observed" / "item_agent_message"),
        ),
        ("plan", fixture_item!("schema-derived" / "item_plan")),
        ("reasoning", fixture_item!("observed" / "item_reasoning")),
        (
            "commandExecution",
            fixture_item!("observed" / "item_command_execution"),
        ),
        (
            "fileChange",
            fixture_item!("observed" / "item_file_change_add"),
        ),
        (
            "mcpToolCall",
            fixture_item!("schema-derived" / "item_mcp_tool_call"),
        ),
        (
            "dynamicToolCall",
            fixture_item!("schema-derived" / "item_dynamic_tool_call"),
        ),
        (
            "collabAgentToolCall",
            fixture_item!("schema-derived" / "item_collab_agent_tool_call"),
        ),
        (
            "subAgentActivity",
            fixture_item!("schema-derived" / "item_sub_agent_activity"),
        ),
        ("webSearch", fixture_item!("observed" / "item_web_search")),
        (
            "imageView",
            fixture_item!("schema-derived" / "item_image_view"),
        ),
        ("sleep", fixture_item!("schema-derived" / "item_sleep")),
        (
            "imageGeneration",
            fixture_item!("schema-derived" / "item_image_generation"),
        ),
        (
            "enteredReviewMode",
            fixture_item!("schema-derived" / "item_entered_review_mode"),
        ),
        (
            "exitedReviewMode",
            fixture_item!("schema-derived" / "item_exited_review_mode"),
        ),
        (
            "contextCompaction",
            fixture_item!("schema-derived" / "item_context_compaction"),
        ),
    ]
}

/// A process with its Session already announced, ready to absorb items.
pub(super) fn open_process() -> CodexProcess {
    let mut p = process();
    p.absorb(thread_started("thread-A"));
    drain(&mut p);
    p
}

/// The canonical blocks one item normalizes to, as `(block type, tool name)`.
pub(super) fn blocks_for(item: Value) -> Vec<(String, String)> {
    let mut p = open_process();
    p.absorb(item_completed(item));
    let events = drain(&mut p);
    sources(&events)
        .iter()
        .map(|s| {
            let block = s.block_json();
            (
                block["type"].as_str().unwrap_or("?").to_string(),
                block["name"].as_str().unwrap_or("-").to_string(),
            )
        })
        .collect()
}
