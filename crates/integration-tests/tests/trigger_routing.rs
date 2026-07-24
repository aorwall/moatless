//! Trigger webhook routing integration tests.
//!
//! Mirrors: tests/api/chains/webhook-routing.spec.ts
//! Spec IDs: INT-10 through INT-33

use integration_tests::{create_webhook_integration, ensure_template_repo, raw_api, test_client};
use moatless_api_schemas::enums::{RoutingMode, TaskStatus};
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
    trigger_id: &str,
    text: &str,
    thread_id: Option<&str>,
) -> (u16, Option<String>) {
    let mut payload = serde_json::json!({ "text": text });
    if let Some(tid) = thread_id {
        payload["threadId"] = serde_json::json!(tid);
    }
    let resp = raw_api::send_webhook_message(http, trigger_id, &payload).await;
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
// Chain 2: Webhook by_subject routing without thread_id (INT-10 through INT-13)
// =============================================================================

#[tokio::test]
async fn trigger_by_subject_routing_without_thread() {
    let client = test_client().await;
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let mut task_ids = Vec::new();

    // INT-10: Create webhook trigger with by_subject routing
    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("by-subject-no-thread-{suffix}"),
        RoutingMode::BySubject,
        None,
    )
    .await;
    assert!(fixture.trigger.active);
    assert_eq!(fixture.trigger.config.routing_mode, RoutingMode::BySubject);
    let trigger_id = fixture.trigger.id;

    // INT-11: Send message via webhook — task created
    let (status, task_id) =
        send_webhook(&http, &trigger_id, "First by_subject webhook test", None).await;
    assert_eq!(status, 200);
    let first_task_id = task_id.expect("should return task_id");
    task_ids.push(first_task_id.clone());

    // INT-11.1: Task is in open state
    let task = client.api().get_task(&first_task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Open);

    // INT-11.2: Task linked to correct repository
    assert_eq!(task.repository_id, repo_id);

    // INT-12: Send second message without thread_id — new task created
    // (each message gets a fresh synthesized subject_key)
    let (status, task_id) =
        send_webhook(&http, &trigger_id, "Second by_subject webhook test", None).await;
    assert_eq!(status, 200);
    let second_task_id = task_id.expect("should return task_id");
    assert_ne!(second_task_id, first_task_id);
    task_ids.push(second_task_id);

    // INT-13: Send message with a brand-new thread_id — new task
    let thread_id = format!("new-thread-{suffix}");
    let (status, task_id) = send_webhook(
        &http,
        &trigger_id,
        "Message with new thread",
        Some(&thread_id),
    )
    .await;
    assert_eq!(status, 200);
    let third_task_id = task_id.expect("should return task_id");
    assert_ne!(third_task_id, first_task_id);
    task_ids.push(third_task_id);

    // Cleanup
    cleanup_tasks(&client, &task_ids).await;
}

// =============================================================================
// Chain 3: Webhook ongoing routing (INT-20 through INT-23)
// =============================================================================

#[tokio::test]
async fn trigger_ongoing_routing() {
    let client = test_client().await;
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let mut task_ids = Vec::new();

    // INT-20: Create webhook trigger with ongoing routing
    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("ongoing-{suffix}"),
        RoutingMode::Ongoing,
        None,
    )
    .await;
    assert!(fixture.trigger.active);
    assert_eq!(fixture.trigger.config.routing_mode, RoutingMode::Ongoing);
    let trigger_id = fixture.trigger.id;

    // INT-21: First message creates ongoing task
    let (status, task_id) = send_webhook(&http, &trigger_id, "First ongoing message", None).await;
    assert_eq!(status, 200);
    let ongoing_task_id = task_id.expect("should return task_id");
    task_ids.push(ongoing_task_id.clone());

    // Wait briefly for async processing
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Check that the embedded rule now has ongoing_task_id
    let trigger = client.api().get_trigger(&trigger_id).await.unwrap();
    assert_eq!(
        trigger.config.ongoing_task_id.as_deref(),
        Some(ongoing_task_id.as_str())
    );

    // INT-22: Second message routes to same task
    let (status, task_id) = send_webhook(&http, &trigger_id, "Second ongoing message", None).await;
    assert_eq!(status, 200);
    assert_eq!(task_id.as_deref(), Some(ongoing_task_id.as_str()));

    // INT-23: After task closed, next message creates new task
    client.api().close_task(&ongoing_task_id).await.unwrap();
    let (status, task_id) = send_webhook(&http, &trigger_id, "Message after close", None).await;
    assert_eq!(status, 200);
    let new_task_id = task_id.expect("should return task_id");
    assert_ne!(new_task_id, ongoing_task_id);
    task_ids.push(new_task_id.clone());

    // Wait briefly for async processing
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Rule should now point to the new task
    let trigger = client.api().get_trigger(&trigger_id).await.unwrap();
    assert_eq!(
        trigger.config.ongoing_task_id.as_deref(),
        Some(new_task_id.as_str())
    );

    cleanup_tasks(&client, &task_ids).await;
}

