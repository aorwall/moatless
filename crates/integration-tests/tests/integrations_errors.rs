//! Integration error case integration tests.
//!
//! Mirrors: tests/api/chains/integrations-errors.spec.ts
//! Spec IDs: INT-50 through INT-54

use integration_tests::{
    create_webhook_integration, ensure_template_repo, raw_admin_client, raw_api, test_client,
};
use moatless_api_schemas::adapters::CreateAdapterConnectionRequest;
use moatless_api_schemas::enums::{AdapterKind, RoutingMode};
use moatless_api_schemas::loops::UpdateLoopRequest;
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
        RoutingMode::BySubject,
        trigger_config,
    )
    .await
    .trigger
    .id
}

// INT-50: Webhook to non-existent config returns error
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

// INT-51: Webhook to inactive config is ignored
#[tokio::test]
async fn webhook_inactive_config() {
    let client = test_client().await;
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let trigger_id =
        create_webhook_trigger(&client, &repo_id, &format!("inactive-{suffix}"), None).await;

    // Deactivate
    let update = UpdateLoopRequest {
        active: Some(false),
        ..Default::default()
    };
    client
        .api()
        .update_trigger(&trigger_id, &update)
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

// INT-52: Webhook with invalid signature is rejected
#[tokio::test]
async fn webhook_invalid_signature() {
    let client = test_client().await;
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

// INT-53: Create route with non-existent repository returns error
#[tokio::test]
async fn integration_create_nonexistent_repo() {
    let client = test_client().await;
    let http = raw_admin_client().await;
    let suffix = timestamp_suffix();
    let source = format!("no-repo-{suffix}");
    let connection = client
        .api()
        .create_adapter_connection(&CreateAdapterConnectionRequest {
            adapter_kind: AdapterKind::Webhook,
            connection_kind: Some("incoming_webhook".into()),
            external_account_id: source.clone(),
            webhook_secret: None,
            source_name: Some(source.clone()),
            transport_config: None,
            scope: None,
        })
        .await
        .unwrap();

    let resp = raw_api::create_trigger_raw(
        &http,
        &serde_json::json!({
            "name": source,
            "kind": "adapter_event",
            "active": true,
            "rule": {
                "repositoryId": format!("non-existent-repo-id-{suffix}"),
                "routingMode": "by_subject"
            },
            "source": {
                "adapterKind": "webhook",
                "connectionId": connection.id,
                "sourceMatcher": format!("no-repo-{suffix}"),
                "sourceName": format!("no-repo-{suffix}"),
                "eventFilter": {},
                "adapterConfig": {},
                "allowDirectSend": false,
                "closeTaskOnClose": true,
                "notifyOnSuccess": false
            }
        }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400);
}

// INT-54: Webhook with missing text field returns error
#[tokio::test]
async fn webhook_missing_text() {
    let client = test_client().await;
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
