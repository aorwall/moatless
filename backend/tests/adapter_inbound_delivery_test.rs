#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! DB-backed tests for Slack and Telegram inbound routing on the durable
//! Adapter delivery seam.
//!
//! These drive the seam at the highest point reachable without a live chat
//! sender: a delivery is admitted exactly as the webhook handlers admit it —
//! same Adapter name, same kind, same event id, same payload shape — and
//! executed by the same handlers `main` registers. The webhook request itself is not
//! replayed because that would need a signed Slack envelope and a live Slack
//! API for `users.info`; what happens either side of that call is what this
//! ticket changed, and that is what is covered here.

mod common;

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use moatless_vibe_backend::adapters::delivery::{
    spawn_adapter_delivery_worker, AdapterDeliveryHandlers, AdapterDeliveryRequest, Admission,
};
use moatless_vibe_backend::state::AppState;

use common::test_app_state;

const SLACK_KIND: &str = "slack.inbound_message";
const TELEGRAM_KIND: &str = "telegram.inbound_message";

/// The payload shape the webhook handlers write, spelled out rather than built
/// from the private struct: it is a wire contract between two processes and a
/// silent change to it strands every record the previous build admitted.
fn payload(connection_id: &str, rule_id: &str, provider: &str) -> serde_json::Value {
    serde_json::json!({
        "connection_id": connection_id,
        "rule_id": rule_id,
        "subject_kind": "slack_thread",
        "subject_key": "T1:C1:1700000000.000100",
        "destination_config": { "channel": "C1", "thread_ts": "1700000000.000100" },
        "event_type": "event_callback",
        "message": {
            "provider": provider,
            "source_id": "C1",
            "thread_id": null,
            "message_thread_id": "1700000000.000100",
            "user_id": "user-1",
            "text": "please look at this",
            "file_ids": [],
            "thread_properties": { "channel": "C1", "ts": "1700000000.000100" },
            "provider_context": {},
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
        "INSERT INTO integration_connections \
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

/// Register every provider exactly as `main` does, then run the worker until
/// every named record is terminal or the budget runs out.
///
/// Registration goes through the real registrar so the test also pins that each
/// provider claims its own kind — the property that keeps two provider
/// migrations from contending for one name.
async fn run_worker_until_terminal(
    state: &AppState,
    pool: &PgPool,
    records: &[(&str, &str)],
    budget: Duration,
) {
    let mut handlers = AdapterDeliveryHandlers::new();
    moatless_vibe_backend::adapters::delivery::register_builtin_delivery_handlers(
        &mut handlers,
        state,
    );

    let cancel = CancellationToken::new();
    let handle = spawn_adapter_delivery_worker(
        state.adapter_delivery.clone(),
        Arc::new(handlers),
        cancel.clone(),
    );

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
    let state = test_app_state(pool.clone());
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

/// The Telegram half of the same property. `update_id` is Telegram's delivery
/// id and survives a re-delivery, which is why it is the admission key.
#[sqlx::test]
async fn a_redelivered_telegram_update_admits_one_unit_of_work(pool: PgPool) {
    let state = test_app_state(pool.clone());
    let work = payload("pc_telegram", "tt_telegram", "telegram");

    assert_eq!(
        state
            .adapter_delivery
            .admit(request("telegram", TELEGRAM_KIND, "90210", work.clone()))
            .await
            .unwrap(),
        Admission::Admitted
    );
    assert_eq!(
        state
            .adapter_delivery
            .admit(request("telegram", TELEGRAM_KIND, "90210", work))
            .await
            .unwrap(),
        Admission::Duplicate
    );

    assert_eq!(count(&pool, TELEGRAM_KIND, "90210").await, 1);
}

/// A trigger deleted between admission and execution ends the record on its
/// first attempt: no retry can bring the row back, and the record has to say so
/// rather than fail silently seven more times.
#[sqlx::test]
async fn work_whose_trigger_was_deleted_goes_terminal_without_burning_the_budget(pool: PgPool) {
    let state = test_app_state(pool.clone());
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

/// Verification: restart recovery, for both providers. Records left `claimed`
/// by a worker that died are picked back up — which is what makes
/// acknowledging the provider before the routing runs safe in the first place.
///
/// The recovery driven here is claim-lease expiry, the mechanism that holds
/// however the dead process was identified: a replaced pod comes back under a
/// new name and never releases its own claims. The same-owner startup release
/// is the faster path for a process that does return, and is already covered
/// by the seam's own tests.
#[sqlx::test]
async fn claims_left_by_a_killed_worker_are_recovered_for_both_providers(pool: PgPool) {
    let state = test_app_state(pool.clone());

    for (provider, kind, event_id, connection) in [
        ("slack", SLACK_KIND, "Ev-restart", "pc_slack"),
        ("telegram", TELEGRAM_KIND, "70001", "pc_telegram"),
    ] {
        state
            .adapter_delivery
            .admit(request(
                provider,
                kind,
                event_id,
                payload(connection, "tt_1", provider),
            ))
            .await
            .unwrap();
    }

    // The kill: both records were claimed by a replica that is never coming
    // back, long enough ago that its lease has expired.
    sqlx::query(
        "UPDATE adapter_delivery SET status = 'claimed', attempts = 1, \
         claimed_at = NOW() - INTERVAL '1 hour', claimed_by = 'dead-replica'",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Neither connection was seeded, so recovery is observable as both records
    // reaching a terminal state: only a record that was reclaimed and executed
    // gets there.
    run_worker_until_terminal(
        &state,
        &pool,
        &[(SLACK_KIND, "Ev-restart"), (TELEGRAM_KIND, "70001")],
        Duration::from_secs(20),
    )
    .await;

    for (kind, event_id) in [(SLACK_KIND, "Ev-restart"), (TELEGRAM_KIND, "70001")] {
        let (status, _, last_error) = record(&pool, kind, event_id).await;
        assert_eq!(status, "failed", "{kind} was recovered and executed");
        assert!(
            last_error
                .unwrap_or_default()
                .contains("adapter connection"),
            "{kind} ran the real handler rather than being abandoned"
        );
    }
}
