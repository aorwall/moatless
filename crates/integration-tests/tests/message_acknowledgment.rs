//! Message acknowledgment integration tests.
//!
//! Mirrors: tests/api/chains/message-acknowledgment.spec.ts
//!
//! Tests the per-message acknowledgment flow with session IDs:
//! - Initial message triggers agent, becomes acknowledged after turn completes
//! - Follow-up message sent during agent idle is queued then processed
//! - SSE events include session_init with session_id
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::enums::MessageRole;
use moatless_api_schemas::message::SendMessageRequest;
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_cli::events::EventCollector;

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn message_acknowledgment_flow() {
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

    // Step 1: Create task with message → triggers agent
    let task_req = CreateTaskRequest {
        repository_id: repo_id.clone(),
        message: Some("Create a file called ack-test.txt with 'hello'".into()),
        name: Some("Message Ack E2E".into()),
        branch: None,
        parent_task_id: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
    };
    let created = client.api().create_task(&task_req).await.unwrap();
    let task_id = &created.task_id;
    println!("Step 1: Created task {task_id}");

    // Wait for sandbox ready
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    // Step 2: Wait for first turn to complete
    let turn1 = collector
        .wait_for_event("taskTurn", Some("completed"), task_id, Duration::from_secs(300))
        .await;
    assert!(turn1.is_some(), "First turn should complete");
    println!("Step 2: First turn completed");

    // Step 3: Verify session_init event was received
    let session_init = collector
        .wait_for_event("agent", Some("session_init"), task_id, Duration::from_secs(5))
        .await;
    assert!(
        session_init.is_some(),
        "Should have received agent.session_init event"
    );
    let session_id = session_init
        .as_ref()
        .and_then(|e| e.payload.get("session_id"))
        .and_then(|v| v.as_str());
    assert!(
        session_id.is_some(),
        "session_init event should contain session_id"
    );
    println!(
        "Step 3: session_init received, session_id={}",
        session_id.unwrap()
    );

    // Step 4: Check initial messages — should be acknowledged (not queued)
    let messages = client.api().get_messages(task_id).await.unwrap();
    assert!(
        !messages.messages.is_empty(),
        "Should have messages after first turn"
    );

    // Find the initial user message
    let initial_msg = messages
        .messages
        .iter()
        .find(|m| m.role == MessageRole::User);
    assert!(
        initial_msg.is_some(),
        "Should have a user message"
    );
    if let Some(msg) = initial_msg {
        // After turn completes, message should not be queued
        let is_queued = msg.is_queued.unwrap_or(false);
        assert!(
            !is_queued,
            "Initial message should be acknowledged (is_queued=false) after turn completes"
        );
        println!("Step 4: Initial message acknowledged");
    }

    // Step 5: Send follow-up message
    let follow_up = SendMessageRequest {
        message: "Also create a file called ack-test-2.txt with 'world'".into(),
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
    client
        .api()
        .send_message(task_id, &follow_up)
        .await
        .expect("Failed to send follow-up");
    println!("Step 5: Follow-up message sent");

    // Step 6: Wait for second turn to complete
    let turn2 = collector
        .wait_for_nth_event("taskTurn", "completed", task_id, 2, Duration::from_secs(300))
        .await;
    assert!(turn2.is_some(), "Second turn should complete");
    println!("Step 6: Second turn completed");

    // Step 7: Verify multiple session_init events (one per turn)
    let session_init_count = collector.count_events("agent", "session_init", task_id);
    assert!(
        session_init_count >= 2,
        "Should have at least 2 session_init events, got {session_init_count}"
    );
    println!("Step 7: {session_init_count} session_init events received");

    // Step 8: All messages should now be acknowledged
    let final_messages = client.api().get_messages(task_id).await.unwrap();
    let queued_count = final_messages
        .messages
        .iter()
        .filter(|m| m.is_queued.unwrap_or(false))
        .count();
    assert_eq!(
        queued_count, 0,
        "All messages should be acknowledged after both turns complete"
    );
    println!("Step 8: All {} messages acknowledged", final_messages.messages.len());

    // Print event summary
    let summary = collector.event_summary(task_id);
    println!("Event summary: {summary:?}");

    // Cleanup
    collector.stop();
    let _ = client.api().stop_task(task_id).await;
    sandbox_helpers::wait_for_sandbox_status(&client, task_id, "stopped", Duration::from_secs(60))
        .await;
    let _ = client.api().delete_task(task_id).await;
    println!("Cleanup complete");
}
