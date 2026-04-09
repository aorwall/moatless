//! Channel config CRUD integration tests.
//!
//! Mirrors: tests/api/chains/channel-config-crud.spec.ts
//! Spec IDs: CHAN-01 through CHAN-05.1

use integration_tests::{ensure_template_repo, test_client};
use moatless_api_schemas::channel::{CreateChannelConfigRequest, UpdateChannelConfigRequest};
use moatless_api_schemas::enums::{ChannelType, RoutingMode};
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

// =============================================================================
// Chain: Channel Config CRUD (CHAN-01 through CHAN-05.1)
// =============================================================================

#[tokio::test]
async fn channel_config_crud_chain() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    // CHAN-01: Create webhook channel config
    let req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("crud-test-{suffix}"),
        source_name: Some("CRUD Test Webhook".into()),
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
    let config = client.api().create_channel_config(&req).await.unwrap();
    assert!(config.active);
    assert!(!config.id.is_empty());
    assert_eq!(config.provider, ChannelType::Webhook);
    assert_eq!(config.repository_id, repo_id);

    let config_id = config.id;

    // CHAN-01.1: Config appears in listing
    let configs = client.api().list_channel_configs(None, None).await.unwrap();
    assert!(configs.iter().any(|c| c.id == config_id));

    // CHAN-02: Get channel config by ID
    let config = client.api().get_channel_config(&config_id).await.unwrap();
    assert_eq!(config.id, config_id);
    assert_eq!(config.provider, ChannelType::Webhook);
    assert_eq!(config.routing_mode, RoutingMode::PerMessage);
    assert!(config.active);

    // CHAN-03: Update channel config
    let update = UpdateChannelConfigRequest {
        source_name: Some("Updated CRUD Webhook".into()),
        prompt_template: Some(Some("Please help: {message}".into())),
        ..Default::default()
    };
    let updated = client
        .api()
        .update_channel_config(&config_id, &update)
        .await
        .unwrap();
    assert_eq!(updated.source_name.as_deref(), Some("Updated CRUD Webhook"));
    assert_eq!(
        updated.prompt_template.as_deref(),
        Some("Please help: {message}")
    );
    assert_eq!(updated.routing_mode, RoutingMode::PerMessage);

    // CHAN-04: Deactivate channel config
    let update = UpdateChannelConfigRequest {
        active: Some(false),
        ..Default::default()
    };
    let deactivated = client
        .api()
        .update_channel_config(&config_id, &update)
        .await
        .unwrap();
    assert!(!deactivated.active);

    // Still appears in listing
    let configs = client.api().list_channel_configs(None, None).await.unwrap();
    assert!(configs.iter().any(|c| c.id == config_id));

    // CHAN-05: Delete channel config
    client
        .api()
        .delete_channel_config(&config_id)
        .await
        .unwrap();

    // CHAN-05.1: Config removed from listing
    let configs = client.api().list_channel_configs(None, None).await.unwrap();
    assert!(!configs.iter().any(|c| c.id == config_id));
}
