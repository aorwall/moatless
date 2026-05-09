//! Message acknowledgment and same-session multi-message integration tests.
//!
//! Mirrors: tests/api/chains/message-acknowledgment.spec.ts
//!
//! Verifies that multiple messages sent while the agent is running are
//! injected into the same CLI session (via `send_user_message` with
//! `MessagePriority::Next`), not queued for separate turns.
//!
//! Key assertion: only 1 `session_init` event — all messages handled
//! in a single session.
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::message::SendMessageRequest;
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_cli::events::EventCollector;

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn same_session_multi_message() {
    if std::env::var("CLAUDE_SDK_E2E").is_err() {
        eprintln!("Skipping: CLAUDE_SDK_E2E not set");
        return;
    }

    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;

    let collector = EventCollector::start(&client)
        .await
        .expect("Failed to start event collector");

    // ── Step 1: Create task with first message ──────────────────────────
    let task_req = CreateTaskRequest {
        repository_id: repo_id.clone(),
        message: Some(
            "Create a file called first.txt with the content 'First Message Created'".into(),
        ),
        name: Some("Multi-Message Same Session E2E".into()),
        branch: None,
        parent_task_id: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        command_name: None,
        headless: None,
        task_template_id: None,
    };
    let created = client.api().create_task(&task_req).await.unwrap();
    let task_id = &created.task_id;
    println!("Step 1: Created task {task_id}");

    // ── Step 2: Wait for sandbox ready ──────────────────────────────────
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );
    println!("Step 2: Sandbox ready");

    // ── Step 3: Wait for session_init (session is alive) ────────────────
    let session_init = collector
        .wait_for_event(
            "agent",
            Some("session_init"),
            task_id,
            Duration::from_secs(120),
        )
        .await;
    assert!(
        session_init.is_some(),
        "Should have received agent.session_init event"
    );
    let session_id = session_init
        .as_ref()
        .and_then(|e| e.payload.get("session_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    println!("Step 3: session_init received, session_id={session_id}");

    // ── Step 4: Send second message while agent is running ──────────────
    let msg2 = SendMessageRequest {
        message: "Also create a file called second.txt with the content 'Second Message Created'"
            .into(),
        agent_mode: None,
        command_name: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        resume: None,
        tool_name: None,
        tool_response: None,
        tool_use_id: None,
        source_task_id: None,
        start_fresh: None,
    };
    client
        .api()
        .send_message(task_id, &msg2)
        .await
        .expect("Failed to send second message");
    println!("Step 4: Second message sent while agent is running");

    // ── Step 5: Send third message after brief delay ────────────────────
    tokio::time::sleep(Duration::from_secs(2)).await;

    let msg3 = SendMessageRequest {
        message: "Also create a file called third.txt with the content 'Third Message Created'"
            .into(),
        agent_mode: None,
        command_name: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        resume: None,
        tool_name: None,
        tool_response: None,
        tool_use_id: None,
        source_task_id: None,
        start_fresh: None,
    };
    client
        .api()
        .send_message(task_id, &msg3)
        .await
        .expect("Failed to send third message");
    println!("Step 5: Third message sent");

    // ── Step 6: Wait for agent to become idle ───────────────────────────
    assert!(
        sandbox_helpers::wait_for_agent_idle(&client, task_id, Duration::from_secs(300)).await,
        "Agent should become idle after processing all messages"
    );
    println!("Step 6: Agent is idle");

    // ── Step 7: Assert only 1 session_init (same session for all) ───────
    let session_init_count = collector.count_events("agent", "session_init", task_id);
    assert_eq!(
        session_init_count, 1,
        "Expected exactly 1 session_init event (all messages in same session), got {session_init_count}"
    );
    println!("Step 7: {session_init_count} session_init event (single session confirmed)");

    // ── Step 8: Verify agent output mentions all three files ─────────────
    // The agent.completed event payload contains the result text.
    // We verify the agent acknowledged all three messages in its output.
    let completed_event = collector
        .wait_for_event("agent", Some("completed"), task_id, Duration::from_secs(5))
        .await;
    assert!(completed_event.is_some(), "Expected agent.completed event");
    let result_text = completed_event
        .as_ref()
        .and_then(|e| e.payload.get("result"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    println!("Step 8: Agent result: {result_text}");
    // The agent should mention all three files in its completion output
    assert!(
        result_text.contains("first") || result_text.contains("First"),
        "Agent result should mention first file, got: {result_text}"
    );
    assert!(
        result_text.contains("second") || result_text.contains("Second"),
        "Agent result should mention second file, got: {result_text}"
    );
    assert!(
        result_text.contains("third") || result_text.contains("Third"),
        "Agent result should mention third file, got: {result_text}"
    );

    // ── Step 9: All messages should be acknowledged ─────────────────────
    let final_messages = client.api().get_messages(task_id).await.unwrap();
    let queued_count = final_messages
        .messages
        .iter()
        .filter(|m| m.is_queued.unwrap_or(false))
        .count();
    assert_eq!(
        queued_count, 0,
        "All messages should be acknowledged, but {queued_count} are still queued"
    );
    println!(
        "Step 9: All {} messages acknowledged",
        final_messages.messages.len()
    );

    // ── Step 10: Verify no error events ─────────────────────────────────
    let error_count = collector.count_events("sandbox", "error", task_id);
    assert_eq!(error_count, 0, "Expected no sandbox.error events");
    println!("Step 10: No error events");

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
