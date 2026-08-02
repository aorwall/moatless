//! Unit + gated E2E coverage for the Codex adapter: native item normalization
//! (one source event per item family, collision-safe ids, usage retention),
//! failure classification, and the one-shot lifecycle through the shared driver
//! (fresh Turn ordering, queued input, cancellation teardown, unexpected exit).
//!
//! The item-family tests are driven from the checked-in transport fixtures
//! (`codex_app_server/fixtures/`), not from hand-written JSON: the mapping is
//! only worth anything if it is built against what the server actually sends.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::agent::codex_app_server::protocol::TurnError;
use crate::agent::codex_app_server::{AppServerSession, ServerEvent};
use crate::agent::types::ResumeSession;
use crate::credentials::recovery::{CredentialFailureDetector, CODEX_AUTH_JSON};

use super::super::error::{ErrorClass, FailureStage, SessionImpact};
use super::super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::super::signal::{Cause, Input, InputId, Outcome, Signal};
use super::super::source::{Role, SourceEvent};
use super::super::Harness;
use super::classify::{
    classify_startup_error, classify_stream_error, classify_turn_error, is_expired_credential,
};
use super::{CodexConfig, CodexProcess, MID_TURN_DELIVERY_TIMEOUT};

fn config() -> CodexConfig {
    CodexConfig {
        config: Arc::new(crate::config::test_support::minimal_config()),
        credential_failures: CredentialFailureDetector::default(),
    }
}

fn process() -> CodexProcess {
    CodexProcess::new(config())
}

fn drain(p: &mut CodexProcess) -> Vec<ProcessEvent> {
    let mut out = Vec::new();
    while let Some(event) = p.pending.pop_front() {
        out.push(event);
    }
    out
}

fn sources(events: &[ProcessEvent]) -> Vec<&SourceEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            ProcessEvent::Block(s) => Some(s),
            _ => None,
        })
        .collect()
}

// ── Server events, built from wire-shaped JSON ───────────────────

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap_or_else(|e| panic!("not wire-shaped: {e}"))
}

fn thread_started(thread_id: &str) -> ServerEvent {
    ServerEvent::ThreadStarted(parse(json!({ "thread": { "id": thread_id } })))
}

fn turn_started(turn_id: &str) -> ServerEvent {
    ServerEvent::TurnStarted(parse(json!({
        "turn": { "id": turn_id, "status": "inProgress" },
    })))
}

/// A `turn/completed` in the shape the server sends it: no items, and an error
/// only when it failed.
fn turn_completed(status: &str, error: Option<Value>) -> ServerEvent {
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

fn item_completed(item: Value) -> ServerEvent {
    ServerEvent::ItemCompleted(parse(json!({ "item": item })))
}

fn token_usage(input: i64, cached: i64, output: i64, reasoning: i64) -> ServerEvent {
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
fn token_usage_in_window(last: (i64, i64), total: (i64, i64), window: i64) -> ServerEvent {
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

fn plan_updated(steps: &[(&str, &str)]) -> ServerEvent {
    let plan: Vec<Value> = steps
        .iter()
        .map(|(step, status)| json!({ "step": step, "status": status }))
        .collect();
    ServerEvent::PlanUpdated(parse(json!({ "plan": plan })))
}

fn turn_error(value: Value) -> TurnError {
    parse(value)
}

// ── Item fixtures ────────────────────────────────────────────────

/// The `item` out of one of the transport's checked-in fixtures.
macro_rules! fixture_item {
    ($group:literal / $name:literal) => {{
        let message: Value = serde_json::from_str(include_str!(concat!(
            "../../codex_app_server/fixtures/",
            $group,
            "/",
            $name,
            ".json"
        )))
        .expect("fixture is JSON");
        message["params"]["item"].clone()
    }};
}

/// Every item family the wire can deliver, as `(family, fixture item)`.
fn every_family() -> Vec<(&'static str, Value)> {
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
fn open_process() -> CodexProcess {
    let mut p = process();
    p.absorb(thread_started("thread-A"));
    drain(&mut p);
    p
}

/// The canonical blocks one item normalizes to, as `(block type, tool name)`.
fn blocks_for(item: Value) -> Vec<(String, String)> {
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

// ── Native normalization: one test per item family ───────────────

/// The whole mapping table in one place: every family the wire can deliver,
/// against the canonical blocks it produces. A family that renders nothing
/// produces none — deliberately, and distinctly from an unknown family, which
/// is forwarded raw (see `unknown_item_family_is_forwarded_with_diagnostics`).
#[test]
fn every_item_family_maps_to_its_canonical_blocks() {
    let expected: &[(&str, &[(&str, &str)])] = &[
        // Rendered as assistant text.
        ("agentMessage", &[("text", "-")]),
        ("reasoning", &[("text", "-")]),
        // Tool families: a tool_use plus its paired tool_result.
        (
            "commandExecution",
            &[("tool_use", "codex.shell"), ("tool_result", "-")],
        ),
        (
            "fileChange",
            &[("tool_use", "codex.edit"), ("tool_result", "-")],
        ),
        (
            "mcpToolCall",
            &[("tool_use", "codex.mcp_tool_call"), ("tool_result", "-")],
        ),
        // A namespaced tool with JSON arguments and content items back is what
        // `codex.mcp_tool_call` already renders.
        (
            "dynamicToolCall",
            &[("tool_use", "codex.mcp_tool_call"), ("tool_result", "-")],
        ),
        (
            "collabAgentToolCall",
            &[("tool_use", "codex.collab_tool_call"), ("tool_result", "-")],
        ),
        // Web search has no result concept, so it is a lone tool_use.
        ("webSearch", &[("tool_use", "codex.web_search")]),
        // The nine families with nothing to render. The prompt echo is already
        // in the conversation; the rest are status notices with no canonical
        // block type to land in.
        ("userMessage", &[]),
        ("hookPrompt", &[]),
        ("plan", &[]),
        ("subAgentActivity", &[]),
        ("imageView", &[]),
        ("sleep", &[]),
        ("imageGeneration", &[]),
        ("enteredReviewMode", &[]),
        ("exitedReviewMode", &[]),
        ("contextCompaction", &[]),
    ];

    for (family, item) in every_family() {
        let want = expected
            .iter()
            .find(|(name, _)| *name == family)
            .unwrap_or_else(|| panic!("{family} has no entry in the mapping table"))
            .1;
        let got = blocks_for(item);
        let got: Vec<(&str, &str)> = got
            .iter()
            .map(|(ty, name)| (ty.as_str(), name.as_str()))
            .collect();
        assert_eq!(got, want, "{family} maps to the wrong blocks");
    }
}

#[test]
fn the_mapping_table_covers_every_family_the_transport_models() {
    // A new family added to `ThreadItem` without a fixture would go unmapped,
    // which the transport's own `every_item_family_has_a_fixture` catches; this
    // pins that the harness reads all eighteen of them.
    assert_eq!(every_family().len(), 18);
}

#[test]
fn agent_message_and_reasoning_are_ordered_assistant_text_blocks() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!("observed" / "item_reasoning")));
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_agent_message"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 2);
    // Reasoning and answer both normalize to renderable assistant text, in order.
    assert_eq!(s[0].block_json()["type"], "text");
    assert_eq!(
        s[0].block_json()["text"],
        "**Planning update and patch sequence**"
    );
    assert_eq!(s[1].block_json()["type"], "text");
    assert!(s[1].block_json()["text"]
        .as_str()
        .expect("text")
        .contains("four steps"));
    assert!(s.iter().all(|e| e.role == Role::Assistant));
    assert_ne!(s[0].source_id, s[1].source_id);
    assert_eq!(s[0].model.as_deref(), None); // model set only after submit
}

/// `codex exec` reported reasoning as one `text`; this transport splits it into
/// `summary` and `content`, and an item with neither is normal.
#[test]
fn reasoning_falls_back_from_summary_to_content_and_may_render_nothing() {
    let summary = blocks_for(json!({
        "type": "reasoning", "id": "rs_0",
        "summary": ["first", "second"], "content": ["raw"],
    }));
    assert_eq!(summary.len(), 1);

    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "reasoning", "id": "rs_0",
        "summary": ["first", "second"], "content": ["raw"],
    })));
    let events = drain(&mut p);
    assert_eq!(
        sources(&events)[0].block_json()["text"],
        "first\n\nsecond",
        "the summary wins, and multiple parts join as paragraphs"
    );

    let content_only = {
        let mut p = open_process();
        p.absorb(item_completed(
            json!({ "type": "reasoning", "id": "rs_1", "summary": [], "content": ["raw"] }),
        ));
        let events = drain(&mut p);
        sources(&events)[0].block_json()["text"].clone()
    };
    assert_eq!(content_only, "raw", "content is read when summary is empty");

    assert!(
        blocks_for(json!({ "type": "reasoning", "id": "rs_2", "summary": [], "content": [] }))
            .is_empty(),
        "an empty reasoning item renders nothing rather than an empty block"
    );
}

