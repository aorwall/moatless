#![cfg(feature = "db-tests")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests fail by panicking"
)]
//! DB-backed tests for Slack and Webhook inbound routing on the durable
//! Adapter delivery seam.
//!
//! These drive the seam at the highest point reachable without a live chat
//! sender: a delivery is admitted exactly as the webhook handlers admit it and
//! executed by the same handlers `main` registers. The webhook request itself
//! isn't replayed — that needs a signed Slack envelope and a live `users.info`
//! call — so only what happens either side of that call is covered here.

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use moatless_adapter_model::delivery::{AdapterDeliveryRequest, Admission};
use moatless_backend::state::AppState;

use common::{test_app_state_with_adapters, test_config};

const SLACK_KIND: &str = "slack.inbound_message";

/// The payload shape the webhook handlers write, spelled out rather than built
/// from the private struct: it is a wire contract between two processes and a
/// silent change to it strands every record the previous build admitted.
fn payload(connection_id: &str, loop_id: &str, adapter_kind: &str) -> serde_json::Value {
    serde_json::json!({
        "connection_id": connection_id,
        "loop_id": loop_id,
        "subject_kind": "slack_thread",
        "subject_key": "T1:C1:1700000000.000100",
        "destination_config": { "channel": "C1", "thread_ts": "1700000000.000100" },
        "event_type": "event_callback",
        "message": {
            "adapter_kind": adapter_kind,
            "source_id": "C1",
            "thread_id": null,
            "message_thread_id": "1700000000.000100",
            "user_id": "user-1",
            "text": "please look at this",
            "file_ids": [],
            "thread_properties": { "channel": "C1", "ts": "1700000000.000100" },
            "adapter_context": {},
            "external_event_id": "slack:ts:1700000000.000100",
            "source_task_id": null,
        },
    })
}

fn request(
    adapter: &'static str,
    kind: &'static str,
    event_id: &str,
    payload: serde_json::Value,
) -> AdapterDeliveryRequest {
    AdapterDeliveryRequest {
        adapter,
        kind,
        event_id: event_id.to_string(),
        payload,
    }
}

async fn seed_connection(pool: &PgPool, id: &str, adapter_kind: &str, account: &str) {
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope) \
         VALUES ($1, $2, 'env', $3, 'user-1', 'global')",
    )
    .bind(id)
    .bind(adapter_kind)
    .bind(account)
    .execute(pool)
    .await
    .unwrap();
}

/// `(status, attempts, last_error)` for one admitted record.
async fn record(pool: &PgPool, kind: &str, event_id: &str) -> (String, i32, Option<String>) {
    sqlx::query_as(
        "SELECT status, attempts, last_error FROM adapter_delivery WHERE kind = $1 AND event_id = $2",
    )
    .bind(kind)
    .bind(event_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn count(pool: &PgPool, kind: &str, event_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery WHERE kind = $1 AND event_id = $2")
        .bind(kind)
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Registers every provider like `main`, then runs the worker until every named
/// record is terminal or the budget runs out.
///
/// Uses the real registrar so the test also pins that each provider claims its
/// own kind, keeping two provider migrations from contending for one name.
async fn run_worker_until_terminal(
    state: &AppState,
    pool: &PgPool,
    records: &[(&str, &str)],
    budget: Duration,
) {
    let handlers = state
        .adapter_installations
        .as_ref()
        .expect("test state installs external Adapters")
        .delivery_handlers(state);

    let cancel = CancellationToken::new();
    let handle = state
        .adapter_delivery_worker
        .spawn(Arc::new(handlers), cancel.clone());

    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let mut all_terminal = true;
        for (kind, event_id) in records {
            let (status, ..) = record(pool, kind, event_id).await;
            all_terminal &= status == "succeeded" || status == "failed";
        }
        if all_terminal || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    cancel.cancel();
    handle.await.expect("worker exits cleanly on cancellation");
}

/// Verification: duplicate delivery. Slack repeats `event_id` when it retries a
/// delivery it did not see acknowledged, and the unique index — not a
/// five-minute in-process cache — is what makes the second one free.
#[sqlx::test]
async fn a_redelivered_slack_event_admits_one_unit_of_work(pool: PgPool) {
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    let work = payload("pc_slack", "tt_slack", "slack");

    assert_eq!(
        state
            .adapter_delivery
            .admit(request("slack", SLACK_KIND, "Ev0001", work.clone()))
            .await
            .unwrap(),
        Admission::Admitted
    );
    assert_eq!(
        state
            .adapter_delivery
            .admit(request("slack", SLACK_KIND, "Ev0001", work))
            .await
            .unwrap(),
        Admission::Duplicate,
        "the re-delivery is the same unit of work, not a second one"
    );

    assert_eq!(
        count(&pool, SLACK_KIND, "Ev0001").await,
        1,
        "one record means one routing run, so no second task and no second message"
    );
}

/// A trigger deleted between admission and execution ends the record on its
/// first attempt: no retry can bring the row back, and the record has to say so
/// rather than fail silently seven more times.
#[sqlx::test]
async fn work_whose_trigger_was_deleted_goes_terminal_without_burning_the_budget(pool: PgPool) {
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    seed_connection(&pool, "pc_slack", "slack", "T1").await;

    state
        .adapter_delivery
        .admit(request(
            "slack",
            SLACK_KIND,
            "Ev-gone",
            payload("pc_slack", "tt_deleted", "slack"),
        ))
        .await
        .unwrap();

    run_worker_until_terminal(
        &state,
        &pool,
        &[(SLACK_KIND, "Ev-gone")],
        Duration::from_secs(20),
    )
    .await;

    let (status, attempts, last_error) = record(&pool, SLACK_KIND, "Ev-gone").await;
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1, "a permanent failure spends no retry budget");
    let last_error = last_error.expect("a terminal record says why it was abandoned");
    assert!(
        last_error.contains("tt_deleted"),
        "the cause names the missing trigger: {last_error}"
    );
}

/// Verification: restart recovery.
///
/// Claimed records survive worker death via claim-lease expiry (works even if a
/// replaced pod returns under a new name), which is what makes acknowledging
/// the provider before routing runs safe.
#[sqlx::test]
async fn claims_left_by_a_killed_worker_are_recovered(pool: PgPool) {
    let state = test_app_state_with_adapters(pool.clone(), test_config());

    state
        .adapter_delivery
        .admit(request(
            "slack",
            SLACK_KIND,
            "Ev-restart",
            payload("pc_slack", "tt_1", "slack"),
        ))
        .await
        .unwrap();

    // The kill: the record was claimed by a replica that is never coming
    // back, long enough ago that its lease has expired.
    sqlx::query(
        "UPDATE adapter_delivery SET status = 'claimed', attempts = 1, \
         claimed_at = NOW() - INTERVAL '1 hour', claimed_by = 'dead-replica'",
    )
    .execute(&pool)
    .await
    .unwrap();

    // The connection was not seeded, so recovery is observable as the record
    // reaching a terminal state: only a record that was reclaimed and executed
    // gets there.
    run_worker_until_terminal(
        &state,
        &pool,
        &[(SLACK_KIND, "Ev-restart")],
        Duration::from_secs(20),
    )
    .await;

    let (status, _, last_error) = record(&pool, SLACK_KIND, "Ev-restart").await;
    assert_eq!(status, "failed", "the record was recovered and executed");
    assert!(
        last_error
            .unwrap_or_default()
            .contains("adapter connection"),
        "the real handler ran rather than the record being abandoned"
    );
}
