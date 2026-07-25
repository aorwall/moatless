//! Unit + gated E2E coverage for the Codex adapter: native item normalization
//! (one source event per item family, collision-safe ids, usage retention),
//! failure classification, and the one-shot lifecycle through the shared driver
//! (fresh Turn ordering, queued input, cancellation teardown, unexpected exit).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::agent::codex_exec::{CodexCliSession, CodexExecEvent};
use crate::credentials::recovery::CredentialFailureDetector;

use super::super::error::{ErrorClass, FailureStage, SessionImpact};
use super::super::process::{AgentProcess, ProcessEvent, ProcessTermination};
use super::super::signal::{Cause, Input, InputId, Outcome, Signal};
use super::super::source::{Role, SourceEvent};
use super::classify::{classify_startup_error, classify_stream_error, classify_turn_error};
use super::{CodexConfig, CodexProcess};

fn config() -> CodexConfig {
    CodexConfig {
        config: Arc::new(crate::config::test_support::minimal_config()),
        credential_failures: CredentialFailureDetector::default(),
    }
}

fn process() -> CodexProcess {
    CodexProcess::new(config())
}

fn parse(line: &str) -> CodexExecEvent {
    serde_json::from_str::<CodexExecEvent>(line).unwrap_or_else(|e| panic!("parse {line}: {e}"))
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

// ── Native normalization ─────────────────────────────────────────

#[test]
fn thread_started_yields_session_ready_then_input_accepted() {
    let mut p = process();
    p.input_id = Some(InputId("in-1".to_string()));
    p.absorb(parse(r#"{"type":"thread.started","thread_id":"thread-A"}"#));
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

#[test]
fn agent_message_and_reasoning_are_ordered_assistant_text_blocks() {
    let mut p = process();
    p.absorb(parse(r#"{"type":"thread.started","thread_id":"t"}"#));
    drain(&mut p);
    p.absorb(parse(
        r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"think"}}"#,
    ));
    p.absorb(parse(
        r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"hi"}}"#,
    ));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 2);
    // Reasoning and prose both normalize to renderable assistant text, in order.
    assert_eq!(s[0].block_json()["type"], "text");
    assert_eq!(s[0].block_json()["text"], "think");
    assert_eq!(s[1].block_json()["type"], "text");
    assert_eq!(s[1].block_json()["text"], "hi");
    assert!(s.iter().all(|e| e.role == Role::Assistant));
    assert_ne!(s[0].source_id, s[1].source_id);
    assert_eq!(s[0].model.as_deref(), None); // model set only after submit
}

#[test]
fn every_item_family_normalizes_to_canonical_blocks() {
    let mut p = process();
    p.absorb(parse(r#"{"type":"thread.started","thread_id":"t"}"#));
    drain(&mut p);
    let lines = [
        r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"ls","aggregated_output":"a\n","exit_code":0,"status":"completed","duration_ms":12}}"#,
        r#"{"type":"item.completed","item":{"id":"item_1","type":"file_change","changes":[{"path":"/a.py","kind":"add","diff":"+x"}],"status":"completed"}}"#,
        r#"{"type":"item.completed","item":{"id":"item_2","type":"mcp_tool_call","server":"db","tool":"q","arguments":{"x":1},"result":{"content":[{"type":"text","text":"ok"}]},"status":"completed"}}"#,
        r#"{"type":"item.completed","item":{"id":"item_3","type":"collab_tool_call","tool":"spawn_agent","sender_thread_id":"main","receiver_thread_ids":["s"],"prompt":"go","agents_states":{},"status":"in_progress"}}"#,
        r#"{"type":"item.completed","item":{"id":"item_4","type":"web_search","query":"rust","action":{"type":"search"}}}"#,
        r#"{"type":"item.completed","item":{"id":"item_5","type":"error","message":"warned"}}"#,
    ];
    for line in lines {
        p.absorb(parse(line));
    }
    let events = drain(&mut p);
    let s = sources(&events);
    // Four tool families each expand to tool_use + tool_result; web_search is a
    // lone tool_use; the non-fatal error becomes assistant text.
    assert_eq!(s.len(), 10);

    // command_execution → codex.shell tool_use + tool_result carrying the typed result.
    assert_eq!(s[0].block_json()["type"], "tool_use");
    assert_eq!(s[0].block_json()["name"], "codex.shell");
    assert_eq!(s[0].block_json()["input"]["command"], "ls");
    assert_eq!(s[0].role, Role::Assistant);
    assert!(s[0].tool_use_result.is_none());
    assert_eq!(s[1].block_json()["type"], "tool_result");
    assert_eq!(s[1].block_json()["tool_use_id"], s[0].block_json()["id"]);
    assert_eq!(s[1].block_json()["is_error"], false);
    assert_eq!(s[1].role, Role::User);
    let shell_result = s[1].tool_use_result.as_ref().expect("typed shell result");
    assert_eq!(shell_result["status"], "completed");
    assert_eq!(shell_result["exitCode"], 0);

    // file_change → codex.edit.
    assert_eq!(s[2].block_json()["name"], "codex.edit");
    assert_eq!(s[2].block_json()["input"]["changes"][0]["path"], "/a.py");
    assert_eq!(s[3].block_json()["type"], "tool_result");
    assert!(s[3].tool_use_result.is_some());

    // mcp_tool_call → codex.mcp_tool_call, typed result reflects the completion.
    assert_eq!(s[4].block_json()["name"], "codex.mcp_tool_call");
    assert_eq!(s[4].block_json()["input"]["server"], "db");
    assert_eq!(
        s[5].tool_use_result.as_ref().unwrap()["status"],
        "completed"
    );

    // collab_tool_call → codex.collab_tool_call.
    assert_eq!(s[6].block_json()["name"], "codex.collab_tool_call");
    assert_eq!(s[6].block_json()["input"]["senderThreadId"], "main");
    assert_eq!(s[7].block_json()["type"], "tool_result");

    // web_search → a lone codex.web_search tool_use (no result concept).
    assert_eq!(s[8].block_json()["type"], "tool_use");
    assert_eq!(s[8].block_json()["name"], "codex.web_search");
    assert_eq!(s[8].block_json()["input"]["query"], "rust");

    // A non-fatal error item renders as assistant text.
    assert_eq!(s[9].block_json()["type"], "text");
    assert_eq!(s[9].block_json()["text"], "warned");
    assert_eq!(s[9].role, Role::Assistant);

    // No diagnostics for known families.
    assert!(s.iter().all(|e| e.diagnostics.is_none()));
}

#[test]
fn unknown_item_kind_retains_bounded_diagnostics() {
    let mut p = process();
    p.absorb(parse(r#"{"type":"thread.started","thread_id":"t"}"#));
    drain(&mut p);
    p.absorb(parse(
        r#"{"type":"item.completed","item":{"id":"item_9","type":"brand_new","foo":"bar"}}"#,
    ));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1);
    // The raw native block is preserved verbatim inside the canonical `unknown`
    // block and flagged with bounded diagnostics so an upstream CLI addition is
    // never silently dropped.
    let block = s[0].block_json();
    assert_eq!(block["type"], "unknown");
    assert_eq!(block["raw"]["type"], "brand_new");
    assert_eq!(block["raw"]["foo"], "bar");
    let diag = s[0].diagnostics.as_ref().expect("diagnostics retained");
    assert_eq!(diag["type"], "brand_new");
}

#[test]
fn todo_list_progress_updates_share_a_stable_source_id() {
    let mut p = process();
    p.absorb(parse(r#"{"type":"thread.started","thread_id":"t"}"#));
    drain(&mut p);
    p.absorb(parse(
        r#"{"type":"item.started","item":{"id":"item_1","type":"todo_list","items":[{"text":"a","completed":false}]}}"#,
    ));
    p.absorb(parse(
        r#"{"type":"item.updated","item":{"id":"item_1","type":"todo_list","items":[{"text":"a","completed":true}]}}"#,
    ));
    p.absorb(parse(
        r#"{"type":"item.completed","item":{"id":"item_1","type":"todo_list","items":[{"text":"a","completed":true}]}}"#,
    ));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(
        s.len(),
        3,
        "started + updated + completed each emit a block"
    );
    // Stable id across the running-plan updates so the backend upserts in place.
    assert_eq!(s[0].source_id, s[1].source_id);
    assert_eq!(s[1].source_id, s[2].source_id);
    // Each emission is a codex.todo_list tool_use whose input tracks completion.
    assert_eq!(s[0].block_json()["name"], "codex.todo_list");
    assert_eq!(s[0].block_json()["input"]["items"][0]["completed"], false);
    assert_eq!(s[2].block_json()["input"]["items"][0]["completed"], true);
}

#[test]
fn duplicate_item_ids_across_spawns_get_distinct_source_ids() {
    // Codex restarts `item_0` on every resume; two spawns must not collide.
    let line =
        r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"x"}}"#;

    let mut first = process();
    first.absorb(parse(r#"{"type":"thread.started","thread_id":"t"}"#));
    drain(&mut first);
    first.absorb(parse(line));
    let first_id = sources(&drain(&mut first))[0].source_id.clone();

    let mut second = process();
    second.absorb(parse(r#"{"type":"thread.started","thread_id":"t"}"#));
    drain(&mut second);
    second.absorb(parse(line));
    let second_id = sources(&drain(&mut second))[0].source_id.clone();

    assert_ne!(
        first_id, second_id,
        "same native item_0 on the same thread must yield collision-safe ids"
    );
}

#[test]
fn turn_completed_emits_usage_result_then_terminated() {
    let mut p = process();
    p.absorb(parse(r#"{"type":"thread.started","thread_id":"t"}"#));
    drain(&mut p);
    p.absorb(parse(
        r#"{"type":"turn.completed","usage":{"input_tokens":12,"cached_input_tokens":3,"output_tokens":4,"reasoning_output_tokens":7}}"#,
    ));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1, "turn usage carried on a single result block");
    assert_eq!(s[0].role, Role::System);
    let usage = s[0].usage.as_ref().expect("usage retained");
    assert_eq!(usage["input_tokens"], 12);
    assert_eq!(usage["cache_read_input_tokens"], 3);
    assert_eq!(usage["reasoning_output_tokens"], 7);
    assert!(matches!(
        events.last(),
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Completed
        })
    ));
}

#[test]
fn turn_failed_with_auth_marker_is_authentication() {
    let mut p = process();
    p.absorb(parse(
        r#"{"type":"turn.failed","error":{"message":"refresh_token_reused"}}"#,
    ));
    let events = drain(&mut p);
    match events.as_slice() {
        [ProcessEvent::Terminated {
            termination: ProcessTermination::Failed(error),
        }] => {
            assert_eq!(error.class, ErrorClass::Authentication);
        }
        other => panic!("expected auth failure, got {other:?}"),
    }
}

#[test]
fn error_event_unavailable_thread_is_lost_session() {
    let mut p = process();
    p.absorb(parse(r#"{"type":"error","message":"no such thread: abc"}"#));
    let events = drain(&mut p);
    match events.as_slice() {
        [ProcessEvent::Terminated {
            termination: ProcessTermination::Failed(error),
        }] => {
            assert_eq!(error.class, ErrorClass::UnavailableSession);
            assert_eq!(error.session_impact, SessionImpact::Lost);
        }
        other => panic!("expected unavailable-session failure, got {other:?}"),
    }
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
    let classified = classify_stream_error("Failed to parse Codex JSONL event: bad");
    assert_eq!(classified.class, ErrorClass::NativeProtocol);
    assert_eq!(classified.stage, FailureStage::Execution);
}

#[test]
fn stream_exit_error_is_process_exit() {
    let classified = classify_stream_error("Codex process exited with status 1");
    assert_eq!(classified.class, ErrorClass::ProcessExit);
}

#[test]
fn rate_limit_turn_error_stays_intact() {
    let classified = classify_turn_error("HTTP 503 overloaded");
    assert_eq!(classified.class, ErrorClass::RateLimit);
    assert_eq!(classified.session_impact, SessionImpact::Intact);
}

// ── One-shot lifecycle through the shared driver ─────────────────

/// Spawn the shared one-shot driver over Codex processes backed by injected
/// streams (one per Turn), returning the command channels plus the per-Turn
/// event senders so a test can feed native events deterministically.
fn scripted_driver(
    sessions: Vec<CodexCliSession>,
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
    let (tx, session) = CodexCliSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(parse(
        r#"{"type":"thread.started","thread_id":"thread-A"}"#,
    )))
    .expect("feed");
    tx.send(Ok(parse(
        r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"4"}}"#,
    )))
    .expect("feed");
    tx.send(Ok(parse(
        r#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}"#,
    )))
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
    let (tx1, s1) = CodexCliSession::scripted();
    let (tx2, s2) = CodexCliSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![s1, s2]);

    // First Turn starts.
    submit_tx
        .send(Input::text("in-1", "one"))
        .await
        .expect("submit 1");
    tx1.send(Ok(parse(
        r#"{"type":"thread.started","thread_id":"thread-A"}"#,
    )))
    .expect("feed");
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
    tx1.send(Ok(parse(
        r#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}"#,
    )))
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
    tx2.send(Ok(parse(
        r#"{"type":"thread.started","thread_id":"thread-A"}"#,
    )))
    .expect("feed");
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
    let (tx, session) = CodexCliSession::scripted();
    let (submit_tx, cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(parse(
        r#"{"type":"thread.started","thread_id":"thread-A"}"#,
    )))
    .expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    // Request cancellation, then close the stream (the real CLI's process tree
    // teardown ends the stream). Biased cancel is observed before the EOF.
    cancel_tx.send(()).await.expect("cancel");
    drop(tx);

    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionFinished {
            outcome: Outcome::Cancelled
        }
    ));
}

#[tokio::test]
async fn cancel_tears_down_the_session() {
    let (_tx, session) = CodexCliSession::scripted();
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
    let (tx, session) = CodexCliSession::scripted();
    let (submit_tx, _cancel_tx, mut signal_rx) = scripted_driver(vec![session]);

    submit_tx
        .send(Input::text("in-1", "hi"))
        .await
        .expect("submit");
    tx.send(Ok(parse(
        r#"{"type":"thread.started","thread_id":"thread-A"}"#,
    )))
    .expect("feed");
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::ExecutionStarted { .. }
    ));
    assert!(matches!(
        next_signal(&mut signal_rx).await,
        Signal::InputAccepted { .. }
    ));

    // Stream closes with no terminal event → unexpected exit.
    drop(tx);
    match next_signal(&mut signal_rx).await {
        Signal::ExecutionFinished {
            outcome: Outcome::Failed(error),
        } => assert_eq!(error.class, ErrorClass::ProcessExit),
        other => panic!("expected failed terminal, got {other:?}"),
    }
}

