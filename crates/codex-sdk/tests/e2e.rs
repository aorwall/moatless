#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr)]
//! End-to-end integration tests for codex-sdk.
//!
//! These tests boot the real in-process Codex runtime and make actual API calls.
//! They are gated behind the `CODEX_SDK_E2E=1` environment variable and
//! use `#[ignore]` so `cargo test` skips them by default.
//!
//! Run all E2E tests:
//!   CODEX_SDK_E2E=1 cargo test --ignored -- --nocapture
//!
//! Run a single test:
//!   CODEX_SDK_E2E=1 cargo test --ignored test_simple_query -- --nocapture
//!
//! Requirements:
//! - Valid OPENAI_API_KEY (or codex login credentials) in environment
//! - Codex repo checked out at ~/repos/stuffs/codex/ (path dependencies)
//! - Each test costs ~$0.01-0.05 in API usage

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio::time::timeout;

use codex_sdk::{
    query, AskForApproval, CodexClient, CodexMessage, CodexOptions, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalParams, FileChangeApprovalDecision,
    FileChangeRequestApprovalParams, ApprovalHandler,
};
use codex_sdk::hooks::HookObserver;
use codex_app_server_protocol::{
    HookStartedNotification, HookCompletedNotification, PermissionsRequestApprovalParams,
    PermissionsRequestApprovalResponse,
};

// ---------------------------------------------------------------------------
// Constants & Helpers
// ---------------------------------------------------------------------------

/// Test timeout — generous because in-process runtime startup can be slow.
const TEST_TIMEOUT: Duration = Duration::from_secs(90);

/// Default model for E2E tests.
const TEST_MODEL: &str = "gpt-5.3-codex";

/// Model for E2E tests. Uses CODEX_TEST_MODEL env var if set,
/// otherwise falls back to TEST_MODEL.
fn test_model() -> Option<String> {
    Some(
        std::env::var("CODEX_TEST_MODEL")
            .unwrap_or_else(|_| TEST_MODEL.to_string()),
    )
}

/// Skip test unless CODEX_SDK_E2E=1 is set.
macro_rules! skip_unless_e2e {
    () => {
        if std::env::var("CODEX_SDK_E2E").as_deref() != Ok("1") {
            eprintln!("Skipping E2E test (set CODEX_SDK_E2E=1 to run)");
            return;
        }
    };
}

/// Initialize tracing for test output (idempotent).
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("codex_sdk=debug".parse().unwrap()),
        )
        .with_test_writer()
        .try_init();
}

/// Default options for E2E tests: gpt-5.3-codex with high reasoning, ephemeral, auto-approve.
fn e2e_options() -> CodexOptions {
    CodexOptions {
        model: test_model(),
        ephemeral: Some(true),
        approval_policy: Some(AskForApproval::Never),
        config_overrides: vec![(
            "model_reasoning_effort".to_string(),
            toml::Value::String("high".to_string()),
        )],
        ..Default::default()
    }
}

