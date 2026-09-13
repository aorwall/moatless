//! Continuing a past conversation, through `--resume` and through `--continue`.

use crate::harness::{
    Backend, LONG_TEST_TIMEOUT, TEST_MODEL, auth_env, collect_messages, extract_result_text,
    fresh_test_cwd, init_tracing,
};
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::SdkMessage;
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
