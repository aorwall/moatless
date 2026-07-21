//! Task CRUD integration tests.
//!
//! Mirrors: tests/api/chains/task-crud.spec.ts
//! Spec IDs: TASK-01 through TASK-09.1

use integration_tests::{ensure_template_repo, test_client};
use moatless_api_schemas::{
    enums::{TaskStatus, Visibility},
    task::{CreateTaskRequest, SetTagsRequest, UpdateTaskMetadataRequest, UpdateVisibilityRequest},
};
use pretty_assertions::assert_eq;

// =============================================================================
// Chain: Task CRUD (TASK-01 through TASK-09.1)
// =============================================================================

#[tokio::test]
async fn task_crud_chain() {
    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;

    // TASK-01: Create task with message
    let req = CreateTaskRequest {
        repository_id: Some(repo_id.clone()),
        workspace_id: None,
        message: Some("E2E CRUD test task".into()),
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
    };
    let created = client.api().create_task(&req).await.unwrap();
    assert_eq!(created.status, TaskStatus::Open);
    assert!(!created.task_id.is_empty());

    let task_id = created.task_id;

    // TASK-01.1: Task appears in listing
    let tasks = client.api().list_tasks(&Default::default()).await.unwrap();
    assert!(tasks.items.iter().any(|t| t.task_id == task_id));

    // TASK-03: Get task by ID
    let task = client.api().get_task(&task_id).await.unwrap();
    assert_eq!(task.task_id, task_id);
    assert_eq!(task.status, TaskStatus::Open);
    assert!(!task.repository_id.is_empty());
    assert!(!task.created_at.is_empty());

    // TASK-04: Update task metadata
    let update = UpdateTaskMetadataRequest {
        name: Some("Updated CRUD Test Task".into()),
        description: None,
    };
    let updated = client
        .api()
        .update_task_metadata(&task_id, &update)
        .await
        .unwrap();
    assert_eq!(updated.name.as_deref(), Some("Updated CRUD Test Task"));

    // Confirm via GET
    let fetched = client.api().get_task(&task_id).await.unwrap();
    assert_eq!(fetched.name.as_deref(), Some("Updated CRUD Test Task"));

    // TASK-05: Set task tags
    let tags = SetTagsRequest {
        tags: vec!["e2e".into(), "crud-test".into()],
    };
    client.api().set_task_tags(&task_id, &tags).await.unwrap();

    let task = client.api().get_task(&task_id).await.unwrap();
    assert!(task.tags.len() >= 2);
    let tag_names: Vec<&str> = task.tags.iter().map(|t| t.name.as_str()).collect();
    assert!(tag_names.contains(&"e2e"));
    assert!(tag_names.contains(&"crud-test"));

    // TASK-05.1: Clear tags
    let empty_tags = SetTagsRequest { tags: vec![] };
    client
        .api()
        .set_task_tags(&task_id, &empty_tags)
        .await
        .unwrap();

    let task = client.api().get_task(&task_id).await.unwrap();
    assert!(task.tags.is_empty());

    // TASK-06: Update task visibility
    let vis = UpdateVisibilityRequest {
        visibility: Visibility::Public,
    };
    let updated = client
        .api()
        .update_visibility(&task_id, &vis)
        .await
        .unwrap();
    assert_eq!(updated.visibility, Visibility::Public);

    // Confirm via GET
    let fetched = client.api().get_task(&task_id).await.unwrap();
    assert_eq!(fetched.visibility, Visibility::Public);

    // TASK-07: List tasks with filters
    let open_tasks = client
        .api()
        .list_tasks(&moatless_api_schemas::task::ListTasksQuery {
            status: Some(TaskStatus::Open),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!open_tasks.items.is_empty());
    for t in &open_tasks.items {
        assert_eq!(t.status, TaskStatus::Open);
    }

    // TASK-08: Close task
    let closed = client.api().close_task(&task_id).await.unwrap();
    assert_eq!(closed.status, TaskStatus::Closed);

    // Confirm via GET
    let fetched = client.api().get_task(&task_id).await.unwrap();
    assert_eq!(fetched.status, TaskStatus::Closed);

    // TASK-09: Delete task
    client.api().delete_task(&task_id).await.unwrap();

    // TASK-09.1: Task removed from listing
    let tasks = client.api().list_tasks(&Default::default()).await.unwrap();
    assert!(!tasks.items.iter().any(|t| t.task_id == task_id));
}
