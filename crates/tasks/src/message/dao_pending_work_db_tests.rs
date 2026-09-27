//! Database tests for the pending-work signal and the resume session.

use super::*;
use crate::message::test_support::{
    acknowledge_messages, queue_message, seed_agent_line, user_item,
};
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
    seed_agent_line(&pool, "t1", "sess-1", Some(1), "hi").await;

    assert_eq!(pending_work_at(&pool, "t1").await, None);
}

/// The session a resume names comes off the newest Message that has one: the
/// delivered item's stamp until the agent's line lands, then that line's.
#[sqlx::test]
async fn the_resume_session_is_the_newest_messages(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let dao = PgMessageDao(pool.clone());
    let m1 = uuid::Uuid::new_v4();
    queue_message(&pool, user_item("t1", m1, "hello"))
        .await
        .unwrap();
    assert_eq!(dao.latest_agent_session_id("t1").await.unwrap(), None);

    acknowledge_messages(&pool, "t1", &[m1.to_string()], "sess-0", 1)
        .await
        .unwrap();
    assert_eq!(
        dao.latest_agent_session_id("t1").await.unwrap().as_deref(),
        Some("sess-0")
    );

    seed_agent_line(&pool, "t1", "sess-1", Some(1), "done").await;
    assert_eq!(
        dao.latest_agent_session_id("t1").await.unwrap().as_deref(),
        Some("sess-1")
    );
}
