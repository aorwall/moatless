#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]
//! Smoke test for codex-sdk.
//!
//! Verifies:
//! 1. Single roundtrip — AgentMessageDelta + TurnCompleted, thread_id non-empty.
//! 2. Multi-turn via CodexClient — turn 2 recalls a phrase from turn 1.
//! 3. Resume session — persistent thread (ephemeral=false), shutdown, then
//!    resume_thread_id recalls the secret word.
//!
//! Unlike `claude-agent-sdk`'s smoke (which asserts a pinned CLI version on
//! PATH), codex-sdk has no runtime binary to interrogate — it embeds the
//! Codex runtime via Rust path dependencies pinned in `Cargo.lock`. Drift
//! is caught at compile time, not runtime.
//!
//! Gated on CODEX_SDK_E2E=1. Run from the codex-sdk crate root (standalone
//! workspace; does not participate in the parent moatless workspace):
//!
//!   cd crates/codex-sdk
//!   CODEX_SDK_E2E=1 cargo test --test smoke -- --ignored --nocapture

use std::time::Duration;

use tokio::time::timeout;

use codex_sdk::{AskForApproval, CodexClient, CodexMessage, CodexOptions, query};

const TEST_MODEL: &str = "gpt-5.3-codex";
const TEST_TIMEOUT: Duration = Duration::from_secs(90);
const MULTI_TURN_TIMEOUT: Duration = Duration::from_secs(120);
const RESUME_TIMEOUT: Duration = Duration::from_secs(120);

fn skip_unless_e2e() -> bool {
    if std::env::var("CODEX_SDK_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping smoke test (set CODEX_SDK_E2E=1 to run)");
        return true;
    }
    false
}

/// Options mirroring `e2e.rs::e2e_options()` — gpt-5.3-codex, high reasoning,
/// auto-approve. Toggle `ephemeral` for the resume test.
fn smoke_options(ephemeral: bool) -> CodexOptions {
    CodexOptions {
        model: Some(TEST_MODEL.to_string()),
        ephemeral: Some(ephemeral),
        approval_policy: Some(AskForApproval::Never),
        config_overrides: vec![(
            "model_reasoning_effort".to_string(),
            toml::Value::String("high".to_string()),
        )],
        ..Default::default()
    }
}

fn extract_agent_text(messages: &[CodexMessage]) -> String {
    let mut text = String::new();
    for msg in messages {
        if let CodexMessage::AgentMessageDelta(delta) = msg {
            text.push_str(&delta.delta);
        }
    }
    text
}

async fn collect_until_turn_completed(
    rx: &mut tokio::sync::mpsc::Receiver<codex_sdk::Result<CodexMessage>>,
) -> Vec<CodexMessage> {
    let mut out = Vec::new();
    while let Some(msg) = rx.recv().await {
        let msg = msg.expect("recv error");
        let done = matches!(&msg, CodexMessage::TurnCompleted(_));
        out.push(msg);
        if done {
            break;
        }
    }
    out
}

// --- Test 1: single roundtrip + thread_id non-empty ---

#[tokio::test]
#[ignore = "spawns codex runtime + makes API calls; gated on CODEX_SDK_E2E=1"]
async fn test_smoke_simple_query() {
    if skip_unless_e2e() {
        return;
    }

    timeout(TEST_TIMEOUT, async {
        let mut handle = query(
            "What is 2+2? Reply with only the number.",
            smoke_options(true),
        )
        .await
        .expect("failed to start query");

        let thread_id = handle.thread_id().to_string();
        assert!(!thread_id.is_empty(), "thread_id should be non-empty");

        let messages = collect_until_turn_completed(&mut handle.messages).await;

        assert!(
            messages
                .iter()
                .any(|m| matches!(m, CodexMessage::AgentMessageDelta(_))),
            "missing AgentMessageDelta"
        );
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, CodexMessage::TurnCompleted(_))),
            "missing TurnCompleted"
        );
        let text = extract_agent_text(&messages);
        assert!(text.contains('4'), "agent text missing '4': {text}");

        handle.cancel();
    })
    .await
    .expect("simple_query timed out");
}

