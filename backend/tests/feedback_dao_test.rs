#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for `feedback_dao`.

use chrono::Utc;
use moatless_vibe_backend::feedback::dao::{FeedbackRepository, PgFeedbackRepository};
use moatless_vibe_backend::feedback::models::Feedback;
use sqlx::PgPool;

fn sample(feedback_id: &str, task_id: &str, surface: &str) -> Feedback {
    Feedback {
        feedback_id: feedback_id.to_string(),
        task_id: task_id.to_string(),
        message_uuid: None,
        tool_use_id: None,
        source: "user".to_string(),
        surface: surface.to_string(),
        comment: "looks good".to_string(),
        metadata: None,
        created_by: "user-1".to_string(),
        created_at: Utc::now(),
    }
}

#[sqlx::test]
async fn insert_and_find_round_trips(pool: PgPool) {
    let dao = PgFeedbackRepository(pool);
    let row = sample("fb-1", "task-1", "task");
    dao.insert(&row).await.expect("insert");

    let fetched = dao.find_by_id("fb-1").await.expect("find").expect("row");
    assert_eq!(fetched.feedback_id, "fb-1");
    assert_eq!(fetched.task_id, "task-1");
    assert_eq!(fetched.surface, "task");
    assert_eq!(fetched.comment, "looks good");
    assert_eq!(fetched.source, "user");
    assert!(fetched.message_uuid.is_none());
    assert!(fetched.tool_use_id.is_none());
}

#[sqlx::test]
async fn list_for_task_filters_and_orders(pool: PgPool) {
    let dao = PgFeedbackRepository(pool);

    let mut a1 = sample("fb-a1", "task-A", "task");
    a1.created_at = Utc::now() - chrono::Duration::seconds(60);
    dao.insert(&a1).await.unwrap();

    let mut a2 = sample("fb-a2", "task-A", "message");
    a2.message_uuid = Some("msg-1".to_string());
    dao.insert(&a2).await.unwrap();

    let b1 = sample("fb-b1", "task-B", "task");
    dao.insert(&b1).await.unwrap();

    let listed = dao.list_for_task("task-A").await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].feedback_id, "fb-a1");
    assert_eq!(listed[1].feedback_id, "fb-a2");

    let other = dao.list_for_task("task-B").await.unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].feedback_id, "fb-b1");
}

#[sqlx::test]
async fn metadata_round_trips(pool: PgPool) {
    let dao = PgFeedbackRepository(pool);
    let mut row = sample("fb-meta", "task-meta", "preview_failure");
    row.metadata = Some(serde_json::json!({"preview_error": "boom"}));
    dao.insert(&row).await.unwrap();

    let fetched = dao.find_by_id("fb-meta").await.unwrap().unwrap();
    assert_eq!(
        fetched.metadata.unwrap()["preview_error"],
        serde_json::json!("boom")
    );
}
