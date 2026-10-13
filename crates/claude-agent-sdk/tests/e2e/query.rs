//! One-shot `query`: the message flow it produces, the session id the `Result`
//! carries, and the model and tool list a real `System` init holds.

use crate::harness::{
    Backend, TEST_TIMEOUT, collect_messages, e2e_options, e2e_options_with_cwd,
    extract_result_text, fresh_test_cwd, init_tracing,
};
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::query;
use tokio::time::timeout;

async fn run_simple_query(backend: Backend) {
    let cwd = fresh_test_cwd("simple-query");
    let mut handle = backend
        .start_one_shot(
            "What is 2+2? Reply with only the number.",
            e2e_options_with_cwd(Some(&cwd)),
        )
        .await
        .expect("Failed to start query");

    let messages = collect_messages(&mut handle.messages).await;

    // Should have at least: System init, Assistant, Result
    assert!(
        messages.iter().any(|m| matches!(m, SdkMessage::System(_))),
        "Expected System init message"
    );
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, SdkMessage::Assistant(_))),
        "Expected Assistant message"
    );
    assert!(
        messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
        "Expected Result message"
    );

    let result_text = extract_result_text(&messages).expect("Expected successful result");
    assert!(
        result_text.contains('4'),
        "Expected result to contain '4', got: {result_text}"
    );

    // Check cost (subprocess only) and turns (both backends)
    if let Some(SdkMessage::Result(ResultMessage::Success(success))) =
        messages.iter().find(|m| matches!(m, SdkMessage::Result(_)))
    {
        if backend.has_cost_data() {
            assert!(
                success.total_cost_usd > 0.0,
                "Expected non-zero cost, got: {}",
                success.total_cost_usd
            );
        }
        assert!(
            success.num_turns >= 1,
            "Expected at least 1 turn, got: {}",
            success.num_turns
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_simple_query_subprocess() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    timeout(TEST_TIMEOUT, run_simple_query(Backend::Subprocess))
        .await
        .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_simple_query_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();
    timeout(TEST_TIMEOUT, run_simple_query(Backend::Tmux))
        .await
        .expect("Test timed out");
}

async fn run_result_contains_session_id(backend: Backend) {
    let cwd = fresh_test_cwd("session-id");
    let mut handle = backend
        .start_one_shot("Say hello.", e2e_options_with_cwd(Some(&cwd)))
        .await
        .expect("Failed to start query");

    let messages = collect_messages(&mut handle.messages).await;

    let system_session_id = messages.iter().find_map(|m| {
        if let SdkMessage::System(init) = m {
            Some(init.session_id.clone())
        } else {
            None
        }
    });

    let result_session_id = messages.iter().find_map(|m| match m {
        SdkMessage::Result(ResultMessage::Success(s)) => Some(s.session_id.clone()),
        SdkMessage::Result(ResultMessage::Error(e)) => Some(e.session_id.clone()),
        _ => None,
    });

    let sys_id = system_session_id.expect("System init should have session_id");
    let res_id = result_session_id.expect("Result should have session_id");

    assert!(!sys_id.is_empty(), "System session_id should be non-empty");
    assert!(!res_id.is_empty(), "Result session_id should be non-empty");
    assert_eq!(sys_id, res_id, "Session IDs should match");
}

#[tokio::test]
#[ignore]
async fn test_result_contains_session_id_subprocess() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    timeout(
        TEST_TIMEOUT,
        run_result_contains_session_id(Backend::Subprocess),
    )
    .await
    .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_result_contains_session_id_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();
    timeout(TEST_TIMEOUT, run_result_contains_session_id(Backend::Tmux))
        .await
        .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_system_init_has_model_and_tools() {
    skip_unless_subprocess_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("Say hi.", e2e_options())
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        let init = messages
            .iter()
            .find_map(|m| {
                if let SdkMessage::System(init) = m {
                    Some(init)
                } else {
                    None
                }
            })
            .expect("Expected System init message");

        assert!(
            init.model.contains("haiku"),
            "Expected model to contain 'haiku', got: {}",
            init.model
        );

        assert!(
            !init.tools.is_empty(),
            "Expected non-empty tools list, got: {:?}",
            init.tools
        );

        eprintln!("Model: {}", init.model);
        eprintln!("Tools ({}): {:?}", init.tools.len(), init.tools);
    })
    .await;

    result.expect("Test timed out");
}
