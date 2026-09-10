//! Fixture and framing tests for the app-server transport.
//!
//! The fixture tests are the cheapest proof the protocol types match the wire:
//! every file under `fixtures/` is a message a real server sent (or, under
//! `schema-derived/`, a minimal example built from the generated schema for a
//! family no probe could trigger), and each one must deserialize into the type
//! the client would pick for it.

use std::collections::HashSet;

use tokio_util::sync::CancellationToken;

use super::super::protocol::*;
use super::*;

// ── Fixtures ─────────────────────────────────────────────────────────

/// Every fixture, as `(relative path, contents)`.
///
/// `include_str!` rather than a directory walk: the fixtures are compiled in, so
/// a file that stops parsing fails the build of the test, and the list of what
/// exists is reviewable in one place.
macro_rules! fixtures {
    ($($group:literal / $name:literal),* $(,)?) => {
        &[$((
            concat!($group, "/", $name),
            include_str!(concat!("fixtures/", $group, "/", $name, ".json")),
        )),*]
    };
}

const OBSERVED: &[(&str, &str)] = fixtures![
    "observed" / "response_initialize",
    "observed" / "response_thread_start",
    "observed" / "response_thread_resume",
    "observed" / "response_turn_start",
    "observed" / "response_turn_steer",
    "observed" / "response_turn_interrupt",
    "observed" / "error_steer_no_active_turn",
    "observed" / "error_steer_empty_expected_turn_id",
    "observed" / "notification_thread_started",
    "observed" / "notification_turn_started",
    "observed" / "notification_turn_completed",
    "observed" / "notification_turn_completed_interrupted",
    "observed" / "notification_turn_completed_failed",
    "observed" / "notification_error",
    "observed" / "notification_thread_token_usage_updated",
    "observed" / "notification_turn_plan_updated",
    "observed" / "notification_item_started",
    "observed" / "item_user_message",
    "observed" / "item_agent_message",
    "observed" / "item_reasoning",
    "observed" / "item_command_execution",
    "observed" / "item_file_change_add",
    "observed" / "item_file_change_update",
    "observed" / "item_web_search",
];

const SCHEMA_DERIVED: &[(&str, &str)] = fixtures![
    "schema-derived" / "item_hook_prompt",
    "schema-derived" / "item_plan",
    "schema-derived" / "item_mcp_tool_call",
    "schema-derived" / "item_dynamic_tool_call",
    "schema-derived" / "item_collab_agent_tool_call",
    "schema-derived" / "item_sub_agent_activity",
    "schema-derived" / "item_image_view",
    "schema-derived" / "item_sleep",
    "schema-derived" / "item_image_generation",
    "schema-derived" / "item_entered_review_mode",
    "schema-derived" / "item_exited_review_mode",
    "schema-derived" / "item_context_compaction",
    "schema-derived" / "item_unknown_family",
];

fn message(source: &str) -> IncomingMessage {
    serde_json::from_str(source)
        .unwrap_or_else(|e| panic!("fixture is not a JSON-RPC message: {e}"))
}

/// The type a response fixture is decoded into, keyed by the request it answered.
fn decode_response(name: &str, result: serde_json::Value) {
    let outcome = match name {
        "observed/response_initialize" => {
            serde_json::from_value::<InitializeResponse>(result).map(|_| ())
        }
        "observed/response_thread_start" | "observed/response_thread_resume" => {
            serde_json::from_value::<ThreadEnvelope>(result).map(|_| ())
        }
        "observed/response_turn_start" => {
            serde_json::from_value::<TurnStartResponse>(result).map(|_| ())
        }
        "observed/response_turn_steer" => {
            serde_json::from_value::<TurnSteerResponse>(result).map(|_| ())
        }
        // `turn/interrupt` answers with an empty object.
        "observed/response_turn_interrupt" => {
            assert_eq!(result, serde_json::json!({}));
            Ok(())
        }
        other => panic!("no protocol type registered for response fixture {other}"),
    };
    outcome.unwrap_or_else(|e| panic!("{name} did not decode: {e}"));
}

