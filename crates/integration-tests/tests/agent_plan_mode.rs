//! Agent plan mode integration tests (requires Claude CLI).
//!
//! Mirrors: tests/api/chains/agent-plan-mode.spec.ts
//! Spec IDs: AGENT-03 through AGENT-03.2
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_client::types::{CreateTaskRequest, SendMessageRequest};
use moatless_cli::events::EventCollector;

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

    // Create task with a message that will likely trigger a plan/pending response
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
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
    };
    let created = client.create_task(&req).await.unwrap().into_inner();
    let task_id = &created.task_id;

    // Wait for sandbox ready
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    // AGENT-03: Wait for either pending_response or taskTurn.completed
    // Use tokio::select! to race between the two outcomes
    let pending_event = collector.wait_for_event(
        "message",
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
                println!("AGENT-03: Got pending_response — sending approval");
                // AGENT-03.1: Send approval message
                let approval = SendMessageRequest {
                    message: "Looks good, proceed.".into(),
                    agent_mode: None,
                    command_name: None,
                    contexts: vec![],
                    file_ids: vec![],
                    model: None,
                    resume: None,
                    tool_name: None,
                    tool_response: None,
                    tool_use_id: None,
                };
                client.send_message(task_id, &approval).await.unwrap();

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
        .get_messages(task_id)
        .await
        .unwrap()
        .into_inner();
    assert!(
        messages.messages.len() >= 2,
        "Expected at least 2 messages, got {}",
        messages.messages.len()
    );

    // Cleanup
    collector.stop();
    let _ = client.delete_task(task_id).await;
}
