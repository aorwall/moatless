//! Feedback API integration tests (db-tier).
//!
//! Exercises POST /api/v1/feedback (user context only — agent context
//! requires a sandbox-issued JWT and is covered by sandbox-tier tests)
//! and GET /api/v1/tasks/{task_id}/feedback.

use integration_tests::{ensure_template_repo, test_client};
use moatless_api_schemas::feedback::{CreateFeedbackRequest, FeedbackSource, FeedbackSurface};
use moatless_api_schemas::task::CreateTaskRequest;

fn create_request(repo_id: String) -> CreateTaskRequest {
    CreateTaskRequest {
        repository_id: repo_id,
        workspace_id: None,
        message: None,
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
    }
}

#[tokio::test]
async fn submit_and_list_feedback_for_task() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;

    let task = client
        .api()
        .create_task(&create_request(repo_id))
        .await
        .unwrap();
    let task_id = task.task_id;

    // Task-level feedback
    let task_fb = client
        .api()
        .submit_feedback(&CreateFeedbackRequest {
            task_id: task_id.clone(),
            message_uuid: None,
            tool_use_id: None,
            surface: FeedbackSurface::Task,
            comment: "task is great".into(),
            metadata: None,
        })
        .await
        .unwrap();
    assert_eq!(task_fb.surface, FeedbackSurface::Task);
    assert_eq!(task_fb.source, FeedbackSource::User);
    assert_eq!(task_fb.task_id, task_id);
    assert!(task_fb.message_uuid.is_none());
    assert!(task_fb.tool_use_id.is_none());

    // Message-level feedback
    let msg_fb = client
        .api()
        .submit_feedback(&CreateFeedbackRequest {
            task_id: task_id.clone(),
            message_uuid: Some("msg-uuid-1".into()),
            tool_use_id: None,
            surface: FeedbackSurface::Message,
            comment: "this answer was unhelpful".into(),
            metadata: None,
        })
        .await
        .unwrap();
    assert_eq!(msg_fb.surface, FeedbackSurface::Message);
    assert_eq!(msg_fb.message_uuid.as_deref(), Some("msg-uuid-1"));

    // Tool-call-level feedback
    let tool_fb = client
        .api()
        .submit_feedback(&CreateFeedbackRequest {
            task_id: task_id.clone(),
            message_uuid: Some("msg-uuid-1".into()),
            tool_use_id: Some("tu-1".into()),
            surface: FeedbackSurface::ToolCall,
            comment: "this Bash invocation was unnecessary".into(),
            metadata: None,
        })
        .await
        .unwrap();
    assert_eq!(tool_fb.surface, FeedbackSurface::ToolCall);
    assert_eq!(tool_fb.tool_use_id.as_deref(), Some("tu-1"));

    // List endpoint returns all three
    let listed = client.api().list_task_feedback(&task_id).await.unwrap();
    assert_eq!(listed.items.len(), 3);
    let ids: Vec<&str> = listed
        .items
        .iter()
        .map(|i| i.feedback_id.as_str())
        .collect();
    assert!(ids.contains(&task_fb.feedback_id.as_str()));
    assert!(ids.contains(&msg_fb.feedback_id.as_str()));
    assert!(ids.contains(&tool_fb.feedback_id.as_str()));

    let _ = client.api().delete_task(&task_id).await;
}

#[tokio::test]
async fn tool_use_id_without_message_uuid_is_rejected() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;

    let task = client
        .api()
        .create_task(&create_request(repo_id))
        .await
        .unwrap();
    let task_id = task.task_id;

    let err = client
        .api()
        .submit_feedback(&CreateFeedbackRequest {
            task_id: task_id.clone(),
            message_uuid: None,
            tool_use_id: Some("tu-orphan".into()),
            surface: FeedbackSurface::ToolCall,
            comment: "broken scope".into(),
            metadata: None,
        })
        .await;
    assert!(
        err.is_err(),
        "expected 400 when tool_use_id has no message_uuid"
    );

    let _ = client.api().delete_task(&task_id).await;
}

#[tokio::test]
async fn preview_failure_feedback_carries_metadata() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;

    let task = client
        .api()
        .create_task(&create_request(repo_id))
        .await
        .unwrap();
    let task_id = task.task_id;

    let fb = client
        .api()
        .submit_feedback(&CreateFeedbackRequest {
            task_id: task_id.clone(),
            message_uuid: None,
            tool_use_id: None,
            surface: FeedbackSurface::PreviewFailure,
            comment: "preview crashed".into(),
            metadata: Some(serde_json::json!({
                "preview_error": "ECONNREFUSED",
                "server_name": "preview",
            })),
        })
        .await
        .unwrap();

    assert_eq!(fb.surface, FeedbackSurface::PreviewFailure);
    let meta = fb.metadata.expect("metadata");
    assert_eq!(meta["preview_error"], serde_json::json!("ECONNREFUSED"));

    let _ = client.api().delete_task(&task_id).await;
}
