//! Webhook outbound callback integration tests (requires sandbox + callback server).
//!
//! Mirrors: tests/api/chains/channel-webhook-callback.spec.ts
//! Spec IDs: CHAN-60 through CHAN-63
//!
//! Env gate: `CHANNEL_CALLBACK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{
    callback_server::CallbackServer, ensure_template_repo, raw_api, sandbox_helpers, test_client,
};
use moatless_api_client::types::{ChannelType, CreateChannelConfigRequest, RoutingMode};
use moatless_cli::events::EventCollector;
use serde_json::json;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

// =============================================================================
// Chain: Webhook Outbound Callback (CHAN-60 through CHAN-63)
// =============================================================================

#[tokio::test]
#[ignore] // Requires CHANNEL_CALLBACK_E2E=1
async fn webhook_outbound_callback() {
    if std::env::var("CHANNEL_CALLBACK_E2E").is_err() {
        eprintln!("Skipping: CHANNEL_CALLBACK_E2E not set");
        return;
    }

    let client = test_client();
    let http = integration_tests::raw_client_as_user("alice-test", "alice", "user");
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    // CHAN-60: Start callback server
    let callback_server = CallbackServer::start().await;
    let callback_url = callback_server.callback_url();
    println!("CHAN-60: Callback server listening at {callback_url}");

    // Start event collector
    let collector = EventCollector::start(&client)
        .await
        .expect("Failed to start event collector");

    // CHAN-61: Create webhook config with callback_url
    let config_req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("callback-{suffix}"),
        source_name: Some("Callback Test Webhook".into()),
        repository_id: repo_id.clone(),
        routing_mode: RoutingMode::PerMessage,
        prompt_template: None,
        provider_config: Some(
            serde_json::from_value(json!({
                "callback_url": callback_url
            }))
            .unwrap(),
        ),
        response_config_id: None,
        scope: None,
        team_id: None,
    };
    let config = client
        .create_config(&config_req)
        .await
        .unwrap()
        .into_inner();
    assert!(!config.id.is_empty());

    // CHAN-62: Send webhook inbound message
    let inbound_payload = json!({
        "message": "Create a file called hello.txt with 'Hello from webhook callback test'"
    });
    let inbound_resp =
        raw_api::send_webhook_message(&http, &config.id, &inbound_payload).await;
    assert_eq!(inbound_resp.status().as_u16(), 200);
    let inbound_body: serde_json::Value = inbound_resp.json().await.unwrap();
    let task_id = inbound_body["taskId"]
        .as_str()
        .expect("Expected taskId in webhook response");

    // Wait for sandbox ready and agent completion
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

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
        "Expected taskTurn.completed event"
    );

    // CHAN-63: Assert callback received
    let callback = callback_server
        .wait_for_callback(Duration::from_secs(60))
        .await;
    assert!(callback.is_some(), "Expected callback to be received");

    let cb = callback.unwrap();
    println!("CHAN-63: Callback body: {}", cb.body);

    // Verify callback contains expected fields
    assert!(
        cb.body.get("taskId").is_some() || cb.body.get("task_id").is_some(),
        "Callback should contain taskId"
    );

    // Cleanup
    collector.stop();
    callback_server.stop();
    let _ = client.delete_config(&config.id).await;
    let _ = client.delete_task(task_id).await;
}
