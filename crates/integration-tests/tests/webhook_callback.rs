//! Webhook outbound callback integration tests (requires sandbox + callback server).
//!
//! Mirrors: tests/api/chains/webhook-callback.spec.ts
//! Spec IDs: INT-60 through INT-63
//!
//! Env gate: `CHANNEL_CALLBACK_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{
    callback_server::CallbackServer, create_webhook_integration, ensure_template_repo, raw_api,
    sandbox_helpers, test_client,
};
use moatless_api_schemas::enums::RoutingMode;
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
// Chain: Webhook Outbound Callback (INT-60 through INT-63)
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

    // INT-60: Start callback server
    let callback_server = CallbackServer::start().await;
    let callback_url = callback_server.callback_url();
    println!("INT-60: Callback server listening at {callback_url}");

    // Start event collector
    let collector = EventCollector::start(&client)
        .await
        .expect("Failed to start event collector");

    // INT-61: Create webhook trigger with callback_url
    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("callback-{suffix}"),
        RoutingMode::BySubject,
        Some(json!({
            "callback_url": callback_url
        })),
    )
    .await;
    assert!(!fixture.trigger.id.is_empty());

    // INT-62: Send webhook inbound message
    let inbound_payload = json!({
        "text": "Create a file called hello.txt with 'Hello from webhook callback test'"
    });
    let inbound_resp =
        raw_api::send_webhook_message(&http, &fixture.trigger.id, &inbound_payload).await;
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

    // INT-63: Assert callback received
    let callback = callback_server
        .wait_for_callback(Duration::from_secs(60))
        .await;
    assert!(callback.is_some(), "Expected callback to be received");

    let cb = callback.unwrap();
    println!("INT-63: Callback body: {}", cb.body);

    // Verify callback contains expected fields
    assert!(
        cb.body.get("taskId").is_some() || cb.body.get("task_id").is_some(),
        "Callback should contain taskId"
    );

    // Cleanup
    collector.stop();
    callback_server.stop();
    let _ = client.api().delete_task(task_id).await;
}