#[test]
fn every_fixture_deserializes_into_its_protocol_type() {
    for (name, source) in OBSERVED.iter().chain(SCHEMA_DERIVED) {
        let message = message(source);
        match (message.method, message.error, message.result) {
            (Some(method), _, _) => {
                let params = message.params.unwrap_or(serde_json::Value::Null);
                let event = ServerEvent::parse(&method, params)
                    .unwrap_or_else(|e| panic!("{name} ({method}) did not decode: {e}"));
                assert!(event.is_some(), "{name}: {method} is not a consumed method");
            }
            (None, Some(_), _) => {
                // A JSON-RPC error object; already typed by `IncomingMessage`.
            }
            (None, None, Some(result)) => decode_response(name, result),
            (None, None, None) => panic!("{name} is neither a notification nor a response"),
        }
    }
}

/// `CodexProcess::absorb` tells this Turn's stream from a native sub-agent's by
/// the thread each notification names, and fails open on one that names none. A
/// CLI that stopped sending `threadId` would therefore restore the bug silently,
/// so every observed notification is pinned to carrying a thread.
#[test]
fn every_observed_notification_names_its_thread() {
    for (name, source) in OBSERVED {
        let message = message(source);
        let Some(method) = message.method else {
            continue;
        };
        let params = message.params.unwrap_or(serde_json::Value::Null);
        let event = ServerEvent::parse(&method, params)
            .unwrap_or_else(|e| panic!("{name} ({method}) did not decode: {e}"))
            .unwrap_or_else(|| panic!("{name}: {method} is not a consumed method"));
        assert!(
            event.thread_id().is_some(),
            "{name} ({method}) names no thread"
        );
    }
}

#[test]
fn observed_and_schema_derived_fixtures_stay_separate() {
    for (name, _) in OBSERVED {
        assert!(
            name.starts_with("observed/"),
            "{name} is in the wrong group"
        );
    }
    for (name, _) in SCHEMA_DERIVED {
        assert!(
            name.starts_with("schema-derived/"),
            "{name} is in the wrong group"
        );
    }
}

