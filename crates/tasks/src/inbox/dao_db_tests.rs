//! Database tests for the inbox: its per-origin shapes, its idempotence, the
//! Sandbox wake signal it raises and settles, and what the Projector writes.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use crate::message::test_support::{
    acknowledge_messages, deliverable_user_messages, messages_for, pending_user_messages,
    queue_message, user_item,
};
use crate::task::test_support::seed_bare_task;

fn adapter_item(task_id: &str, id: Uuid, external_id: &str) -> NewInboxItem {
    NewInboxItem {
        origin: InboxOrigin::Adapter {
            channel: AdapterChannel {
                adapter: "slack".to_string(),
                external_message_id: external_id.to_string(),
                subject_key: Some("T1:C1:1.0".to_string()),
                source_name: None,
            },
            sender: ExternalSender {
                id: Some("U1".to_string()),
                author: Some("ada".to_string()),
            },
            sent_by: None,
        },
        ..user_item(task_id, id, "from slack")
    }
}

async fn pending_work_at(pool: &PgPool, sandbox_id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT pending_work_at FROM sandboxes WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .expect("read pending_work_at")
}

/// Inserts a row by hand, bypassing [`InboxOrigin`], so each CHECK is the only
/// thing standing between it and the table.
async fn raw_insert(pool: &PgPool, columns: &str, values: &str) -> Result<(), sqlx::Error> {
    // The columns and values are this file's own literals.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO task_inbox (id, task_id, text, {columns}) \
         VALUES (gen_random_uuid(), 't1', 'x', {values})"
    )))
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test]
async fn each_origin_refuses_another_origins_sender(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let refused = [
        ("user without a sender", "origin", "'user'"),
        (
            "user naming a Task",
            "origin, sent_by, source_task_id",
            "'user', 'u', 'p'",
        ),
        ("task without its Task", "origin", "'task'"),
        (
            "task through an Adapter",
            "origin, source_task_id, adapter, external_message_id",
            "'task', 'p', 'slack', 'e1'",
        ),
        (
            "adapter without its message id",
            "origin, adapter",
            "'adapter', 'slack'",
        ),
        (
            "adapter naming a Task",
            "origin, adapter, external_message_id, source_task_id",
            "'adapter', 'slack', 'e1', 'p'",
        ),
        ("system with a sender", "origin, sent_by", "'system', 'u'"),
        (
            "sandbox with an external author",
            "origin, external_author",
            "'sandbox', 'ada'",
        ),
        ("unknown still pending", "origin", "'unknown'"),
        (
            "a subject without an Adapter",
            "origin, sent_by, external_subject_key",
            "'user', 'u', 'k'",
        ),
        (
            "delivered with no time",
            "origin, status",
            "'system', 'delivered'",
        ),
        (
            "a pending item stamped with a turn",
            "origin, turn_number",
            "'system', 3",
        ),
    ];
    for (case, columns, values) in refused {
        let result = raw_insert(&pool, columns, values).await;
        assert!(result.is_err(), "{case} was stored");
    }

    for (columns, values) in [
        ("origin, sent_by", "'user', 'u'"),
        ("origin, source_task_id", "'task', 'p'"),
        (
            "origin, adapter, external_message_id, sent_by",
            "'adapter', 'slack', 'e1', 'u'",
        ),
        ("origin", "'system'"),
        ("origin", "'sandbox'"),
        (
            "origin, status, delivered_at, turn_number",
            "'unknown', 'delivered', now(), 1",
        ),
    ] {
        raw_insert(&pool, columns, values)
            .await
            .unwrap_or_else(|e| panic!("{values} was refused: {e}"));
    }
}