#[test]
fn command_execution_carries_the_same_shell_payload_exec_produced() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_command_execution"
    )));
    let events = drain(&mut p);
    let s = sources(&events);

    let use_block = s[0].block_json();
    assert_eq!(use_block["name"], "codex.shell");
    assert_eq!(
        use_block["input"]["command"],
        "/bin/bash -lc 'echo hello-from-codex'"
    );
    assert_eq!(use_block["input"]["cwd"], "/tmp/probe/turn/ws");
    assert_eq!(s[0].role, Role::Assistant);
    assert!(s[0].tool_use_result.is_none());

    let result_block = s[1].block_json();
    assert_eq!(result_block["type"], "tool_result");
    assert_eq!(result_block["tool_use_id"], use_block["id"]);
    assert_eq!(result_block["is_error"], false);
    assert_eq!(s[1].role, Role::User);
    let typed = s[1].tool_use_result.as_ref().expect("typed shell result");
    assert_eq!(typed["status"], "completed");
    assert_eq!(typed["exitCode"], 0);
    assert_eq!(typed["aggregatedOutput"], "hello-from-codex\n");
}

#[test]
fn a_failed_command_is_flagged_as_an_error_result() {
    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "commandExecution", "id": "call_0",
        "command": "false", "cwd": "/w", "status": "failed", "exitCode": 1,
        "aggregatedOutput": "boom\n",
    })));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s[1].block_json()["is_error"], true);
    assert_eq!(
        s[1].tool_use_result.as_ref().expect("typed")["status"],
        "failed"
    );
}

/// The app-server tags the change kind (`{"type":"update"}`) where exec sent a
/// bare `"update"`; the canonical payload must not change shape because of it.
#[test]
fn file_change_kinds_map_to_the_canonical_flat_kind() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_file_change_add"
    )));
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_file_change_update"
    )));
    p.absorb(item_completed(json!({
        "type": "fileChange", "id": "call_del", "status": "completed",
        "changes": [{ "path": "/gone.txt", "kind": { "type": "delete" }, "diff": "-x\n" }],
    })));
    let events = drain(&mut p);
    let s = sources(&events);

    let add = s[0].block_json();
    assert_eq!(add["name"], "codex.edit");
    assert_eq!(
        add["input"]["changes"][0]["path"],
        "/tmp/probe/turn/ws/note.txt"
    );
    assert_eq!(add["input"]["changes"][0]["kind"], "add");
    assert_eq!(add["input"]["changes"][0]["diff"], "ALPHA\n");
    assert_eq!(
        s[1].tool_use_result.as_ref().expect("typed")["status"],
        "completed"
    );

    assert_eq!(s[2].block_json()["input"]["changes"][0]["kind"], "update");
    assert_eq!(s[4].block_json()["input"]["changes"][0]["kind"], "delete");
}

#[test]
fn mcp_and_dynamic_tool_calls_share_the_mcp_card() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_mcp_tool_call"
    )));
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_dynamic_tool_call"
    )));
    let events = drain(&mut p);
    let s = sources(&events);

    assert_eq!(s[0].block_json()["input"]["server"], "atlassian");
    assert_eq!(s[0].block_json()["input"]["tool"], "jira_search");
    assert_eq!(s[0].block_json()["input"]["arguments"]["q"], "foo");
    let mcp = s[1].tool_use_result.as_ref().expect("typed mcp result");
    assert_eq!(mcp["status"], "completed");
    assert_eq!(mcp["content"][0]["text"], "match");

    // The namespace is the server, so a dynamic tool renders as a named card
    // rather than an anonymous one.
    assert_eq!(s[2].block_json()["input"]["server"], "acme");
    assert_eq!(s[2].block_json()["input"]["tool"], "lookup");
    let dynamic = s[3].tool_use_result.as_ref().expect("typed dynamic result");
    assert_eq!(dynamic["status"], "completed");
    assert_eq!(dynamic["content"][0]["text"], "result text");
}

#[test]
fn a_failed_mcp_call_carries_its_error_message() {
    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "mcpToolCall", "id": "call_0", "server": "db", "tool": "q",
        "arguments": {}, "status": "failed", "error": { "message": "connection refused" },
    })));
    let events = drain(&mut p);
    let s = sources(&events);
    let typed = s[1].tool_use_result.as_ref().expect("typed");
    assert_eq!(typed["status"], "failed");
    assert_eq!(typed["errorMessage"], "connection refused");
    assert_eq!(s[1].block_json()["is_error"], true);
}

#[test]
fn collab_tool_call_carries_its_agents_states() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_collab_agent_tool_call"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    let input = s[0].block_json();
    assert_eq!(input["name"], "codex.collab_tool_call");
    assert_eq!(input["input"]["tool"], "spawn_agent");
    assert_eq!(
        input["input"]["senderThreadId"],
        "019f0000-0000-7000-8000-00000000thre"
    );
    assert_eq!(input["input"]["prompt"], "help with the migration");
    let typed = s[1].tool_use_result.as_ref().expect("typed collab result");
    assert_eq!(typed["status"], "in_progress");
    assert_eq!(
        typed["agentsStates"]["019f0000-0000-7000-8000-0000000000sub"]["status"],
        "running"
    );
}

/// `resumeAgent` has no canonical equivalent — the schema's collab tool enum
/// does not carry it — so it maps to the explicit `unknown` rather than being
/// silently reported as some other tool.
#[test]
fn an_unmodelled_collab_tool_maps_to_unknown() {
    let mut p = open_process();
    p.absorb(item_completed(json!({
        "type": "collabAgentToolCall", "id": "call_0", "tool": "resumeAgent",
        "senderThreadId": "main", "status": "completed",
    })));
    let events = drain(&mut p);
    assert_eq!(sources(&events)[0].block_json()["input"]["tool"], "unknown");
}

#[test]
fn web_search_is_a_lone_tool_use_carrying_the_query() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "observed" / "item_web_search"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1, "web search has no result concept");
    assert_eq!(s[0].block_json()["name"], "codex.web_search");
    assert!(s[0].block_json()["input"]["query"]
        .as_str()
        .expect("query")
        .contains("Rust release"));
    assert_eq!(s[0].block_json()["input"]["action"]["type"], "search");
}

