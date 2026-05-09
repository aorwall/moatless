//! Channel error case integration tests.
//!
//! Mirrors: tests/api/chains/channel-errors.spec.ts
//! Spec IDs: CHAN-50 through CHAN-54

use integration_tests::{create_webhook_integration, ensure_template_repo, raw_api, test_client};
use moatless_api_schemas::channel::UpdateTaskTriggerRequest;
use moatless_api_schemas::enums::RoutingMode;
use moatless_cli::client::MoatClient;
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

/// Create a webhook trigger helper.
async fn create_webhook_trigger(
    client: &MoatClient,
    repo_id: &str,
    source_matcher: &str,
    trigger_config: Option<serde_json::Value>,
) -> String {
    create_webhook_integration(
        client,
        repo_id,
        source_matcher,
        RoutingMode::PerMessage,
        trigger_config,
    )
    .await
    .trigger
    .id
}

// CHAN-50: Webhook to non-existent config returns error
#[tokio::test]
async fn webhook_nonexistent_config() {
    let http = reqwest::Client::new();
    let resp = raw_api::send_webhook_message(
        &http,
        "non-existent-config-id",
        &serde_json::json!({ "text": "This should fail" }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 404);
}

// CHAN-51: Webhook to inactive config is ignored
#[tokio::test]
async fn webhook_inactive_config() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let trigger_id =
        create_webhook_trigger(&client, &repo_id, &format!("inactive-{suffix}"), None).await;

    // Deactivate
    let update = UpdateTaskTriggerRequest {
        active: Some(false),
        ..Default::default()
    };
    client
        .api()
        .update_task_trigger(&trigger_id, &update)
        .await
        .unwrap();

    // Send message — should be accepted but ignored
    let resp = raw_api::send_webhook_message(
        &http,
        &trigger_id,
        &serde_json::json!({ "text": "This should be ignored" }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
}

// CHAN-52: Webhook with invalid signature is rejected
#[tokio::test]
async fn webhook_invalid_signature() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let mut provider_config = serde_json::Map::new();
    provider_config.insert(
        "webhook_secret".into(),
        serde_json::Value::String("my-secret-key".into()),
    );

    let trigger_id = create_webhook_trigger(
        &client,
        &repo_id,
        &format!("sig-test-{suffix}"),
        Some(serde_json::Value::Object(provider_config)),
    )
    .await;

    // Send message with invalid signature
    let resp = raw_api::send_webhook_message_with_headers(
        &http,
        &trigger_id,
        &serde_json::json!({ "text": "This should fail" }),
        &[("X-Webhook-Signature".into(), "sha256=invalid".into())],
    )
    .await;
    assert_eq!(resp.status().as_u16(), 401);
}

// CHAN-53: Create route with non-existent repository returns error
#[tokio::test]
async fn channel_create_nonexistent_repo() {
    let suffix = timestamp_suffix();

    let resp = raw_api::create_task_route_raw(
        &reqwest::Client::new(),
        &serde_json::json!({
            "repositoryId": format!("non-existent-repo-id-{suffix}"),
            "routingMode": "per_message"
        }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400);
}

// CHAN-54: Webhook with missing text field returns error
#[tokio::test]
async fn webhook_missing_text() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let trigger_id =
        create_webhook_trigger(&client, &repo_id, &format!("no-text-{suffix}"), None).await;

    // Send message without text field
    let resp = raw_api::send_webhook_message(
        &http,
        &trigger_id,
        &serde_json::json!({ "threadId": "some-thread" }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400);
}