#[sqlx::test]
async fn an_adapter_message_is_stored_once_per_task(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_bare_task(&pool, "t2", "open").await;

    let first = queue_message(&pool, adapter_item("t1", Uuid::new_v4(), "slack:1"))
        .await
        .unwrap();
    assert!(first.is_some());
    // A re-sync hands the same event over under a fresh id.
    let resync = queue_message(&pool, adapter_item("t1", Uuid::new_v4(), "slack:1"))
        .await
        .unwrap();
    assert!(resync.is_none(), "the re-synced event is a redelivery");
    let elsewhere = queue_message(&pool, adapter_item("t2", Uuid::new_v4(), "slack:1"))
        .await
        .unwrap();
    assert!(elsewhere.is_some(), "another Task may hold the same event");

    assert_eq!(pending_user_messages(&pool, "t1").await.unwrap().len(), 1);
}

#[sqlx::test]
async fn a_redelivered_item_inserts_nothing_and_raises_no_wake(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let id = Uuid::new_v4();
    queue_message(&pool, user_item("t1", id, "hello"))
        .await
        .unwrap()
        .expect("a new item");
    acknowledge_messages(&pool, "t1", &[id.to_string()], "sess-1", 1)
        .await
        .unwrap();
    assert_eq!(pending_work_at(&pool, "t1").await, None);

    let again = queue_message(&pool, user_item("t1", id, "hello"))
        .await
        .unwrap();

    assert!(again.is_none());
    assert_eq!(pending_work_at(&pool, "t1").await, None, "nobody is woken");
    let (messages, _, _) = messages_for(&pool, "t1").await.unwrap();
    assert_eq!(messages.len(), 1, "the Message is written once");
}

/// The wake signal is a fact on the Sandbox row, so the reconciler sees waiting
/// work while the pod is down.
#[sqlx::test]
async fn an_item_queues_work_on_a_stopped_sandbox(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    sqlx::query("UPDATE sandboxes SET desired_state = 'stopped' WHERE sandbox_id = 't1'")
        .execute(&pool)
        .await
        .unwrap();

    queue_message(&pool, user_item("t1", Uuid::new_v4(), "hello"))
        .await
        .unwrap();

    assert!(pending_work_at(&pool, "t1").await.is_some());
}

/// An early clear drops work: delivering one item leaves the signal up while
/// another, on this Task or a peer sharing its Sandbox, is still pending.
#[sqlx::test]
async fn the_signal_clears_only_once_every_member_is_delivered(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_bare_task(&pool, "t2", "open").await;
    sqlx::query("UPDATE tasks SET sandbox_id = 't1' WHERE task_id = 't2'")
        .execute(&pool)
        .await
        .unwrap();
    let (m1, m2, m3) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    for (task, id) in [("t1", m1), ("t1", m2), ("t2", m3)] {
        queue_message(&pool, user_item(task, id, "hello"))
            .await
            .unwrap();
    }
    let awaiting = || dao::members_awaiting_dispatch(&pool, "t1");
    assert_eq!(awaiting().await.unwrap(), ["t1", "t2"]);

    acknowledge_messages(&pool, "t1", &[m1.to_string()], "s", 1)
        .await
        .unwrap();
    assert!(pending_work_at(&pool, "t1").await.is_some(), "m2 waits");
    acknowledge_messages(&pool, "t1", &[m2.to_string()], "s", 1)
        .await
        .unwrap();
    assert!(
        pending_work_at(&pool, "t1").await.is_some(),
        "t2's m3 waits"
    );
    assert_eq!(awaiting().await.unwrap(), ["t2"]);

    acknowledge_messages(&pool, "t2", &[m3.to_string()], "s", 1)
        .await
        .unwrap();
    assert_eq!(pending_work_at(&pool, "t1").await, None);
    assert!(awaiting().await.unwrap().is_empty());
}

#[sqlx::test]
async fn pending_reads_oldest_first_up_to_its_limit(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let ids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
    for (n, id) in ids.iter().enumerate() {
        queue_message(&pool, user_item("t1", *id, &n.to_string()))
            .await
            .unwrap();
    }

    let pending = InboxService::new(pool.clone())
        .pending("t1", 2, false)
        .await
        .unwrap();

    let texts: Vec<&str> = pending.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, ["0", "1"]);
}

