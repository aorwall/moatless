//! Plugin repos end-to-end integration test.
//!
//! Mirrors: tests/api/chains/plugin-repos-e2e.spec.ts
//!
//! Creates a plugin repo, links it to a repository, creates a task,
//! and verifies the sandbox provisions with the plugin cloned.
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::plugin_repo::CreatePluginRepoRequest;
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_cli::events::EventCollector;

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn plugin_repos_e2e_chain() {
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

    // PLUGIN-01: Create plugin repo
    let plugin_req = CreatePluginRepoRequest {
        name: "E2E Test Plugin".into(),
        git_url: "https://github.com/aorwall/hello-world-ui.git".into(),
    };
    let plugin = client
        .api()
        .create_plugin_repo(&plugin_req)
        .await
        .expect("Failed to create plugin repo");
    let plugin_id = &plugin.id;
    println!("PLUGIN-01: Created plugin repo id={plugin_id}");

    // PLUGIN-01.1: List plugin repos, verify new one appears
    let plugins = client
        .api()
        .list_plugin_repos()
        .await
        .expect("Failed to list plugin repos");
    assert!(
        plugins.iter().any(|p| p.id == *plugin_id),
        "New plugin repo should appear in list"
    );
    println!(
        "PLUGIN-01.1: Plugin appears in list ({} total)",
        plugins.len()
    );

    // PLUGIN-02: Link plugin to repository
    client
        .api()
        .link_plugin_to_repo(&repo_id, plugin_id)
        .await
        .expect("Failed to link plugin to repo");
    println!("PLUGIN-02: Linked plugin {plugin_id} to repo {repo_id}");

    // PLUGIN-02.1: Verify link appears in repo plugins
    let repo_plugins = client
        .api()
        .list_repo_plugins(&repo_id)
        .await
        .expect("Failed to list repo plugins");
    assert!(
        repo_plugins.iter().any(|p| p.id == *plugin_id),
        "Plugin should appear in repo plugins"
    );
    println!(
        "PLUGIN-02.1: Plugin linked ({} plugins)",
        repo_plugins.len()
    );

    // TASK-01: Create task with message
    let task_req = CreateTaskRequest {
        repository_id: Some(repo_id.clone()),
        workspace_id: None,
        message: Some("What skills do you have available? List them briefly.".into()),
        name: Some("Plugin E2E Test".into()),
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
    let created = client.api().create_task(&task_req).await.unwrap();
    let task_id = &created.task_id;
    println!("TASK-01: Created task {task_id}");

    // SANDBOX-01: Wait for sandbox ready (plugin should be cloned during init)
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(180)).await,
        "Sandbox did not become ready with plugin"
    );
    println!("SANDBOX-01: Sandbox ready");

    // AGENT-01: Wait for agent turn to complete
    let turn_completed = collector
        .wait_for_event(
            "taskTurn",
            Some("completed"),
            task_id,
            Duration::from_secs(300),
        )
        .await;
    assert!(turn_completed.is_some(), "Agent turn should complete");
    println!("AGENT-01: Agent turn completed");

    // AGENT-01.1: Get messages, verify agent responded
    let messages = client
        .api()
        .get_messages(task_id)
        .await
        .expect("Failed to get messages");
    assert!(
        messages.messages.len() >= 2,
        "Should have at least user + assistant messages, got {}",
        messages.messages.len()
    );
    println!("AGENT-01.1: Got {} messages", messages.messages.len());

    // Print event summary for diagnostics
    let summary = collector.event_summary(task_id);
    println!("Event summary: {summary:?}");

    // Cleanup
    collector.stop();

    // Stop task
    let _ = client.api().stop_task(task_id).await;
    sandbox_helpers::wait_for_sandbox_status(&client, task_id, "stopped", Duration::from_secs(60))
        .await;

    // Unlink plugin
    let _ = client
        .api()
        .unlink_plugin_from_repo(&repo_id, plugin_id)
        .await;

    // Delete plugin repo
    let _ = client.api().delete_plugin_repo(plugin_id).await;

    // Delete task
    let _ = client.api().delete_task(task_id).await;

    println!("Cleanup complete");
}