#[test]
fn unknown_item_family_is_forwarded_with_diagnostics() {
    let mut p = open_process();
    p.absorb(item_completed(fixture_item!(
        "schema-derived" / "item_unknown_family"
    )));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1);
    // The raw native block is preserved verbatim inside the canonical `unknown`
    // block and flagged with bounded diagnostics so an upstream CLI addition is
    // never silently dropped.
    let block = s[0].block_json();
    assert_eq!(block["type"], "unknown");
    assert_eq!(block["raw"]["type"], "brandNewThing");
    assert_eq!(block["raw"]["foo"], "bar");
    let diag = s[0].diagnostics.as_ref().expect("diagnostics retained");
    assert_eq!(diag["type"], "brandNewThing");
}

#[test]
fn known_families_carry_no_diagnostics() {
    let mut p = open_process();
    for (_, item) in every_family() {
        p.absorb(item_completed(item));
    }
    let events = drain(&mut p);
    assert!(sources(&events).iter().all(|e| e.diagnostics.is_none()));
}

/// The running plan is a notification here, not the re-emitted `todo_list` item
/// exec sent — but it must still upsert one live card.
#[test]
fn plan_updates_share_a_stable_source_id() {
    let mut p = open_process();
    p.absorb(plan_updated(&[("a", "inProgress"), ("b", "pending")]));
    p.absorb(plan_updated(&[("a", "completed"), ("b", "inProgress")]));
    p.absorb(plan_updated(&[("a", "completed"), ("b", "completed")]));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 3, "every update emits a block");
    assert_eq!(s[0].source_id, s[1].source_id);
    assert_eq!(s[1].source_id, s[2].source_id);
    // Each emission is a codex.todo_list tool_use whose input tracks completion.
    assert_eq!(s[0].block_json()["name"], "codex.todo_list");
    assert_eq!(s[0].block_json()["input"]["items"][0]["text"], "a");
    assert_eq!(s[0].block_json()["input"]["items"][0]["completed"], false);
    assert_eq!(s[2].block_json()["input"]["items"][0]["completed"], true);
    // Only `completed` counts as done; in-progress is still outstanding.
    assert_eq!(s[1].block_json()["input"]["items"][1]["completed"], false);
}

#[test]
fn duplicate_item_ids_across_spawns_get_distinct_source_ids() {
    // Codex restarted `item_0` on every resume; two spawns must not collide.
    let item = json!({ "type": "agentMessage", "id": "item_0", "text": "x" });

    let mut first = open_process();
    first.absorb(item_completed(item.clone()));
    let first_id = sources(&drain(&mut first))[0].source_id.clone();

    let mut second = open_process();
    second.absorb(item_completed(item));
    let second_id = sources(&drain(&mut second))[0].source_id.clone();

    assert_ne!(
        first_id, second_id,
        "same native item_0 on the same thread must yield collision-safe ids"
    );
}

// ── Session announcement ─────────────────────────────────────────

#[test]
fn thread_started_yields_session_ready_then_input_accepted() {
    let mut p = process();
    p.input_id = Some(InputId("in-1".to_string()));
    p.absorb(thread_started("thread-A"));
    let events = drain(&mut p);
    assert!(matches!(
        events[0],
        ProcessEvent::SessionReady { ref session } if session.0 == "thread-A"
    ));
    assert!(matches!(
        events[1],
        ProcessEvent::InputAccepted { ref input } if input.0 == "in-1"
    ));
    assert_eq!(events.len(), 2);
}

/// A resume emits no `thread/started`, so the Session is announced from the
/// request response instead. When both happen — a fresh thread does notify —
/// the second announcement must not duplicate the pair.
#[test]
fn the_session_is_announced_exactly_once() {
    let mut p = process();
    p.input_id = Some(InputId("in-1".to_string()));
    p.announce_session("thread-A".to_string());
    p.absorb(thread_started("thread-A"));
    let events = drain(&mut p);
    assert_eq!(events.len(), 2, "got {events:?}");
}

// ── Turn terminal and usage ──────────────────────────────────────

#[test]
fn turn_completed_emits_usage_result_then_terminated() {
    let mut p = open_process();
    p.absorb(turn_started("turn-1"));
    p.absorb(token_usage(12, 3, 4, 7));
    p.absorb(turn_completed("completed", None));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1, "turn usage carried on a single result block");
    assert_eq!(s[0].role, Role::System);
    assert_eq!(s[0].block_json()["type"], "turn_result");
    let usage = s[0].usage.as_ref().expect("usage retained");
    assert_eq!(usage["input_tokens"], 12);
    assert_eq!(usage["cache_read_input_tokens"], 3);
    assert_eq!(usage["output_tokens"], 4);
    assert_eq!(usage["reasoning_output_tokens"], 7);
    assert!(matches!(
        events.last(),
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Completed
        })
    ));
}

/// The meter measures what occupies the window, which is the last request's own
/// input and output. `total` is the whole Session's spend and keeps rising past
/// the window it would be measured against.
#[test]
fn the_meter_reads_the_last_request_not_the_sessions_spend() {
    let mut p = open_process();
    p.absorb(turn_started("turn-1"));
    p.absorb(token_usage_in_window(
        (30_000, 500),
        (400_000, 9_000),
        200_000,
    ));

    let window = drain(&mut p)
        .into_iter()
        .find_map(|event| match event {
            ProcessEvent::ContextWindow(window) => Some(window),
            _ => None,
        })
        .expect("a reading");
    assert_eq!(window.used_tokens, 30_500);
    assert_eq!(window.max_tokens, Some(200_000));
}

/// A used figure past the window cannot be right. Full is the honest reading of
/// it; a percentage over 100 is not.
#[test]
fn a_reading_past_the_window_reads_as_full() {
    let mut p = open_process();
    p.absorb(turn_started("turn-1"));
    p.absorb(token_usage_in_window(
        (250_000, 1_000),
        (250_000, 1_000),
        200_000,
    ));

    let window = drain(&mut p)
        .into_iter()
        .find_map(|event| match event {
            ProcessEvent::ContextWindow(window) => Some(window),
            _ => None,
        })
        .expect("a reading");
    assert_eq!(window.used_tokens, 200_000);
    assert_eq!(window.max_tokens, Some(200_000));
}

/// `total` is cumulative over the Session and is replayed on resume, so the
/// Turn's own spend is its growth — not the last reading, which would bill the
/// resumed Turn for the whole conversation.
#[test]
fn usage_on_a_resumed_thread_reports_only_what_this_turn_spent() {
    let mut p = open_process();
    // Replayed before the Turn opens: what the Session had already spent.
    p.absorb(token_usage(1000, 100, 50, 10));
    p.absorb(turn_started("turn-2"));
    // Two model requests inside the Turn, each a fresh cumulative reading.
    p.absorb(token_usage(1600, 150, 80, 20));
    p.absorb(token_usage(2200, 200, 110, 30));
    p.absorb(turn_completed("completed", None));

    let events = drain(&mut p);
    let usage = sources(&events)[0].usage.as_ref().expect("usage").clone();
    assert_eq!(usage["input_tokens"], 1200);
    assert_eq!(usage["cache_read_input_tokens"], 100);
    assert_eq!(usage["output_tokens"], 60);
    assert_eq!(usage["reasoning_output_tokens"], 20);
}

#[test]
fn a_turn_with_no_usage_reported_still_emits_a_result_block() {
    let mut p = open_process();
    p.absorb(turn_completed("completed", None));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].usage.as_ref().expect("usage")["input_tokens"], 0);
}

/// An interrupted Turn is a normal end, not a failure: the driver owns
/// cancelled-ness and maps the termination once it knows it asked for it.
#[test]
fn an_interrupted_turn_terminates_as_completed_not_failed() {
    let mut p = open_process();
    p.absorb(turn_completed("interrupted", None));
    let events = drain(&mut p);
    assert!(
        matches!(
            events.last(),
            Some(ProcessEvent::Terminated {
                termination: ProcessTermination::Completed
            })
        ),
        "got {events:?}"
    );
}

