//! Channel webhook routing integration tests.
//!
//! Mirrors: tests/api/chains/channel-webhook-routing.spec.ts
//! Spec IDs: CHAN-10 through CHAN-33

use integration_tests::{ensure_template_repo, raw_api, test_client};
use moatless_api_schemas::channel::CreateChannelConfigRequest;
use moatless_api_schemas::enums::{ChannelType, RoutingMode, TaskStatus};
use moatless_cli::client::MoatClient;
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

/// Send a webhook message and extract task_id from response body.
async fn send_webhook(
    http: &reqwest::Client,
    config_id: &str,
    text: &str,
    thread_id: Option<&str>,
) -> (u16, Option<String>) {
    let mut payload = serde_json::json!({ "text": text });
    if let Some(tid) = thread_id {
        payload["threadId"] = serde_json::json!(tid);
    }
    let resp = raw_api::send_webhook_message(http, config_id, &payload).await;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let task_id = body["taskId"].as_str().map(String::from);
    (status, task_id)
}

/// Helper to clean up tasks
async fn cleanup_tasks(client: &MoatClient, task_ids: &[String]) {
    for id in task_ids {
        let _ = client.api().stop_task(id).await;
        let _ = client.api().delete_task(id).await;
    }
}

// =============================================================================
// Chain 2: Webhook per_message routing (CHAN-10 through CHAN-13)
// =============================================================================

#[tokio::test]
async fn channel_per_message_routing() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let mut task_ids = Vec::new();

    // CHAN-10: Create webhook config with per_message routing
    let req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("per-msg-{suffix}"),
        source_name: None,
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
    };
    let config = client.api().create_channel_config(&req).await.unwrap();
    assert!(config.active);
    assert_eq!(config.routing_mode, RoutingMode::PerMessage);
    let config_id = config.id;

    // CHAN-11: Send message via webhook — task created
    let (status, task_id) =
        send_webhook(&http, &config_id, "First per_message webhook test", None).await;
    assert_eq!(status, 200);
    let first_task_id = task_id.expect("should return task_id");
    task_ids.push(first_task_id.clone());

    // CHAN-11.1: Task is in open state
    let task = client
        .api()
        .get_task(&first_task_id)
        .await
        .unwrap();
    assert_eq!(task.status, TaskStatus::Open);

    // CHAN-11.2: Task linked to correct repository
    assert_eq!(task.repository_id, repo_id);

    // CHAN-12: Send second message — new task created
    let (status, task_id) =
        send_webhook(&http, &config_id, "Second per_message webhook test", None).await;
    assert_eq!(status, 200);
    let second_task_id = task_id.expect("should return task_id");
    assert_ne!(second_task_id, first_task_id);
    task_ids.push(second_task_id);

    // CHAN-13: Send message with thread_id — new task (per_message ignores threads)
    let thread_id = format!("new-thread-{suffix}");
    let (status, task_id) = send_webhook(
        &http,
        &config_id,
        "Message with new thread",
        Some(&thread_id),
    )
    .await;
    assert_eq!(status, 200);
    let third_task_id = task_id.expect("should return task_id");
    assert_ne!(third_task_id, first_task_id);
    task_ids.push(third_task_id);

    // Cleanup
    let _ = client.api().delete_channel_config(&config_id).await;
    cleanup_tasks(&client, &task_ids).await;
}

// =============================================================================
// Chain 3: Webhook ongoing routing (CHAN-20 through CHAN-23)
// =============================================================================

