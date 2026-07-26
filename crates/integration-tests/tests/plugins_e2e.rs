//! Plugins end-to-end integration test.
//!
//! Mirrors: tests/api/chains/plugins-e2e.spec.ts
//!
//! Registers a plugin, syncs its skills, turns it on for everyone, checks the
//! effective set, escapes it with a personal record, turns that back on, then
//! creates a task and verifies the sandbox provisions with the plugin present.
//!
//! Uses <https://github.com/amekala/adspirer-mcp-plugin.git>, which contains the
//! skill `ad-campaign-best-practices`. A plugin with no skills is never
//! delivered, so the test needs one that has some.
//!
//! Env gate: `CLAUDE_SDK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::plugin::{ActivationReach, CreatePluginRequest, SetActivationRequest};
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_cli::events::EventCollector;

const PLUGIN_GIT_URL: &str = "https://github.com/amekala/adspirer-mcp-plugin.git";

#[tokio::test]
#[ignore] // Requires CLAUDE_SDK_E2E=1
async fn plugins_e2e_chain() {
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

    // PLUGIN-01: Register plugin
    let plugin_req = CreatePluginRequest {
        name: "E2E Test Plugin".into(),
        git_url: PLUGIN_GIT_URL.into(),
    };
    let plugin = client
        .api()
        .create_plugin(&plugin_req)
        .await
        .expect("Failed to create plugin");
    let plugin_id = &plugin.id;
    println!("PLUGIN-01: Registered plugin id={plugin_id}");

    // PLUGIN-01.1: List plugins, verify new one appears
    let plugins = client
        .api()
        .list_plugins()
        .await
        .expect("Failed to list plugins");
    assert!(
        plugins.iter().any(|p| p.id == *plugin_id),
        "New plugin should appear in list"
    );
    println!(
        "PLUGIN-01.1: Plugin appears in list ({} total)",
        plugins.len()
    );

    // PLUGIN-02: Reading the skills syncs the plugin from its source.
    let skills = client
        .api()
        .list_plugin_skills(plugin_id)
        .await
        .expect("Failed to list plugin skills");
    assert!(
        !skills.is_empty(),
        "Syncing the plugin should have stored its skills"
    );
    println!(
        "PLUGIN-02: Synced {} skill(s): {:?}",
        skills.len(),
        skills.iter().map(|s| &s.name).collect::<Vec<_>>()
    );

    // PLUGIN-02.1: Nothing is delivered until somebody turns it on.
    let effective = client
        .api()
        .list_effective_plugins()
        .await
        .expect("Failed to list effective plugins");
    assert!(
        !effective.iter().any(|p| p.plugin_id == *plugin_id),
        "A registered plugin nobody turned on must reach nobody"
    );
    println!("PLUGIN-02.1: Registered but not yet turned on — not delivered");

    // PLUGIN-03: Turn it on for everyone.
    client
        .api()
        .set_plugin_activation(
            plugin_id,
            &SetActivationRequest {
                reach: ActivationReach::Everyone,
                skill_name: None,
                enabled: true,
            },
        )
        .await
        .expect("Failed to turn the plugin on for everyone");

    let effective = client
        .api()
        .list_effective_plugins()
        .await
        .expect("Failed to list effective plugins");
    let entry = effective
        .iter()
        .find(|p| p.plugin_id == *plugin_id)
        .expect("A plugin turned on for everyone should be in the effective set");
    assert_eq!(
        entry.skill_names.len(),
        skills.len(),
        "Turning on the whole plugin should deliver all of its skills"
    );
    println!("PLUGIN-03: Turned on for everyone — {} skill(s)", entry.skill_names.len());

    // PLUGIN-04: A personal record escapes it.
    client
        .api()
        .set_plugin_activation(
            plugin_id,
            &SetActivationRequest {
                reach: ActivationReach::Personal,
                skill_name: None,
                enabled: false,
            },
        )
        .await
        .expect("Failed to turn the plugin off for myself");

    let effective = client
        .api()
        .list_effective_plugins()
        .await
        .expect("Failed to list effective plugins");
    assert!(
        !effective.iter().any(|p| p.plugin_id == *plugin_id),
        "A personal record must override the global default"
    );
    println!("PLUGIN-04: Personal record overrides the global default");

    // PLUGIN-04.1: Clearing it hands me back to the default.
    client
        .api()
        .clear_plugin_activation(plugin_id, ActivationReach::Personal, None)
        .await
        .expect("Failed to clear the personal record");

    let effective = client
        .api()
        .list_effective_plugins()
        .await
        .expect("Failed to list effective plugins");
    assert!(
        effective.iter().any(|p| p.plugin_id == *plugin_id),
        "Clearing a personal record hands the person back to the default"
    );
    println!("PLUGIN-04.1: Cleared personal record — back to the default");

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
        headless: None,
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

    // Deleting the plugin cascades its skills and activation records away.
    let _ = client.api().delete_plugin(plugin_id).await;

    // Delete task
    let _ = client.api().delete_task(task_id).await;

    println!("Cleanup complete");
}