#[test]
fn a_failed_turn_with_no_named_cause_still_terminates_as_a_failure() {
    let mut p = open_process();
    p.absorb(turn_completed(
        "failed",
        Some(json!({ "message": "something went wrong" })),
    ));
    let events = drain(&mut p);
    match events.last() {
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Failed(error),
        }) => {
            assert_eq!(error.class, ErrorClass::InvalidInput);
            assert_eq!(error.session_impact, SessionImpact::Intact);
        }
        other => panic!("expected a failed terminal, got {other:?}"),
    }
}

/// The `error` notification is not a terminal: Codex sends it while it is still
/// retrying, and `turn/completed` is the authority.
#[test]
fn an_error_notification_does_not_end_the_turn() {
    let mut p = open_process();
    p.absorb(ServerEvent::Error(parse(json!({
        "error": { "message": "stream disconnected" },
        "willRetry": true,
    }))));
    assert!(drain(&mut p).is_empty());
}

// ── Failure classification ───────────────────────────────────────

#[test]
fn startup_spawn_failure_is_invalid_configuration() {
    let error = std::io::Error::new(std::io::ErrorKind::NotFound, "codex: not found");
    let classified = classify_startup_error(&error);
    assert_eq!(classified.class, ErrorClass::InvalidConfiguration);
    assert_eq!(classified.stage, FailureStage::Startup);
}

#[test]
fn stream_parse_error_is_native_protocol() {
    let classified = classify_stream_error("Failed to parse an app-server frame: bad");
    assert_eq!(classified.class, ErrorClass::NativeProtocol);
    assert_eq!(classified.stage, FailureStage::Execution);
}

#[test]
fn stream_exit_error_is_process_exit() {
    let classified = classify_stream_error("Codex app-server exited with status 1");
    assert_eq!(classified.class, ErrorClass::ProcessExit);
}

/// The whole `codexErrorInfo` table, in the wire's own spelling.
#[test]
fn every_named_turn_error_maps_to_its_class_and_impact() {
    let cases: &[(Value, ErrorClass, SessionImpact)] = &[
        (
            json!("unauthorized"),
            ErrorClass::Authentication,
            SessionImpact::Unknown,
        ),
        (
            json!("usageLimitExceeded"),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!("serverOverloaded"),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!("internalServerError"),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "httpConnectionFailed": { "httpStatusCode": 502 } }),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "responseStreamConnectionFailed": { "httpStatusCode": 503 } }),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "responseStreamDisconnected": { "httpStatusCode": null } }),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!({ "responseTooManyFailedAttempts": { "httpStatusCode": 500 } }),
            ErrorClass::RateLimit,
            SessionImpact::Intact,
        ),
        (
            json!("sandboxError"),
            ErrorClass::InvalidConfiguration,
            SessionImpact::Intact,
        ),
        (
            json!("threadRollbackFailed"),
            ErrorClass::UnavailableSession,
            SessionImpact::Lost,
        ),
        (
            json!("contextWindowExceeded"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("sessionBudgetExceeded"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("badRequest"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("cyberPolicy"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!("other"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!({ "activeTurnNotSteerable": { "turnKind": "review" } }),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        // Not in this build's table: conservative default, and logged with its
        // raw JSON so a new upstream code is visible.
        (
            json!("someBrandNewCode"),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
        (
            json!({ "brandNewObjectCode": { "detail": 1 } }),
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
        ),
    ];

    for (info, class, impact) in cases {
        let error = turn_error(json!({ "message": "boom", "codexErrorInfo": info }));
        let classified = classify_turn_error(&error);
        assert_eq!(classified.class, *class, "codexErrorInfo {info}");
        assert_eq!(classified.session_impact, *impact, "codexErrorInfo {info}");
    }
}

#[test]
fn a_turn_error_with_no_named_cause_takes_the_conservative_default() {
    let classified = classify_turn_error(&turn_error(json!({ "message": "boom" })));
    assert_eq!(classified.class, ErrorClass::InvalidInput);
    assert_eq!(classified.session_impact, SessionImpact::Intact);
}

#[test]
fn a_turn_errors_additional_details_reach_the_diagnostic() {
    let classified = classify_turn_error(&turn_error(json!({
        "message": "request failed",
        "additionalDetails": "upstream returned 429",
    })));
    assert!(
        classified.diagnostic.contains("upstream returned 429"),
        "{}",
        classified.diagnostic
    );
}

#[test]
fn only_an_unauthorized_turn_means_the_credential_expired() {
    assert!(is_expired_credential(&turn_error(
        json!({ "message": "no", "codexErrorInfo": "unauthorized" })
    )));
    assert!(!is_expired_credential(&turn_error(
        json!({ "message": "no", "codexErrorInfo": "badRequest" })
    )));
    assert!(!is_expired_credential(&turn_error(
        json!({ "message": "no" })
    )));
}

/// The coordinator polls this flag at the turn-failure boundary and owns the
/// pull/compare/retry from there; the harness only reports the fact.
#[test]
fn an_unauthorized_turn_flags_the_codex_credential() {
    let cfg = config();
    let detector = cfg.credential_failures.clone();
    let mut p = CodexProcess::new(cfg);
    p.absorb(thread_started("thread-A"));
    drain(&mut p);

    p.absorb(turn_completed(
        "failed",
        Some(json!({ "message": "unauthorized", "codexErrorInfo": "unauthorized" })),
    ));
    let events = drain(&mut p);
    match events.last() {
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Failed(error),
        }) => assert_eq!(error.class, ErrorClass::Authentication),
        other => panic!("expected an authentication failure, got {other:?}"),
    }
    assert!(detector.take(CODEX_AUTH_JSON), "credential must be flagged");
}

#[test]
fn a_turn_that_fails_for_another_reason_leaves_the_credential_alone() {
    let cfg = config();
    let detector = cfg.credential_failures.clone();
    let mut p = CodexProcess::new(cfg);
    p.absorb(turn_completed(
        "failed",
        Some(json!({ "message": "nope", "codexErrorInfo": "badRequest" })),
    ));
    drain(&mut p);
    assert!(!detector.take(CODEX_AUTH_JSON));
}

// ── One-shot lifecycle through the shared driver ─────────────────

/// A message that arrives while a Codex Turn runs is folded into that Turn, the
/// same as the Claude harness does, instead of waiting for a Turn of its own.
#[test]
fn the_codex_process_accepts_injection() {
    assert!(process().accepts_injection());
}

/// The window this covers is the whole of `open_turn`: `initialize`,
/// `thread/start` or `thread/resume`, and `turn/start`, none of which is bounded
/// (see `PendingRequests`). Until they land there is no session, so a cancel
/// that could only reach one would be a no-op for exactly as long as the server
/// stayed silent. The handle exists from construction and is the token
/// `codex_app_server::start` is handed, so cancelling it kills the child
/// mid-handshake.
#[tokio::test]
async fn cancelling_before_the_session_opens_still_tears_the_server_down() {
    let mut process = process();
    let handle = process.cancel_handle();
    assert!(
        !handle.is_cancelled(),
        "a fresh process is not already cancelled"
    );

    process.cancel().await;

    assert!(
        handle.is_cancelled(),
        "a cancel with no session yet must still fire the handle open_turn passes to start()"
    );
}

// ── Steering a running Turn ──────────────────────────────────────

/// A process whose opening input has been submitted against an injected session
/// and whose thread is announced, with `turn/started` absorbed only when
/// `steerable` — the gate is what most of these tests are about.
async fn opened_process(session: AppServerSession, steerable: bool) -> CodexProcess {
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    p.submit(Input::text("in-1", "one"))
        .await
        .expect("submit the opening input");
    p.absorb(thread_started("thread-A"));
    if steerable {
        p.absorb(turn_started("turn-1"));
    }
    drain(&mut p);
    p
}

/// The steer request the harness sends, and the acceptance it reports for it.
/// `expectedTurnId` is a precondition the server checks: an empty or stale one
/// is rejected.
#[tokio::test]
async fn a_message_sent_while_the_turn_runs_is_steered_into_it() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, true).await;

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "reply with BANANA")).await;
        (p, outcome)
    });
    let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
    let (mut p, outcome) = steering.await.expect("the steering task joins");

    outcome.expect("the running Turn takes the message");
    assert_eq!(request["method"], "turn/steer");
    assert_eq!(request["params"]["threadId"], "thread-A");
    assert_eq!(request["params"]["expectedTurnId"], "turn-1");
    assert_eq!(request["params"]["input"][0]["text"], "reply with BANANA");
    assert_eq!(
        drain(&mut p),
        vec![ProcessEvent::InputAccepted {
            input: InputId("in-2".to_string()),
        }],
        "a steered message reports one bare acceptance and nothing else"
    );
}

