//! Continuing a past conversation, through `--resume` and through `--continue`.

use crate::harness::{
    Backend, LONG_TEST_TIMEOUT, TEST_MODEL, auth_env, collect_messages, e2e_options_with_cwd,
    extract_result_text, fresh_test_cwd, init_tracing,
};
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::transcript_dir;
use tokio::time::timeout;

async fn run_session_resume_recalls_context(backend: Backend) {
    let tmp = fresh_test_cwd("resume");

    let base_options = || ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        cwd: Some(tmp.to_string_lossy().to_string()),
        env: auth_env(),
        ..Default::default()
    };

    // Turn 1: Ask the model to write code containing a distinctive string
    let mut handle = backend
        .start_one_shot(
            "Write a Rust function called `get_answer()` that returns the &str \"blue elephant\". \
             Just show the function, nothing else.",
            base_options(),
        )
        .await
        .expect("Failed to start turn 1");

    let messages = collect_messages(&mut handle.messages).await;

    let session_id = messages
        .iter()
        .find_map(|m| {
            if let SdkMessage::System(init) = m {
                Some(init.session_id.clone())
            } else {
                None
            }
        })
        .expect("Turn 1 should have System init with session_id");

    let turn1_result = extract_result_text(&messages).expect("Turn 1 should succeed");
    eprintln!("Turn 1 session_id: {session_id}");
    eprintln!("Turn 1 result: {turn1_result}");

    let turn1_lower = turn1_result.to_lowercase();
    assert!(
        turn1_lower.contains("blue elephant"),
        "Turn 1 should contain 'blue elephant', got: {turn1_result}"
    );

    // Turn 2: Resume the session and ask about the previous code
    let resume_options = ClaudeOptions {
        resume: Some(session_id.clone()),
        ..base_options()
    };

    let mut handle = backend
        .start_one_shot(
            "What string does the `get_answer()` function return? Reply with just the string value.",
            resume_options,
        )
        .await
        .expect("Failed to start turn 2");

    let messages = collect_messages(&mut handle.messages).await;

    let turn2_session_id = messages.iter().find_map(|m| {
        if let SdkMessage::System(init) = m {
            Some(init.session_id.clone())
        } else {
            None
        }
    });

    let turn2_result = extract_result_text(&messages).expect("Turn 2 should succeed");
    eprintln!("Turn 2 session_id: {turn2_session_id:?}");
    eprintln!("Turn 2 result: {turn2_result}");

    // Session ID should match (same session resumed)
    if let Some(ref t2_id) = turn2_session_id {
        assert_eq!(
            &session_id, t2_id,
            "Resumed session should have the same session_id"
        );
    }

    // Model should recall the string from turn 1
    let turn2_lower = turn2_result.to_lowercase();
    assert!(
        turn2_lower.contains("blue elephant"),
        "Turn 2 should recall 'blue elephant' from turn 1, got: {turn2_result}"
    );
}

#[tokio::test]
#[ignore]
async fn test_session_resume_recalls_context_subprocess() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_session_resume_recalls_context(Backend::Subprocess),
    )
    .await
    .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_session_resume_recalls_context_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_session_resume_recalls_context(Backend::Tmux),
    )
    .await
    .expect("Test timed out");
}