// =============================================================================
// Chain 4: Webhook by_subject routing with thread_id (INT-30 through INT-33)
// =============================================================================

#[tokio::test]
async fn trigger_by_subject_routing_with_thread() {
    let client = test_client().await;
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let mut task_ids = Vec::new();

    // INT-30: Create webhook trigger with by_subject routing
    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("thread-{suffix}"),
        RoutingMode::BySubject,
        None,
    )
    .await;
    assert!(fixture.trigger.active);
    assert_eq!(fixture.trigger.config.routing_mode, RoutingMode::BySubject);
    let trigger_id = fixture.trigger.id;

    // INT-31: New message creates task
    let thread_a = format!("thread-a-{suffix}");
    let (status, task_id) =
        send_webhook(&http, &trigger_id, "First thread message", Some(&thread_a)).await;
    assert_eq!(status, 200);
    let first_task_id = task_id.expect("should return task_id");
    task_ids.push(first_task_id.clone());

    // INT-32: Reply in same thread routes to same task
    let (status, task_id) =
        send_webhook(&http, &trigger_id, "Reply in same thread", Some(&thread_a)).await;
    assert_eq!(status, 200);
    assert_eq!(task_id.as_deref(), Some(first_task_id.as_str()));

    // INT-33: New thread creates new task
    let thread_b = format!("thread-b-{suffix}");
    let (status, task_id) =
        send_webhook(&http, &trigger_id, "Message in new thread", Some(&thread_b)).await;
    assert_eq!(status, 200);
    let second_task_id = task_id.expect("should return task_id");
    assert_ne!(second_task_id, first_task_id);
    task_ids.push(second_task_id);

    // Cleanup
    cleanup_tasks(&client, &task_ids).await;
}

/// A closed task is terminal: a later reply in the same thread must not
/// resurrect it. The close path deactivates the task's binding, so by_subject
/// routing spawns a fresh task instead of reusing the closed one, and no new
/// message is persisted onto the closed task.
#[tokio::test]
async fn trigger_by_subject_closed_task_not_resurrected() {
    let client = test_client().await;
    let http = reqwest::Client::new();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let mut task_ids = Vec::new();

    let fixture = create_webhook_integration(
        &client,
        &repo_id,
        &format!("closed-{suffix}"),
        RoutingMode::BySubject,
        None,
    )
    .await;
    let trigger_id = fixture.trigger.id;

    // First message in the thread creates a task.
    let thread = format!("thread-closed-{suffix}");
    let (status, task_id) = send_webhook(&http, &trigger_id, "First message", Some(&thread)).await;
    assert_eq!(status, 200);
    let first_task_id = task_id.expect("should return task_id");
    task_ids.push(first_task_id.clone());

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let msgs_before = client
        .api()
        .get_messages(&first_task_id)
        .await
        .unwrap()
        .messages
        .len();

    // Close the task — this deactivates its binding.
    client.api().close_task(&first_task_id).await.unwrap();
    let closed = client.api().get_task(&first_task_id).await.unwrap();
    assert_eq!(closed.status, TaskStatus::Closed);

    // A reply in the SAME thread must not reuse the closed task.
    let (status, task_id) =
        send_webhook(&http, &trigger_id, "Reply after close", Some(&thread)).await;
    assert_eq!(status, 200);
    let after_close_task_id = task_id.expect("should return task_id");
    assert_ne!(
        after_close_task_id, first_task_id,
        "reply after close must not resurrect the closed task"
    );
    task_ids.push(after_close_task_id);

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // The closed task stays closed and gains no new message.
    let still = client.api().get_task(&first_task_id).await.unwrap();
    assert_eq!(still.status, TaskStatus::Closed);
    let msgs_after = client
        .api()
        .get_messages(&first_task_id)
        .await
        .unwrap()
        .messages
        .len();
    assert_eq!(
        msgs_after, msgs_before,
        "no message should be persisted onto a closed task"
    );

    cleanup_tasks(&client, &task_ids).await;
}
