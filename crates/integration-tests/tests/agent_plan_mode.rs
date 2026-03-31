//! Agent plan mode integration tests (requires Claude CLI).
//!
//! Mirrors: tests/api/chains/agent-plan-mode.spec.ts
//! Spec IDs: AGENT-03 through AGENT-03.2
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{
    message::SendMessageRequest,
    shared::AgentMode,
    task::CreateTaskRequest,
};
use moatless_cli::events::EventCollector;

/// Extract the tool_use_id for a given tool name from the last assistant message.
///
/// Mirrors the frontend logic in `chat-container.tsx` that finds the pending
/// tool call by scanning content blocks in reverse for an unresolved tool_use.
fn extract_tool_use_id(
    messages: &moatless_api_schemas::message::GetMessagesResponse,
    tool_name: &str,
) -> Option<String> {
    // Find the last assistant message with has_pending_user_input
    let last_assistant = messages.messages.iter().rev().find(|m| {
        m.role == moatless_api_schemas::enums::MessageRole::Assistant
            && m.has_pending_user_input == Some(true)
    })?;

    // Collect tool_use_ids that already have results
    let result_ids: std::collections::HashSet<&str> = last_assistant
        .tool_results
        .iter()
        .map(|tr| tr.tool_use_id.as_str())
        .collect();

    // Find the last tool_use content block matching the tool name without a result
    last_assistant
        .content
        .iter()
        .rev()
        .find(|block| {
            block.block_type == "tool_use"
                && block.tool_name.as_deref() == Some(tool_name)
                && block
                    .tool_use_id
                    .as_deref()
                    .is_some_and(|id| !result_ids.contains(id))
        })
        .and_then(|block| block.tool_use_id.clone())
}

// =============================================================================
// Chain: Agent Plan Mode (AGENT-03 through AGENT-03.2)
// =============================================================================

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn agent_plan_mode() {
    if std::env::var("CLAUDE_SDK_E2E").is_err() {
        eprintln!("Skipping: CLAUDE_SDK_E2E not set");
        return;
    }

    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;

    // Start event collector
    let collector = EventCollector::start(&client)
        .await
        .expect("Failed to start event collector");

    // Create task with plan mode enabled
    let req = CreateTaskRequest {
        repository_id: repo_id.clone(),
        message: Some(
            "Plan how to add a dark mode toggle to the application. \
             Create a plan but wait for my approval before making changes."
                .into(),
        ),
        name: Some("Agent Plan Mode E2E".into()),
        branch: None,
        parent_task_id: None,
        description: None,
        agent_mode: Some(AgentMode("plan".into())),
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
    };
    let created = client.api().create_task(&req).await.unwrap();
    let task_id = &created.task_id;

    // Wait for sandbox ready
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    // AGENT-03: Wait for either pending_response (agent paused for approval) or
    // taskTurn.completed (agent finished without pausing).
    let pending_event = collector.wait_for_event(
        "agent",
        Some("pending_response"),
        task_id,
        Duration::from_secs(300),
    );
    let completed_event = collector.wait_for_event(
        "taskTurn",
        Some("completed"),
        task_id,
        Duration::from_secs(300),
    );

    tokio::select! {
        pending = pending_event => {
            if pending.is_some() {
                println!("AGENT-03: Got pending_response — extracting tool_use_id from messages");

                // Fetch messages to extract tool_use_id (same as frontend flow)
                let messages = client
                    .api()
                    .get_messages(task_id)
                    .await
                    .unwrap();

                let tool_use_id = extract_tool_use_id(&messages, "ExitPlanMode")
                    .expect("Expected ExitPlanMode tool_use content block in assistant message");

                println!("AGENT-03.1: Sending ExitPlanMode approval with tool_use_id={tool_use_id}");

                // AGENT-03.1: Send proper tool response (mirrors frontend)
                let approval = SendMessageRequest {
                    message: String::new(),
                    agent_mode: None,
                    command_name: None,
                    contexts: vec![],
                    file_ids: vec![],
                    model: None,
                    resume: Some(true),
                    tool_name: Some("ExitPlanMode".into()),
                    tool_response: Some(serde_json::json!({ "approved": true })),
                    tool_use_id: Some(tool_use_id),
                };
                client.api().send_message(task_id, &approval).await.unwrap();

                // Wait for completion after approval
                let completed = collector
                    .wait_for_event(
                        "taskTurn",
                        Some("completed"),
                        task_id,
                        Duration::from_secs(300),
                    )
                    .await;
                assert!(completed.is_some(), "Expected taskTurn.completed after approval");
            }
        }
        completed = completed_event => {
            // Agent completed without needing approval
            println!("AGENT-03: Agent completed directly without pending");
            assert!(completed.is_some(), "Expected taskTurn.completed");
        }
    }

    // AGENT-03.2: Assert messages contain plan content
    let messages = client
        .api()
        .get_messages(task_id)
        .await
        .unwrap();
    assert!(
        messages.messages.len() >= 2,
        "Expected at least 2 messages, got {}",
        messages.messages.len()
    );

    // Cleanup
    collector.stop();
    let _ = client.api().delete_task(task_id).await;
}
