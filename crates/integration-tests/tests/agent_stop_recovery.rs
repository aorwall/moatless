//! Agent stop and recovery integration tests (requires Claude CLI).
//!
//! Mirrors: tests/api/chains/agent-stop-recovery.spec.ts
//! Spec IDs: AGENT-04 through AGENT-06.5
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{message::SendMessageRequest, task::CreateTaskRequest};
use moatless_cli::events::EventCollector;

// =============================================================================
// Chain: Agent Stop and Recovery (AGENT-04 through AGENT-06.5)
// =============================================================================

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn agent_stop_and_recovery() {
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

    // AGENT-04: Create task with a long-running message
    let req = CreateTaskRequest {
        repository_id: Some(repo_id.clone()),
        workspace_id: None,
        message: Some(
            "Create 5 files named step1.txt through step5.txt, each with the content \
             'Step N completed' where N is the step number. Take your time and create them \
             one at a time."
                .into(),
        ),
        name: Some("Agent Stop Recovery E2E".into()),
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

    // AGENT-04.1: Wait for agent.started
    let agent_started = collector
        .wait_for_event("agent", Some("started"), task_id, Duration::from_secs(120))
        .await;
    assert!(agent_started.is_some(), "Expected agent.started event");

    // Give the agent a few seconds to start working
    tokio::time::sleep(Duration::from_secs(5)).await;

    // AGENT-04.2: Stop agent mid-execution
    let _ = client.api().stop_task(task_id).await;

    // Verify agent becomes idle/none after stop
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(status) = client.api().sandbox_status(task_id).await {
            let agent_state_str = format!("{}", status.agent_state);
            if agent_state_str == "idle" || agent_state_str == "none" {
                println!("AGENT-04.2: Agent stopped, state={}", status.agent_state);
                break;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("Agent did not stop within timeout");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // AGENT-05: Restart sandbox, verify files persist
    client.api().restart_sandbox(task_id).await.unwrap();
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready after restart"
    );

    // Check that at least some files were created before the stop
    let files = client.api().list_files(task_id).await.unwrap();
    println!(
        "AGENT-05: Files after restart: {:?}",
        files
            .files
            .iter()
            .filter(|f| f.contains("step"))
            .collect::<Vec<_>>()
    );

    // AGENT-06: Send follow-up message after recovery
    let follow_up = SendMessageRequest {
        // Pinned to the working directory: that is the root the file API
        // resolves against, and an unpinned prompt lands the file in the cloned
        // repo subdirectory instead, where the read below cannot see it.
        message: "Create a file named resumed.txt directly in your current working \
                  directory — not inside the cloned repository subdirectory — with the \
                  content 'Resumed Session Works'"
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
        .send_message(task_id, &follow_up)
        .await
        .unwrap();

    // Wait for next taskTurn.completed
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
        "Expected taskTurn.completed after follow-up"
    );

    // AGENT-06.5: Assert messages and search for resumed content
    let messages = client.api().get_messages(task_id).await.unwrap();
    assert!(
        messages.messages.len() >= 4,
        "Expected at least 4 messages (initial + stop + follow-up + response), got {}",
        messages.messages.len()
    );

    // (The old `/search-files` content-search endpoint was removed in
    // 143bba2e0; reading the file asserts the agent's work directly.)
    let file = client
        .api()
        .read_file(task_id, "resumed.txt")
        .await
        .unwrap();
    assert!(
        file.content.contains("Resumed Session Works"),
        "Expected resumed.txt to contain 'Resumed Session Works', got: {}",
        file.content
    );

    // Cleanup
    collector.stop();
    let _ = client.api().delete_task(task_id).await;
}
