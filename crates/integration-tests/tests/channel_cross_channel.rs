//! Cross-channel response routing integration tests.
//!
//! Mirrors: tests/api/chains/channel-cross-channel.spec.ts
//! Spec IDs: CHAN-40 through CHAN-42

use integration_tests::{ensure_template_repo, raw_api, test_client};
use moatless_api_schemas::channel::CreateChannelConfigRequest;
use moatless_api_schemas::enums::{ChannelType, RoutingMode};
use pretty_assertions::assert_eq;
use serde_json::json;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

// =============================================================================
// Chain: Cross-Channel Response Routing (CHAN-40 through CHAN-42)
// =============================================================================

#[tokio::test]
async fn cross_channel_response_routing() {
    let client = test_client();
    let http = integration_tests::raw_client_as_user("alice-test", "alice", "user");
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    // CHAN-40: Create trigger webhook config
    let trigger_req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("trigger-{suffix}"),
        source_name: Some("Trigger Webhook".into()),
        repository_id: repo_id.clone(),
        routing_mode: RoutingMode::PerMessage,
        prompt_template: None,
        provider_config: None,
        response_config_id: None,
        scope: None,
        team_id: None,
        task_name: None,
        tag_ids: None,
        run_as_user_id: None,
        allow_direct_send: None,
    };
    let trigger = client
        .api()
        .create_channel_config(&trigger_req)
        .await
        .unwrap();
    assert!(!trigger.id.is_empty());

    // Create response webhook config
    let response_req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("response-{suffix}"),
        source_name: Some("Response Webhook".into()),
        repository_id: repo_id.clone(),
        routing_mode: RoutingMode::PerMessage,
        prompt_template: None,
        provider_config: None,
        response_config_id: None,
        scope: None,
        team_id: None,
        task_name: None,
        tag_ids: None,
        run_as_user_id: None,
        allow_direct_send: None,
    };
    let response_config = client
        .api()
        .create_channel_config(&response_req)
        .await
        .unwrap();
    assert!(!response_config.id.is_empty());

    // CHAN-41: Link response_config_id on trigger via PUT
    let update_payload = json!({
        "response_config_id": response_config.id
    });
    let update_resp = raw_api::update_channel_config(&http, &trigger.id, &update_payload).await;
    assert!(
        update_resp.status().is_success(),
        "Failed to link response_config_id: {}",
        update_resp.status()
    );

    // CHAN-41.1: Send webhook inbound to trigger, assert 200 + taskId
    let inbound_payload = json!({
        "message": "Cross-channel test message"
    });
    let inbound_resp = raw_api::send_webhook_message(&http, &trigger.id, &inbound_payload).await;
    assert_eq!(inbound_resp.status().as_u16(), 200);
    let inbound_body: serde_json::Value = inbound_resp.json().await.unwrap();
    assert!(
        inbound_body.get("taskId").is_some(),
        "Expected taskId in response"
    );

    // CHAN-42: GET trigger config, assert response_config_id persisted
    let fetched = client.api().get_channel_config(&trigger.id).await.unwrap();
    assert_eq!(
        fetched.response_config_id.as_deref(),
        Some(response_config.id.as_str()),
        "response_config_id should be persisted on trigger config"
    );

    // Cleanup
    let _ = client.api().delete_channel_config(&trigger.id).await;
    let _ = client
        .api()
        .delete_channel_config(&response_config.id)
        .await;
}
