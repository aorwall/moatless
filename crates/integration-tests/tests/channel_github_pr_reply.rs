//! Integration tests for the count-and-dispatch behavior of
//! `POST /api/v1/tasks/{task_id}/channels/send`.
//!
//! Verifies the contract introduced by the GitHub PR ingress flow:
//! - 0 destinations → 404
//! - 1 destination (legacy thread or subscription) → 200
//! - 2+ destinations → 400 with a structured `MultipleTargetsErrorBody`
//!   so the agent can pick one with `--thread`, `--subscription`, or `--pr`.
//!
//! Subscription-only (GitHub PR) cases are smoke-tested via the routing
//! logic that doesn't require GitHub-API-mocked providers — full E2E
//! GitHub PR ingestion lives in the agent-tier suite.

use integration_tests::{base_url, ensure_template_repo, test_client};
use moatless_api_schemas::channel::{
    ConnectChannelRequest, CreateChannelConfigRequest, MultipleTargetsErrorBody,
    SendChannelMessageRequest,
};
use moatless_api_schemas::enums::{ChannelType, RoutingMode};
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_cli::client::MoatClient;
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

async fn create_bare_task(client: &MoatClient, repo_id: &str) -> String {
    client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: repo_id.to_string(),
            message: Some("Reply-routing fixture task".into()),
            name: Some("reply-routing-test".into()),
            branch: None,
            parent_task_id: None,
            description: None,
            agent_mode: None,
            agent_type: None,
            contexts: vec![],
            file_ids: vec![],
            model: None,
            command_name: None,
        })
        .await
        .unwrap()
        .task_id
}

/// Webhook channel + a connected thread for `task_id`. The webhook provider
/// silently no-ops `send_reply` when no `callback_url` is configured, so this
/// is a clean stand-in for "task has exactly one legacy thread destination".
async fn seed_webhook_thread(
    client: &MoatClient,
    repo_id: &str,
    task_id: &str,
    suffix: &str,
) -> String {
    let req = CreateChannelConfigRequest {
        provider: ChannelType::Webhook,
        source_id: format!("reply-route-{suffix}"),
        source_name: Some(format!("Reply route {suffix}")),
        repository_id: repo_id.to_string(),
        routing_mode: RoutingMode::ThreadBased,
        prompt_template: None,
        provider_config: None,
        response_config_id: None,
        scope: None,
        team_id: None,
        task_name: None,
        tag_ids: None,
        run_as_user_id: None,
        allow_direct_send: None,
        agent_type: None,
    };
    let config = client.api().create_channel_config(&req).await.unwrap();

    client
        .api()
        .connect_channel(&ConnectChannelRequest {
            task_id: task_id.to_string(),
            provider: ChannelType::Webhook,
            source_id: Some(config.source_id.clone()),
            external_thread_id: format!("thread-{suffix}"),
            thread_properties: None,
            filter_user_id: None,
        })
        .await
        .unwrap();

    config.id
}

async fn cleanup(client: &MoatClient, task_id: &str, config_ids: &[&str]) {
    let _ = client.api().stop_task(task_id).await;
    let _ = client.api().delete_task(task_id).await;
    for cid in config_ids {
        let _ = client.api().delete_channel_config(cid).await;
    }
}

// -----------------------------------------------------------------------------
// Bare task — no destinations
// -----------------------------------------------------------------------------

#[tokio::test]
async fn reply_returns_404_when_no_targets() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let task_id = create_bare_task(&client, &repo_id).await;

    let req = SendChannelMessageRequest {
        text: "should fail — no channels".into(),
        file_ids: vec![],
    };
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/tasks/{}/channels/send",
            base_url(),
            task_id
        ))
        .json(&req)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 404, "expected 404 for bare task");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no channels"),
        "expected 'no channels' message, got {body}"
    );

    cleanup(&client, &task_id, &[]).await;
}

// -----------------------------------------------------------------------------
// Single legacy thread → routes through send_to_channels
// -----------------------------------------------------------------------------