// --- Test 2: multi-turn via CodexClient (in-process turn-over-turn) ---

#[tokio::test]
#[ignore = "spawns codex runtime + makes API calls; gated on CODEX_SDK_E2E=1"]
async fn test_smoke_multi_turn() {
    if skip_unless_e2e() {
        return;
    }

    timeout(MULTI_TURN_TIMEOUT, async {
        let mut client = CodexClient::new(smoke_options(true))
            .await
            .expect("failed to create CodexClient");

        // Turn 1
        client
            .send("Remember the number 42. Reply with only 'OK'.")
            .await
            .expect("failed to send turn 1");
        let mut turn1_text = String::new();
        while let Some(msg) = client.recv().await {
            match msg.expect("turn 1 error") {
                CodexMessage::AgentMessageDelta(d) => turn1_text.push_str(&d.delta),
                CodexMessage::TurnCompleted(_) => break,
                _ => {}
            }
        }
        eprintln!("Turn 1 response: {turn1_text}");

        // Turn 2
        client
            .send("What number did I ask you to remember? Reply with only the number.")
            .await
            .expect("failed to send turn 2");
        let mut turn2_text = String::new();
        while let Some(msg) = client.recv().await {
            match msg.expect("turn 2 error") {
                CodexMessage::AgentMessageDelta(d) => turn2_text.push_str(&d.delta),
                CodexMessage::TurnCompleted(_) => break,
                _ => {}
            }
        }
        eprintln!("Turn 2 response: {turn2_text}");

        assert!(
            turn2_text.contains("42"),
            "turn 2 should recall '42', got: {turn2_text}"
        );

        client.cancel();
    })
    .await
    .expect("multi_turn timed out");
}

// --- Test 3: resume session across processes (needs 8MB worker stack) ---

/// Resume requires a custom runtime with 8MB worker stacks because the
/// in-process codex app-server has deep call chains that overflow the default
/// 2MB stack in debug builds. Mirrors `tests/e2e.rs::test_resume_session`.
#[test]
#[ignore = "spawns codex runtime + makes API calls; gated on CODEX_SDK_E2E=1"]
fn test_smoke_session_resume() {
    if skip_unless_e2e() {
        return;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        timeout(RESUME_TIMEOUT, async {
            // Session 1: persistent (ephemeral=false) so the thread is saved.
            let mut h1 = query(
                "Remember: the secret word is 'pineapple'. Confirm with OK.",
                smoke_options(false),
            )
            .await
            .expect("failed to start session 1");

            let thread_id = h1.thread_id().to_string();
            assert!(!thread_id.is_empty(), "session 1 thread_id empty");
            eprintln!("Session 1 thread_id: {thread_id}");

            let msgs1 = collect_until_turn_completed(&mut h1.messages).await;
            assert!(
                msgs1
                    .iter()
                    .any(|m| matches!(m, CodexMessage::TurnCompleted(_))),
                "session 1 should complete"
            );

            // Let the app-server persist to disk before shutting down.
            tokio::time::sleep(Duration::from_secs(1)).await;
            h1.shutdown().await;

            // Session 2: resume the same thread.
            let mut resume_opts = smoke_options(false);
            resume_opts.resume_thread_id = Some(thread_id.clone());

            let mut h2 = query(
                "What is the secret word I told you earlier? Reply with only the word.",
                resume_opts,
            )
            .await
            .expect("failed to start session 2 (resume)");

            let msgs2 = collect_until_turn_completed(&mut h2.messages).await;
            let text2 = extract_agent_text(&msgs2);
            eprintln!("Session 2 response: {text2}");
            assert!(
                text2.to_lowercase().contains("pineapple"),
                "resumed session should recall 'pineapple', got: {text2}"
            );

            h2.cancel();
        })
        .await
        .expect("session_resume timed out");
    });
}
