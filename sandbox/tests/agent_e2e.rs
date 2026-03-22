#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr)]
//! End-to-end integration tests for the sandbox agent module.
//!
//! Test 1 (`test_agent_state_transitions`) is a pure state-machine test —
//! no CLI or API key needed, runs with `cargo test -p moatless-sandbox`.
//!
//! Tests 2–5 are E2E tests gated behind `CLAUDE_SDK_E2E=1` and `#[ignore]`.
//! They spawn the real `claude` CLI and make actual API calls.
//!
//! Run all E2E tests:
//!   CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
//!
//! Requirements (for tests 2–5):
//! - `claude` CLI installed and on PATH
//! - Valid ANTHROPIC_API_KEY in environment
//! - Each test costs ~$0.01-0.05 in API usage

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::{query, ClaudeOptions, PermissionMode, SdkMessage};
use tokio_util::sync::CancellationToken;

use moatless_sandbox::agent::agent_state::AgentState;
use moatless_sandbox::agent::permission_handler::InteractivePermissionHandler;
use moatless_sandbox::agent::prompt::{PendingPrompt, PromptResponse, PromptToolName};
use moatless_sandbox::agent::types::AgentStatus;
use moatless_sandbox::error::SandboxError;

// ---------------------------------------------------------------------------
// Constants & Helpers
// ---------------------------------------------------------------------------

/// Test timeout — generous because CLI startup + API call can be slow.
const TEST_TIMEOUT: Duration = Duration::from_secs(90);

/// Cheapest, fastest model for E2E tests.
const TEST_MODEL: &str = "claude-haiku-4-5-20251001";

/// Skip test unless CLAUDE_SDK_E2E=1 is set.
macro_rules! skip_unless_e2e {
    () => {
        if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
            eprintln!("Skipping E2E test (set CLAUDE_SDK_E2E=1 to run)");
            return;
        }
    };
}

/// Collect auth-related env vars to forward explicitly to the CLI subprocess.
fn auth_env() -> Option<HashMap<String, String>> {
    let mut env = HashMap::new();
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        env.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token);
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env.insert("ANTHROPIC_API_KEY".to_string(), key);
    }
    if env.is_empty() { None } else { Some(env) }
}

/// Initialize tracing for test output (idempotent).
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("moatless_sandbox=debug".parse().unwrap())
                .add_directive("claude_agent_sdk=debug".parse().unwrap()),
        )
        .with_test_writer()
        .try_init();
}