/// The eighteen families the wire can deliver, each covered by exactly one
/// item fixture. Ticket 02's mapping is written against this set.
#[test]
fn every_item_family_has_a_fixture() {
    let mut families = HashSet::new();
    for (name, source) in OBSERVED.iter().chain(SCHEMA_DERIVED) {
        let message = message(source);
        if message.method.as_deref() != Some("item/completed") {
            continue;
        }
        let params: ItemNotification = serde_json::from_value(message.params.expect("params"))
            .unwrap_or_else(|e| panic!("{name} is not an item notification: {e}"));
        families.insert(params.item.type_name().to_string());
    }

    let expected: HashSet<String> = [
        "userMessage",
        "hookPrompt",
        "agentMessage",
        "plan",
        "reasoning",
        "commandExecution",
        "fileChange",
        "mcpToolCall",
        "dynamicToolCall",
        "collabAgentToolCall",
        "subAgentActivity",
        "webSearch",
        "imageView",
        "sleep",
        "imageGeneration",
        "enteredReviewMode",
        "exitedReviewMode",
        "contextCompaction",
        // The forward-compatibility fallthrough, which keeps its wire name.
        "brandNewThing",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    assert_eq!(families, expected);
}

fn item_from(source: &str) -> ThreadItem {
    let message = message(source);
    let params: ItemNotification =
        serde_json::from_value(message.params.expect("params")).expect("item notification");
    params.item
}

#[test]
fn observed_command_execution_carries_output_and_exit_code() {
    let ThreadItem::CommandExecution(cmd) = item_from(include_str!(
        "fixtures/observed/item_command_execution.json"
    )) else {
        panic!("expected commandExecution");
    };
    assert_eq!(cmd.command, "/bin/bash -lc 'echo hello-from-codex'");
    assert_eq!(cmd.aggregated_output.as_deref(), Some("hello-from-codex\n"));
    assert_eq!(cmd.exit_code, Some(0));
    assert_eq!(cmd.status, CommandExecutionStatus::Completed);
    assert_eq!(cmd.source, Some(CommandExecutionSource::UnifiedExecStartup));
}

#[test]
fn observed_file_change_kinds_are_internally_tagged() {
    let ThreadItem::FileChange(add) =
        item_from(include_str!("fixtures/observed/item_file_change_add.json"))
    else {
        panic!("expected fileChange");
    };
    assert_eq!(add.status, PatchApplyStatus::Completed);
    assert_eq!(add.changes[0].kind, PatchChangeKind::Add);

    let ThreadItem::FileChange(update) = item_from(include_str!(
        "fixtures/observed/item_file_change_update.json"
    )) else {
        panic!("expected fileChange");
    };
    assert_eq!(
        update.changes[0].kind,
        PatchChangeKind::Update { move_path: None }
    );
}

#[test]
fn steered_input_arrives_as_a_user_message_item() {
    let ThreadItem::UserMessage(message) =
        item_from(include_str!("fixtures/observed/item_user_message.json"))
    else {
        panic!("expected userMessage");
    };
    let [UserInput::Text { text }] = message.content.as_slice() else {
        panic!("expected a single text input");
    };
    assert!(text.contains("BANANA"), "{text}");
}

#[test]
fn unknown_item_family_keeps_its_name_and_payload() {
    let item = item_from(include_str!(
        "fixtures/schema-derived/item_unknown_family.json"
    ));
    let ThreadItem::Other { type_name, id, raw } = &item else {
        panic!("expected Other, got {item:?}");
    };
    assert_eq!(type_name, "brandNewThing");
    assert_eq!(id, "new_0");
    assert_eq!(raw["foo"], "bar");
}

#[test]
fn a_failed_turn_carries_a_named_error() {
    let message = message(include_str!(
        "fixtures/observed/notification_turn_completed_failed.json"
    ));
    let Some(ServerEvent::TurnCompleted(completed)) =
        ServerEvent::parse("turn/completed", message.params.expect("params")).expect("parse")
    else {
        panic!("expected turn/completed");
    };
    assert_eq!(completed.turn.status, TurnStatus::Failed);
    let error = completed.turn.error.expect("failed turn carries an error");
    assert_eq!(error.codex_error_info, Some(CodexErrorInfo::Other));
}

#[test]
fn an_interrupted_turn_is_not_a_failed_one() {
    let message = message(include_str!(
        "fixtures/observed/notification_turn_completed_interrupted.json"
    ));
    let Some(ServerEvent::TurnCompleted(completed)) =
        ServerEvent::parse("turn/completed", message.params.expect("params")).expect("parse")
    else {
        panic!("expected turn/completed");
    };
    assert_eq!(completed.turn.status, TurnStatus::Interrupted);
    assert!(completed.turn.error.is_none());
}

/// The terminal is a terminal, not a replay: content has to come from the
/// streamed `item/completed` notifications.
#[test]
fn turn_completed_carries_no_items() {
    let message = message(include_str!(
        "fixtures/observed/notification_turn_completed.json"
    ));
    let Some(ServerEvent::TurnCompleted(completed)) =
        ServerEvent::parse("turn/completed", message.params.expect("params")).expect("parse")
    else {
        panic!("expected turn/completed");
    };
    assert_eq!(completed.turn.items_view, TurnItemsView::NotLoaded);
    assert!(completed.turn.items.is_empty());
}

#[test]
fn token_usage_carries_the_four_fields_the_turn_result_publishes() {
    let message = message(include_str!(
        "fixtures/observed/notification_thread_token_usage_updated.json"
    ));
    let Some(ServerEvent::TokenUsage(usage)) =
        ServerEvent::parse("thread/tokenUsage/updated", message.params.expect("params"))
            .expect("parse")
    else {
        panic!("expected thread/tokenUsage/updated");
    };
    let last = usage.token_usage.last;
    assert_eq!(last.input_tokens, 15888);
    assert_eq!(last.cached_input_tokens, 0);
    assert_eq!(last.output_tokens, 147);
    assert_eq!(last.reasoning_output_tokens, 41);
}

#[test]
fn the_running_plan_arrives_as_a_notification_not_an_item() {
    let message = message(include_str!(
        "fixtures/observed/notification_turn_plan_updated.json"
    ));
    let Some(ServerEvent::PlanUpdated(plan)) =
        ServerEvent::parse("turn/plan/updated", message.params.expect("params")).expect("parse")
    else {
        panic!("expected turn/plan/updated");
    };
    assert_eq!(plan.plan.len(), 4);
    assert_eq!(plan.plan[0].status, TurnPlanStepStatus::InProgress);
    assert_eq!(plan.plan[1].status, TurnPlanStepStatus::Pending);
}

/// Every steer rejection uses `-32600` with a different sentence, so the code is
/// all a caller may read.
#[test]
fn steer_rejections_are_all_the_same_code() {
    for source in [
        include_str!("fixtures/observed/error_steer_no_active_turn.json"),
        include_str!("fixtures/observed/error_steer_empty_expected_turn_id.json"),
    ] {
        let error = message(source).error.expect("a JSON-RPC error object");
        assert_eq!(error.code, -32600);
        assert!(error.data.is_none());
    }
}

#[test]
fn ignored_notifications_are_not_events() {
    for method in [
        "configWarning",
        "remoteControl/status/changed",
        "thread/status/changed",
        "account/rateLimits/updated",
        "item/agentMessage/delta",
        "turn/diff/updated",
    ] {
        let event = ServerEvent::parse(method, serde_json::json!({"anything": true }))
            .unwrap_or_else(|e| panic!("{method} should be ignored, not fail: {e}"));
        assert!(event.is_none(), "{method} should be ignored");
    }
}

// ── codexErrorInfo ───────────────────────────────────────────────────

fn error_info(value: serde_json::Value) -> CodexErrorInfo {
    serde_json::from_value(value).expect("codexErrorInfo")
}

#[test]
fn codex_error_info_covers_string_and_object_variants() {
    assert_eq!(
        error_info(serde_json::json!("unauthorized")),
        CodexErrorInfo::Unauthorized
    );
    assert_eq!(
        error_info(serde_json::json!("usageLimitExceeded")),
        CodexErrorInfo::UsageLimitExceeded
    );
    assert_eq!(
        error_info(serde_json::json!({"httpConnectionFailed": {"httpStatusCode": 503}})),
        CodexErrorInfo::HttpConnectionFailed {
            http_status_code: Some(503)
        }
    );
    assert_eq!(
        error_info(serde_json::json!({"responseStreamConnectionFailed": {}})),
        CodexErrorInfo::ResponseStreamConnectionFailed {
            http_status_code: None
        }
    );
    assert_eq!(
        error_info(serde_json::json!({"activeTurnNotSteerable": {"turnKind": "review"}})),
        CodexErrorInfo::ActiveTurnNotSteerable {
            turn_kind: NonSteerableTurnKind::Review
        }
    );
}

/// A code this build has never seen must survive with its raw value intact so it
/// can be logged, not collapse silently into the default class.
#[test]
fn unrecognized_codex_error_info_keeps_its_raw_value() {
    assert_eq!(
        error_info(serde_json::json!("brandNewFailure")),
        CodexErrorInfo::Unrecognized(serde_json::json!("brandNewFailure"))
    );
    let object = serde_json::json!({"brandNewFailure": {"httpStatusCode": 418}});
    assert_eq!(
        error_info(object.clone()),
        CodexErrorInfo::Unrecognized(object)
    );
}

// ── JSON-RPC framing ─────────────────────────────────────────────────

/// The request half of a scripted connection. [`ScriptedServer`] and the pipe
/// itself live in the parent module so the harness's driver tests share them.
fn scripted() -> (
    Arc<Rpc>,
    mpsc::UnboundedReceiver<Result<ServerEvent, String>>,
    ScriptedServer,
) {
    let (rpc, _tx, rx, server, _cancel) = scripted_pipe();
    (rpc, rx, server)
}

/// Two requests in flight, answered out of order and interleaved with
/// notifications: each waiter must get its own response.
#[tokio::test]
async fn interleaved_responses_and_notifications_reach_the_right_waiter() {
    let (rpc, mut events, mut server) = scripted();

    let first = {
        let rpc = Arc::clone(&rpc);
        tokio::spawn(async move { rpc.request("turn/start", serde_json::json!({})).await })
    };
    let a = server.next_request().await;
    let second = {
        let rpc = Arc::clone(&rpc);
        tokio::spawn(async move { rpc.request("turn/steer", serde_json::json!({})).await })
    };
    let b = server.next_request().await;
    assert_ne!(a["id"], b["id"]);

    server
        .write(serde_json::json!({
            "method": "turn/started",
            "params": {"threadId": "t", "turn": {"id": "turn-1", "status": "inProgress", "items": []}},
        }))
        .await;
    // Answer the second request first.
    server
        .write(serde_json::json!({"id": b["id"], "result": {"turnId": "turn-1"}}))
        .await;
    server
        .write(serde_json::json!({
            "id": a["id"],
            "result": {"turn": {"id": "turn-1", "status": "inProgress", "items": []}},
        }))
        .await;

    let first = first.await.expect("join").expect("turn/start result");
    assert_eq!(first["turn"]["id"], "turn-1");
    let second = second.await.expect("join").expect("turn/steer result");
    assert_eq!(second["turnId"], "turn-1");

    let event = events.recv().await.expect("event").expect("ok");
    assert!(matches!(event, ServerEvent::TurnStarted(_)));
}

#[test]
fn user_input_serializes_as_the_measured_text_shape() {
    assert_eq!(
        serde_json::to_value(UserInput::text("hello")).expect("serialize"),
        serde_json::json!({"type": "text", "text": "hello"})
    );
}

#[tokio::test]
async fn a_json_rpc_error_response_becomes_an_rpc_error() {
    let (rpc, _events, mut server) = scripted();
    let pending = {
        let rpc = Arc::clone(&rpc);
        tokio::spawn(async move { rpc.request("turn/steer", serde_json::json!({})).await })
    };
    let request = server.next_request().await;
    server
        .write(serde_json::json!({
            "id": request["id"],
            "error": {"code": -32600, "message": "no active turn to steer"},
        }))
        .await;

    match pending.await.expect("join") {
        Err(RpcError::Rpc(info)) => {
            assert_eq!(info.code, -32600);
            assert_eq!(info.message, "no active turn to steer");
        }
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

/// An unanswered server request can stall the Turn, so anything unrecognised is
/// refused rather than ignored.
#[tokio::test]
async fn a_server_request_is_always_answered() {
    let (_rpc, _events, mut server) = scripted();
    server
        .write(serde_json::json!({
            "jsonrpc": "2.0",
            "id": "srv-1",
            "method": "item/commandExecution/requestApproval",
            "params": {},
        }))
        .await;

    let reply = server.next_request().await;
    assert_eq!(reply["id"], "srv-1");
    assert_eq!(reply["error"]["code"], -32601);
}

/// A request in flight when the stream ends must fail, not hang.
#[tokio::test]
async fn a_closed_stream_fails_the_requests_in_flight() {
    let (rpc, _events, mut server) = scripted();
    let pending = {
        let rpc = Arc::clone(&rpc);
        tokio::spawn(async move { rpc.request("turn/start", serde_json::json!({})).await })
    };
    server.next_request().await;
    drop(server);

    match pending.await.expect("join") {
        Err(RpcError::Transport(message)) => assert!(message.contains("turn/start"), "{message}"),
        other => panic!("expected a transport error, got {other:?}"),
    }
}

/// Losing the handle has to kill the server: an idle session that is simply
/// dropped, or a handshake that fails after `start` spawned the child, must not
/// leave `codex app-server` running with nobody left to talk to it.
#[tokio::test]
async fn dropping_a_session_tears_the_server_down() {
    let (client_read, _server_write) = tokio::io::duplex(1024);
    let (_server_read, client_write) = tokio::io::duplex(1024);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let cancel_token = CancellationToken::new();
    let rpc = connect(client_read, client_write, cancel_token.clone(), events_tx);
    let session = AppServerSession {
        notifications: events_rx,
        rpc,
        cancel_token: cancel_token.clone(),
    };
    assert!(!cancel_token.is_cancelled());

    drop(session);

    assert!(cancel_token.is_cancelled());
}

/// A frame that is not JSON means the stream is no longer trustworthy.
#[tokio::test]
async fn an_unparseable_frame_reports_a_framing_error() {
    let (_rpc, mut events, mut server) = scripted();
    server.writer.write_all(b"not json\n").await.expect("write");
    server.writer.flush().await.expect("flush");

    let error = events.recv().await.expect("event").expect_err("an error");
    assert!(error.contains("Failed to parse"), "{error}");
}

// ── Spawn parity ─────────────────────────────────────────────────────

fn parity_config() -> crate::config::Config {
    let mut config = crate::config::test_support::minimal_config();
    config.workspace_dir = "/workspace".to_string();
    config
}

fn parity_request() -> AgentRunRequest {
    AgentRunRequest {
        content: vec![],
        model: Some("gpt-test".to_string()),
        system_prompt: None,
        resume: None,
        agent_type: None,
        agent_mode: None,
        effort: None,
    }
}

/// The `codex exec` flags that have no app-server flag become thread params.
#[test]
fn thread_params_carry_the_exec_spawn_parity_set() {
    let config = parity_config();
    let params = thread_start_params(&config, &parity_request());
    assert_eq!(params.cwd.as_deref(), Some("/workspace"));
    assert_eq!(params.model.as_deref(), Some("gpt-test"));
    assert_eq!(params.sandbox, Some(SandboxMode::DangerFullAccess));
    assert_eq!(params.approval_policy, Some(AskForApproval::Never));
    assert!(params.developer_instructions.is_some());

    let resumed = thread_resume_params(&config, &parity_request(), "thread-1");
    assert_eq!(resumed.thread_id, "thread-1");
    assert_eq!(resumed.sandbox, Some(SandboxMode::DangerFullAccess));

    let turn = turn_start_params("thread-1", "do the thing", &config, &parity_request());
    assert_eq!(turn.effort.as_deref(), Some("high"));
    assert!(matches!(
        turn.input.as_slice(),
        [UserInput::Text { text }] if text == "do the thing"
    ));
}

/// A Turn that asked for an effort runs at it; one that asked for none keeps
/// the default asserted above.
#[test]
fn turn_params_carry_the_requested_effort() {
    let mut request = parity_request();
    request.effort = Some("minimal".to_string());
    let turn = turn_start_params("thread-1", "do the thing", &parity_config(), &request);
    assert_eq!(turn.effort.as_deref(), Some("minimal"));
}

/// A fork at a past turn cuts on `lastTurnId` and excludes the cut turn's own
/// boundary; a fork at head omits `lastTurnId` to copy the whole thread.
#[test]
fn fork_params_carry_the_cut_and_exclude_the_boundary() {
    let config = parity_config();

    let at_turn = thread_fork_params(&config, "thread-1", Some("turn-7"));
    assert_eq!(at_turn.thread_id, "thread-1");
    assert_eq!(at_turn.last_turn_id.as_deref(), Some("turn-7"));
    assert_eq!(at_turn.cwd.as_deref(), Some("/workspace"));
    assert!(at_turn.exclude_turns);

    let value = serde_json::to_value(&at_turn).expect("serialize");
    assert_eq!(value["threadId"], "thread-1");
    assert_eq!(value["lastTurnId"], "turn-7");
    assert_eq!(value["excludeTurns"], true);

    // At head there is no turn to cut on: `lastTurnId` must be absent, not null,
    // so the server forks the whole thread rather than rejecting an empty id.
    let at_head = thread_fork_params(&config, "thread-1", None);
    assert!(at_head.last_turn_id.is_none());
    let head_value = serde_json::to_value(&at_head).expect("serialize");
    assert!(head_value.get("lastTurnId").is_none());
    assert_eq!(head_value["excludeTurns"], true);
}

#[test]
fn params_serialize_as_camel_case() {
    let value = serde_json::to_value(thread_resume_params(
        &parity_config(),
        &parity_request(),
        "thread-1",
    ))
    .expect("serialize");
    assert_eq!(value["threadId"], "thread-1");
    assert_eq!(value["approvalPolicy"], "never");
    assert_eq!(value["sandbox"], "danger-full-access");
    assert!(value.get("developerInstructions").is_some());
}

// ── Real-CLI E2E ─────────────────────────────────────────────────────

/// Whether the real-CLI E2E tests are enabled. They spawn the actual `codex
/// app-server` subprocess and spend API credit, so they stay off unless asked
/// for — the same gate `agent::harness::codex::tests` uses.
fn skip_unless_e2e() -> bool {
    if std::env::var("CODEX_SDK_E2E").as_deref() == Ok("1") {
        return false;
    }
    eprintln!("skipping: set CODEX_SDK_E2E=1 to run");
    true
}

/// Config for a real-CLI E2E run: a throwaway workspace, with the CLI path,
/// credentials home and model taken from the environment so the subprocess
/// authenticates instead of hanging on an interactive login.
fn e2e_config(workspace_name: &str) -> crate::config::Config {
    let mut config = crate::config::test_support::minimal_config();
    let workspace = std::env::temp_dir().join(workspace_name);
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    config.workspace_dir = workspace.to_string_lossy().into_owned();
    config.repo_dir = config.workspace_dir.clone();
    if let Ok(path) = std::env::var("SANDBOX_CODEX_EXECUTABLE_PATH") {
        config.codex_cli_path = path;
    } else {
        config.codex_cli_path = "/usr/local/bin/codex".to_string();
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
    config
}

async fn e2e_session(workspace: &str) -> (crate::config::Config, AppServerSession) {
    let config = e2e_config(workspace);
    let session = start(&config, None, CancellationToken::new())
        .await
        .expect("spawn codex app-server");
    (config, session)
}

/// Everything one Turn reported, folded out of the notification stream.
#[derive(Default)]
struct TurnLog {
    turn_id: Option<String>,
    /// Whether the `turn/started` notification arrived — the point the Turn
    /// becomes steerable.
    started: bool,
    item_types: Vec<String>,
    /// All agent message text, newline-joined.
    text: String,
    status: Option<TurnStatus>,
}

/// Drain notifications until `turn/completed`. `on_started` runs once, the
/// moment the Turn becomes steerable.
async fn collect_turn<F, Fut>(session: &mut AppServerSession, on_started: Option<F>) -> TurnLog
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut log = TurnLog::default();
    let mut on_started = on_started;

    while let Some(message) = session.notifications.recv().await {
        let event = message.unwrap_or_else(|e| panic!("app-server stream error: {e}"));
        match event {
            ServerEvent::TurnStarted(started) => {
                eprintln!("*** turn/started {}", started.turn.id);
                log.turn_id = Some(started.turn.id.clone());
                log.started = true;
                if let Some(hook) = on_started.take() {
                    hook(started.turn.id).await;
                }
            }
            ServerEvent::ItemCompleted(item) => {
                eprintln!(
                    "  item/completed {} {}",
                    item.item.type_name(),
                    item.item.id()
                );
                log.item_types.push(item.item.type_name().to_string());
                if let ThreadItem::AgentMessage(message) = &item.item {
                    log.text.push_str(&message.text);
                    log.text.push('\n');
                }
            }
            ServerEvent::Error(error) => eprintln!("*** error {:?}", error.error),
            ServerEvent::TurnCompleted(completed) => {
                eprintln!("*** turn/completed status={:?}", completed.turn.status);
                log.status = Some(completed.turn.status);
                break;
            }
            _ => {}
        }
    }
    log
}

/// No hook — spelled out because `None` alone cannot infer the closure type.
const NO_HOOK: Option<fn(String) -> std::future::Ready<()>> = None;

/// Start a Session, run a Turn to completion, and read the agent's answer back.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_a_turn_runs_to_completion_and_answers() {
    if skip_unless_e2e() {
        return;
    }

    let (config, mut session) = e2e_session("codex-app-server-e2e-turn").await;
    let request = parity_request_without_model();
    let thread_id = session
        .start_thread(thread_start_params(&config, &request))
        .await
        .expect("thread/start");
    assert!(!thread_id.is_empty());

    let turn_id = session
        .start_turn(turn_start_params(
            &thread_id,
            "Run the shell command `echo hello-from-codex` and then tell me its output.",
            &config,
            &request,
        ))
        .await
        .expect("turn/start");

    let log = collect_turn(&mut session, NO_HOOK).await;
    assert_eq!(log.status, Some(TurnStatus::Completed));
    assert_eq!(log.turn_id.as_deref(), Some(turn_id.as_str()));
    assert!(
        log.item_types.iter().any(|t| t == "commandExecution"),
        "expected a shell command, got {:?}",
        log.item_types
    );
    assert!(
        log.text.contains("hello-from-codex"),
        "expected the command output in the answer, got {:?}",
        log.text
    );
    session.cancel();
}

/// Steering after `turn/started` folds the message into that same Turn: the
/// steered text lands as a `userMessage` item and changes the answer, with no
/// second Turn id anywhere in the stream.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_a_steered_message_changes_the_running_turn() {
    if skip_unless_e2e() {
        return;
    }

    let (config, mut session) = e2e_session("codex-app-server-e2e-steer").await;
    let request = parity_request_without_model();
    let thread_id = session
        .start_thread(thread_start_params(&config, &request))
        .await
        .expect("thread/start");
    let turn_id = session
        .start_turn(turn_start_params(
            &thread_id,
            "Do these steps in order. 1. Run `echo step-one` in the shell. \
             2. Run `sleep 25` in the shell. 3. Reply with the word ALPHA.",
            &config,
            &request,
        ))
        .await
        .expect("turn/start");

    // Before `turn/started` the Turn is not steerable, whatever the id says.
    let too_early = session
        .steer(&thread_id, &turn_id, "too early")
        .await
        .expect_err("steering before turn/started is rejected");
    let RpcError::Rpc(info) = too_early else {
        panic!("expected a JSON-RPC error, got {too_early:?}");
    };
    assert_eq!(info.code, -32600);

    let steered = std::sync::Arc::new(std::sync::Mutex::new(None));
    let log = {
        let thread_id = thread_id.clone();
        let steered = std::sync::Arc::clone(&steered);
        // The session is borrowed by `collect_turn`, so the hook steers through
        // its own handle to the same process.
        let rpc = std::sync::Arc::clone(&session.rpc);
        collect_turn(
            &mut session,
            Some(move |turn: String| async move {
                let params = TurnSteerParams {
                    thread_id,
                    expected_turn_id: turn,
                    input: vec![UserInput::text(
                        "Change of plan: skip every remaining step and reply \
                         with the single word BANANA.",
                    )],
                };
                let outcome = rpc.request("turn/steer", params).await;
                *steered.lock().expect("steered") = Some(outcome);
            }),
        )
        .await
    };

    let steered = steered.lock().expect("steered").take().expect("steer sent");
    let accepted = steered.expect("steering after turn/started is accepted");
    assert_eq!(
        accepted["turnId"], turn_id,
        "the steer joined the same Turn"
    );

    assert_eq!(log.status, Some(TurnStatus::Completed));
    assert_eq!(log.turn_id.as_deref(), Some(turn_id.as_str()));
    assert!(
        log.item_types
            .iter()
            .filter(|t| *t == "userMessage")
            .count()
            >= 2,
        "expected the steered input as a second userMessage, got {:?}",
        log.item_types
    );
    assert!(
        log.text.contains("BANANA"),
        "expected the steered text to change the answer, got {:?}",
        log.text
    );
    session.cancel();
}

/// `turn/interrupt` terminates the running Turn.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_an_interrupted_turn_terminates() {
    if skip_unless_e2e() {
        return;
    }

    let (config, mut session) = e2e_session("codex-app-server-e2e-interrupt").await;
    let request = parity_request_without_model();
    let thread_id = session
        .start_thread(thread_start_params(&config, &request))
        .await
        .expect("thread/start");
    let turn_id = session
        .start_turn(turn_start_params(
            &thread_id,
            "Run `sleep 120` in the shell, then tell me it finished. Do nothing else.",
            &config,
            &request,
        ))
        .await
        .expect("turn/start");

    let rpc = std::sync::Arc::clone(&session.rpc);
    let thread = thread_id.clone();
    let log = collect_turn(
        &mut session,
        Some(move |turn: String| async move {
            // Interrupt once the first tool call is plausibly under way.
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let params = TurnInterruptParams {
                thread_id: thread,
                turn_id: turn,
            };
            rpc.request("turn/interrupt", params)
                .await
                .expect("turn/interrupt");
        }),
    )
    .await;

    assert_eq!(log.status, Some(TurnStatus::Interrupted));
    assert_eq!(log.turn_id.as_deref(), Some(turn_id.as_str()));
    session.cancel();
}

/// The E2E runs take the model from the environment, not from the request.
fn parity_request_without_model() -> AgentRunRequest {
    AgentRunRequest {
        model: None,
        ..parity_request()
    }
}