/// Focused real-CLI E2E: launch the actual `codex exec` subprocess through the
/// production [`super::super::Harness::codex`] seam and assert the external
/// signal ordering for a trivial Turn. Gated behind `CODEX_SDK_E2E=1` and
/// `#[ignore]` — the full E2E matrix runs at ticket 10, per the guide.
#[tokio::test]
#[ignore = "requires the real codex CLI and credentials (CODEX_SDK_E2E=1)"]
async fn e2e_codex_turn_through_the_harness() {
    if std::env::var("CODEX_SDK_E2E").as_deref() != Ok("1") {
        eprintln!("skipping: set CODEX_SDK_E2E=1 to run");
        return;
    }
    use super::super::Harness;

    let mut config = crate::config::test_support::minimal_config();
    let workspace = std::env::temp_dir().join("harness-codex-e2e");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    config.workspace_dir = workspace.to_string_lossy().into_owned();
    config.repo_dir = config.workspace_dir.clone();
    if let Ok(path) = std::env::var("SANDBOX_CODEX_EXECUTABLE_PATH") {
        config.codex_cli_path = path;
    }
    // Point the CLI at the real credentials/home so it authenticates instead of
    // hanging on an interactive login against the throwaway test home.
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        config.codex_home_dir = codex_home;
    }
    if let Ok(home) = std::env::var("HOME") {
        config.home_dir = home;
    }
    if let Ok(model) = std::env::var("DEFAULT_CODEX_MODEL") {
        config.default_codex_model = Some(model);
    }

    let cfg = CodexConfig {
        config: Arc::new(config),
        credential_failures: CredentialFailureDetector::default(),
    };

    let mut harness = Harness::codex(cfg);
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
        "agent_message",
        "command_execution",
        "reasoning",
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
