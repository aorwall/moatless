#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for `event_handler::agent_handlers::handle_agent_message`
//! → `message::hooks::on_agent_message_persisted`.
//!
//! Covers:
//! - `gh pr create` tool result auto-connects a `channel_threads` row.
//! - `gh pr comment` tool result tags the persisted tool_result row's
//!   `channel_external_id` with the comment id.

mod common;

use moatless_vibe_backend::event_handler::agent_handlers::handle_agent_message;
use moatless_vibe_backend::event_handler::models::AgentEvent;
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::task::dao as task_dao;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row, test_app_state};

/// Build an `AgentEvent` for an SDK assistant or user message with the
/// given content array.
fn agent_message_event(
    event_id: &str,
    task_id: &str,
    role: &str,
    uuid: &str,
    content: serde_json::Value,
) -> AgentEvent {
    let payload = serde_json::json!({
        "uuid": uuid,
        "session_id": "session-1",
        "message": {
            "role": role,
            "content": content,
        },
    });
    let value = serde_json::json!({
        "eventId": event_id,
        "type": "agent",
        "subType": "message",
        "taskId": task_id,
        "timestamp": "2026-04-30T00:00:00.000Z",
        "isError": false,
        "agentType": "claude-code",
        "turnNumber": 1,
        "payload": payload,
    });
    serde_json::from_value(value).unwrap()
}

/// Seed a repo + task and return an `AppState` wired to the pool.
async fn seed(pool: &PgPool, task_id: &str) -> moatless_vibe_backend::state::AppState {
    let repo = make_repository_row("repo-1", "user-1");
    repository_dao::insert(pool, &repo).await.unwrap();
    let task = make_task_row(task_id, "user-1");
    task_dao::insert_task(pool, &task).await.unwrap();
    test_app_state(pool.clone())
}

#[sqlx::test]
async fn gh_pr_create_auto_connects_channel_thread(pool: PgPool) {
    let task_id = "task-pr-create";
    let state = seed(&pool, task_id).await;

    // 1. Persist the assistant tool_use(Bash, "gh pr create ...") message.
    let assistant_event = agent_message_event(
        "evt-1",
        task_id,
        "assistant",
        "asst-uuid-1",
        serde_json::json!([
            {
                "type": "tool_use",
                "id": "tu-1",
                "name": "Bash",
                "input": {
                    "command": "gh pr create --title \"foo\" --body \"bar\"",
                },
            }
        ]),
    );
    handle_agent_message(&state, &assistant_event).await;

    // 2. Persist the matching tool_result with the PR URL in stdout.
    let result_event = agent_message_event(
        "evt-2",
        task_id,
        "user",
        "tr-uuid-1",
        serde_json::json!([
            {
                "type": "tool_result",
                "tool_use_id": "tu-1",
                "content": "Creating pull request for branch...\nhttps://github.com/foo/bar/pull/99\n",
                "is_error": false,
            }
        ]),
    );
    handle_agent_message(&state, &result_event).await;

    // 3. Verify a channel_threads row was created for this task with the
    //    parsed external_thread_id.
    let threads = state.channel_threads.find_by_task_id(task_id).await.unwrap();
    assert_eq!(threads.len(), 1, "expected exactly one connected thread");
    assert_eq!(threads[0].external_thread_id, "99");
    assert_eq!(threads[0].task_id, task_id);
}

#[sqlx::test]
async fn gh_pr_comment_tags_tool_result_with_external_id(pool: PgPool) {
    let task_id = "task-pr-comment";
    let state = seed(&pool, task_id).await;

    let assistant_event = agent_message_event(
        "evt-c1",
        task_id,
        "assistant",
        "asst-uuid-c1",
        serde_json::json!([
            {
                "type": "tool_use",
                "id": "tu-c1",
                "name": "Bash",
                "input": {
                    "command": "gh pr comment 42 --body \"agent reply\"",
                },
            }
        ]),
    );
    handle_agent_message(&state, &assistant_event).await;

    let result_uuid = "tr-uuid-c1";
    let result_event = agent_message_event(
        "evt-c2",
        task_id,
        "user",
        result_uuid,
        serde_json::json!([
            {
                "type": "tool_result",
                "tool_use_id": "tu-c1",
                "content": "https://github.com/foo/bar/pull/42#issuecomment-555\n",
                "is_error": false,
            }
        ]),
    );
    handle_agent_message(&state, &result_event).await;

    // The persisted tool_result row should now carry channel_external_id="555".
    let has = state
        .messages
        .has_channel_external_id(task_id, "555")
        .await
        .unwrap();
    assert!(has, "tool_result row should be tagged with comment id");
}
