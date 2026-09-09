//! PostgreSQL tests for the ingress outbox: the claim schedule, the idempotent
//! admission, and the record's lifetime against its Task.

use chrono::Utc;
use sqlx::PgPool;

use crate::event_ingress::outbox_dao::{IngressOutbox, IngressPayload, PgIngressOutbox};
use crate::task::test_support::make_task_row;
use moatless_workspaces::test_support::make_repository_row;

async fn seed_task(pool: &PgPool, task_id: &str) {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .unwrap();
    let task = make_task_row(task_id, "user-1");
    crate::task::test_support::seed_task(pool, &task)
        .await
        .unwrap();
}

async fn outbox_event_ids(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT event_id FROM event_ingress_outbox ORDER BY event_id")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn claiming_reschedules_with_backoff_and_admission_is_idempotent(pool: PgPool) {
    let task_id = "task-outbox-1";
    seed_task(&pool, task_id).await;
    let outbox = PgIngressOutbox(pool.clone());
    let payload = IngressPayload {
        event_id: "evt-1".to_string(),
        task_id: task_id.to_string(),
        event_type: "agent".to_string(),
        sub_type: "message".to_string(),
        turn_number: Some(1),
        payload: serde_json::json!({ "uuid": "msg-1" }),
        timestamp: "2026-07-20T00:00:00.000Z".to_string(),
    };
    let due = Utc::now() - chrono::Duration::seconds(60);

    outbox.admit(&payload, due).await.unwrap();
    // A retried delivery of an admitted event neither duplicates the record nor
    // resets its schedule.
    outbox
        .admit(&payload, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(outbox_event_ids(&pool).await, ["evt-1"]);

    let claimed = outbox.claim_due(Utc::now(), 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempts, 0);
    assert_eq!(claimed[0].payload, payload);

    // Claiming reschedules in the same statement, so a second pass — or a
    // second replica — does not re-publish the same record.
    assert!(outbox.claim_due(Utc::now(), 10).await.unwrap().is_empty());

    let pending = outbox.pending_counts(Utc::now()).await.unwrap();
    assert_eq!(pending.total, 1);

    assert!(outbox.delete_delivered("evt-1").await.unwrap());
    assert!(!outbox.delete_delivered("evt-1").await.unwrap());
    assert!(outbox_event_ids(&pool).await.is_empty());
}

#[sqlx::test]
async fn records_die_with_their_task(pool: PgPool) {
    let task_id = "task-outbox-2";
    seed_task(&pool, task_id).await;
    PgIngressOutbox(pool.clone())
        .admit(
            &IngressPayload {
                event_id: "evt-1".to_string(),
                task_id: task_id.to_string(),
                event_type: "agent".to_string(),
                sub_type: "message".to_string(),
                turn_number: None,
                payload: serde_json::json!({}),
                timestamp: "2026-07-20T00:00:00.000Z".to_string(),
            },
            Utc::now(),
        )
        .await
        .unwrap();

    sqlx::query("DELETE FROM tasks WHERE task_id = $1")
        .bind(task_id)
        .execute(&pool)
        .await
        .unwrap();

    // The spool is transient by construction: nothing outlives its Task.
    assert!(outbox_event_ids(&pool).await.is_empty());
}
