//! Database tests for the trigger `mark_delivered` decides for a Turn.

use sqlx::PgPool;
use uuid::Uuid;

use crate::inbox::{DeliveryOptions, InboxOrigin, NewInboxItem};
use crate::message::test_support::{acknowledge_messages, queue_message};
use crate::task::test_support::seed_bare_task;

fn id(name: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes())
}

async fn append(pool: &PgPool, name: &str, source_task_id: Option<&str>, origin: &str) {
    let origin = match (origin, source_task_id) {
        ("sandbox", _) => InboxOrigin::Sandbox,
        (_, Some(source)) => InboxOrigin::Task {
            source_task_id: source.to_string(),
            sent_by: None,
        },
        (_, None) => InboxOrigin::User {
            sent_by: "user-1".to_string(),
        },
    };
    queue_message(
        pool,
        NewInboxItem {
            id: id(name),
            task_id: "t1".to_string(),
            text: "hello".to_string(),
            file_ids: Vec::new(),
            options: DeliveryOptions::default(),
            answers_tool_use_id: None,
            origin,
        },
    )
    .await
    .expect("queue an item")
    .expect("a new item");
}

async fn open_turn(pool: &PgPool, turn_number: i32) {
    sqlx::query("INSERT INTO task_turns (task_id, turn_number) VALUES ('t1', $1)")
        .bind(turn_number)
        .execute(pool)
        .await
        .unwrap();
}

async fn ack_all(pool: &PgPool, names: &[&str], turn_number: i32) -> u64 {
    let ids: Vec<String> = names.iter().map(|name| id(name).to_string()).collect();
    acknowledge_messages(pool, "t1", &ids, "sess-1", turn_number)
        .await
        .unwrap()
}

async fn ack(pool: &PgPool, name: &str, turn_number: i32) {
    ack_all(pool, &[name], turn_number).await;
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
    append(&pool, "m1", Some("parent"), "user").await;

    ack(&pool, "m1", 1).await;

    assert_eq!(trigger_of(&pool, 1).await.as_deref(), Some("parent"));
}

#[sqlx::test]
async fn a_person_joining_the_parents_turn_leaves_it_the_parents(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    append(&pool, "m1", Some("parent"), "user").await;
    append(&pool, "m2", None, "user").await;
    append(&pool, "m3", Some("other"), "user").await;

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
    append(&pool, "m1", Some("parent"), "user").await;
    append(&pool, "m2", None, "user").await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "m2", 2).await;

    assert_eq!(trigger_of(&pool, 2).await, None);
}

#[sqlx::test]
async fn a_wake_turn_answers_to_the_turn_that_armed_it(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    open_turn(&pool, 2).await;
    append(&pool, "m1", Some("parent"), "user").await;
    append(&pool, "wake", None, "sandbox").await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "wake", 2).await;

    assert_eq!(trigger_of(&pool, 2).await.as_deref(), Some("parent"));
}

#[sqlx::test]
async fn a_task_joining_a_persons_turn_leaves_it_nobodys(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    append(&pool, "m1", None, "user").await;
    append(&pool, "m2", Some("parent"), "user").await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "m2", 1).await;

    assert_eq!(trigger_of(&pool, 1).await, None);
}

#[sqlx::test]
async fn a_batch_answers_to_its_earliest_message(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    append(&pool, "m1", None, "user").await;
    append(&pool, "m2", Some("parent"), "user").await;

    ack_all(&pool, &["m1", "m2"], 1).await;

    assert_eq!(trigger_of(&pool, 1).await, None);
}

#[sqlx::test]
async fn a_wake_after_a_persons_turn_is_nobodys(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    open_turn(&pool, 2).await;
    append(&pool, "m1", None, "user").await;
    append(&pool, "wake", None, "sandbox").await;

    ack(&pool, "m1", 1).await;
    ack(&pool, "wake", 2).await;

    assert_eq!(trigger_of(&pool, 2).await, None);
}

#[sqlx::test]
async fn a_repeated_acknowledgement_changes_nothing(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    open_turn(&pool, 1).await;
    open_turn(&pool, 2).await;
    append(&pool, "m1", Some("parent"), "user").await;
    assert_eq!(ack_all(&pool, &["m1"], 1).await, 1);
    let stamp = || async {
        sqlx::query_as::<_, (Option<i32>, Option<chrono::DateTime<chrono::Utc>>)>(
            "SELECT turn_number, delivered_at FROM task_inbox WHERE id = $1",
        )
        .bind(id("m1"))
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let first = stamp().await;

    assert_eq!(ack_all(&pool, &["m1"], 2).await, 0, "nothing is pending");

    assert_eq!(stamp().await, first);
    assert_eq!(trigger_of(&pool, 1).await.as_deref(), Some("parent"));
    assert_eq!(trigger_of(&pool, 2).await, None, "turn 2 took no item");
}

#[sqlx::test]
async fn a_trigger_is_only_recorded_once_decided(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let refused = sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, triggered_by_task_id) VALUES ('t1', 1, 'p')",
    )
    .execute(&pool)
    .await;
    assert!(refused.is_err(), "an undecided turn names no trigger");
}
