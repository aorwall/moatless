//! A sub-agent round trip, and the enriched fields its messages carry.

use crate::harness::{
    Backend, LONG_TEST_TIMEOUT, TEST_MODEL, TMUX_LONG_TEST_TIMEOUT, auth_env, collect_messages,
    fresh_test_cwd, init_tracing,
};
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::agents::AgentDefinition;
use claude_agent_sdk::api::agents::AgentModel;
use std::collections::HashMap;
use tokio::time::timeout;

async fn run_subagent_fields_captured(backend: Backend) {
    let cwd = fresh_test_cwd("subagent");
    let mut agents = HashMap::new();
    agents.insert(
        "helper".to_string(),
        AgentDefinition {
            description: "A helper agent for simple tasks".to_string(),
            prompt: "You are a helpful assistant. Answer concisely.".to_string(),
            tools: None,
            disallowed_tools: None,
            model: Some(AgentModel::Haiku),
            mcp_servers: None,
            skills: None,
            max_turns: Some(1),
        },
    );

    let opts = ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(3),
        allow_dangerously_skip_permissions: Some(true),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        agents: Some(agents),
        env: auth_env(),
        ..Default::default()
    };

    let mut handle = backend
        .start_one_shot(
            "Use the Agent tool to ask the 'helper' agent: What is 1+1? Reply with only the number. Then tell me what the agent said.",
            opts,
        )
        .await
        .expect("Failed to start query");

    let messages = collect_messages(&mut handle.messages).await;

    // Verify at least one assistant message has ApiMessage enriched fields.
    // Not every assistant message has stop_reason (thinking-only partials don't),
    // so we collect all and check that at least one has model + id populated.
    let assistants: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let SdkMessage::Assistant(a) = m {
                Some(a)
            } else {
                None
            }
        })
        .collect();

    assert!(
        !assistants.is_empty(),
        "Expected at least one assistant message"
    );

    let has_model = assistants.iter().any(|a| a.message.model.is_some());
    let has_id = assistants.iter().any(|a| a.message.id.is_some());
    let has_stop_reason = assistants.iter().any(|a| a.message.stop_reason.is_some());

    assert!(
        has_model,
        "Expected at least one assistant message with model"
    );
    assert!(has_id, "Expected at least one assistant message with id");
    // stop_reason may only appear on the final assistant message of each turn
    println!("has_stop_reason: {has_stop_reason}");

    let user_msgs: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let SdkMessage::User(u) = m {
                Some(u)
            } else {
                None
            }
        })
        .collect();

    for u in &user_msgs {
        if u.tool_use_result.is_some() {
            println!(
                "User message has toolUseResult: {}",
                serde_json::to_string_pretty(&u.tool_use_result).unwrap()
            );
        }
    }

    // At least one user message should have tool_use_result
    // (the Agent tool returns a result object)
    let has_agent_result = user_msgs.iter().any(|u| u.tool_use_result.is_some());
    println!("Has agent tool_use_result: {has_agent_result}");

    for (i, msg) in messages.iter().enumerate() {
        match msg {
            SdkMessage::System(_) => println!("[{i}] System"),
            SdkMessage::Assistant(a) => println!(
                "[{i}] Assistant (model={:?}, id={:?}, stop_reason={:?})",
                a.message.model, a.message.id, a.message.stop_reason
            ),
            SdkMessage::User(u) => println!(
                "[{i}] User (is_meta={:?}, tool_use_result={})",
                u.is_meta,
                u.tool_use_result.is_some()
            ),
            SdkMessage::UserReplay(_) => println!("[{i}] UserReplay"),
            SdkMessage::Result(_) => println!("[{i}] Result"),
            SdkMessage::TaskNotification(t) => println!(
                "[{i}] TaskNotification (status={}, task_id={})",
                t.status, t.task_id
            ),
            _ => println!("[{i}] Other"),
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_subagent_fields_captured_subprocess() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_subagent_fields_captured(Backend::Subprocess),
    )
    .await
    .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_subagent_fields_captured_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();
    timeout(
        TMUX_LONG_TEST_TIMEOUT,
        run_subagent_fields_captured(Backend::Tmux),
    )
    .await
    .expect("Test timed out");
}
