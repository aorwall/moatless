//! Message send/list integration tests.
//!
//! Tests sending messages and retrieving them.

use integration_tests::{ensure_template_repo, test_client};
use moatless_api_schemas::{message::SendMessageRequest, task::CreateTaskRequest};

#[tokio::test]
async fn send_and_list_messages() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;

    // Create a task
    let task = client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: repo_id,
            workspace_id: None,
            message: Some("Initial message".into()),
            name: None,
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
        .unwrap();

    let task_id = task.task_id;

    // Send a follow-up message
    let msg = SendMessageRequest {
        message: "Follow-up message".into(),
        agent_mode: None,
        command_name: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        resume: None,
        tool_name: None,
        tool_response: None,
        tool_use_id: None,
        source_task_id: None,
        start_fresh: None,
    };
    let _resp = client.api().send_message(&task_id, &msg).await.unwrap();

    // List messages
    let messages = client.api().get_messages(&task_id).await.unwrap();
    assert!(
        !messages.messages.is_empty(),
        "expected at least one message"
    );

    // Cleanup
    let _ = client.api().delete_task(&task_id).await;
}