#[tokio::test]
async fn channel_ongoing_routing() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let mut task_ids = Vec::new();

    // CHAN-20: Create webhook config with ongoing routing
    let req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("ongoing-{suffix}"),
        source_name: None,
        repository_id: repo_id,
        routing_mode: RoutingMode::Ongoing,
        prompt_template: None,
        provider_config: None,
        response_config_id: None,
        scope: None,
        team_id: None,
        task_name: None,
        tag_ids: None,
        run_as_user_id: None,
    };
    let config = client.api().create_channel_config(&req).await.unwrap();
    assert!(config.active);
    assert_eq!(config.routing_mode, RoutingMode::Ongoing);
    let config_id = config.id;

    // CHAN-21: First message creates ongoing task
    let (status, task_id) =
        send_webhook(&http, &config_id, "First ongoing message", None).await;
    assert_eq!(status, 200);
    let ongoing_task_id = task_id.expect("should return task_id");
    task_ids.push(ongoing_task_id.clone());

    // Wait briefly for async processing
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Check that config now has ongoing_task_id
    let config = client.api().get_channel_config(&config_id).await.unwrap();
    assert_eq!(
        config.ongoing_task_id.as_deref(),
        Some(ongoing_task_id.as_str())
    );

    // CHAN-22: Second message routes to same task
    let (status, task_id) =
        send_webhook(&http, &config_id, "Second ongoing message", None).await;
    assert_eq!(status, 200);
    assert_eq!(task_id.as_deref(), Some(ongoing_task_id.as_str()));

    // CHAN-23: After task closed, next message creates new task
    client.api().close_task(&ongoing_task_id).await.unwrap();
    let (status, task_id) =
        send_webhook(&http, &config_id, "Message after close", None).await;
    assert_eq!(status, 200);
    let new_task_id = task_id.expect("should return task_id");
    assert_ne!(new_task_id, ongoing_task_id);
    task_ids.push(new_task_id.clone());

    // Wait briefly for async processing
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Config should now point to the new task
    let config = client.api().get_channel_config(&config_id).await.unwrap();
    assert_eq!(config.ongoing_task_id.as_deref(), Some(new_task_id.as_str()));

    // Cleanup
    let _ = client.api().delete_channel_config(&config_id).await;
    cleanup_tasks(&client, &task_ids).await;
}

// =============================================================================
// Chain 4: Webhook thread_based routing (CHAN-30 through CHAN-33)
// =============================================================================

#[tokio::test]
async fn channel_thread_based_routing() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let mut task_ids = Vec::new();

    // CHAN-30: Create webhook config with thread_based routing
    let req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("thread-{suffix}"),
        source_name: None,
        repository_id: repo_id,
        routing_mode: RoutingMode::ThreadBased,
        prompt_template: None,
        provider_config: None,
        response_config_id: None,
        scope: None,
        team_id: None,
        task_name: None,
        tag_ids: None,
        run_as_user_id: None,
    };
    let config = client.api().create_channel_config(&req).await.unwrap();
    assert!(config.active);
    assert_eq!(config.routing_mode, RoutingMode::ThreadBased);
    let config_id = config.id;

    // CHAN-31: New message creates task
    let thread_a = format!("thread-a-{suffix}");
    let (status, task_id) = send_webhook(
        &http,
        &config_id,
        "First thread message",
        Some(&thread_a),
    )
    .await;
    assert_eq!(status, 200);
    let first_task_id = task_id.expect("should return task_id");
    task_ids.push(first_task_id.clone());

    // CHAN-32: Reply in same thread routes to same task
    let (status, task_id) = send_webhook(
        &http,
        &config_id,
        "Reply in same thread",
        Some(&thread_a),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(task_id.as_deref(), Some(first_task_id.as_str()));

    // CHAN-33: New thread creates new task
    let thread_b = format!("thread-b-{suffix}");
    let (status, task_id) = send_webhook(
        &http,
        &config_id,
        "Message in new thread",
        Some(&thread_b),
    )
    .await;
    assert_eq!(status, 200);
    let second_task_id = task_id.expect("should return task_id");
    assert_ne!(second_task_id, first_task_id);
    task_ids.push(second_task_id);

    // Cleanup
    let _ = client.api().delete_channel_config(&config_id).await;
    cleanup_tasks(&client, &task_ids).await;
}
