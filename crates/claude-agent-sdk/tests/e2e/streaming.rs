//! `ClaudeClient`: several user turns over one live process.

use crate::harness::{
    Backend, LONG_TEST_TIMEOUT, TEST_MODEL, auth_env, extract_assistant_text, extract_result_text,
    fresh_test_cwd, init_tracing,
};
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::SdkMessage;
use tokio::time::timeout;

async fn run_streaming_client_multi_turn(backend: Backend) {
    let cwd = fresh_test_cwd("streaming-multi-turn");
    let options = ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        env: auth_env(),
        ..Default::default()
    };

    let mut client = backend
        .create_client(options)
        .await
        .expect("Failed to create ClaudeClient");

    // Turn 1: Ask to remember a number
    client
        .send(
            "Remember the number 42. Just confirm you've noted it. Be very brief.",
            "",
        )
        .await
        .expect("Failed to send first message");

    let mut session_id = String::new();
    let mut turn1_messages = Vec::new();
    while let Some(msg) = client.recv().await {
        match msg {
            Ok(msg) => {
                if let SdkMessage::System(ref init) = msg {
                    session_id = init.session_id.clone();
                }
                let is_result = matches!(&msg, SdkMessage::Result(_));
                turn1_messages.push(msg);
                if is_result {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Turn 1 error: {e}");
                break;
            }
        }
    }

    assert!(
        turn1_messages
            .iter()
            .any(|m| matches!(m, SdkMessage::Result(_))),
        "Turn 1 should produce a Result"
    );
    assert!(
        !session_id.is_empty(),
        "Should have received session_id from System init"
    );
    eprintln!(
        "Turn 1 result: {}",
        extract_result_text(&turn1_messages).unwrap_or_default()
    );

    // Turn 2: Ask what number was remembered.
    // For subprocess: the orchestrator exits after the first Result, so the
    // send may fail or yield no messages — log and return without panicking.
    // For tmux: the TUI keeps the channel open across turns; turn 2 is
    // expected to succeed.
    let send_result = client
        .send(
            "What number did I ask you to remember? Reply with just the number.",
            &session_id,
        )
        .await;

    match send_result {
        Ok(()) => {}
        Err(e) => {
            assert!(
                !backend.channel_stays_open_after_result(),
                "Tmux turn 2 send failed unexpectedly: {e}"
            );
            eprintln!(
                "WARNING: Failed to send turn 2 message \
                 (expected for subprocess where orchestrator exits on Result): {e}"
            );
            client.close();
            return;
        }
    }

    let mut turn2_messages = Vec::new();
    while let Some(msg) = client.recv().await {
        match msg {
            Ok(msg) => {
                let is_result = matches!(&msg, SdkMessage::Result(_));
                turn2_messages.push(msg);
                if is_result {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Turn 2 error: {e}");
                break;
            }
        }
    }

    if turn2_messages.is_empty() {
        assert!(
            !backend.channel_stays_open_after_result(),
            "Tmux turn 2 produced no messages"
        );
        eprintln!(
            "WARNING: Turn 2 produced no messages. Subprocess multi-turn is \
             not yet supported — the orchestrator exits after delivering a \
             Result message."
        );
        client.close();
        return;
    }

    let turn2_text = extract_assistant_text(&turn2_messages);
    let turn2_result = extract_result_text(&turn2_messages).unwrap_or_default();
    let combined = format!("{turn2_text} {turn2_result}");
    assert!(
        combined.contains("42"),
        "Expected turn 2 response to mention '42', got: {combined}"
    );

    client.close();
}

#[tokio::test]
#[ignore]
async fn test_streaming_client_multi_turn_subprocess() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_streaming_client_multi_turn(Backend::Subprocess),
    )
    .await
    .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_streaming_client_multi_turn_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_streaming_client_multi_turn(Backend::Tmux),
    )
    .await
    .expect("Test timed out");
}
