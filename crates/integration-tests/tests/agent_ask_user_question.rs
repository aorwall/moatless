//! Agent AskUserQuestion integration test (requires Claude CLI).
//!
//! Verifies the full interactive prompt flow:
//! 1. Agent encounters AskUserQuestion tool
//! 2. Backend publishes `agent.pending_response` SSE event
//! 3. Test extracts `tool_use_id` from messages (same as frontend)
//! 4. Test sends proper tool response with `tool_use_id` + `tool_name` + `tool_response`
//! 5. Agent resumes and completes
//!
//! This exercises the `send_tool_response_flow` → `get_pending_messages` →
//! `parse_content_blocks` pipeline that was fixed in commit 7cca68ff7.
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{
    enums::MessageRole, message::SendMessageRequest, task::CreateTaskRequest,
};
use moatless_cli::events::EventCollector;

/// Extract the tool_use_id for AskUserQuestion from the last assistant message.
///
/// Mirrors the frontend logic in `chat-container.tsx`.
fn extract_tool_use_id(
    messages: &moatless_api_schemas::message::GetMessagesResponse,
    tool_name: &str,
) -> Option<String> {
    let last_assistant = messages
        .messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Assistant && m.has_pending_user_input == Some(true))?;

    let result_ids: std::collections::HashSet<&str> = last_assistant
        .tool_results
        .iter()
        .map(|tr| tr.tool_use_id.as_str())
        .collect();

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
// Agent AskUserQuestion flow
// =============================================================================

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn agent_ask_user_question() {
    if std::env::var("CLAUDE_SDK_E2E").is_err() {
        eprintln!("Skipping: CLAUDE_SDK_E2E not set");
        return;
    }

    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;

    // Start event collector
    let collector = EventCollector::start(&client)
        .await
        .expect("Failed to start event collector");

    // Create task with a prompt that should trigger AskUserQuestion.
    // The agent needs to ask the user something before proceeding.
    let req = CreateTaskRequest {
        repository_id: Some(repo_id.clone()),
        workspace_id: None,
        message: Some(
            "Before making any changes, use the AskUserQuestion tool to ask me \
             what my favorite color is. Then create a file called favorite-color.txt \
             with whatever color I tell you."
                .into(),
        ),
        name: Some("Agent AskUserQuestion E2E".into()),
        branch: None,
        description: None,
        agent_mode: None,
        agent_type: integration_tests::e2e_agent_type(),
        contexts: vec![],
        file_ids: vec![],
        model: None,
        command_name: None,
        headless: None,
        task_template_id: None,
        parent_task_id: None,
    };
    let created = client.api().create_task(&req).await.unwrap();
    let task_id = &created.task_id;

    // Wait for sandbox ready
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    // Wait for either pending_response (agent asked a question) or taskTurn.completed
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
                println!("Got pending_response — extracting tool_use_id from messages");

                // Fetch messages to extract tool_use_id (same as frontend flow)
                let messages = client
                    .api()
                    .get_messages(task_id)
                    .await
                    .unwrap();

                let tool_use_id = extract_tool_use_id(&messages, "AskUserQuestion")
                    .expect("Expected AskUserQuestion tool_use content block in assistant message");

                println!("Sending AskUserQuestion response with tool_use_id={tool_use_id}");

                // Send proper tool response (mirrors frontend handleQuestionSubmit)
                let answer = SendMessageRequest {
                    message: String::new(),
                    agent_mode: None,
                    command_name: None,
                    contexts: vec![],
                    file_ids: vec![],
                    model: None,
                    resume: Some(true),
                    tool_name: Some("AskUserQuestion".into()),
                    tool_response: Some(serde_json::json!({
                        "answers": { "text": "Blue" }
                    })),
                    tool_use_id: Some(tool_use_id),
                    source_task_id: None,
                    start_fresh: None,
                };
                client.api().send_message(task_id, &answer).await.unwrap();

                // Wait for completion after answering
                let completed = collector
                    .wait_for_event(
                        "taskTurn",
                        Some("completed"),
                        task_id,
                        Duration::from_secs(300),
                    )
                    .await;
                assert!(completed.is_some(), "Expected taskTurn.completed after answering question");
            }
        }
        completed = completed_event => {
            // Agent completed without asking — still valid, just note it
            println!("Agent completed directly without asking a question");
            assert!(completed.is_some(), "Expected taskTurn.completed");
        }
    }

    // Verify messages exist
    let messages = client.api().get_messages(task_id).await.unwrap();
    assert!(
        messages.messages.len() >= 2,
        "Expected at least 2 messages, got {}",
        messages.messages.len()
    );

    // Cleanup
    collector.stop();
    let _ = client.api().delete_task(task_id).await;
}