#[tokio::test]
async fn reply_succeeds_with_single_legacy_thread() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let task_id = create_bare_task(&client, &repo_id).await;
    let config_id = seed_webhook_thread(&client, &repo_id, &task_id, &suffix).await;

    let req = SendChannelMessageRequest {
        text: "single-thread reply".into(),
        file_ids: vec![],
    };
    let resp = client
        .api()
        .send_to_task_channels(&task_id, &req)
        .await
        .unwrap();

    // Webhook send_reply with no callback_url is a silent success: the call
    // returns Ok(None) which counts as 1 sent destination.
    assert_eq!(resp.sent_count, 1, "expected one destination targeted");
    assert!(
        resp.errors.is_empty(),
        "expected no provider errors, got {:?}",
        resp.errors
    );

    cleanup(&client, &task_id, &[&config_id]).await;
}

// -----------------------------------------------------------------------------
// Multiple destinations (two legacy threads) → 400 with structured body
// -----------------------------------------------------------------------------

#[tokio::test]
async fn reply_returns_400_when_multiple_targets() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let suffix_a = timestamp_suffix();
    let suffix_b = format!("{}-b", timestamp_suffix());

    let task_id = create_bare_task(&client, &repo_id).await;
    let cfg_a = seed_webhook_thread(&client, &repo_id, &task_id, &suffix_a).await;
    let cfg_b = seed_webhook_thread(&client, &repo_id, &task_id, &suffix_b).await;

    let req = SendChannelMessageRequest {
        text: "should fail — multiple destinations".into(),
        file_ids: vec![],
    };
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/tasks/{}/channels/send",
            base_url(),
            task_id
        ))
        .json(&req)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status().as_u16(),
        400,
        "expected 400 for multi-target task"
    );
    let body: MultipleTargetsErrorBody = resp.json().await.unwrap();
    assert_eq!(body.targets.len(), 2, "expected both targets listed");
    for target in &body.targets {
        assert_eq!(target.kind, "thread", "expected thread-kind targets");
        assert!(
            target.flag_hint.starts_with("--thread "),
            "expected --thread flag hint, got {}",
            target.flag_hint
        );
    }
    assert!(
        body.hint.contains("--thread") && body.hint.contains("--subscription"),
        "expected hint to mention both flags, got {}",
        body.hint
    );

    cleanup(&client, &task_id, &[&cfg_a, &cfg_b]).await;
}

// -----------------------------------------------------------------------------
// Explicit thread targeting works alongside the multi-target scenario
// -----------------------------------------------------------------------------

#[tokio::test]
async fn reply_succeeds_with_explicit_thread_flag() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let suffix_a = timestamp_suffix();
    let suffix_b = format!("{}-b", timestamp_suffix());

    let task_id = create_bare_task(&client, &repo_id).await;
    let cfg_a = seed_webhook_thread(&client, &repo_id, &task_id, &suffix_a).await;
    let cfg_b = seed_webhook_thread(&client, &repo_id, &task_id, &suffix_b).await;

    let channels = client.api().list_task_channels(&task_id).await.unwrap();
    let thread_id = channels
        .iter()
        .find_map(|c| c.thread_id)
        .expect("expected at least one connected thread");

    let req = SendChannelMessageRequest {
        text: "explicit-thread reply".into(),
        file_ids: vec![],
    };
    let resp = client
        .api()
        .send_to_task_channel_thread(&task_id, thread_id, &req)
        .await
        .unwrap();

    assert_eq!(resp.sent_count, 1);
    assert!(resp.errors.is_empty(), "got errors: {:?}", resp.errors);

    cleanup(&client, &task_id, &[&cfg_a, &cfg_b]).await;
}

// -----------------------------------------------------------------------------
// Explicit subscription targeting hits the new route and 404s when the
// subscription doesn't exist. This exercises the route registration without
// requiring a seeded `task_subscriptions` row (which needs a bootstrapped
// provider_connection — outside the db-tier test scope).
// -----------------------------------------------------------------------------

#[tokio::test]
async fn reply_subscription_route_returns_404_for_unknown_subscription() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let task_id = create_bare_task(&client, &repo_id).await;

    let req = SendChannelMessageRequest {
        text: "subscription that does not exist".into(),
        file_ids: vec![],
    };
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/tasks/{}/channels/subscriptions/{}/send",
            base_url(),
            task_id,
            "sub_does_not_exist"
        ))
        .json(&req)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status().as_u16(),
        404,
        "expected 404 for unknown subscription"
    );

    cleanup(&client, &task_id, &[]).await;
}
