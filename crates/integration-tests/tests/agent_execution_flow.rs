//! Agent execution and multi-turn integration tests (requires Claude CLI).
//!
//! Mirrors: tests/api/chains/agent-execution-flow.spec.ts
//! Spec IDs: AGENT-01 through AGENT-02.3
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{
    enums::MessageType, message::SendMessageRequest, task::CreateTaskRequest,
};
use moatless_cli::events::EventCollector;

// =============================================================================
// Chain: Agent Execution and Multi-Turn (AGENT-01 through AGENT-02.3)
// =============================================================================

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn agent_execution_and_multi_turn() {
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

    // AGENT-01: Create task WITH message to trigger agent execution
    let req = CreateTaskRequest {
        repository_id: Some(repo_id.clone()),
        workspace_id: None,
        // The location is pinned to the working directory because that is the
        // root the file API resolves against. Left unpinned, the agent
        // reasonably writes into the cloned repo subdirectory and the read
        // below 404s on a file that was created correctly.
        message: Some(
            "Create a file named e2e-success.txt directly in your current working \
             directory — not inside the cloned repository subdirectory — with the \
             content 'E2E Test Success'"
                .into(),
        ),
        name: Some("Agent Execution E2E".into()),
        branch: None,
        description: None,
        agent_mode: None,
        agent_type: integration_tests::e2e_agent_type(),
        contexts: vec![],
        file_ids: vec![],
        model: None,
        headless: None,
        parent_task_id: None,
    };
    let created = client.api().create_task(&req).await.unwrap();
    let task_id = &created.task_id;

    // Wait for sandbox ready
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    // AGENT-01.1: Wait for taskTurn.completed (up to 300s for Claude execution)
    let turn_completed = collector
        .wait_for_event(
            "taskTurn",
            Some("completed"),
            task_id,
            Duration::from_secs(300),
        )
        .await;
    assert!(
        turn_completed.is_some(),
        "Expected taskTurn.completed event within 300s"
    );

    // AGENT-01.2: Assert agent is idle after completion
    assert!(
        sandbox_helpers::wait_for_agent_idle(&client, task_id, Duration::from_secs(30)).await,
        "Agent should be idle after turn completion"
    );

    // AGENT-01.2b: Verify cost was recorded on the task (skipped for the
    // tmux transport, which has no machine-readable cost stream).
    let task = client.api().get_task(task_id).await.unwrap();
    println!("AGENT-01.2b: total_cost_usd = {}", task.total_cost_usd);
    if integration_tests::e2e_agent_reports_cost() {
        assert!(
            task.total_cost_usd > 0.0,
            "Expected non-zero total_cost_usd after agent execution, got: {}",
            task.total_cost_usd
        );
    }

    // AGENT-01.3: Verify SSE events were received
    let agent_started = collector
        .wait_for_event("agent", Some("started"), task_id, Duration::from_secs(5))
        .await;
    assert!(agent_started.is_some(), "Expected agent.started SSE event");

    // AGENT-02: Get messages — assert ≥2 messages (user + assistant)
    let messages = client.api().get_messages(task_id).await.unwrap();
    assert!(
        messages.messages.len() >= 2,
        "Expected at least 2 messages (user + assistant), got {}",
        messages.messages.len()
    );

    // AGENT-02.1: Read the created file and verify its content. (The old
    // `/search-files` content-search endpoint was removed in 143bba2e0;
    // reading the file asserts the agent's work directly.)
    let file = client
        .api()
        .read_file(task_id, "e2e-success.txt")
        .await
        .unwrap();
    assert!(
        file.content.contains("E2E Test Success"),
        "Expected e2e-success.txt to contain 'E2E Test Success', got: {}",
        file.content
    );

    // AGENT-02.2: Send follow-up message (multi-turn)
    let follow_up = SendMessageRequest {
        message: "Create another file named e2e-multiturn.txt directly in your current working \
             directory — not inside the cloned repository subdirectory — with the content \
             'Multi-Turn Success'"
            .into(),
        agent_mode: None,
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
        .send_message(task_id, &follow_up)
        .await
        .unwrap();

    // Wait for 2nd taskTurn.completed
    let turn2 = collector
        .wait_for_nth_event(
            "taskTurn",
            "completed",
            task_id,
            2,
            Duration::from_secs(300),
        )
        .await;
    assert!(turn2.is_some(), "Expected 2nd taskTurn.completed event");

    // A follow-up sent after the agent went idle starts a new agent run, and a
    // run is a turn — so the two user messages must land on turns 1 and 2.
    let messages = client.api().get_messages(task_id).await.unwrap();
    let user_turns: Vec<Option<i32>> = messages
        .messages
        .iter()
        .filter(|m| matches!(m.message_type, MessageType::User))
        .map(|m| m.turn_number)
        .collect();
    assert_eq!(
        user_turns,
        vec![Some(1), Some(2)],
        "Expected the follow-up to open turn 2, got {user_turns:?}"
    );

    // AGENT-02.3: Read the second file and verify its content.
    let file2 = client
        .api()
        .read_file(task_id, "e2e-multiturn.txt")
        .await
        .unwrap();
    assert!(
        file2.content.contains("Multi-Turn Success"),
        "Expected e2e-multiturn.txt to contain 'Multi-Turn Success', got: {}",
        file2.content
    );

    // Cleanup
    collector.stop();
    let _ = client.api().delete_task(task_id).await;
}
