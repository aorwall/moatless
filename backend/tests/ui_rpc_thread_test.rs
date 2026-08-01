#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! What a batch of conversation changes is read as: a change names the row it is
//! about, and reading exactly the named rows is complete.
//!
//! The shape that makes this worth holding is the `tool_result`. It arrives on
//! its own Message, but what it changes is the *call*, on an earlier row. A
//! result settles the activity that call already wrote, and the change is
//! recorded against the calling Message — so no reader has to widen its window
//! to a run of Messages before each result and hope the run starts early enough.
//!
//! This file holds that property against a real database, because it is a
//! property of two SQL statements: what `append_activities` records, and what
//! `changed_messages` reads back.

mod common;

use moatless_vibe_backend::message::dao as message_dao;
use moatless_vibe_backend::message::dao::{MessageDao, PgMessageDao};
use moatless_vibe_backend::message::models::MessageType;
use moatless_vibe_backend::task_activities::mapping::{
    map_content, ActivityStatus, BlockContext, ConversationWrite,
};
use moatless_vibe_backend::ui_rpc::dao::{ConversationDao, PgConversationDao};
use sqlx::PgPool;

use common::{make_message_row, make_repository_row, make_task_row, USER_1};

const TASK: &str = "task-thread-window";

async fn seed_task(pool: &PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_vibe_backend::repository::dao::insert(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row(TASK, USER_1);
    moatless_vibe_backend::task::dao::insert_task(pool, &task)
        .await
        .expect("seed task");
}

/// Map one Message's content the way the write path does.
fn write(uuid: &str, message_type: MessageType, content: &serde_json::Value) -> ConversationWrite {
    map_content(
        BlockContext {
            task_id: TASK,
            message_uuid: uuid,
            parent_uuid: None,
            message_type,
            block_index: 0,
        },
        content,
    )
}

/// Append one Message and return its row id, in the order the calls are made.
async fn append(pool: &PgPool, uuid: &str, content: serde_json::Value) -> i32 {
    let msg = make_message_row(TASK, uuid);
    let conversation = write(uuid, msg.message_type, &content);
    message_dao::append(pool, &msg, &[], &conversation)
        .await
        .unwrap()
}

fn tool_use(id: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "tool_use", "id": id, "name": "Bash", "input": {} }])
}

fn tool_result(id: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "tool_result", "tool_use_id": id, "content": "ok" }])
}

/// A result reaches the reader as a change to the row it answers.
///
/// The result carries no prose, so it is no Message of its own. It settles the
/// call's activity and records a change naming the *calling* Message — which is
/// why the reader needs no run before it, and why the rows the batch happens to
/// name alongside it no longer matter.
#[sqlx::test]
async fn a_result_is_read_as_a_change_to_the_call_it_answers(pool: PgPool) {
    seed_task(&pool).await;

    let rewritten = append(
        &pool,
        "old-text",
        serde_json::json!([{"type": "text", "text": "hi"}]),
    )
    .await;
    let call = append(&pool, "the-call", tool_use("toolu_1")).await;

    // The result arrives later, on its own source event, and settles in place.
    PgMessageDao(pool.clone())
        .append_activities(
            TASK,
            "the-call",
            &write("the-result", MessageType::User, &tool_result("toolu_1")),
        )
        .await
        .unwrap();

    let dao = PgConversationDao::new(pool.clone());

    let window = dao
        .changed_messages(TASK, &["the-call".to_string()])
        .await
        .unwrap();

    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        vec![call],
        "the change names the call, so exactly that row is read — not a run \
         reaching back over {rewritten}"
    );
    assert_eq!(window.activities.len(), 1, "the call's activity comes with it");
    assert_eq!(
        window.activities[0].status,
        ActivityStatus::Completed,
        "the result settled the activity rather than arriving as a row of its own"
    );
    assert!(
        window.activities[0].result().is_some(),
        "the outcome is on the activity the reader already has"
    );
}

/// Only the rows a batch names are read.
///
/// Every change now describes the row it names, so a batch is read as exactly
/// those rows — no lookback, and no row dragged along for context.
#[sqlx::test]
async fn a_batch_is_read_as_the_rows_it_names(pool: PgPool) {
    seed_task(&pool).await;

    append(
        &pool,
        "first",
        serde_json::json!([{"type": "text", "text": "one"}]),
    )
    .await;
    append(
        &pool,
        "second",
        serde_json::json!([{"type": "text", "text": "two"}]),
    )
    .await;
    let third = append(
        &pool,
        "third",
        serde_json::json!([{"type": "text", "text": "three"}]),
    )
    .await;

    let window = PgConversationDao::new(pool.clone())
        .changed_messages(TASK, &["third".to_string()])
        .await
        .unwrap();

    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        vec![third],
        "a change must not drag the rows before it along"
    );
}
