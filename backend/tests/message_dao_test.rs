#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for message_dao.

mod common;

use moatless_vibe_backend::message::dao::{
    self as message_dao, MessageRepository, PgMessageRepository,
};
use moatless_vibe_backend::message::models::{
    ContentBlockType, MessageRole, MessageType, TaskMessageContentBlock, TaskMessageContext,
};
use sqlx::PgPool;

use common::{make_message_row, make_repository_row, make_task_row};

/// Seed a repo + task so message FKs work.
async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    moatless_vibe_backend::repository::dao::insert(pool, &repo)
        .await
        .expect("seed repo");

    let task = make_task_row(task_id, user);
    moatless_vibe_backend::task::dao::insert_task(pool, &task)
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

/// The full raw `assistant_usage` blob (including the ephemeral cache-write
/// breakdown that the typed columns don't capture) round-trips through the
/// `task_messages` column. This is the data cost is derived from when the
/// transport reports no `total_cost_usd` (tmux).
#[sqlx::test]
async fn append_persists_assistant_usage(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let usage = serde_json::json!({
        "input_tokens": 10,
        "output_tokens": 197,
        "cache_creation_input_tokens": 3980,
        "cache_read_input_tokens": 18111,
        "cache_creation": {
            "ephemeral_1h_input_tokens": 3980,
            "ephemeral_5m_input_tokens": 0
        },
        "server_tool_use": { "web_search_requests": 0, "web_fetch_requests": 0 },
        "service_tier": "standard"
    });

    let mut msg = make_message_row("task-1", "usage-msg-1");
    msg.assistant_message_model = Some("claude-haiku-4-5-20251001".to_string());
    msg.assistant_usage = Some(usage.clone());

    message_dao::append(&pool, &msg, &[], &[]).await.unwrap();

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    let stored = &messages[0];
    assert_eq!(
        stored.assistant_message_model.as_deref(),
        Some("claude-haiku-4-5-20251001")
    );
    assert_eq!(stored.assistant_usage.as_ref(), Some(&usage));
    // The ephemeral cache-write breakdown (absent from the typed columns) is
    // preserved for downstream cost derivation.
    assert_eq!(
        stored
            .assistant_usage
            .as_ref()
            .and_then(|u| u.get("cache_creation"))
            .and_then(|c| c.get("ephemeral_1h_input_tokens"))
            .and_then(serde_json::Value::as_i64),
        Some(3980)
    );
}

/// Codex reports token usage only on the terminal result — its assistant
/// message row is inserted with no usage. `backfill_last_assistant_usage` fills
/// the most recent assistant row (guarded by `assistant_usage IS NULL`), so the
/// turn's tokens + raw usage blob (incl. `reasoning_output_tokens`) land on the
/// message. A second call is a no-op because the row now has usage.
#[sqlx::test]
async fn backfill_fills_last_assistant_usage_once(pool: PgPool) {
    use moatless_vibe_backend::message::models::{MessageRole, MessageType};

    seed_task(&pool, "task-1", "user-1").await;

    // Assistant reply inserted WITHOUT usage (the Codex shape).
    let mut assistant = make_message_row("task-1", "codex-assistant-1");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    assistant.assistant_message_model = Some("gpt-5.5".to_string());
    message_dao::append(&pool, &assistant, &[], &[])
        .await
        .unwrap();

    let usage = serde_json::json!({
        "input_tokens": 10943,
        "output_tokens": 5,
        "cache_read_input_tokens": 9088,
        "cache_creation_input_tokens": serde_json::Value::Null,
        "reasoning_output_tokens": 128
    });

    let updated = message_dao::backfill_last_assistant_usage(
        &pool,
        "task-1",
        Some(10943),
        Some(5),
        None,
        Some(9088),
        Some(&usage),
    )
    .await
    .unwrap();
    assert!(updated, "expected the assistant row to be backfilled");

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let stored = messages
        .iter()
        .find(|m| m.uuid == "codex-assistant-1")
        .expect("assistant row present");
    assert_eq!(stored.assistant_usage_input_tokens, Some(10943));
    assert_eq!(stored.assistant_usage_output_tokens, Some(5));
    assert_eq!(stored.assistant_usage_cache_read_input_tokens, Some(9088));
    assert_eq!(stored.assistant_message_model.as_deref(), Some("gpt-5.5"));
    assert_eq!(stored.assistant_usage.as_ref(), Some(&usage));
    // Codex reasoning tokens survive in the raw blob.
    assert_eq!(
        stored
            .assistant_usage
            .as_ref()
            .and_then(|u| u.get("reasoning_output_tokens"))
            .and_then(serde_json::Value::as_i64),
        Some(128)
    );

    // Idempotent: the row now has usage, so a second backfill touches nothing.
    let again = message_dao::backfill_last_assistant_usage(
        &pool,
        "task-1",
        Some(1),
        Some(1),
        None,
        Some(1),
        Some(&serde_json::json!({"input_tokens": 1})),
    )
    .await
    .unwrap();
    assert!(!again, "second backfill must be a no-op (usage already set)");
}

