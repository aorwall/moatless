//! Canonical task reply routing tests for task bindings.

use integration_tests::{
    base_url, create_webhook_integration, ensure_template_repo, raw_api, test_client,
};
use moatless_api_schemas::bindings::{
    CreateTaskBindingRequest, MultipleTargetsErrorBody, SendTaskReplyRequest,
};
use moatless_api_schemas::enums::{AdapterKind, RoutingMode};
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
            workspace_id: None,
            message: Some("Reply-routing fixture task".into()),
            name: Some("reply-routing-test".into()),
            branch: None,
            description: None,
            agent_mode: None,
            agent_type: None,
            contexts: vec![],
            file_ids: vec![],
            model: None,
            command_name: None,
            headless: None,
            task_template_id: None,
            parent_task_id: None,
        })
        .await
        .unwrap()
        .task_id
}

async fn cleanup(client: &MoatClient, task_id: &str) {
    let _ = client.api().stop_task(task_id).await;
    let _ = client.api().delete_task(task_id).await;
}

#[tokio::test]
async fn reply_returns_404_when_no_bindings() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let task_id = create_bare_task(&client, &repo_id).await;

    let resp = raw_api::send_task_reply_raw(
        &reqwest::Client::new(),
        &task_id,
        &serde_json::json!({
            "text": "should fail - no bindings",
            "fileIds": []
        }),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 404, "expected 404 for bare task");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no bindings"),
        "expected 'no bindings' message, got {body}"
    );

    cleanup(&client, &task_id).await;
}

#[tokio::test]
async fn reply_succeeds_with_single_binding() {
    let client = test_client().await;
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();

    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("reply-single-{suffix}"),
        RoutingMode::BySubject,
        None,
    )
    .await;

    let inbound_resp = raw_api::send_webhook_message(
        &http,
        &fixture.trigger.id,
        &serde_json::json!({
            "text": "seed one binding",
            "threadId": format!("thread-{suffix}")
        }),
    )
    .await;
    assert_eq!(inbound_resp.status().as_u16(), 200);
    let inbound_body: serde_json::Value = inbound_resp.json().await.unwrap();
    let task_id = inbound_body["taskId"].as_str().unwrap().to_string();

    let bindings = client.api().list_task_bindings(&task_id).await.unwrap();
    assert_eq!(bindings.len(), 1);

    let req = SendTaskReplyRequest {
        text: "single-binding reply".into(),
        file_ids: vec![],
        binding_id: None,
    };
    let resp = client.api().send_task_reply(&task_id, &req).await.unwrap();

    assert_eq!(resp.sent_count, 1, "expected one destination targeted");
    assert!(
        resp.errors.is_empty(),
        "expected no provider errors, got {:?}",
        resp.errors
    );
    assert_eq!(resp.targets.len(), 1);
    assert_eq!(resp.targets[0].kind, "binding");

    cleanup(&client, &task_id).await;
}

#[tokio::test]
async fn reply_returns_400_when_multiple_bindings() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let task_id = create_bare_task(&client, &repo_id).await;

    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("reply-multi-{suffix}"),
        RoutingMode::BySubject,
        None,
    )
    .await;

    for idx in 1..=2 {
        client
            .api()
            .create_task_binding(
                &task_id,
                &CreateTaskBindingRequest {
                    adapter_kind: AdapterKind::Webhook,
                    connection_id: fixture.connection.id.clone(),
                    trigger_id: Some(fixture.trigger.id.clone()),
                    subject_kind: "webhook_thread".into(),
                    subject_key: format!("manual-{suffix}-{idx}"),
                    destination_config: Some(serde_json::json!({})),
                    metadata: None,
                    origin: Some("test".into()),
                },
            )
            .await
            .unwrap();
    }

    let resp = raw_api::send_task_reply_raw(
        &reqwest::Client::new(),
        &task_id,
        &serde_json::json!({
            "text": "should fail - multiple bindings",
            "fileIds": []
        }),
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        400,
        "expected 400 for multi-target task"
    );
    let body: MultipleTargetsErrorBody = resp.json().await.unwrap();
    assert_eq!(body.targets.len(), 2, "expected both targets listed");
    for target in &body.targets {
        assert_eq!(target.kind, "binding");
        assert!(
            target.flag_hint.starts_with("--binding "),
            "expected --binding flag hint, got {}",
            target.flag_hint
        );
    }

    cleanup(&client, &task_id).await;
}

#[tokio::test]
async fn reply_succeeds_with_explicit_binding_flag() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let task_id = create_bare_task(&client, &repo_id).await;

    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("reply-explicit-{suffix}"),
        RoutingMode::BySubject,
        None,
    )
    .await;

    let binding = client
        .api()
        .create_task_binding(
            &task_id,
            &CreateTaskBindingRequest {
                adapter_kind: AdapterKind::Webhook,
                connection_id: fixture.connection.id,
                trigger_id: Some(fixture.trigger.id),
                subject_kind: "webhook_thread".into(),
                subject_key: format!("manual-{suffix}"),
                destination_config: Some(serde_json::json!({})),
                metadata: None,
                origin: Some("test".into()),
            },
        )
        .await
        .unwrap();

    let resp = raw_api::send_task_reply_to_binding_raw(
        &reqwest::Client::new(),
        &task_id,
        &binding.id,
        "explicit-binding reply",
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["sentCount"].as_u64(), Some(1));

    cleanup(&client, &task_id).await;
}

#[tokio::test]
async fn reply_binding_route_returns_404_for_unknown_binding() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let task_id = create_bare_task(&client, &repo_id).await;

    let resp = reqwest::Client::new()
        .post(format!("{}/api/v1/tasks/{}/reply", base_url(), task_id))
        .json(&serde_json::json!({
            "text": "binding that does not exist",
            "bindingId": "tb_does_not_exist",
            "fileIds": []
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status().as_u16(),
        404,
        "expected 404 for unknown binding"
    );

    cleanup(&client, &task_id).await;
}