/// The gate: a message that arrives before `turn/started` is not sent
/// optimistically — steering that early is rejected with `-32600 "no active turn
/// to steer"` — so it waits, and goes the moment the notification lands.
#[tokio::test]
async fn a_message_that_arrives_before_the_gate_opens_is_held_until_it_does() {
    let (tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, false).await;

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "held")).await;
        (p, outcome)
    });

    // Nothing may reach the server while the Turn is not steerable.
    let early = timeout(Duration::from_millis(100), server.next_request()).await;
    assert!(
        early.is_err(),
        "a message must not be steered before turn/started, got {early:?}"
    );

    tx.send(Ok(turn_started("turn-1"))).expect("feed");
    let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
    let (mut p, outcome) = steering.await.expect("the steering task joins");

    outcome.expect("the held message is delivered once the gate opens");
    assert_eq!(request["method"], "turn/steer");
    assert_eq!(request["params"]["expectedTurnId"], "turn-1");
    assert_eq!(
        drain(&mut p),
        vec![ProcessEvent::InputAccepted {
            input: InputId("in-2".to_string()),
        }]
    );
}

/// A Stop must not wait out the gate. `cancel` is unreachable while this
/// `submit` borrows the process, so the handle is what ends the wait — and the
/// message is handed back for the next Turn rather than dying with this one.
/// What fires that handle in production is the driver, which races it against
/// this call: `a_stop_reaches_the_driver_while_a_message_waits_on_the_gate`.
#[tokio::test]
async fn a_cancel_while_a_message_is_held_hands_it_back_at_once() {
    let (_tx, _server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, false).await;
    let handle = p.cancel_handle();

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "held")).await;
        (p, outcome)
    });
    // Let the submit reach the gate before cancelling, so this is the wait being
    // interrupted and not a cancel that was already set.
    tokio::task::yield_now().await;
    handle.cancel();

    let (p, outcome) = timeout(Duration::from_secs(1), steering)
        .await
        .expect("a cancelled wait ends promptly")
        .expect("the steering task joins");
    assert!(
        outcome.is_err(),
        "a cancelled Turn cannot take the message, so the driver must get it back"
    );
    assert!(p.held.is_none(), "nothing may stay held after a failure");
}

/// The bound on the wait. `turn/started` follows `turn/start` immediately in a
/// healthy Turn, so this only fires when the server has gone quiet — and then the
/// message becomes the next Turn rather than waiting on a notification that is
/// never coming.
///
/// What makes the timeout safe is that it can only fire *before* anything was
/// sent, which is why this asserts the wire stayed empty: a message the server
/// might have taken cannot also be handed back, or it is answered twice.
#[tokio::test(start_paused = true)]
async fn a_gate_that_never_opens_hands_the_message_back_having_sent_nothing() {
    // `_tx` stays alive: dropping it would end the stream, which is a different
    // reason to give up than the one under test.
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, false).await;

    let outcome = p.submit(Input::text("in-2", "held")).await;

    assert!(
        outcome.is_err(),
        "a gate that never opens hands the message back"
    );
    assert!(p.held.is_none(), "nothing may stay held after a failure");
    let sent = timeout(Duration::from_millis(100), server.next_request()).await;
    assert!(
        sent.is_err(),
        "the timeout may only fire before anything was written, got {sent:?}"
    );
}

/// The timeout bounds the gate, not the request. Once `turn/steer` is on the
/// wire the server may accept it at any moment, so abandoning it would hand the
/// driver a message the Turn had in fact taken — and it would be acted on twice,
/// once inside this Turn and once as the next one. A steer waits as long as the
/// Turn does; the stream ending is what releases it.
#[tokio::test(start_paused = true)]
async fn a_steer_already_on_the_wire_is_never_abandoned_for_the_timeout() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = opened_process(session, true).await;

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "reply with BANANA")).await;
        (p, outcome)
    });
    let request = server.next_request().await;
    assert_eq!(request["method"], "turn/steer");

    // Long past the bound, and the server has still not answered.
    tokio::time::advance(MID_TURN_DELIVERY_TIMEOUT * 4).await;
    server
        .write(json!({ "id": request["id"], "result": { "turnId": "turn-1" } }))
        .await;

    let (mut p, outcome) = steering.await.expect("the steering task joins");
    outcome.expect("a steer the server took is taken, however long it took to answer");
    assert_eq!(
        drain(&mut p),
        vec![ProcessEvent::InputAccepted {
            input: InputId("in-2".to_string()),
        }]
    );
}

/// Usage is the growth of the Session's cumulative total while the Turn ran, so a
/// Turn that a message was steered into bills the model requests the steer caused
/// as well as its own. The baseline is the reading replayed before `turn/started`.
#[tokio::test]
async fn a_steered_turn_bills_every_model_request_it_made() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    p.submit(Input::text("in-1", "one")).await.expect("submit");
    p.absorb(thread_started("thread-A"));
    // A resumed thread replays what the Session had already spent.
    p.absorb(token_usage(1000, 0, 100, 0));
    p.absorb(turn_started("turn-1"));
    // The Turn's own first model request.
    p.absorb(token_usage(1400, 40, 140, 20));
    drain(&mut p);

    let steering = tokio::spawn(async move {
        let outcome = p.submit(Input::text("in-2", "and BANANA")).await;
        (p, outcome)
    });
    server.answer_next(json!({ "turnId": "turn-1" })).await;
    let (mut p, outcome) = steering.await.expect("the steering task joins");
    outcome.expect("steered");

    // The request the steered message caused, then the terminal.
    p.absorb(token_usage(1900, 80, 260, 60));
    p.absorb(turn_completed("completed", None));
    let events = drain(&mut p);

    let usage = sources(&events)
        .last()
        .expect("the turn-result block")
        .usage
        .clone()
        .expect("usage on the turn-result block");
    assert_eq!(usage["input_tokens"], 900, "1900 - 1000");
    assert_eq!(usage["cache_read_input_tokens"], 80, "80 - 0");
    assert_eq!(usage["output_tokens"], 160, "260 - 100");
    assert_eq!(usage["reasoning_output_tokens"], 60, "60 - 0");
}

/// Spawn the shared one-shot driver over Codex processes backed by scripted
/// sessions (one per Turn), returning the command channels plus the per-Turn
/// event senders so a test can feed server events deterministically.
fn scripted_driver(
    sessions: Vec<AppServerSession>,
) -> (
    mpsc::Sender<Input>,
    mpsc::Sender<()>,
    mpsc::Receiver<Signal>,
) {
    let (submit_tx, submit_rx) = mpsc::channel(16);
    let (cancel_tx, cancel_rx) = mpsc::channel(1);
    let (signal_tx, signal_rx) = mpsc::channel(64);

    let cfg = config();
    let mut queue = std::collections::VecDeque::from(sessions);
    let factory = move || {
        let session = queue.pop_front().expect("a session per Turn");
        CodexProcess::with_session(cfg.clone(), session, "gpt-5")
    };
    tokio::spawn(super::super::driver::run(
        factory, submit_rx, cancel_rx, signal_tx,
    ));

    (submit_tx, cancel_tx, signal_rx)
}