/// Collect all messages from a query handle until Result or channel close.
async fn collect_messages(
    rx: &mut tokio::sync::mpsc::Receiver<claude_agent_sdk::Result<SdkMessage>>,
) -> Vec<SdkMessage> {
    let mut messages = Vec::new();
    while let Some(msg) = rx.recv().await {
        match msg {
            Ok(msg) => {
                let is_result = matches!(&msg, SdkMessage::Result(_));
                messages.push(msg);
                if is_result {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Error receiving message: {e}");
                break;
            }
        }
    }
    messages
}

/// Extract the result text from a successful Result message.
fn extract_result_text(messages: &[SdkMessage]) -> Option<String> {
    messages.iter().find_map(|m| {
        if let SdkMessage::Result(ResultMessage::Success(s)) = m {
            Some(s.result.clone())
        } else {
            None
        }
    })
}

/// Poll AgentState until `has_pending_prompt()` returns true, with timeout.
async fn wait_for_pending_prompt(state: &AgentState, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if state.has_pending_prompt().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ===========================================================================
// Test 1: AgentState transitions (unit test — no CLI needed)
// ===========================================================================

#[tokio::test]
async fn test_agent_state_transitions() {
    let state = AgentState::new();

    // Initial state is Idle
    assert_eq!(state.status().await, AgentStatus::Idle);
    assert!(state.last_session_id().await.is_none());
    assert!(state.last_error().await.is_none());
    assert!(!state.has_pending_prompt().await);

    // Idle → Running
    let cancel_token = CancellationToken::new();
    state.set_running(cancel_token.clone()).await;
    assert_eq!(state.status().await, AgentStatus::Running);

    // Set session ID
    state.set_session_id("session-123".to_string()).await;
    assert_eq!(
        state.last_session_id().await,
        Some("session-123".to_string())
    );

    // Set pending prompt
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let prompt = PendingPrompt {
        tool_name: PromptToolName::AskUserQuestion,
        input: serde_json::json!({"question": "What color?"}),
        sender: tx,
    };
    state.set_pending_prompt(prompt).await;
    assert!(state.has_pending_prompt().await);

    // Take pending prompt
    let taken = state.take_pending_prompt().await;
    assert!(taken.is_some());
    assert!(!state.has_pending_prompt().await);

    // Running → Idle
    state.set_idle().await;
    assert_eq!(state.status().await, AgentStatus::Idle);

    // Idle → Running → Stopped
    let cancel_token2 = CancellationToken::new();
    state.set_running(cancel_token2.clone()).await;
    assert_eq!(state.status().await, AgentStatus::Running);

    // set_stopped clears pending prompt
    let (tx2, _rx2) = tokio::sync::oneshot::channel();
    let prompt2 = PendingPrompt {
        tool_name: PromptToolName::ExitPlanMode,
        input: serde_json::json!({}),
        sender: tx2,
    };
    state.set_pending_prompt(prompt2).await;
    assert!(state.has_pending_prompt().await);

    state.set_stopped().await;
    assert_eq!(state.status().await, AgentStatus::Stopped);
    assert!(!state.has_pending_prompt().await);

    // set_error clears prompt and resets to Idle
    let cancel_token3 = CancellationToken::new();
    state.set_running(cancel_token3).await;
    let (tx3, _rx3) = tokio::sync::oneshot::channel();
    state
        .set_pending_prompt(PendingPrompt {
            tool_name: PromptToolName::AskUserQuestion,
            input: serde_json::json!({}),
            sender: tx3,
        })
        .await;
    state.set_error("Something went wrong".to_string()).await;
    assert_eq!(state.status().await, AgentStatus::Idle);
    assert!(!state.has_pending_prompt().await);
    assert_eq!(
        state.last_error().await,
        Some("Something went wrong".to_string())
    );

    // stop() returns error when no cancel token
    let fresh = AgentState::new();
    let result = fresh.stop().await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        SandboxError::AgentNotRunning
    ));

    // stop() cancels token when running
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    fresh.set_running(cancel).await;
    assert!(!cancel_clone.is_cancelled());
    fresh.stop().await.expect("stop should succeed");
    assert!(cancel_clone.is_cancelled());

    // --- Pending message UUIDs ---
    let uuid_state = AgentState::new();

    // Initially empty
    assert!(uuid_state.take_pending_message_uuids().await.is_empty());

    // Set and take
    uuid_state
        .set_pending_message_uuids(vec!["msg-1".to_string(), "msg-2".to_string()])
        .await;
    let taken = uuid_state.take_pending_message_uuids().await;
    assert_eq!(taken, vec!["msg-1", "msg-2"]);
    assert!(uuid_state.take_pending_message_uuids().await.is_empty());

    // set_idle clears UUIDs
    uuid_state
        .set_pending_message_uuids(vec!["msg-3".to_string()])
        .await;
    uuid_state.set_idle().await;
    assert!(uuid_state.take_pending_message_uuids().await.is_empty());

    // set_error clears UUIDs
    uuid_state
        .set_pending_message_uuids(vec!["msg-4".to_string()])
        .await;
    uuid_state.set_error("fail".to_string()).await;
    assert!(uuid_state.take_pending_message_uuids().await.is_empty());
}

