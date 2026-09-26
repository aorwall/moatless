//! The migration's backfill, rerun over Messages queued and delivered before
//! the inbox existed.

use sqlx::PgPool;
use uuid::Uuid;

use crate::message::dao::{MessageDao, PgMessageDao};
use crate::message::test_support::{
    acknowledge_messages, make_message_row, messages_for, pending_user_messages, seed_message,
};
use crate::task::test_support::seed_bare_task;
use crate::task_activities::mapping::ConversationWrite;

/// `id`, `origin`, `status`, `session_id`, `turn_number`.
type InboxStamp = (Uuid, String, String, Option<String>, Option<i32>);

const MIGRATION: &str = include_str!("../../../core/migrations/0176_task_inbox.sql");

fn backfill() -> &'static str {
    let start = MIGRATION.find("-- backfill:begin").expect("the backfill");
    let end = MIGRATION
        .find("-- backfill:end")
        .expect("the backfill's end");
    &MIGRATION[start..end]
}

async fn seed_user_row(pool: &PgPool, uuid: &str, sent_by: Option<&str>, run: Option<(&str, i32)>) {
    let mut msg = make_message_row("t1", uuid);
    msg.message_text = Some(uuid.to_string());
    msg.sent_by = sent_by.map(str::to_string);
    if let Some((session, turn)) = run {
        msg.agent_session_id = Some(session.to_string());
        msg.session_id = Some(session.to_string());
        msg.turn_number = Some(turn);
        msg.sent_at = Some(chrono::Utc::now());
    }
    seed_message(pool, &msg, &[], &ConversationWrite::default())
        .await
        .unwrap();
}

async fn seed_turn(pool: &PgPool, turn: i32, status: &str) {
    sqlx::query("INSERT INTO task_turns (task_id, turn_number, status) VALUES ('t1', $1, $2)")
        .bind(turn)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn the_backfill_moves_every_queued_message_and_every_open_turns_delivery(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_turn(&pool, 1, "completed").await;
    seed_turn(&pool, 2, "active").await;
    let (closed, open, queued) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    seed_user_row(&pool, &closed.to_string(), Some("user-1"), Some(("s1", 1))).await;
    seed_user_row(&pool, &open.to_string(), Some("user-1"), Some(("s2", 2))).await;
    seed_user_row(&pool, &queued.to_string(), None, None).await;
    // The send path accepted any client id before the inbox.
    seed_user_row(&pool, "legacy-queued", Some("user-1"), None).await;

    sqlx::raw_sql(backfill()).execute(&pool).await.unwrap();

    let renamed: Uuid = sqlx::query_scalar("SELECT md5('task_inbox:t1:legacy-queued')::uuid")
        .fetch_one(&pool)
        .await
        .unwrap();
    let rows: Vec<InboxStamp> = sqlx::query_as(
        "SELECT id, origin, status, session_id, turn_number FROM task_inbox ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [
            (
                open,
                "user".into(),
                "delivered".into(),
                Some("s2".into()),
                Some(2)
            ),
            (queued, "system".into(), "pending".into(), None, None),
            (renamed, "user".into(), "pending".into(), None, None),
        ],
        "a closed turn's delivery stays history"
    );

    let recovered = PgMessageDao(pool.clone())
        .find_interrupted_turn("t1")
        .await
        .unwrap()
        .expect("the open turn is still recoverable");
    assert_eq!(
        (recovered.turn_number, recovered.session_id.as_str()),
        (2, "s2")
    );

    let (messages, _, _) = messages_for(&pool, "t1").await.unwrap();
    assert!(messages.iter().all(|m| m.uuid != "legacy-queued"));
    let changes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_conversation_changes WHERE message_uuid = $1",
    )
    .bind(renamed.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(changes, 1, "the change record follows the rename");

    assert_eq!(pending_user_messages(&pool, "t1").await.unwrap().len(), 2);
    acknowledge_messages(&pool, "t1", &[renamed.to_string()], "s3", 3)
        .await
        .unwrap();
    let (messages, _, _) = messages_for(&pool, "t1").await.unwrap();
    let delivered = messages
        .iter()
        .find(|m| m.uuid == renamed.to_string())
        .expect("the renamed Message");
    assert!(
        delivered.sent_at.is_some(),
        "delivery stamps it by the item's id"
    );
}
