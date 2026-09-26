//! Database tests for the trigger `acknowledge_messages` stamps on a Turn.

use super::*;
use crate::task::test_support::seed_bare_task;
use sqlx::PgPool;

async fn append(pool: &PgPool, uuid: &str, source_task_id: Option<&str>, user_type: &str) {
    let mut msg = TaskMessage::new_user_message(
        "t1".to_string(),
        uuid.to_string(),
        "hello".to_string(),
        Utc::now().to_rfc3339(),
    );
    msg.source_task_id = source_task_id.map(str::to_string);
    msg.user_type = user_type.to_string();
    PgMessageDao(pool.clone())
        .append(
            &msg,
            &[],
            &crate::task_activities::mapping::ConversationWrite::default(),
        )
        .await
        .expect("append message");
}

async fn open_turn(pool: &PgPool, turn_number: i32) {
    sqlx::query("INSERT INTO task_turns (task_id, turn_number) VALUES ('t1', $1)")
        .bind(turn_number)
        .execute(pool)
        .await
        .unwrap();
}

async fn ack(pool: &PgPool, uuid: &str, turn_number: i32) {
    PgMessageDao(pool.clone())
        .acknowledge_messages("t1", &[uuid.to_string()], "sess-1", turn_number)
        .await
        .unwrap();
}

async fn trigger_of(pool: &PgPool, turn_number: i32) -> Option<String> {
    sqlx::query_scalar(
        "SELECT triggered_by_task_id FROM task_turns WHERE task_id = 't1' AND turn_number = $1",
    )
    .bind(turn_number)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn a_message_from_another_task_stamps_its_turn(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    append(&pool, "m1", Some("parent"), "human").await;

    ack(&pool, "m1", 1).await;

    assert_eq!(trigger_of(&pool, 1).await.as_deref(), Some("parent"));
}

#[sqlx::test]
async fn a_person_joining_the_parents_turn_leaves_it_the_parents(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    append(&pool, "m1", Some("parent"), "human").await;
    append(&pool, "m2", None, "human").await;
    append(&pool, "m3", Some("other"), "human").await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "m2", 1).await;
    ack(&pool, "m3", 1).await;

    assert_eq!(trigger_of(&pool, 1).await.as_deref(), Some("parent"));
}

#[sqlx::test]
async fn a_persons_turn_is_nobodys(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    open_turn(&pool, 2).await;
    append(&pool, "m1", Some("parent"), "human").await;
    append(&pool, "m2", None, "human").await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "m2", 2).await;

    assert_eq!(trigger_of(&pool, 2).await, None);
}

#[sqlx::test]
async fn a_wake_turn_answers_to_the_turn_that_armed_it(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    open_turn(&pool, 2).await;
    append(&pool, "m1", Some("parent"), "human").await;
    append(
        &pool,
        "wake",
        None,
        moatless_types::message::HARNESS_USER_TYPE,
    )
    .await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "wake", 2).await;

    assert_eq!(trigger_of(&pool, 2).await.as_deref(), Some("parent"));
}

#[sqlx::test]
async fn a_task_joining_a_persons_turn_leaves_it_nobodys(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    append(&pool, "m1", None, "human").await;
    append(&pool, "m2", Some("parent"), "human").await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "m2", 1).await;

    assert_eq!(trigger_of(&pool, 1).await, None);
}

#[sqlx::test]
async fn a_batch_answers_to_its_earliest_message(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    append(&pool, "m1", None, "human").await;
    append(&pool, "m2", Some("parent"), "human").await;

    PgMessageDao(pool.clone())
        .acknowledge_messages("t1", &["m1".to_string(), "m2".to_string()], "sess-1", 1)
        .await
        .unwrap();

    assert_eq!(trigger_of(&pool, 1).await, None);
}
