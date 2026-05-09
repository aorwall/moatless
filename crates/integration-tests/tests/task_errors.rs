//! Task error case integration tests.
//!
//! Mirrors: tests/api/chains/task-errors.spec.ts
//! Spec IDs: TASK-10 through TASK-11

use integration_tests::{ensure_template_repo, raw_api, test_client};
use moatless_api_schemas::task::CreateTaskRequest;

// TASK-10: Create task with invalid repository (404)
#[tokio::test]
async fn task_create_invalid_repository() {
    let http = reqwest::Client::new();
    let resp = raw_api::create_task_raw(
        &http,
        &serde_json::json!({
            "repositoryId": "00000000-0000-0000-0000-000000000000",
            "message": "This should fail",
        }),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 404);
}

// TASK-11: Send message to closed task — backend accepts and reopens
#[tokio::test]
async fn task_send_message_to_closed_task() {
    let client = test_client();
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;

    // Create a task
    let req = CreateTaskRequest {
        repository_id: repo_id,
        message: Some("Temp task for close test".into()),
        name: None,
        branch: None,
        parent_task_id: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        command_name: None,
        headless: None,
    };
    let created = client.api().create_task(&req).await.unwrap();
    let task_id = created.task_id;

    // Close it
    client.api().close_task(&task_id).await.unwrap();

    // Send message — should be accepted (backend reopens)
    let resp = raw_api::send_message_raw(
        &http,
        &task_id,
        &serde_json::json!({
            "message": "This should be accepted",
        }),
    )
    .await;

    assert!(
        resp.status().as_u16() < 300,
        "expected success, got {}",
        resp.status()
    );

    // Cleanup
    let _ = client.api().delete_task(&task_id).await;
}