/// Claude reports usage inline, but one API response can span several rows when
/// the CLI interleaves other traffic between its content-block lines — and the
/// repeats deliberately carry none, so the tokens are counted once. The backfill
/// must leave those alone: writing the run's cumulative total onto a repeat
/// would count the same tokens a second time.
#[sqlx::test]
async fn backfill_skips_tasks_that_already_report_usage(pool: PgPool) {
    use moatless_vibe_backend::message::models::{MessageRole, MessageType};

    seed_task(&pool, "task-1", "user-1").await;

    // First row of the response — carries the per-message usage.
    let mut first = make_message_row("task-1", "claude-assistant-1");
    first.message_type = MessageType::Assistant;
    first.message_role = MessageRole::Assistant;
    first.assistant_message_id = Some("msg_split".to_string());
    first.assistant_usage_input_tokens = Some(10);
    first.assistant_usage_output_tokens = Some(6);
    first.assistant_usage = Some(serde_json::json!({"input_tokens": 10, "output_tokens": 6}));
    message_dao::append(&pool, &first, &[], &[]).await.unwrap();

    // Repeat of the same API message after interleaved traffic — no usage.
    let mut repeat = make_message_row("task-1", "claude-assistant-2");
    repeat.message_type = MessageType::Assistant;
    repeat.message_role = MessageRole::Assistant;
    repeat.assistant_message_id = Some("msg_split".to_string());
    message_dao::append(&pool, &repeat, &[], &[]).await.unwrap();

    let updated = message_dao::backfill_last_assistant_usage(
        &pool,
        "task-1",
        Some(20),
        Some(1050),
        None,
        Some(37987),
        Some(&serde_json::json!({"input_tokens": 20, "output_tokens": 1050})),
    )
    .await
    .unwrap();
    assert!(
        !updated,
        "the task already reports per-message usage; the repeat must stay empty"
    );

    let (messages, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let repeat_row = messages
        .iter()
        .find(|m| m.uuid == "claude-assistant-2")
        .expect("repeat row present");
    assert_eq!(repeat_row.assistant_usage_output_tokens, None);
    assert_eq!(repeat_row.assistant_usage, None);
}

#[sqlx::test]
async fn append_with_contexts_and_blocks(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "msg-uuid-2");
    let contexts = vec![
        TaskMessageContext {
            id: 0,
            task_message_id: 0, // ignored, set by append
            context_type: "file".to_string(),
            reference_id: "ref-1".to_string(),
            data: serde_json::json!({"path": "/test.ts"}),
            created_at: None,
        },
        TaskMessageContext {
            id: 0,
            task_message_id: 0,
            context_type: "url".to_string(),
            reference_id: "ref-2".to_string(),
            data: serde_json::json!({"url": "https://example.com"}),
            created_at: None,
        },
    ];
    let blocks = vec![
        TaskMessageContentBlock {
            id: 0,
            task_message_id: 0,
            sort_order: 0,
            block_type: ContentBlockType::Text,
            show_in_ui: true,
            text_content: Some("Hello world".to_string()),
            tool_use_id: None,
            tool_result_content: None,
            tool_result_is_error: None,
            image_media_type: None,
            image_data: None,
            created_at: None,
        },
        TaskMessageContentBlock {
            id: 0,
            task_message_id: 0,
            sort_order: 1,
            block_type: ContentBlockType::ToolUse,
            show_in_ui: true,
            text_content: None,
            tool_use_id: Some("tu-1".to_string()),
            tool_result_content: None,
            tool_result_is_error: None,
            image_media_type: None,
            image_data: None,
            created_at: None,
        },
        TaskMessageContentBlock {
            id: 0,
            task_message_id: 0,
            sort_order: 2,
            block_type: ContentBlockType::ToolResult,
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
    assert_eq!(blk_rows[0].block_type, ContentBlockType::Text);
    assert_eq!(blk_rows[1].block_type, ContentBlockType::ToolUse);
    assert_eq!(blk_rows[2].block_type, ContentBlockType::ToolResult);
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
async fn set_session_id_also_sets_sent_at(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Two pending user messages (no agent_session_id, no sent_at)
    let msg1 = make_message_row("task-1", "sess-msg-1");
    message_dao::append(&pool, &msg1, &[], &[]).await.unwrap();

    let msg2 = make_message_row("task-1", "sess-msg-2");
    message_dao::append(&pool, &msg2, &[], &[]).await.unwrap();

    // Simulate session_init: sets session_id, agent_session_id, and sent_at
    let count = message_dao::set_session_id_for_task(&pool, "task-1", "session-abc")
        .await
        .unwrap();
    assert_eq!(count, 2);

    // Messages should no longer be pending
    let pending = message_dao::get_pending_user(&pool, "task-1")
        .await
        .unwrap();
    assert!(pending.is_empty());

    // Verify sent_at is set on all messages
    let (all_msgs, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    for msg in &all_msgs {
        assert!(
            msg.sent_at.is_some(),
            "sent_at should be set after set_session_id_for_task"
        );
        assert_eq!(msg.session_id.as_deref(), Some("session-abc"));
        assert_eq!(msg.agent_session_id.as_deref(), Some("session-abc"));
    }
}

#[sqlx::test]
async fn get_last_result(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Insert two result messages with different timestamps
    let mut msg1 = make_message_row("task-1", "result-1");
    msg1.message_type = MessageType::Result;
    msg1.timestamp = "2024-01-01T00:00:00.000Z".to_string();
    message_dao::append(&pool, &msg1, &[], &[]).await.unwrap();

    let mut msg2 = make_message_row("task-1", "result-2");
    msg2.message_type = MessageType::Result;
    msg2.timestamp = "2024-01-02T00:00:00.000Z".to_string();
    message_dao::append(&pool, &msg2, &[], &[]).await.unwrap();

    let last = message_dao::get_last_result(&pool, "task-1").await.unwrap();
    assert!(last.is_some());
    assert_eq!(last.unwrap().uuid, "result-2");
}

#[sqlx::test]
async fn get_last_assistant_message_picks_latest_assistant(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Earlier assistant row.
    let mut asst1 = make_message_row("task-1", "asst-1");
    asst1.message_type = MessageType::Assistant;
    asst1.message_role = MessageRole::Assistant;
    asst1.timestamp = "2024-01-01T00:00:00.000Z".to_string();
    asst1.message_content = serde_json::json!([{"type": "text", "text": "first"}]);
    message_dao::append(&pool, &asst1, &[], &[]).await.unwrap();

    // Result row in between — must NOT be selected.
    let mut result = make_message_row("task-1", "result-1");
    result.message_type = MessageType::Result;
    result.timestamp = "2024-01-02T00:00:00.000Z".to_string();
    message_dao::append(&pool, &result, &[], &[]).await.unwrap();

    // Most recent assistant row.
    let mut asst2 = make_message_row("task-1", "asst-2");
    asst2.message_type = MessageType::Assistant;
    asst2.message_role = MessageRole::Assistant;
    asst2.timestamp = "2024-01-03T00:00:00.000Z".to_string();
    asst2.message_content = serde_json::json!([{"type": "text", "text": "latest"}]);
    message_dao::append(&pool, &asst2, &[], &[]).await.unwrap();

    let last = message_dao::get_last_assistant_message(&pool, "task-1")
        .await
        .unwrap();
    let last = last.expect("expected an assistant row");
    assert_eq!(last.uuid, "asst-2");
    assert_eq!(last.message_type, MessageType::Assistant);
}

#[sqlx::test]
async fn get_last_assistant_message_returns_none_when_absent(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Only a non-assistant row exists.
    let mut result = make_message_row("task-1", "result-1");
    result.message_type = MessageType::Result;
    message_dao::append(&pool, &result, &[], &[]).await.unwrap();

    let last = message_dao::get_last_assistant_message(&pool, "task-1")
        .await
        .unwrap();
    assert!(last.is_none());
}

#[sqlx::test]
async fn set_channel_metadata_writes_all_columns_and_is_idempotent(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    let msg = make_message_row("task-1", "tag-msg-1");
    message_dao::append(&pool, &msg, &[], &[]).await.unwrap();

    let repo = PgMessageRepository(pool.clone());
    let updated = repo
        .set_channel_metadata(
            "tag-msg-1",
            "github:pr:opened:42",
            "github_pr",
            Some("foo/bar"),
        )
        .await
        .unwrap();
    assert!(updated);

    let (rows, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let row = rows.iter().find(|m| m.uuid == "tag-msg-1").unwrap();
    assert_eq!(
        row.channel_external_id.as_deref(),
        Some("github:pr:opened:42")
    );
    assert_eq!(row.channel_provider.as_deref(), Some("github_pr"));
    assert_eq!(row.channel_source_name.as_deref(), Some("foo/bar"));

    // Second call must be a no-op (idempotency guard on channel_external_id IS NULL).
    let again = repo
        .set_channel_metadata(
            "tag-msg-1",
            "github:pr:opened:99",
            "github_pr",
            Some("other/repo"),
        )
        .await
        .unwrap();
    assert!(!again);

    let (rows2, _, _) = message_dao::get_all(&pool, "task-1").await.unwrap();
    let row2 = rows2.iter().find(|m| m.uuid == "tag-msg-1").unwrap();
    assert_eq!(
        row2.channel_external_id.as_deref(),
        Some("github:pr:opened:42")
    );
    assert_eq!(row2.channel_source_name.as_deref(), Some("foo/bar"));
}

#[sqlx::test]
async fn find_task_id_by_any_channel_external_id_matches_globally(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let task2 = make_task_row("task-2", "user-2");
    moatless_vibe_backend::task::dao::insert_task(&pool, &task2)
        .await
        .unwrap();

    let msg1 = make_message_row("task-1", "tag-origin-1");
    message_dao::append(&pool, &msg1, &[], &[]).await.unwrap();
    let msg2 = make_message_row("task-2", "tag-origin-2");
    message_dao::append(&pool, &msg2, &[], &[]).await.unwrap();

    let repo = PgMessageRepository(pool.clone());
    repo.set_channel_metadata(
        "tag-origin-1",
        "github:ic:555",
        "github_pr",
        Some("foo/bar"),
    )
    .await
    .unwrap();
    repo.set_channel_metadata(
        "tag-origin-2",
        "github:ic:777",
        "github_pr",
        Some("foo/bar"),
    )
    .await
    .unwrap();

    let found = repo
        .find_task_id_by_any_channel_external_id(&["github:ic:555".to_string(), "555".to_string()])
        .await
        .unwrap();
    assert_eq!(found.as_deref(), Some("task-1"));
}

#[sqlx::test]
async fn find_tool_call_returns_parent_message_uuid(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;

    // Assistant row containing the tool_use block.
    let mut assistant = make_message_row("task-1", "assistant-uuid");
    assistant.message_type = MessageType::Assistant;
    assistant.message_role = MessageRole::Assistant;
    assistant.user_type = "assistant".to_string();
    assistant.message_content = serde_json::json!([
        { "type": "text", "text": "running" },
        {
            "type": "tool_use",
            "id": "tu-123",
            "name": "Bash",
            "input": { "command": "echo hi" }
        }
    ]);
    message_dao::append(&pool, &assistant, &[], &[])
        .await
        .unwrap();

    // System tool_result row (the "hidden" row in the UI).
    let mut system = make_message_row("task-1", "system-uuid");
    system.message_type = MessageType::System;
    system.message_role = MessageRole::User;
    system.user_type = "tool".to_string();
    system.message_content = serde_json::json!([
        {
            "type": "tool_result",
            "tool_use_id": "tu-123",
            "content": "hi",
            "is_error": false
        }
    ]);
    message_dao::append(&pool, &system, &[], &[]).await.unwrap();

    let repo = PgMessageRepository(pool.clone());
    let found = repo
        .find_tool_call("task-1", "tu-123")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.tool_use_id, "tu-123");
    assert_eq!(found.tool_name, "Bash");
    assert_eq!(found.parent_message_uuid, "assistant-uuid");
    assert_eq!(found.result_content.as_deref(), Some("hi"));
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

async fn seed_acknowledged_turn(pool: &PgPool, task_id: &str, uuid: &str, turn_number: i32) {
    let mut msg = make_message_row(task_id, uuid);
    msg.session_id = Some(format!("session-{turn_number}"));
    msg.agent_session_id = Some(format!("session-{turn_number}"));
    msg.turn_number = Some(turn_number);
    msg.sent_at = Some(chrono::Utc::now());
    message_dao::append(pool, &msg, &[], &[]).await.unwrap();
}

async fn seed_turn_event(
    pool: &PgPool,
    task_id: &str,
    event_type: &str,
    sub_type: &str,
    turn_number: i32,
) {
    sqlx::query(
        r#"INSERT INTO task_events
               (event_id, task_id, type, sub_type, turn_number, agent_type, payload, timestamp)
           VALUES ($1, $2, $3, $4, $5, 'claude-code', '{}'::jsonb, '')"#,
    )
    .bind(format!("{task_id}-{event_type}-{sub_type}-{turn_number}"))
    .bind(task_id)
    .bind(event_type)
    .bind(sub_type)
    .bind(turn_number)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn find_interrupted_turn_returns_acknowledged_unfinished_turn(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "interrupted-user", 1).await;

    let repo = PgMessageRepository(pool.clone());
    let interrupted = repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .expect("expected interrupted turn");

    assert_eq!(interrupted.turn_number, 1);
    assert_eq!(interrupted.session_id, "session-1");
}

#[sqlx::test]
async fn find_interrupted_turn_suppresses_terminal_agent_event(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "agent-completed-user", 1).await;
    seed_turn_event(&pool, "task-1", "agent", "completed", 1).await;

    let repo = PgMessageRepository(pool.clone());
    assert!(repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .is_none());
}

#[sqlx::test]
async fn find_interrupted_turn_suppresses_terminal_task_turn_event(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "turn-completed-user", 1).await;
    seed_turn_event(&pool, "task-1", "taskTurn", "completed", 1).await;

    let repo = PgMessageRepository(pool.clone());
    assert!(repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .is_none());
}

#[sqlx::test]
async fn find_interrupted_turn_suppresses_explicit_sandbox_stop(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "sandbox-stopped-user", 1).await;
    seed_turn_event(&pool, "task-1", "sandbox", "stopped", 1).await;

    let repo = PgMessageRepository(pool.clone());
    assert!(repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .is_none());
}

/// Build an `assistant` message row with a controlled `stop_reason`,
/// `turn_number`, and timestamp so ordering/scoping is deterministic.
fn make_assistant_row(
    task_id: &str,
    uuid: &str,
    stop_reason: Option<&str>,
    turn_number: Option<i32>,
    ts: &str,
) -> moatless_vibe_backend::message::models::TaskMessage {
    let mut msg = make_message_row(task_id, uuid);
    msg.message_type = MessageType::Assistant;
    msg.message_role = MessageRole::Assistant;
    msg.assistant_stop_reason = stop_reason.map(ToString::to_string);
    msg.turn_number = turn_number;
    msg.timestamp = ts.to_string();
    msg
}

#[sqlx::test]
async fn last_message_is_completed_reply_detects_final_assistant(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageRepository(pool.clone());

    // No messages yet — not a completed reply.
    assert!(!repo
        .last_message_is_completed_reply("task-1", Some(1))
        .await
        .unwrap());

    // A user message alone is a genuine orphan (agent never replied).
    let mut user = make_message_row("task-1", "u-1");
    user.turn_number = Some(1);
    message_dao::append(&pool, &user, &[], &[]).await.unwrap();
    assert!(!repo
        .last_message_is_completed_reply("task-1", Some(1))
        .await
        .unwrap());

    // An assistant message that finished its turn is a completed reply.
    let done = make_assistant_row(
        "task-1",
        "a-1",
        Some("end_turn"),
        Some(1),
        "2026-07-14T11:47:30.000Z",
    );
    message_dao::append(&pool, &done, &[], &[]).await.unwrap();
    assert!(repo
        .last_message_is_completed_reply("task-1", Some(1))
        .await
        .unwrap());
    // A None scope falls back to the task-wide last message.
    assert!(repo
        .last_message_is_completed_reply("task-1", None)
        .await
        .unwrap());
}

#[sqlx::test]
async fn last_message_is_completed_reply_ignores_mid_tool_interruption(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageRepository(pool.clone());

    // An assistant message that stopped to call a tool is NOT a final reply —
    // the turn was cut off mid-tool and stays a genuine orphan.
    let tool = make_assistant_row(
        "task-1",
        "a-1",
        Some("tool_use"),
        Some(1),
        "2026-07-14T11:47:30.000Z",
    );
    message_dao::append(&pool, &tool, &[], &[]).await.unwrap();
    assert!(!repo
        .last_message_is_completed_reply("task-1", Some(1))
        .await
        .unwrap());

    // An assistant message with no stop_reason is likewise not treated as final.
    let unknown = make_assistant_row("task-1", "a-2", None, Some(1), "2026-07-14T11:47:31.000Z");
    message_dao::append(&pool, &unknown, &[], &[])
        .await
        .unwrap();
    assert!(!repo
        .last_message_is_completed_reply("task-1", Some(1))
        .await
        .unwrap());
}

#[sqlx::test]
async fn last_message_is_completed_reply_is_scoped_to_turn(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageRepository(pool.clone());

    // Turn 1 was cut off mid-tool (no final reply); turn 2 later finished.
    let turn1_tool = make_assistant_row(
        "task-1",
        "a-1",
        Some("tool_use"),
        Some(1),
        "2026-07-14T11:00:00.000Z",
    );
    message_dao::append(&pool, &turn1_tool, &[], &[])
        .await
        .unwrap();
    let turn2_done = make_assistant_row(
        "task-1",
        "a-2",
        Some("end_turn"),
        Some(2),
        "2026-07-14T12:00:00.000Z",
    );
    message_dao::append(&pool, &turn2_done, &[], &[])
        .await
        .unwrap();

    // A stale open turn 1 must NOT be closed as completed on the strength of
    // turn 2's final reply.
    assert!(!repo
        .last_message_is_completed_reply("task-1", Some(1))
        .await
        .unwrap());
    // Turn 2 did finish.
    assert!(repo
        .last_message_is_completed_reply("task-1", Some(2))
        .await
        .unwrap());
}

#[sqlx::test]
async fn last_message_is_completed_reply_detects_coalesced_later_reply(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageRepository(pool.clone());

    // Turn 1 was part of the same sandbox batch, but lifecycle/result events
    // were attributed to turn 2 by current-turn resolution. Because turn 1 has
    // no substantive assistant/system/result state, the reaper can close it
    // silently as completed instead of surfacing a false orphan error.
    let mut turn1_user = make_message_row("task-1", "u-1");
    turn1_user.message_type = MessageType::User;
    turn1_user.message_role = MessageRole::User;
    turn1_user.turn_number = Some(1);
    turn1_user.timestamp = "2026-07-14T11:00:00.000Z".to_string();
    message_dao::append(&pool, &turn1_user, &[], &[])
        .await
        .unwrap();

    let mut empty_placeholder = make_assistant_row(
        "task-1",
        "a-empty",
        None,
        Some(1),
        "2026-07-14T11:00:01.000Z",
    );
    empty_placeholder.message_content = serde_json::json!([]);
    message_dao::append(&pool, &empty_placeholder, &[], &[])
        .await
        .unwrap();

    let mut turn2_user = make_message_row("task-1", "u-2");
    turn2_user.message_type = MessageType::User;
    turn2_user.message_role = MessageRole::User;
    turn2_user.turn_number = Some(2);
    turn2_user.timestamp = "2026-07-14T11:00:02.000Z".to_string();
    message_dao::append(&pool, &turn2_user, &[], &[])
        .await
        .unwrap();

    let turn2_done = make_assistant_row(
        "task-1",
        "a-2",
        Some("end_turn"),
        Some(2),
        "2026-07-14T11:01:00.000Z",
    );
    message_dao::append(&pool, &turn2_done, &[], &[])
        .await
        .unwrap();

    assert!(repo
        .last_message_is_completed_reply("task-1", Some(1))
        .await
        .unwrap());
    assert!(repo
        .last_message_is_completed_reply("task-1", Some(2))
        .await
        .unwrap());
}

#[sqlx::test]
async fn find_interrupted_turn_aggregates_sibling_user_messages(pool: PgPool) {
    // One agent run can consume several pending messages, so a turn holds all
    // of them. Recovery must replay the same joined prompt the dispatcher built,
    // not an arbitrary one of the siblings.
    seed_task(&pool, "task-1", "user-1").await;
    for (uuid, text) in [("sib-a", "first"), ("sib-b", "second")] {
        let mut msg = make_message_row("task-1", uuid);
        msg.session_id = Some("session-1".to_string());
        msg.agent_session_id = Some("session-1".to_string());
        msg.turn_number = Some(1);
        msg.sent_at = Some(chrono::Utc::now());
        msg.message_content = serde_json::json!([{ "type": "text", "text": text }]);
        message_dao::append(&pool, &msg, &[], &[]).await.unwrap();
    }

    let repo = PgMessageRepository(pool.clone());
    let interrupted = repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .expect("expected interrupted turn");

    assert_eq!(interrupted.turn_number, 1);
    assert_eq!(interrupted.prompt_text, "first\nsecond");
}

#[sqlx::test]
async fn find_interrupted_turn_reads_recovery_attempts_from_task_turns(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    seed_acknowledged_turn(&pool, "task-1", "recovering-user", 1).await;
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status, recovery_attempts) \
         VALUES ($1, 1, 'interrupted', 2)",
    )
    .bind("task-1")
    .execute(&pool)
    .await
    .unwrap();

    let repo = PgMessageRepository(pool.clone());
    let interrupted = repo
        .find_interrupted_turn("task-1")
        .await
        .unwrap()
        .expect("expected interrupted turn");

    assert_eq!(interrupted.recovery_attempts, 2);
}
