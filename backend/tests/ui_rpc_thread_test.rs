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

use moatless_backend_core::activity::mapping::{ActivityKind, ActivityStatus};
use moatless_events::SandboxEventPayload;
use moatless_t3code::dao::{ConversationDao, PgConversationDao};
use moatless_vibe_backend::message::dao as message_dao;
use moatless_vibe_backend::message::dao::{MessageDao, PgMessageDao};
use moatless_vibe_backend::message::models::MessageType;
use moatless_vibe_backend::task_activities::mapping::{
    context_window_updated, map_content, sandbox_activity, BlockContext, ConversationWrite,
};
use moatless_vibe_backend::task_activities::{changes_from, current_head};
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
/// The result carries no text, so it is no Message of its own. It settles the
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
        .changed_messages(TASK, &["the-call".to_string()], &[])
        .await
        .unwrap();

    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        vec![call],
        "the change names the call, so exactly that row is read — not a run \
         reaching back over {rewritten}"
    );
    assert_eq!(
        window.activities.len(),
        1,
        "the call's activity comes with it"
    );
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
        .changed_messages(TASK, &["third".to_string()], &[])
        .await
        .unwrap();

    let ids: Vec<i32> = window.messages.iter().map(|m| m.id).collect();
    assert_eq!(
        ids,
        vec![third],
        "a change must not drag the rows before it along"
    );
}

/// A question is stored twice: as the call it is, and as the question the panel
/// is derived from. Both rows sit in the same change, at the same block, so they
/// carry one position and one ordinal — which is what the numbering the client
/// sorts by is built from.
///
/// Held against a real database because it is a property of the column, the
/// upsert and the read together: the ordinal has to survive a redelivery of the
/// same block, and the read has to hand both rows back in written order.
#[sqlx::test]
async fn a_question_is_stored_as_a_call_and_as_the_question_it_asked(pool: PgPool) {
    seed_task(&pool).await;

    let content = serde_json::json!([
        { "type": "thinking", "thinking": "what to ask" },
        {
            "type": "tool_use",
            "id": "toolu_ask",
            "name": "AskUserQuestion",
            "input": { "questions": [{
                "question": "Which store?",
                "options": [{ "label": "Postgres" }, { "label": "Redis" }]
            }] }
        }
    ]);
    append(&pool, "the-question", content.clone()).await;
    // The same source event redelivered must change nothing about where its
    // rows sit.
    PgMessageDao(pool.clone())
        .append_activities(
            TASK,
            "the-question",
            &write("the-question", MessageType::Assistant, &content),
        )
        .await
        .unwrap();

    let transcript = PgConversationDao::new(pool.clone())
        .transcript(TASK)
        .await
        .unwrap();

    let rows: Vec<(&str, ActivityKind, i32)> = transcript
        .activities
        .iter()
        .map(|activity| {
            (
                activity.activity_id.as_str(),
                activity.kind,
                activity.ordinal,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("the-question:0", ActivityKind::Thinking, 0),
            ("the-question:1", ActivityKind::ToolCall, 1),
            (
                "the-question:1:user-input",
                ActivityKind::UserInputRequested,
                1
            ),
        ],
        "the joined row shares the call's block, and the read hands them back in \
         written order"
    );

    // All three are one Message's, so they share its change position — the
    // ordinal is what separates them.
    let positions: Vec<i64> = transcript
        .activities
        .iter()
        .map(|activity| activity.position)
        .collect();
    assert_eq!(positions[0], positions[1]);
    assert_eq!(positions[1], positions[2]);

    let asked = &transcript.activities[2];
    assert_eq!(asked.payload["requestId"], "toolu_ask");
    assert_eq!(asked.payload["questions"][0]["id"], "Which store?");

    // The call itself is still a call, with its input where it has always been.
    let call = PgConversationDao::new(pool.clone())
        .interactive_call(TASK, "toolu_ask")
        .await
        .unwrap()
        .expect("the call is stored");
    assert_eq!(call.tool_name, "AskUserQuestion");
    assert!(!call.settled, "nobody has answered it yet");
}

/// An activity that happened on no Message is named by the change, and read back
/// by that name.
///
/// A context-window reading is the live case that already ships: it is written
/// through the standalone path, and until the change named it a subscriber only
/// saw the meter move on a full re-read. The change now carries the activity id,
/// and `changed_messages` reads the row by it.
#[sqlx::test]
async fn a_standalone_activity_change_names_the_activity_and_reads_back(pool: PgPool) {
    seed_task(&pool).await;

    let reading = context_window_updated("evt-window", TASK, 1_000, Some(200_000));
    let written = PgMessageDao(pool.clone())
        .record_standalone_activity(&reading)
        .await
        .unwrap();
    assert!(written, "the first reading is a new row");

    let changes = changes_from(&pool, TASK, 0, 10).await.unwrap();
    assert_eq!(changes.len(), 1, "one change, for the one reading");
    assert_eq!(
        changes[0].activity_id.as_deref(),
        Some("evt-window"),
        "the change names the activity, not a Message"
    );
    assert!(changes[0].message_uuid.is_none());

    let window = PgConversationDao::new(pool.clone())
        .changed_messages(TASK, &[], &["evt-window".to_string()])
        .await
        .unwrap();
    assert!(window.messages.is_empty(), "it happened on no Message");
    assert_eq!(window.activities.len(), 1);
    assert_eq!(window.activities[0].activity_id, "evt-window");
    assert_eq!(
        window.activities[0].kind,
        ActivityKind::ContextWindowUpdated
    );
    assert_eq!(window.activities[0].payload["usedTokens"], 1_000);
}

/// A Sandbox status that restates the one before it writes nothing and spends no
/// position: the collapse check runs before the allocation, so a restated status
/// records no change for a subscriber to be woken by.
#[sqlx::test]
async fn a_restated_sandbox_status_records_no_change_and_spends_no_position(pool: PgPool) {
    seed_task(&pool).await;
    let dao = PgMessageDao(pool.clone());

    let ready = sandbox_activity(
        "evt-1",
        TASK,
        "ready",
        false,
        &SandboxEventPayload::default(),
    );
    assert!(
        dao.record_sandbox_activity(&ready).await.unwrap(),
        "the first status is a new row"
    );
    let after_first = current_head(&pool, TASK).await.unwrap().position;

    // The same status restated, under a new event id.
    let restated = sandbox_activity(
        "evt-2",
        TASK,
        "ready",
        false,
        &SandboxEventPayload::default(),
    );
    assert!(
        !dao.record_sandbox_activity(&restated).await.unwrap(),
        "a restated status writes no row"
    );
    let after_restate = current_head(&pool, TASK).await.unwrap().position;
    assert_eq!(
        after_first, after_restate,
        "the collapse spends no position, so it records no change"
    );

    let changes = changes_from(&pool, TASK, 0, 10).await.unwrap();
    assert_eq!(changes.len(), 1, "only the first status recorded a change");
    assert_eq!(changes[0].activity_id.as_deref(), Some("evt-1"));
}