async fn next_signal(rx: &mut mpsc::Receiver<Signal>) -> Signal {
    timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("signal within 2s")
        .expect("harness still open")
}

#[tokio::test]
async fn fresh_turn_orders_started_accepted_source_finished() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(thread_started("thread-A"))).expect("feed");
    tx.send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_0", "text": "4" }),
    )))
    .expect("feed");
    tx.send(Ok(turn_completed("completed", None)))
        .expect("feed");

    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { session, cause: Cause::Input(id) }
            if session.0 == "thread-A" && id.0 == "in-1"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));
    // The agent block, then the usage-bearing turn-result block.
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));
}

#[tokio::test]
async fn queued_input_opens_the_next_turn_and_is_accepted_against_it() {
    let (tx1, _s1, session1) = AppServerSession::scripted();
    let (tx2, _s2, session2) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session1, session2]);

    // First Turn starts.
    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx1.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-1"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));

    // Queue a second input while the first Turn is active.
    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");

    // First Turn terminates; its result block then a Completed terminal.
    tx1.send(Ok(turn_completed("completed", None)))
        .expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionFinished {
            outcome: Outcome::Completed
        }
    ));

    // Only now does the queued input open a replacement Turn and get accepted.
    tx2.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
}

/// The whole point of the change, at the driver seam: a message submitted while
/// the execution is open joins it. One `ExecutionStarted`, two acceptances, and
/// no second `SessionReady` — which is what keeps a steered message on the
/// running Turn's Run.
#[tokio::test]
async fn a_message_submitted_mid_execution_joins_it_and_opens_no_second_one() {
    let (tx, mut server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx.send(Ok(thread_started("thread-A"))).expect("feed");
    tx.send(Ok(turn_started("turn-1"))).expect("feed");
    tx.send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_0", "text": "working" }),
    )))
    .expect("feed");

    let mut signals = vec![
        next_signal(&mut signal_rx).await,
        next_signal(&mut signal_rx).await,
        // The block proves the Turn is running and `turn/started` was absorbed,
        // so the next submit takes the steer path rather than the held one.
        next_signal(&mut signal_rx).await,
    ];

    submit_tx
        .send(Input::text("in-2", "and BANANA"))
        .await
        .expect("submit 2");
    let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
    assert_eq!(request["method"], "turn/steer");

    tx.send(Ok(turn_completed("completed", None)))
        .expect("feed");
    loop {
        let signal = next_signal(&mut signal_rx).await;
        let terminal = matches!(signal, Signal::ExecutionFinished { .. });
        signals.push(signal);
        if terminal {
            break;
        }
    }

    let starts = signals
        .iter()
        .filter(|s| matches!(s, Signal::ExecutionStarted { .. }))
        .count();
    assert_eq!(starts, 1, "the steered message opens no second execution");
    let accepted: Vec<&str> = signals
        .iter()
        .filter_map(|s| match s {
            Signal::InputAccepted { input } => Some(input.0.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(accepted, vec!["in-1", "in-2"]);
    assert!(matches!(
        signals.last(),
        Some(Signal::ExecutionFinished {
            outcome: Outcome::Completed
        })
    ));
}

/// Messages that arrive inside the steerability window are held and delivered in
/// submit order once `turn/started` lands, one acceptance each. The window is
/// real — `turn/start`'s response is too early to steer — and a submit can land
/// in it.
#[tokio::test]
async fn messages_held_for_the_gate_are_delivered_in_submit_order() {
    let (tx, mut server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));

    // Both arrive while the Turn is still opening: nothing may be steered yet.
    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");
    submit_tx
        .send(Input::text("in-3", "three"))
        .await
        .expect("submit 3");
    let early = timeout(Duration::from_millis(100), server.next_request()).await;
    assert!(early.is_err(), "nothing may be steered before turn/started");

    tx.send(Ok(turn_started("turn-1"))).expect("feed");
    for _ in 0..2 {
        let request = server.answer_next(json!({ "turnId": "turn-1" })).await;
        assert_eq!(request["method"], "turn/steer");
    }

    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-3"
    ));
}

/// A Stop issued while a message is waiting on the gate lands then, not when the
/// wait gives up. The driver's injection arm is the only reader of `cancel_rx`,
/// so unless it races the Stop against `submit`, nothing can fire the process's
/// handle and the wait runs to its bound with the Stop unread behind it. The
/// message survives the Stop and opens the next Turn.
///
/// The notifications here come over the pipe rather than through `tx`, and `tx`
/// is dropped: then the reader task holds the only sender, and tearing the
/// session down ends the stream exactly as a dying server would. A test that
/// kept `tx` would have to close the stream by hand, which would end the gate
/// wait on its own and prove nothing about the Stop.
#[tokio::test]
async fn a_stop_reaches_the_driver_while_a_message_waits_on_the_gate() {
    let (tx1, mut server1, session1) = AppServerSession::scripted();
    let (tx2, _server2, session2) = AppServerSession::scripted();
    let (submit_tx, cancel_tx, mut signal_rx) = scripted_driver(vec![session1, session2]);
    drop(tx1);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    server1
        .write(json!({
            "method": "thread/started",
            "params": { "thread": { "id": "thread-A" } },
        }))
        .await;
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));

    // `turn/started` never arrives, so the driver is parked inside `submit`.
    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");
    let early = timeout(Duration::from_millis(100), server1.next_request()).await;
    assert!(early.is_err(), "nothing may be steered before turn/started");

    cancel_tx.send(()).await.expect("stop");

    // `next_signal` gives up after 2s, well inside the gate's own bound: a Stop
    // that only landed when the wait expired would not be seen here at all.
    assert!(
        matches!(
            next_signal(&mut signal_rx).await,
            Signal::ExecutionFinished {
                outcome: Outcome::Cancelled
            }
        ),
        "a Stop during a held message ends the execution without waiting out the gate"
    );

    // And the message it interrupted is still owed an answer.
    tx2.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
}

/// Every steer rejection is the same `-32600` with a different sentence, so the
/// harness never reads the sentence: any rejection means this Turn cannot take
/// the message, and it opens the next one instead. The running execution carries
/// on untouched.
#[tokio::test]
async fn a_rejected_steer_leaves_the_message_for_the_next_turn() {
    let (tx1, mut server1, session1) = AppServerSession::scripted();
    let (tx2, _server2, session2) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session1, session2]);

    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx1.send(Ok(thread_started("thread-A"))).expect("feed");
    tx1.send(Ok(turn_started("turn-1"))).expect("feed");
    tx1.send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_0", "text": "working" }),
    )))
    .expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-1"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::Source(_)
    ));

    submit_tx
        .send(Input::text("in-2", "two"))
        .await
        .expect("submit 2");
    let request = server1.next_request().await;
    assert_eq!(request["method"], "turn/steer");
    server1
        .write(json!({
            "id": request["id"],
            "error": { "code": -32600, "message": "no active turn to steer" },
        }))
        .await;

    // The running execution is undisturbed: it finishes on its own terms, with
    // no acceptance of the refused message against it.
    tx1.send(Ok(item_completed(
        json!({ "type": "agentMessage", "id": "item_1", "text": "done" }),
    )))
    .expect("feed");
    tx1.send(Ok(turn_completed("completed", None)))
        .expect("feed");
    loop {
        match next_signal(&mut signal_rx).await {
            Signal::Source(_) => continue,
            Signal::ExecutionFinished { outcome } => {
                assert_eq!(outcome, Outcome::Completed);
                break;
            }
            other => panic!("the refused message must not join this Turn: {other:?}"),
        }
    }

    // And it opens the next Turn on the same thread instead of being lost.
    tx2.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { cause: Cause::Input(id), .. } if id.0 == "in-2"
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { input } if input.0 == "in-2"
    ));
}