async fn run_session_continue_recalls_context(backend: Backend) {
    let tmp = fresh_test_cwd("continue");

    let base_options = || ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        cwd: Some(tmp.to_string_lossy().to_string()),
        env: auth_env(),
        ..Default::default()
    };

    // Turn 1: Ask the model to write code containing a distinctive string
    let mut handle = backend
        .start_one_shot(
            "Write a Rust function called `get_answer()` that returns the &str \"purple giraffe\". \
             Just show the function, nothing else.",
            base_options(),
        )
        .await
        .expect("Failed to start turn 1");

    let messages = collect_messages(&mut handle.messages).await;

    let turn1_result = extract_result_text(&messages).expect("Turn 1 should succeed");
    eprintln!("Turn 1 result: {turn1_result}");

    let turn1_lower = turn1_result.to_lowercase();
    assert!(
        turn1_lower.contains("purple giraffe"),
        "Turn 1 should contain 'purple giraffe', got: {turn1_result}"
    );

    // Turn 2: Continue the most recent session in this directory
    let continue_options = ClaudeOptions {
        continue_conversation: Some(true),
        ..base_options()
    };

    let mut handle = backend
        .start_one_shot(
            "What string does the `get_answer()` function return? Reply with just the string value.",
            continue_options,
        )
        .await
        .expect("Failed to start turn 2");

    let messages = collect_messages(&mut handle.messages).await;

    let turn2_result = extract_result_text(&messages).expect("Turn 2 should succeed");
    eprintln!("Turn 2 result: {turn2_result}");

    // Model should recall the string from turn 1
    let turn2_lower = turn2_result.to_lowercase();
    assert!(
        turn2_lower.contains("purple giraffe"),
        "Turn 2 should recall 'purple giraffe' from turn 1, got: {turn2_result}"
    );
}

#[tokio::test]
#[ignore]
async fn test_session_continue_recalls_context_subprocess() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_session_continue_recalls_context(Backend::Subprocess),
    )
    .await
    .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_session_continue_recalls_context_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_session_continue_recalls_context(Backend::Tmux),
    )
    .await
    .expect("Test timed out");
}

fn result_cost(messages: &[SdkMessage]) -> f64 {
    messages
        .iter()
        .find_map(|m| match m {
            SdkMessage::Result(ResultMessage::Success(s)) => Some(s.total_cost_usd),
            _ => None,
        })
        .expect("run should end in a successful Result")
}

/// Waits for the CLI to exit, which is when it writes its `cost-state` line.
async fn wait_for_exit(handle: &claude_agent_sdk::query::QueryHandle) {
    while handle.is_running() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// The CLI reports a Session's running total; the SDK reports each run's own
/// share, so the two runs add up to the transcript's last `cost-state`.
#[tokio::test]
#[ignore]
async fn test_resumed_result_reports_the_runs_own_cost() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    let tmp = fresh_test_cwd("resume-cost");
    let base_options = || e2e_options_with_cwd(Some(&tmp));

    let run = async {
        let mut handle = Backend::Subprocess
            .start_one_shot("Reply with the word apple.", base_options())
            .await
            .expect("Failed to start turn 1");
        // Resume as soon as the `Result` arrives, as the sandbox does, while
        // the first CLI is still exiting and writing its cost-state.
        let turn1 = collect_messages(&mut handle.messages).await;
        drop(handle);
        let session_id = turn1
            .iter()
            .find_map(|m| match m {
                SdkMessage::System(init) => Some(init.session_id.clone()),
                _ => None,
            })
            .expect("Turn 1 should have System init with session_id");

        let resume_options = ClaudeOptions {
            resume: Some(session_id.clone()),
            ..base_options()
        };
        let mut handle = Backend::Subprocess
            .start_one_shot("Reply with the word pear.", resume_options)
            .await
            .expect("Failed to start turn 2");
        let turn2 = collect_messages(&mut handle.messages).await;
        wait_for_exit(&handle).await;
        (session_id, result_cost(&turn1), result_cost(&turn2))
    };
    let (session_id, turn1, turn2) = timeout(LONG_TEST_TIMEOUT, run)
        .await
        .expect("Test timed out");

    let path = transcript_dir(Some(&tmp.to_string_lossy()))
        .expect("transcript dir")
        .join(format!("{session_id}.jsonl"));
    let body = std::fs::read_to_string(&path).expect("read transcript");
    let session_total = body
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["type"] == "cost-state")
        .find_map(|v| v["totalCostUSD"].as_f64())
        .expect("transcript should hold a cost-state line");
    eprintln!("turn 1 ${turn1}, turn 2 ${turn2}, Session ${session_total}");
    assert!(turn1 > 0.0 && turn2 > 0.0);
    assert!(
        (turn1 + turn2 - session_total).abs() < 1e-6,
        "runs ${turn1} + ${turn2} should add up to the Session's ${session_total}"
    );
}
