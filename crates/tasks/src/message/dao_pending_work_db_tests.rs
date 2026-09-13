//! Database tests for the Sandbox wake signal a Message raises and a claim
//! clears, and for the fork anchor read off the rows ingest writes.

use super::*;
use crate::task::test_support::seed_bare_task;
use sqlx::PgPool;

async fn append_user_message(pool: &PgPool, task_id: &str, uuid: &str) {
    PgMessageDao(pool.clone())
        .append(
            &TaskMessage::new_user_message(
                task_id.to_string(),
                uuid.to_string(),
                "hello".to_string(),
                Utc::now().to_rfc3339(),
            ),
            &[],
            &crate::task_activities::mapping::ConversationWrite::default(),
        )
        .await
        .expect("append message");
}

async fn pending_work_at(pool: &PgPool, sandbox_id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT pending_work_at FROM sandboxes WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .expect("read pending_work_at")
}

/// The wake signal is a fact on the Sandbox row, so the reconciler can see that
/// work is waiting while the pod is down. A stopped Sandbox is exactly that
/// case, and the one the signal exists for.
#[sqlx::test]
async fn a_user_message_queues_work_on_a_stopped_sandbox(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    sqlx::query("UPDATE sandboxes SET desired_state = 'stopped' WHERE sandbox_id = 't1'")
        .execute(&pool)
        .await
        .unwrap();

    append_user_message(&pool, "t1", "m1").await;

    assert!(
        pending_work_at(&pool, "t1").await.is_some(),
        "a message written to a stopped Sandbox still queues its wake"
    );
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

/// Claiming the last unacknowledged Message settles the Sandbox.
#[sqlx::test]
async fn claiming_every_message_clears_the_signal(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    append_user_message(&pool, "t1", "m1").await;
    assert!(pending_work_at(&pool, "t1").await.is_some());

    PgMessageDao(pool.clone())
        .set_session_id_for_task("t1", "sess-1")
        .await
        .unwrap();

    assert_eq!(
        pending_work_at(&pool, "t1").await,
        None,
        "nothing is waiting any more"
    );
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
    append_user_message(&pool, "t1", "m1").await;

    let dao = PgMessageDao(pool.clone());
    dao.acknowledge_messages("t1", &["m1".to_string()], "sess-1", 1)
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

/// An early clear drops work: acknowledging one Message must leave the signal
/// up while another is still unacknowledged.
#[sqlx::test]
async fn acknowledging_one_of_two_keeps_the_signal(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    append_user_message(&pool, "t1", "m1").await;
    append_user_message(&pool, "t1", "m2").await;

    PgMessageDao(pool.clone())
        .acknowledge_messages("t1", &["m1".to_string()], "sess-1", 1)
        .await
        .unwrap();

    assert!(
        pending_work_at(&pool, "t1").await.is_some(),
        "m2 is still waiting"
    );

    PgMessageDao(pool.clone())
        .acknowledge_messages("t1", &["m2".to_string()], "sess-1", 1)
        .await
        .unwrap();

    assert_eq!(pending_work_at(&pool, "t1").await, None);
}

/// The signal belongs to the Sandbox, not the Task: a peer member's
/// unacknowledged Message keeps it up.
#[sqlx::test]
async fn a_peer_members_message_keeps_the_signal(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_bare_task(&pool, "t2", "open").await;
    // t2 joins t1's Sandbox; its own row goes with the fixture's default.
    sqlx::query("UPDATE tasks SET sandbox_id = 't1' WHERE task_id = 't2'")
        .execute(&pool)
        .await
        .unwrap();

    append_user_message(&pool, "t1", "m1").await;
    append_user_message(&pool, "t2", "m2").await;
    assert!(pending_work_at(&pool, "t1").await.is_some());

    PgMessageDao(pool.clone())
        .set_session_id_for_task("t1", "sess-1")
        .await
        .unwrap();

    assert!(
        pending_work_at(&pool, "t1").await.is_some(),
        "t2's message is still waiting on the shared Sandbox"
    );

    PgMessageDao(pool.clone())
        .set_session_id_for_task("t2", "sess-1")
        .await
        .unwrap();

    assert_eq!(pending_work_at(&pool, "t1").await, None);
}