/// The Projector writes a person's item as their Message and a Sandbox wake as
/// nothing; delivery stamps the Message without moving it.
#[sqlx::test]
async fn the_projector_shows_what_was_said_and_nothing_the_sandbox_wrote(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let said = Uuid::new_v4();
    let wake = Uuid::new_v4();
    queue_message(&pool, user_item("t1", said, "hello"))
        .await
        .unwrap();
    queue_message(
        &pool,
        NewInboxItem {
            origin: InboxOrigin::Sandbox,
            ..user_item("t1", wake, "background command finished")
        },
    )
    .await
    .unwrap();

    let (messages, _, _) = messages_for(&pool, "t1").await.unwrap();
    let uuids: Vec<&str> = messages.iter().map(|m| m.uuid.as_str()).collect();
    assert_eq!(uuids, [said.to_string()]);
    assert_eq!(messages[0].sent_by.as_deref(), Some("user-1"));
    assert_eq!(
        pending_user_messages(&pool, "t1").await.unwrap().len(),
        2,
        "the wake is still pending work"
    );

    acknowledge_messages(&pool, "t1", &[said.to_string(), wake.to_string()], "s", 4)
        .await
        .unwrap();
    let (messages, _, _) = messages_for(&pool, "t1").await.unwrap();
    assert_eq!(messages[0].turn_number, Some(4));
    assert_eq!(messages[0].agent_session_id.as_deref(), Some("s"));
    let wake_turn: Option<i32> =
        sqlx::query_scalar("SELECT turn_number FROM task_inbox WHERE id = $1")
            .bind(wake)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(wake_turn, Some(4));
}

/// A mid-turn fetch leaves a `queue` item pending and still takes what was
/// sent to steer; steering the queued one makes the next mid-turn fetch take it.
#[sqlx::test]
async fn a_queued_item_waits_out_the_turn_until_it_is_steered(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let queued = Uuid::new_v4();
    let mut item = user_item("t1", queued, "after this turn");
    item.options.queue = true;
    queue_message(&pool, item).await.unwrap();
    queue_message(&pool, user_item("t1", Uuid::new_v4(), "right now"))
        .await
        .unwrap();
    let texts = |messages: Vec<crate::message::models::TaskMessage>| {
        messages
            .into_iter()
            .map(|m| m.message_text.unwrap_or_default())
            .collect::<Vec<_>>()
    };

    let mid_turn = deliverable_user_messages(&pool, "t1", true).await.unwrap();
    assert_eq!(texts(mid_turn), ["right now"]);
    let between_turns = deliverable_user_messages(&pool, "t1", false).await.unwrap();
    assert_eq!(texts(between_turns), ["after this turn", "right now"]);

    let inbox = InboxService::new(pool.clone());
    assert!(inbox.steer("t1", queued).await.unwrap());
    let mid_turn = deliverable_user_messages(&pool, "t1", true).await.unwrap();
    assert_eq!(texts(mid_turn), ["after this turn", "right now"]);
    assert!(
        !inbox.steer("t2", queued).await.unwrap(),
        "another Task's id steers nothing"
    );
}

#[sqlx::test]
async fn disposing_a_subtree_leaves_what_it_sent_to_other_inboxes(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_bare_task(&pool, "t2", "open").await;
    queue_message(&pool, user_item("t1", Uuid::new_v4(), "to the subtree"))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO task_inbox (id, task_id, text, origin, source_task_id) \
         VALUES (gen_random_uuid(), 't2', 'from the subtree', 'task', 't1')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let cancelled = dao::dispose_pending(&mut tx, &["t1".to_string()])
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(cancelled, 1);
    let statuses: Vec<(String, String)> =
        sqlx::query_as("SELECT task_id, status FROM task_inbox ORDER BY task_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        statuses,
        [
            ("t1".to_string(), "cancelled".to_string()),
            ("t2".to_string(), "pending".to_string()),
        ]
    );
}
