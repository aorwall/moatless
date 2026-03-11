//! Channel error case integration tests.
//!
//! Mirrors: tests/api/chains/channel-errors.spec.ts
//! Spec IDs: CHAN-50 through CHAN-54

use integration_tests::{ensure_template_repo, raw_api, test_client};
use moatless_api_client::types::{
    ChannelType, CreateChannelConfigRequest, RoutingMode, UpdateChannelConfigRequest,
};
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

/// Create a webhook config helper.
async fn create_webhook_config(
    client: &moatless_api_client::Client,
    repo_id: &str,
    source_id: &str,
    provider_config: Option<serde_json::Map<String, serde_json::Value>>,
) -> String {
    let req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: source_id.into(),
        source_name: None,
        repository_id: repo_id.into(),
        routing_mode: RoutingMode::PerMessage,
        prompt_template: None,
        provider_config,
        response_config_id: None,
        scope: None,
        team_id: None,
    };
    client
        .create_config(&req)
        .await
        .unwrap()
        .into_inner()
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

    let config_id = create_webhook_config(&client, &repo_id, &format!("inactive-{suffix}"), None)
        .await;

    // Deactivate
    let update = UpdateChannelConfigRequest {
        active: Some(false),
        ..Default::default()
    };
    client.update_config(&config_id, &update).await.unwrap();

    // Send message — should be accepted but ignored
    let resp = raw_api::send_webhook_message(
        &http,
        &config_id,
        &serde_json::json!({ "text": "This should be ignored" }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);

    // Cleanup
    let _ = client.delete_config(&config_id).await;
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

    let config_id = create_webhook_config(
        &client,
        &repo_id,
        &format!("sig-test-{suffix}"),
        Some(provider_config),
    )
    .await;

    // Send message with invalid signature
    let resp = raw_api::send_webhook_message_with_headers(
        &http,
        &config_id,
        &serde_json::json!({ "text": "This should fail" }),
        &[("X-Webhook-Signature".into(), "sha256=invalid".into())],
    )
    .await;
    assert_eq!(resp.status().as_u16(), 401);

    // Cleanup
    let _ = client.delete_config(&config_id).await;
}

// CHAN-53: Create config with non-existent repository returns error
#[tokio::test]
async fn channel_create_nonexistent_repo() {
    let client = test_client();
    let suffix = timestamp_suffix();

    let req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("no-repo-{suffix}"),
        source_name: None,
        repository_id: "non-existent-repo-id".into(),
        routing_mode: RoutingMode::PerMessage,
        prompt_template: None,
        provider_config: None,
        response_config_id: None,
        scope: None,
        team_id: None,
    };
    let err = client.create_config(&req).await.unwrap_err();
    match &err {
        progenitor_client::Error::ErrorResponse(resp) => {
            assert_eq!(resp.status().as_u16(), 400);
        }
        other => panic!("expected ErrorResponse, got {other:?}"),
    }
}

// CHAN-54: Webhook with missing text field returns error
#[tokio::test]
async fn webhook_missing_text() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let config_id =
        create_webhook_config(&client, &repo_id, &format!("no-text-{suffix}"), None).await;

    // Send message without text field
    let resp = raw_api::send_webhook_message(
        &http,
        &config_id,
        &serde_json::json!({ "threadId": "some-thread" }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400);

    // Cleanup
    let _ = client.delete_config(&config_id).await;
}
