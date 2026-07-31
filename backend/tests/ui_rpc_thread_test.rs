#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! What a batch of conversation changes is read alongside, and why it is not
//! one run of Messages.
//!
//! A change names a Message and nothing else, so the reader re-reads the row at
//! delivery time. That is enough for every Message that stands on its own, and
//! not enough for the one shape that does not: a `tool_result` is a change to
//! the *call* it answers, which lives on an earlier row that nothing rewrites.
//! Resolving it means reading a short run of Messages before the result — and
//! the whole question this file settles is where that run starts.
//!
//! It is not observable without a real database. The window is a single SQL
//! statement, the failure is that statement returning one row too few, and a
//! change that resolves against nothing projects to no events at all: no error,
//! no log, just a tool card left running until the conversation is read again.

mod common;

use moatless_vibe_backend::message::dao as message_dao;
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

/// Append one Message and return its row id, in the order the calls are made.
async fn append(pool: &PgPool, uuid: &str, content: serde_json::Value) -> i32 {
    let mut msg = make_message_row(TASK, uuid);
    msg.message_content = content;
    message_dao::append(pool, &msg, &[], &[]).await.unwrap()
}

fn tool_use(id: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "tool_use", "id": id, "name": "Bash", "input": {} }])
}

fn tool_result(id: &str) -> serde_json::Value {
    serde_json::json!([{ "type": "tool_result", "tool_use_id": id, "content": "ok" }])
}

/// The run before a result is taken from that result, not from the batch.
///
/// Positions are allocated in the order changes are *recorded*, so a batch is
/// contiguous in position — never in Message id. Rewriting a Message the
/// conversation has already moved past records a change naming an old row, and
/// one of those landing in the same poll as a newer result puts the call that
/// result answers between the two: inside the batch's span, but below the
/// newest row and above the oldest. A single run taken from the batch's oldest
/// row steps straight over it.
#[sqlx::test]
async fn a_result_is_read_with_its_call_however_old_the_rest_of_the_batch_is(pool: PgPool) {
    seed_task(&pool).await;

    let rewritten = append(&pool, "old-text", serde_json::json!([{"type": "text", "text": "hi"}])).await;
    let call = append(&pool, "the-call", tool_use("toolu_1")).await;
    let result = append(&pool, "the-result", tool_result("toolu_1")).await;

    let dao = PgConversationDao::new(pool.clone());

    // Alone, the result's own run reaches back over the call.
    let window = dao
        .changed_messages(TASK, &["the-result".to_string()])
        .await
        .unwrap();
    assert!(
        window.messages.iter().any(|m| m.id == call),
        "a result read on its own must be read with the call it answers"
    );

    // Batched with a change naming a Message *older* than the call, which is
    // what a rewrite of an earlier row produces.
    let window = dao
        .changed_messages(TASK, &["the-result".to_string(), "old-text".to_string()])
        .await
        .unwrap();
    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert!(
        ids.contains(&call),
        "the call at {call} sits between the batch's oldest row {rewritten} and its \
         result {result}; a run taken from the batch rather than from the result \
         misses it. Window was {ids:?}"
    );
}

/// Only the rows that answer something pay for a run.
///
/// The run exists for `tool_result` blocks and nothing else — every other
/// change projects from the row it names alone. A batch carrying no results is
/// therefore read as exactly the rows it names, which is what keeps a run per
/// changed row down to a run per answered call.
#[sqlx::test]
async fn a_batch_that_answers_nothing_is_read_as_the_rows_it_names(pool: PgPool) {
    seed_task(&pool).await;

    append(&pool, "first", serde_json::json!([{"type": "text", "text": "one"}])).await;
    append(&pool, "second", serde_json::json!([{"type": "text", "text": "two"}])).await;
    let third = append(&pool, "third", serde_json::json!([{"type": "text", "text": "three"}])).await;

    let window = PgConversationDao::new(pool.clone())
        .changed_messages(TASK, &["third".to_string()])
        .await
        .unwrap();

    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        vec![third],
        "a change with no result to resolve must not drag the rows before it along"
    );
}
