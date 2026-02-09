//! Integration tests for message_dao.

mod common;

use sandbox_controller::db::message_dao;
use sandbox_controller::db::models::{TaskMessageContentBlockRow, TaskMessageContextRow};
use sqlx::PgPool;

use common::{make_message_row, make_repository_row, make_task_row};

/// Seed a repo + task so message FKs work.
async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    sandbox_controller::db::repository_dao::insert(pool, &repo)
        .await
        .expect("seed repo");

    let task = make_task_row(task_id, user);
    sandbox_controller::db::task_dao::insert_task(pool, &task)
        .await
        .expect("seed task");
}

#[sqlx::test]
async fn append_and_get_all(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "msg-uuid-1");
    let msg_id = message_dao::append(&pool, &msg, &[], &[]).await.unwrap();
    assert!(msg_id > 0);

    let (messages, contexts, blocks) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].uuid, "msg-uuid-1");
    assert!(contexts.is_empty());
    assert!(blocks.is_empty());
}

#[sqlx::test]
async fn append_with_contexts_and_blocks(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "msg-uuid-2");
    let contexts = vec![
        TaskMessageContextRow {
            id: 0,
            task_message_id: 0, // ignored, set by append
            context_type: "file".to_string(),
            reference_id: "ref-1".to_string(),
            data: serde_json::json!({"path": "/test.ts"}),
            created_at: None,
        },
        TaskMessageContextRow {
            id: 0,
            task_message_id: 0,
            context_type: "url".to_string(),
            reference_id: "ref-2".to_string(),
            data: serde_json::json!({"url": "https://example.com"}),
            created_at: None,
        },
    ];
    let blocks = vec![
        TaskMessageContentBlockRow {
            id: 0,
            task_message_id: 0,
            sort_order: 0,
            block_type: "text".to_string(),
            show_in_ui: true,
            text_content: Some("Hello world".to_string()),
            tool_use_id: None,
            tool_result_content: None,
            tool_result_is_error: None,
            image_media_type: None,
            image_data: None,
            created_at: None,
        },
        TaskMessageContentBlockRow {
            id: 0,
            task_message_id: 0,
            sort_order: 1,
            block_type: "tool_use".to_string(),
            show_in_ui: true,
            text_content: None,
            tool_use_id: Some("tu-1".to_string()),
            tool_result_content: None,
            tool_result_is_error: None,
            image_media_type: None,
            image_data: None,
            created_at: None,
        },
        TaskMessageContentBlockRow {
            id: 0,
            task_message_id: 0,
            sort_order: 2,
            block_type: "tool_result".to_string(),
            show_in_ui: false,
            text_content: None,
            tool_use_id: None,
            tool_result_content: Some("OK".to_string()),
            tool_result_is_error: Some(false),
            image_media_type: None,
            image_data: None,
            created_at: None,
        },
    ];

    message_dao::append(&pool, &msg, &contexts, &blocks)
        .await
        .unwrap();

    let (messages, ctx_rows, blk_rows) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(ctx_rows.len(), 2);
    assert_eq!(blk_rows.len(), 3);
    assert_eq!(blk_rows[0].block_type, "text");
    assert_eq!(blk_rows[1].block_type, "tool_use");
    assert_eq!(blk_rows[2].block_type, "tool_result");
}

#[sqlx::test]
async fn append_upsert_on_conflict(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "msg-uuid-dup");
    message_dao::append(&pool, &msg, &[], &[]).await.unwrap();

    // Re-append with same uuid — should update, not duplicate
    let mut msg2 = make_message_row("task-1", "msg-uuid-dup");
    msg2.message_content = serde_json::json!([{"type": "text", "text": "updated"}]);
    message_dao::append(&pool, &msg2, &[], &[]).await.unwrap();

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    // Content should be updated
    let content = &messages[0].message_content;
    assert!(content.to_string().contains("updated"));
}

#[sqlx::test]
async fn get_pending_user(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Two pending user messages (no agent_session_id)
    let msg1 = make_message_row("task-1", "pending-1");
    message_dao::append(&pool, &msg1, &[], &[]).await.unwrap();

    let msg2 = make_message_row("task-1", "pending-2");
    message_dao::append(&pool, &msg2, &[], &[]).await.unwrap();

    // One acknowledged message
    let mut msg3 = make_message_row("task-1", "acked-1");
    msg3.agent_session_id = Some("sess-1".to_string());
    message_dao::append(&pool, &msg3, &[], &[]).await.unwrap();

    let pending = message_dao::get_pending_user(&pool, "task-1")
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
}

#[sqlx::test]
async fn mark_acknowledged(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg1 = make_message_row("task-1", "ack-msg-1");
    message_dao::append(&pool, &msg1, &[], &[]).await.unwrap();

    let msg2 = make_message_row("task-1", "ack-msg-2");
    message_dao::append(&pool, &msg2, &[], &[]).await.unwrap();

    let count = message_dao::mark_acknowledged(&pool, "task-1", "sess-abc")
        .await
        .unwrap();
    assert_eq!(count, 2);

    // After ack, pending should be empty
    let pending = message_dao::get_pending_user(&pool, "task-1")
        .await
        .unwrap();
    assert!(pending.is_empty());
}

#[sqlx::test]
async fn get_last_result(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Insert two result messages with different timestamps
    let mut msg1 = make_message_row("task-1", "result-1");
    msg1.message_type = "result".to_string();
    msg1.timestamp = "2024-01-01T00:00:00.000Z".to_string();
    message_dao::append(&pool, &msg1, &[], &[]).await.unwrap();

    let mut msg2 = make_message_row("task-1", "result-2");
    msg2.message_type = "result".to_string();
    msg2.timestamp = "2024-01-02T00:00:00.000Z".to_string();
    message_dao::append(&pool, &msg2, &[], &[]).await.unwrap();

    let last = message_dao::get_last_result(&pool, "task-1")
        .await
        .unwrap();
    assert!(last.is_some());
    assert_eq!(last.unwrap().uuid, "result-2");
}

#[sqlx::test]
async fn delete_queued(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "queued-msg");
    message_dao::append(&pool, &msg, &[], &[]).await.unwrap();

    let deleted = message_dao::delete_queued(&pool, "task-1", "queued-msg")
        .await
        .unwrap();
    assert!(deleted);

    let pending = message_dao::get_pending_user(&pool, "task-1")
        .await
        .unwrap();
    assert!(pending.is_empty());

    // Delete again should return false
    let deleted_again = message_dao::delete_queued(&pool, "task-1", "queued-msg")
        .await
        .unwrap();
    assert!(!deleted_again);
}