#[tokio::test]
async fn cancellation_finishes_the_turn_as_cancelled() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    // Request cancellation, then close the stream (the real server's teardown
    // ends it). Biased cancel is observed before the EOF.
    cancel_tx.send(()).await.expect("cancel");
    drop(tx);

    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionFinished {
            outcome: Outcome::Cancelled
        }
    ));
}

/// An interrupted Turn reports `Completed`; it is the driver, which knows it
/// asked, that turns that into a cancelled outcome.
#[tokio::test]
async fn a_turn_interrupted_after_cancel_still_finishes_as_cancelled() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    cancel_tx.send(()).await.expect("cancel");
    tx.send(Ok(turn_completed("interrupted", None)))
        .expect("feed");

    loop {
        match next_signal(&mut signal_rx).await {
            Signal::Source(_) => continue,
            Signal::ExecutionFinished { outcome } => {
                assert_eq!(outcome, Outcome::Cancelled);
                break;
            }
            other => panic!("unexpected signal {other:?}"),
        }
    }
}

/// Cancel asks the Turn to stop before killing the server, so Codex writes its
/// rollout and the Session stays resumable.
#[tokio::test]
async fn cancel_interrupts_the_running_turn_first() {
    let (_tx, mut server, session) = AppServerSession::scripted();
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    p.thread_id = Some("thread-A".to_string());
    p.turn_id = Some("turn-1".to_string());

    let cancelling = tokio::spawn(async move {
        p.cancel().await;
        p
    });
    let request = server.answer_next(json!({})).await;
    assert_eq!(request["method"], "turn/interrupt");
    assert_eq!(request["params"]["threadId"], "thread-A");
    assert_eq!(request["params"]["turnId"], "turn-1");

    let p = cancelling.await.expect("cancel finishes");
    assert!(p.session_cancelled());
}

/// Before `turn/start` answers there is no Turn to interrupt, so tearing the
/// process down is the whole of cancellation.
#[tokio::test]
async fn cancel_tears_down_the_session() {
    let (_tx, _server, session) = AppServerSession::scripted();
    let mut p = CodexProcess::with_session(config(), session, "gpt-5");
    assert!(!p.session_cancelled());
    p.cancel().await;
    assert!(
        p.session_cancelled(),
        "cancel must tear down the codex process tree"
    );
}

#[tokio::test]
async fn eof_mid_turn_is_an_unexpected_exit() {
    let (tx, server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    // Stream closes with no terminal event → unexpected exit. The server end
    // goes too: the reader task holds its own handle on the event stream, and
    // it only lets go when the pipe reaches EOF, exactly as a dead subprocess
    // would.
    drop(tx);
    drop(server);
    match next_signal(&mut signal_rx).await {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(error),
        } => assert_eq!(error.class, ErrorClass::ProcessExit),
        other => panic!("expected failed terminal, got {other:?}"),
    }
}

/// A transport failure mid-Turn is a terminal, classified from the stream text.
#[tokio::test]
async fn a_stream_error_ends_the_turn_as_a_failure() {
    let (tx, _server, session) = AppServerSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(thread_started("thread-A"))).expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    tx.send(Err("Codex app-server exited with status 1".to_string()))
        .expect("feed");
    match next_signal(&mut signal_rx).await {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(error),
        } => assert_eq!(error.class, ErrorClass::ProcessExit),
        other => panic!("expected failed terminal, got {other:?}"),
    }
}

// ── Real-CLI E2E ─────────────────────────────────────────────────

/// Whether the real-CLI E2E tests are enabled. They spawn the actual `codex
/// app-server` subprocess and spend API credit, so they stay off unless asked
/// for.
fn skip_unless_e2e() -> bool {
    if std::env::var("CODEX_SDK_E2E").as_deref() == Ok("1") {
        return false;
    }
    eprintln!("skipping: set CODEX_SDK_E2E=1 to run");
    true
}

/// Config for a real-CLI E2E run: a throwaway workspace named `workspace_name`,
/// with the CLI path, credentials home and model taken from the environment so
/// the subprocess authenticates instead of hanging on an interactive login
/// against the throwaway test home.
fn e2e_config(workspace_name: &str) -> CodexConfig {
    let mut config = crate::config::test_support::minimal_config();
    let workspace = std::env::temp_dir().join(workspace_name);
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    config.workspace_dir = workspace.to_string_lossy().into_owned();
    config.repo_dir = config.workspace_dir.clone();
    if let Ok(path) = std::env::var("SANDBOX_CODEX_EXECUTABLE_PATH") {
        config.codex_cli_path = path;
    }
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        config.codex_home_dir = codex_home;
    }
    if let Ok(home) = std::env::var("HOME") {
        config.home_dir = home;
    }
    if let Ok(model) = std::env::var("DEFAULT_CODEX_MODEL") {
        config.default_codex_model = Some(model);
    }

    CodexConfig {
        config: Arc::new(config),
        credential_failures: CredentialFailureDetector::default(),
    }
}

/// Focused real-CLI E2E: launch the actual `codex app-server` subprocess through
/// the production [`Harness::codex`] seam and assert the external signal
/// ordering for a trivial Turn. Gated behind `CODEX_SDK_E2E=1` and `#[ignore]` —
/// the full E2E matrix runs at ticket 10, per the guide.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_codex_turn_through_the_harness() {
    if skip_unless_e2e() {
        return;
    }

    let mut harness = Harness::codex(e2e_config("harness-codex-e2e"));
    harness
        .submit(Input::text(
            "in-1",
            "Run the shell command `echo hello-from-codex` and then tell me its output.",
        ))
        .await
        .expect("submit input");

    let (mut started, mut accepted) = (false, false);
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut outcome = None;
    while let Some(signal) = harness.next_signal().await {
        match signal {
            Signal::ExecutionStarted { .. } => started = true,
            Signal::InputAccepted { .. } => accepted = true,
            Signal::Source(event) => {
                eprintln!(
                    "SOURCE role={:?} type={} name={} tool_use_result={}",
                    event.role,
                    event.block_json()["type"].as_str().unwrap_or("?"),
                    event.block_json()["name"].as_str().unwrap_or("-"),
                    event.tool_use_result.is_some(),
                );
                blocks.push(event.block_json());
            }
            Signal::ExecutionFinished { outcome: finished } => {
                outcome = Some(finished);
                break;
            }
            _ => {}
        }
    }

    assert!(started, "expected ExecutionStarted");
    assert!(accepted, "expected InputAccepted");
    assert!(!blocks.is_empty(), "expected at least one source block");
    assert!(
        matches!(outcome, Some(Outcome::Completed)),
        "expected Completed, got {outcome:?}"
    );

    // Every emitted block must be a canonical Anthropic-shaped block the UI can
    // render — never a raw native Codex type.
    let types: std::collections::HashSet<&str> =
        blocks.iter().filter_map(|b| b["type"].as_str()).collect();
    eprintln!("DISTINCT BLOCK TYPES: {types:?}");
    for raw in [
        "agentMessage",
        "commandExecution",
        "reasoning",
        "fileChange",
        // The exec transport's spellings, kept so the assertion still catches a
        // leak if anything reintroduces them.
        "agent_message",
        "command_execution",
        "file_change",
    ] {
        assert!(!types.contains(raw), "raw native block type leaked: {raw}");
    }
    for b in &blocks {
        let ty = b["type"].as_str().unwrap_or("");
        assert!(
            matches!(ty, "text" | "tool_use" | "tool_result" | "turn_result"),
            "non-canonical block type: {ty}"
        );
    }
    assert!(
        blocks.iter().any(|b| b["type"] == "text"),
        "expected at least one assistant text block"
    );
}