// ===========================================================================
// Test 2: Basic agent execution (E2E)
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_basic_agent_execution() {
    skip_unless_e2e!();
    init_tracing();

    let result = tokio::time::timeout(TEST_TIMEOUT, async {
        let agent_state = Arc::new(AgentState::new());
        let cancel_token = CancellationToken::new();
        agent_state.set_running(cancel_token.clone()).await;

        let permission_handler = InteractivePermissionHandler::new(agent_state.clone());

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(1),
            allow_dangerously_skip_permissions: Some(true),
            can_use_tool: Some(Arc::new(permission_handler)),
            cancellation_token: Some(cancel_token),
            env: auth_env(),
            ..Default::default()
        };

        let mut handle = query("What is 2+2? Reply with only the number.", options)
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Should have System, Assistant, and Result messages
        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::System(_))),
            "Expected System init message"
        );
        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        // Verify session_id was captured
        let system_session_id = messages.iter().find_map(|m| {
            if let SdkMessage::System(init) = m {
                Some(init.session_id.clone())
            } else {
                None
            }
        });
        assert!(
            system_session_id.is_some(),
            "Expected system init with session_id"
        );

        // Check result contains "4"
        let result_text = extract_result_text(&messages).expect("Expected successful result");
        assert!(
            result_text.contains('4'),
            "Expected result to contain '4', got: {result_text}"
        );
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 3: AskUserQuestion interactive flow (E2E)
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_ask_user_question_flow() {
    skip_unless_e2e!();
    init_tracing();

    let result = tokio::time::timeout(TEST_TIMEOUT, async {
        let agent_state = Arc::new(AgentState::new());
        let cancel_token = CancellationToken::new();
        agent_state.set_running(cancel_token.clone()).await;

        let permission_handler = InteractivePermissionHandler::new(agent_state.clone());

        // Use Default permission mode — AskUserQuestion will trigger the handler
        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(5),
            can_use_tool: Some(Arc::new(permission_handler)),
            cancellation_token: Some(cancel_token),
            env: auth_env(),
            ..Default::default()
        };

        let prompt = "You MUST use the AskUserQuestion tool to ask the user \
                       what their favorite color is before you do anything else. \
                       Use exactly one question with two options: Blue and Red. \
                       After the user answers, respond with 'Your favorite color is: <answer>'.";

        let mut handle = query(prompt, options)
            .await
            .expect("Failed to start query");

        // Spawn a task to resolve the prompt
        let state_for_resolver = agent_state.clone();
        let resolver = tokio::spawn(async move {
            let found =
                wait_for_pending_prompt(&state_for_resolver, Duration::from_secs(60)).await;
            assert!(found, "Timed out waiting for pending prompt");

            // Verify it's AskUserQuestion
            let tool_name = state_for_resolver
                .pending_prompt_tool_name()
                .await
                .expect("Should have pending prompt tool name");
            eprintln!("Pending prompt tool: {tool_name}");

            // Take and resolve the prompt
            let prompt = state_for_resolver
                .take_pending_prompt()
                .await
                .expect("Should have pending prompt to take");
            assert_eq!(prompt.tool_name, PromptToolName::AskUserQuestion);

            let response = PromptResponse::AskUserAnswer {
                answers: serde_json::json!({"text": "Blue"}),
            };
            let _ = prompt.sender.send(response);
            eprintln!("Resolved AskUserQuestion with 'Blue'");
        });

        let messages = collect_messages(&mut handle.messages).await;
        resolver.await.expect("Resolver task panicked");

        // Should complete with a Result
        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        eprintln!(
            "Result: {}",
            extract_result_text(&messages).unwrap_or_default()
        );
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 4: ExitPlanMode interactive flow (E2E)
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_exit_plan_mode_flow() {
    skip_unless_e2e!();
    init_tracing();

    let result = tokio::time::timeout(TEST_TIMEOUT, async {
        // Use a fresh temp dir each run to avoid stale files from previous tests
        let tmp = std::env::temp_dir().join(format!(
            "sandbox-e2e-plan-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let _ = std::fs::remove_dir_all(&tmp); // clean up if exists
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let agent_state = Arc::new(AgentState::new());
        let cancel_token = CancellationToken::new();
        agent_state.set_running(cancel_token.clone()).await;

        // Plan mode handler: blocks Write/Edit/Bash until ExitPlanMode is approved
        let permission_handler =
            InteractivePermissionHandler::with_plan_mode(agent_state.clone(), true);

        // Use Plan permission mode — forces plan mode which triggers ExitPlanMode
        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(10),
            permission_mode: Some(PermissionMode::Plan),
            cwd: Some(tmp.to_string_lossy().to_string()),
            can_use_tool: Some(Arc::new(permission_handler)),
            cancellation_token: Some(cancel_token),
            env: auth_env(),
            ..Default::default()
        };

        let prompt = "Write a simple hello world Python script and save it to hello.py";

        let mut handle = query(prompt, options)
            .await
            .expect("Failed to start query");

        // Track whether the ExitPlanMode flow was exercised
        let plan_approved = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let plan_approved_flag = plan_approved.clone();

        // Spawn a task to approve the plan
        let state_for_resolver = agent_state.clone();
        let resolver = tokio::spawn(async move {
            let found =
                wait_for_pending_prompt(&state_for_resolver, Duration::from_secs(60)).await;
            assert!(found, "Timed out waiting for pending prompt (ExitPlanMode)");

            let tool_name = state_for_resolver
                .pending_prompt_tool_name()
                .await
                .expect("Should have pending prompt tool name");
            eprintln!("Pending prompt tool: {tool_name}");

            let prompt = state_for_resolver
                .take_pending_prompt()
                .await
                .expect("Should have pending prompt to take");
            assert_eq!(prompt.tool_name, PromptToolName::ExitPlanMode);

            let _ = prompt.sender.send(PromptResponse::PlanApproved);
            plan_approved_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            eprintln!("Approved plan via ExitPlanMode");
        });

        let messages = collect_messages(&mut handle.messages).await;
        resolver.await.expect("Resolver task panicked");

        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        // Log the full result for debugging
        for msg in &messages {
            match msg {
                SdkMessage::Result(ResultMessage::Success(s)) => {
                    eprintln!(
                        "Result: Success (is_error={}, turns={}, cost=${:.4})",
                        s.is_error, s.num_turns, s.total_cost_usd
                    );
                    eprintln!("  Text: {}", &s.result[..s.result.len().min(300)]);
                }
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    eprintln!(
                        "Result: Error (subtype={}, turns={})",
                        e.subtype, e.num_turns
                    );
                    for err in &e.errors {
                        eprintln!("  Error: {err}");
                    }
                }
                _ => {}
            }
        }

        assert!(
            plan_approved.load(std::sync::atomic::Ordering::SeqCst),
            "ExitPlanMode flow was never exercised — agent returned Result \
             before triggering plan approval"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 5: Cancel during pending prompt (E2E)
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_cancel_during_pending_prompt() {
    skip_unless_e2e!();
    init_tracing();

    let result = tokio::time::timeout(TEST_TIMEOUT, async {
        let agent_state = Arc::new(AgentState::new());
        let cancel_token = CancellationToken::new();
        agent_state.set_running(cancel_token.clone()).await;

        let permission_handler = InteractivePermissionHandler::new(agent_state.clone());

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(5),
            can_use_tool: Some(Arc::new(permission_handler)),
            cancellation_token: Some(cancel_token.clone()),
            env: auth_env(),
            ..Default::default()
        };

        let prompt = "You MUST use the AskUserQuestion tool to ask the user \
                       what their favorite color is. Use exactly one question \
                       with two options: Blue and Red.";

        let mut handle = query(prompt, options)
            .await
            .expect("Failed to start query");

        // Wait for the pending prompt, then cancel instead of resolving
        let state_for_canceller = agent_state.clone();
        let canceller = tokio::spawn(async move {
            let found =
                wait_for_pending_prompt(&state_for_canceller, Duration::from_secs(60)).await;
            assert!(
                found,
                "Timed out waiting for pending prompt before cancel"
            );

            eprintln!("Pending prompt detected, cancelling agent...");

            // Cancel the agent — this drops the pending prompt sender,
            // causing the handler's rx.await to return Err (Deny)
            state_for_canceller.set_stopped().await;
            cancel_token.cancel();

            eprintln!("Agent cancelled");
        });

        // Collect messages — agent should terminate
        let messages = collect_messages(&mut handle.messages).await;
        canceller.await.expect("Canceller task panicked");

        // The agent should have terminated — either with a Result or
        // the channel just closed. Both are acceptable.
        eprintln!(
            "Messages received after cancel: {}",
            messages.len()
        );

        // Verify agent state is Stopped
        assert_eq!(
            agent_state.status().await,
            AgentStatus::Stopped,
            "Agent should be in Stopped state after cancel"
        );

        // Verify pending prompt was cleared
        assert!(
            !agent_state.has_pending_prompt().await,
            "Pending prompt should be cleared after cancel"
        );
    })
    .await;

    result.expect("Test timed out");
}
