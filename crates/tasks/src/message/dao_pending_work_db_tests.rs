//! Database tests for the fork anchor read off the rows ingest writes.

use super::*;
use crate::message::test_support::{acknowledge_messages, queue_message, user_item};
use crate::task::test_support::seed_bare_task;
use sqlx::PgPool;

async fn pending_work_at(pool: &PgPool, sandbox_id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT pending_work_at FROM sandboxes WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .expect("read pending_work_at")
}

/// An agent Message is not work waiting on the pod.
#[sqlx::test]
async fn an_assistant_message_queues_nothing(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let mut msg = TaskMessage::new_user_message(
        "t1".to_string(),
        "m1".to_string(),
        "hi".to_string(),
        Utc::now().to_rfc3339(),
    );
    msg.message_type = MessageType::Assistant;
    PgMessageDao(pool.clone())
        .append(
            &msg,
            &[],
            &crate::task_activities::mapping::ConversationWrite::default(),
        )
        .await
        .unwrap();

    assert_eq!(pending_work_at(&pool, "t1").await, None);
}

/// Seeds like ingest, not a hand-stamped fixture, so a fork anchor keyed on the
/// wrong column silently starts fresh.
///
/// `agent_session_id` is set only by
/// `set_session_id_for_task`/`acknowledge_messages` on `user`/`error` rows, so
/// an assistant row's session lives in `session_id` alone.
#[sqlx::test]
async fn the_fork_anchor_comes_off_the_assistant_row_ingest_actually_writes(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let m1 = uuid::Uuid::new_v4();
    queue_message(&pool, user_item("t1", m1, "hello"))
        .await
        .unwrap();

    let dao = PgMessageDao(pool.clone());
    acknowledge_messages(&pool, "t1", &[m1.to_string()], "sess-1", 1)
        .await
        .unwrap();

    // The assistant row as ingest builds it: its uuid is the Anthropic
    // message id, the session lands in `session_id`, and nothing sets
    // `agent_session_id`.
    let mut assistant = TaskMessage::new_user_message(
        "t1".to_string(),
        "msg_01ANCHOR".to_string(),
        "done".to_string(),
        Utc::now().to_rfc3339(),
    );
    assistant.message_type = MessageType::Assistant;
    assistant.session_id = Some("sess-1".to_string());
    assistant.agent_session_id = None;
    assistant.turn_number = Some(1);
    dao.append(
        &assistant,
        &[],
        &crate::task_activities::mapping::ConversationWrite::default(),
    )
    .await
    .unwrap();

    let anchors = dao
        .fork_anchors("t1", Some(1))
        .await
        .expect("anchor query runs")
        .expect("a forkable turn resolves an anchor");

    assert_eq!(anchors.session_id, "sess-1");
    assert_eq!(
        anchors.message_uuid.as_deref(),
        Some("msg_01ANCHOR"),
        "the anchor is the assistant message id the transcript repeats as message.id"
    );
}