/// Everything one execution reported, folded out of the signal stream.
#[derive(Default)]
struct TurnLog {
    /// Thread id announced by `ExecutionStarted`.
    session: Option<String>,
    /// The input named as the execution's cause (`None` for a harness-internal one).
    cause: Option<InputId>,
    /// Every input accepted while this execution was open, in order. More than
    /// one would mean an input joined the running Turn.
    accepted: Vec<InputId>,
    block_types: Vec<String>,
    /// Whether the Turn had already committed a `tool_use` block when the
    /// mid-Turn message was submitted — the evidence that it was busy working,
    /// not idling at its start.
    tool_use_before_mid_turn: bool,
    /// All assistant/user text emitted by the Turn, newline-joined.
    text: String,
    outcome: Option<Outcome>,
}

/// Drain signals until the open execution reports its terminal, folding them
/// into a [`TurnLog`].
///
/// When `mid_turn` is `Some((id, text))`, that input is submitted the moment the
/// execution commits its first `tool_use` block — demonstrably while the Turn is
/// running and doing work, with its terminal still to come. It resumes the
/// thread the running Turn announced, which is what
/// `CoordinatorService::build_input` does for a follow-up Turn (the pod's last
/// known Session id wins).
async fn collect_turn(harness: &mut Harness, mid_turn: Option<(&str, &str)>) -> TurnLog {
    let mut log = TurnLog::default();
    let mut mid_turn = mid_turn;

    loop {
        let Some(signal) = harness.next_signal().await else {
            break;
        };
        match signal {
            Signal::ExecutionStarted { session, cause } => {
                eprintln!("EXECUTION STARTED session={} cause={cause:?}", session.0);
                log.session = Some(session.0);
                if let Cause::Input(id) = cause {
                    log.cause = Some(id);
                }
            }
            Signal::InputAccepted { input } => {
                eprintln!("INPUT ACCEPTED {}", input.0);
                log.accepted.push(input);
            }
            Signal::Source(event) => {
                let block = event.block_json();
                log.block_types
                    .push(block["type"].as_str().unwrap_or("?").to_string());
                if let Some(text) = block["text"].as_str() {
                    log.text.push_str(text);
                    log.text.push('\n');
                }

                if block["type"] == "tool_use" {
                    if let Some((id, text)) = mid_turn.take() {
                        eprintln!("SUBMITTING {id} while the Turn is running");
                        log.tool_use_before_mid_turn = true;
                        let input = Input {
                            resume: log.session.clone().map(ResumeSession::WithId),
                            ..Input::text(id, text)
                        };
                        harness.submit(input).await.expect("submit mid-Turn input");
                    }
                }
            }
            Signal::Activity(_) => {}
            Signal::ContextWindow(window) => {
                eprintln!("CONTEXT WINDOW {window:?}");
            }
            Signal::ExecutionFinished { outcome } => {
                eprintln!("EXECUTION FINISHED {outcome:?}");
                log.outcome = Some(outcome);
                break;
            }
            Signal::InputRejected { input, reason } => {
                panic!("input {} was rejected: {reason:?}", input.0)
            }
        }
    }

    log
}

/// Real-CLI E2E: a message sent while a Codex Turn is running is answered *in
/// that Turn*. This is the behaviour the whole change exists for.
///
/// The message is submitted the moment the Turn commits a `tool_use` block, so
/// it demonstrably lands while the Turn is working and its terminal is still to
/// come. `turn/steer` folds it in: one execution, two acceptances against it, no
/// second Turn, and the Turn's own answer reflects the message.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_message_sent_mid_turn_is_answered_inside_the_running_codex_turn() {
    if skip_unless_e2e() {
        return;
    }

    let result = timeout(Duration::from_mins(6), async {
        let mut harness = Harness::codex(e2e_config("harness-codex-mid-turn-e2e"));

        // Two separate commands keep the Turn demonstrably busy: the first
        // completes early so the test can submit while tool work is still in
        // flight, and the sleep holds the Turn open long enough for it to land.
        harness
            .submit(Input::text(
                "in-1",
                "Run two separate shell commands, not combined into one: first \
                 `echo step-one`, then `sleep 25`. After both have finished, reply \
                 with the single word ALPHA and nothing else.",
            ))
            .await
            .expect("submit the opening input");

        let turn = collect_turn(
            &mut harness,
            Some((
                "in-2",
                "Change of plan: reply with the single word BANANA and nothing else.",
            )),
        )
        .await;

        assert_eq!(
            turn.cause,
            Some(InputId("in-1".to_string())),
            "the Turn must be opened by the first input"
        );
        assert!(
            turn.tool_use_before_mid_turn,
            "the message was supposed to arrive while the Turn was doing tool work, \
             got blocks: {:?}",
            turn.block_types
        );
        assert_eq!(
            turn.accepted,
            vec![InputId("in-1".to_string()), InputId("in-2".to_string())],
            "the mid-Turn message must be accepted against the Turn already running"
        );
        assert_eq!(
            turn.outcome,
            Some(Outcome::Completed),
            "the steered Turn must still finish normally"
        );
        assert!(
            turn.text.to_uppercase().contains("BANANA"),
            "the running Turn must act on the message that arrived mid-Turn, got: {}",
            turn.text
        );

        // No second execution: the mid-Turn message opened no Turn of its own.
        // Nothing more is submitted, so the harness has nothing left to run and
        // any further execution would be one this change was meant to remove.
        let extra = timeout(Duration::from_secs(20), harness.next_signal()).await;
        assert!(
            matches!(extra, Err(_) | Ok(None)),
            "the steered message must not open a second execution, got {extra:?}"
        );
    })
    .await;

    result.expect("test timed out");
}

/// Real-CLI E2E: the other half of the contract. A message sent when no Turn is
/// running still opens a Turn of its own, on the Session the previous one used —
/// steering changed what happens *inside* a Turn, not what happens between them.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_message_sent_between_turns_opens_a_second_codex_turn() {
    if skip_unless_e2e() {
        return;
    }

    let result = timeout(Duration::from_mins(6), async {
        let mut harness = Harness::codex(e2e_config("harness-codex-between-turns-e2e"));

        harness
            .submit(Input::text(
                "in-1",
                "Reply with the single word ALPHA and nothing else.",
            ))
            .await
            .expect("submit the opening input");
        let first = collect_turn(&mut harness, None).await;

        assert_eq!(first.outcome, Some(Outcome::Completed));
        assert_eq!(first.accepted, vec![InputId("in-1".to_string())]);
        let session = first
            .session
            .clone()
            .expect("the first Turn announced a thread");

        // The first Turn has reported its terminal, so this one arrives between
        // Turns and resumes the thread it announced.
        harness
            .submit(Input {
                resume: Some(ResumeSession::WithId(session.clone())),
                ..Input::text(
                    "in-2",
                    "Reply with the single word BANANA and nothing else.",
                )
            })
            .await
            .expect("submit the follow-up input");
        let second = collect_turn(&mut harness, None).await;

        assert_eq!(
            second.cause,
            Some(InputId("in-2".to_string())),
            "a message sent between Turns opens the next Turn"
        );
        assert_eq!(second.accepted, vec![InputId("in-2".to_string())]);
        assert_eq!(second.outcome, Some(Outcome::Completed));
        assert_eq!(
            second.session,
            Some(session),
            "the second Turn must resume the same Session, not start a fresh one"
        );
        assert!(
            second.text.to_uppercase().contains("BANANA"),
            "the second Turn must answer its own message, got: {}",
            second.text
        );
    })
    .await;

    result.expect("test timed out");
}