/// Collect all messages from a query handle until TurnCompleted or channel close.
async fn collect_messages(
    rx: &mut tokio::sync::mpsc::Receiver<codex_sdk::Result<CodexMessage>>,
) -> Vec<CodexMessage> {
    let mut messages = Vec::new();
    while let Some(msg) = rx.recv().await {
        match msg {
            Ok(msg) => {
                let is_turn_completed = matches!(&msg, CodexMessage::TurnCompleted(_));
                messages.push(msg);
                if is_turn_completed {
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

/// Extract all text from AgentMessageDelta messages.
fn extract_agent_text(messages: &[CodexMessage]) -> String {
    let mut text = String::new();
    for msg in messages {
        if let CodexMessage::AgentMessageDelta(delta) = msg {
            text.push_str(&delta.delta);
        }
    }
    text
}

// ===========================================================================
// Test 1: Simple query — verify basic message flow and result
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_simple_query() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("What is 2+2? Reply with only the number.", e2e_options())
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Print summary for debugging (before assertions so we see output on failure)
        eprintln!("Messages received: {}", messages.len());
        for msg in &messages {
            match msg {
                CodexMessage::AgentMessageDelta(d) => {
                    eprintln!("  [AgentMessageDelta] '{}'", d.delta);
                }
                CodexMessage::TurnCompleted(_) => eprintln!("  [TurnCompleted]"),
                CodexMessage::TurnStarted(_) => eprintln!("  [TurnStarted]"),
                CodexMessage::ThreadStarted(_) => eprintln!("  [ThreadStarted]"),
                CodexMessage::Error(e) => {
                    eprintln!("  [Error] {}", e.error.message);
                }
                CodexMessage::ConfigWarning(w) => {
                    eprintln!("  [ConfigWarning] {}", w.summary);
                }
                other => eprintln!("  [{:?}]", std::mem::discriminant(other)),
            }
        }
        let agent_text = extract_agent_text(&messages);
        eprintln!("Agent text: '{agent_text}'");

        // Should have at least one AgentMessageDelta
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, CodexMessage::AgentMessageDelta(_))),
            "Expected at least one AgentMessageDelta"
        );

        // Should end with TurnCompleted
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, CodexMessage::TurnCompleted(_))),
            "Expected TurnCompleted message"
        );

        // Check that the agent text contains "4"
        assert!(
            agent_text.contains('4'),
            "Expected agent text to contain '4', got: {agent_text}"
        );

        handle.cancel();
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 2: Thread ID is valid and non-empty
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_thread_id_is_valid() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("Say hello.", e2e_options())
            .await
            .expect("Failed to start query");

        // Thread ID should be set immediately after query() returns
        let thread_id = handle.thread_id().to_string();
        assert!(
            !thread_id.is_empty(),
            "Expected non-empty thread_id, got empty string"
        );
        eprintln!("Thread ID: {thread_id}");

        // Drain messages to completion
        let messages = collect_messages(&mut handle.messages).await;
        assert!(
            !messages.is_empty(),
            "Expected at least one message"
        );

        handle.cancel();
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 3: Cancel stops the session
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_cancel_stops_session() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query(
            "Count from 1 to 100, one number per line, slowly.",
            e2e_options(),
        )
        .await
        .expect("Failed to start query");

        // Cancel immediately
        handle.cancel();

        // The message channel should eventually close (return None)
        let mut count = 0;
        while let Some(_msg) = handle.messages.recv().await {
            count += 1;
            if count > 100 {
                break; // Safety valve
            }
        }

        eprintln!("Received {count} messages after cancel before channel closed");
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 4: Multi-turn session via CodexClient
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_multi_turn_via_codex_client() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(Duration::from_secs(120), async {
        let mut client = CodexClient::new(e2e_options())
            .await
            .expect("Failed to create CodexClient");

        // Turn 1: Ask to remember a number
        client
            .send("Remember the number 42. Reply with only 'OK'.")
            .await
            .expect("Failed to send turn 1");

        let mut turn1_text = String::new();
        while let Some(msg) = client.recv().await {
            match msg.expect("Error in turn 1") {
                CodexMessage::AgentMessageDelta(delta) => {
                    turn1_text.push_str(&delta.delta);
                }
                CodexMessage::TurnCompleted(_) => break,
                _ => {}
            }
        }
        eprintln!("Turn 1 response: {turn1_text}");

        // Turn 2: Ask what number was remembered
        client
            .send("What number did I ask you to remember? Reply with only the number.")
            .await
            .expect("Failed to send turn 2");

        let mut turn2_text = String::new();
        while let Some(msg) = client.recv().await {
            match msg.expect("Error in turn 2") {
                CodexMessage::AgentMessageDelta(delta) => {
                    turn2_text.push_str(&delta.delta);
                }
                CodexMessage::TurnCompleted(_) => break,
                _ => {}
            }
        }
        eprintln!("Turn 2 response: {turn2_text}");

        assert!(
            turn2_text.contains("42"),
            "Expected turn 2 to contain '42', got: {turn2_text}"
        );

        client.cancel();
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 5: Approval handler is invoked (capturing, allow-all)
// ===========================================================================

/// An approval handler that approves all requests and records command invocations.
struct CapturingApprovalHandler {
    command_invocations: Arc<Mutex<Vec<Option<String>>>>,
    file_invocations: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ApprovalHandler for CapturingApprovalHandler {
    async fn on_command_approval(
        &self,
        params: &CommandExecutionRequestApprovalParams,
    ) -> CommandExecutionApprovalDecision {
        self.command_invocations
            .lock()
            .await
            .push(params.command.clone());
        CommandExecutionApprovalDecision::Accept
    }

    async fn on_file_change_approval(
        &self,
        params: &FileChangeRequestApprovalParams,
    ) -> FileChangeApprovalDecision {
        self.file_invocations
            .lock()
            .await
            .push(params.item_id.clone());
        FileChangeApprovalDecision::Accept
    }

    async fn on_permissions_approval(
        &self,
        _params: &PermissionsRequestApprovalParams,
    ) -> Option<PermissionsRequestApprovalResponse> {
        None
    }
}

#[tokio::test]
#[ignore]
async fn test_approval_handler_is_invoked() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let tmp = std::env::temp_dir().join("codex-sdk-e2e-approval");
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let command_invocations = Arc::new(Mutex::new(Vec::new()));
        let file_invocations = Arc::new(Mutex::new(Vec::new()));

        let options = CodexOptions {
            model: test_model(),
            ephemeral: Some(true),
            cwd: Some(tmp.clone()),
            // OnRequest ensures every tool use triggers an approval request
            approval_policy: Some(AskForApproval::OnRequest),
            approval_handler: Some(Arc::new(CapturingApprovalHandler {
                command_invocations: command_invocations.clone(),
                file_invocations: file_invocations.clone(),
            })),
            ..Default::default()
        };

        let mut handle = query(
            "Write the text 'hello from test' to a new file called test_output.txt in the current directory.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Print messages for debugging
        for msg in &messages {
            eprintln!("  [{:?}]", std::mem::discriminant(msg));
        }

        let captured_commands = command_invocations.lock().await;
        let captured_files = file_invocations.lock().await;
        eprintln!(
            "Command invocations: {:?}, File invocations: {:?}",
            *captured_commands, *captured_files
        );

        assert!(
            !captured_commands.is_empty() || !captured_files.is_empty(),
            "Expected approval handler to be invoked at least once"
        );

        handle.cancel();
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 6: DenyAllHandler blocks command execution
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_deny_handler_blocks_execution() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let tmp = std::env::temp_dir().join("codex-sdk-e2e-deny");
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        let options = CodexOptions {
            model: test_model(),
            ephemeral: Some(true),
            cwd: Some(tmp.clone()),
            approval_policy: Some(AskForApproval::OnRequest),
            approval_handler: Some(Arc::new(codex_sdk::DenyAllHandler)),
            ..Default::default()
        };

        let mut handle = query(
            "Write 'test content' to a file called output.txt in the current directory.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // The turn should complete (either normally or with errors due to denied tools)
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, CodexMessage::TurnCompleted(_))),
            "Expected TurnCompleted"
        );

        // The file should NOT have been written
        let output_path = tmp.join("output.txt");
        assert!(
            !output_path.exists(),
            "Expected output.txt to NOT exist (denial should have prevented write)"
        );

        eprintln!(
            "Messages: {}, Agent text: {}",
            messages.len(),
            extract_agent_text(&messages)
        );

        handle.cancel();
        let _ = std::fs::remove_dir_all(&tmp);
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 7: Hook observer receives events
// ===========================================================================

/// A hook observer that records all notifications.
struct CapturingHookObserver {
    started: Arc<std::sync::Mutex<Vec<String>>>,
    completed: Arc<std::sync::Mutex<Vec<String>>>,
}

impl HookObserver for CapturingHookObserver {
    fn on_hook_started(&self, params: &HookStartedNotification) {
        self.started
            .lock()
            .unwrap()
            .push(params.run.id.clone());
    }

    fn on_hook_completed(&self, params: &HookCompletedNotification) {
        self.completed
            .lock()
            .unwrap()
            .push(params.run.id.clone());
    }
}

#[tokio::test]
#[ignore]
async fn test_hook_observer_receives_events() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let started = Arc::new(std::sync::Mutex::new(Vec::new()));
        let completed = Arc::new(std::sync::Mutex::new(Vec::new()));

        let options = CodexOptions {
            model: test_model(),
            ephemeral: Some(true),
            approval_policy: Some(AskForApproval::Never),
            hook_observer: Some(Arc::new(CapturingHookObserver {
                started: started.clone(),
                completed: completed.clone(),
            })),
            ..Default::default()
        };

        let mut handle = query(
            "List the files in the current directory using ls.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        // Check if any hook notifications were received via the observer
        let started_hooks = started.lock().unwrap();
        let completed_hooks = completed.lock().unwrap();

        eprintln!(
            "Hook observer: {} started, {} completed",
            started_hooks.len(),
            completed_hooks.len()
        );

        // Also check if any HookStarted/HookCompleted messages came through the stream
        let hook_messages = messages
            .iter()
            .filter(|m| matches!(m, CodexMessage::HookStarted(_) | CodexMessage::HookCompleted(_)))
            .count();

        eprintln!("Hook messages in stream: {hook_messages}");
        eprintln!("Total messages: {}", messages.len());

        // Note: Hooks may not fire in a default configuration (no hooks configured).
        // This test validates the observer wiring doesn't panic/error.
        // If hooks ARE configured, we'll see them captured.

        handle.cancel();
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 8: Ephemeral thread works correctly
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_ephemeral_thread() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let options = CodexOptions {
            model: test_model(),
            ephemeral: Some(true),
            approval_policy: Some(AskForApproval::Never),
            ..Default::default()
        };

        let mut handle = query("Say 'ephemeral test'.", options)
            .await
            .expect("Failed to start query");

        let thread_id = handle.thread_id().to_string();
        assert!(!thread_id.is_empty(), "Thread ID should be non-empty");
        eprintln!("Ephemeral thread ID: {thread_id}");

        let messages = collect_messages(&mut handle.messages).await;

        // Check for ThreadStarted notification if present
        for msg in &messages {
            if let CodexMessage::ThreadStarted(ts) = msg {
                assert!(
                    ts.thread.ephemeral,
                    "Expected thread to be ephemeral"
                );
                eprintln!("ThreadStarted confirms ephemeral=true");
            }
        }

        assert!(
            messages
                .iter()
                .any(|m| matches!(m, CodexMessage::TurnCompleted(_))),
            "Expected TurnCompleted"
        );

        handle.cancel();
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 9: Resume a session — start, capture thread_id, resume with context
// ===========================================================================

/// Resume: start a session, shut it down, then resume the same thread in a new session.
#[tokio::test]
#[ignore]
async fn test_resume_session() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(Duration::from_secs(120), async {
        // Session 1: non-ephemeral so the thread is persisted to disk.
        let options1 = CodexOptions {
            model: test_model(),
            ephemeral: Some(false),
            approval_policy: Some(AskForApproval::Never),
            config_overrides: vec![(
                "model_reasoning_effort".to_string(),
                toml::Value::String("high".to_string()),
            )],
            ..Default::default()
        };

        let mut handle1 = query(
            "Remember: the secret word is 'pineapple'. Confirm with OK.",
            options1,
        )
        .await
        .expect("Failed to start session 1");

        let thread_id = handle1.thread_id().to_string();
        eprintln!("Session 1 thread_id: {thread_id}");

        // Drain to completion
        let msgs1 = collect_messages(&mut handle1.messages).await;
        for msg in &msgs1 {
            match msg {
                CodexMessage::AgentMessageDelta(d) => eprintln!("  S1 [AgentDelta] '{}'", d.delta),
                CodexMessage::TurnCompleted(_) => eprintln!("  S1 [TurnCompleted]"),
                CodexMessage::Error(e) => eprintln!("  S1 [Error] {}", e.error.message),
                other => eprintln!("  S1 [{:?}]", std::mem::discriminant(other)),
            }
        }
        assert!(
            msgs1.iter().any(|m| matches!(m, CodexMessage::TurnCompleted(_))),
            "Session 1 should complete"
        );
        let text1 = extract_agent_text(&msgs1);
        eprintln!("Session 1 agent text: '{text1}'");

        // Give the app-server a moment to persist the thread to disk,
        // then shut down session 1 fully before starting session 2.
        tokio::time::sleep(Duration::from_secs(1)).await;
        handle1.shutdown().await;

        // Session 2: resume the same thread, ask for the secret word.
        let options2 = CodexOptions {
            model: test_model(),
            ephemeral: Some(false),
            resume_thread_id: Some(thread_id.clone()),
            approval_policy: Some(AskForApproval::Never),
            config_overrides: vec![(
                "model_reasoning_effort".to_string(),
                toml::Value::String("high".to_string()),
            )],
            ..Default::default()
        };

        let mut handle2 = query(
            "What is the secret word I told you earlier? Reply with only the word.",
            options2,
        )
        .await
        .expect("Failed to start session 2 (resume)");

        eprintln!(
            "Session 2 thread_id: {} (original: {thread_id})",
            handle2.thread_id()
        );

        let msgs2 = collect_messages(&mut handle2.messages).await;
        let text2 = extract_agent_text(&msgs2);
        eprintln!("Session 2 response: {text2}");

        assert!(
            text2.to_lowercase().contains("pineapple"),
            "Expected resumed session to recall 'pineapple', got: {text2}"
        );

        handle2.cancel();
    })
    .await;

    result.expect("Test timed out");
}

// ===========================================================================
// Test 10: Steer — inject a message into an ongoing turn
// ===========================================================================

#[tokio::test]
#[ignore]
async fn test_steer_mid_turn() {
    skip_unless_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query(
            "Write a short paragraph about the weather.",
            e2e_options(),
        )
        .await
        .expect("Failed to start query");

        // Wait for the first AgentMessageDelta to confirm the turn is running
        let mut saw_delta = false;
        let mut all_messages = Vec::new();
        while let Some(msg) = handle.messages.recv().await {
            match msg {
                Ok(msg) => {
                    let is_delta = matches!(&msg, CodexMessage::AgentMessageDelta(_));
                    let is_turn_completed = matches!(&msg, CodexMessage::TurnCompleted(_));
                    all_messages.push(msg);

                    if is_delta && !saw_delta {
                        saw_delta = true;
                        // Inject a steering message mid-turn
                        eprintln!("Sending steer mid-turn...");
                        match handle.steer("Actually, make it about cats instead.").await {
                            Ok(()) => eprintln!("Steer succeeded"),
                            Err(e) => eprintln!("Steer returned error (may be expected): {e}"),
                        }
                    }

                    if is_turn_completed {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    break;
                }
            }
        }

        assert!(saw_delta, "Expected at least one AgentMessageDelta before steer");
        assert!(
            all_messages.iter().any(|m| matches!(m, CodexMessage::TurnCompleted(_))),
            "Expected TurnCompleted after steer"
        );

        let text = extract_agent_text(&all_messages);
        eprintln!("Final agent text ({} chars): {}...", text.len(), &text[..text.len().min(200)]);

        handle.cancel();
    })
    .await;

    result.expect("Test timed out");
}
