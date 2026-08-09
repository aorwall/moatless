#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on, here \
              and in the shared `common` module compiled into this binary; both lints only \
              fire when the db-tests feature is enabled"
)]
//! What the Linear route promises the sender, at the two edges of it.
//!
//! The identity tests next door assert that the right work reaches the Agent.
//! These assert the property underneath: a `200` means a durable record exists,
//! and where one cannot exist the `200` is withheld. Both are observed at the
//! real inbound route, because both live between the signature check and the
//! response — the one place a unit test cannot stand.
//!
//! Neither case below needs a task, a binding, or a rule. That is deliberate:
//! whether a delivery is *recorded* must not depend on whether anything is
//! listening to it.

use crate::common;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use sqlx::Row;

use moatless_backend::adapters::dao::PgAdapterConnectionDao;
use moatless_backend::adapters::delivery::AdapterDeliveryStore;
use moatless_backend::adapters::error::{AdapterError, SharedAdapterError};
use moatless_backend::adapters::providers::linear;
use moatless_backend::adapters::providers::linear::adapter::LinearAdapter;
use moatless_backend::adapters::providers::linear::handlers::linear_webhook;
use moatless_backend::loops::dao::LoopDao;
use moatless_backend::state::AppState;

use common::{
    USER_1, drain_linear_adapter_delivery, linear_config, test_app_state_with_linear, test_config,
};

const SECRET: &str = "linear-webhook-secret";
const ORG: &str = "org-1";
const TEAM: &str = "team-1";
const ISSUE: &str = "issue-1";
const CONNECTION: &str = "conn-linear-1";

type HmacSha256 = Hmac<Sha256>;

/// A backend replica with the Linear adapter installed. It holds no state
/// between deliveries — everything that survives one is in the database.
fn replica(pool: &PgPool) -> AppState {
    let mut config = test_config();
    config.linear = linear_config(Some(SECRET), None);
    let linear = Arc::new(
        LinearAdapter::from_db(Arc::new(config.clone()), pool.clone()).with_ingress_ports(
            Arc::new(PgAdapterConnectionDao::new(pool.clone())),
            Arc::new(LoopDao::new(pool.clone())),
            Arc::new(AdapterDeliveryStore::new(pool.clone())),
        ),
    );
    test_app_state_with_linear(pool.clone(), linear, config)
}

/// Deliver a signed envelope the way Linear does, and return whatever the
/// route answered — including a refusal, which is what half of this file is
/// about.
async fn deliver(
    state: &AppState,
    envelope: &serde_json::Value,
) -> Result<StatusCode, AdapterError> {
    let body = serde_json::to_vec(envelope).expect("serialize envelope");

    let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).expect("hmac key");
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());

    let mut headers = HeaderMap::new();
    headers.insert("linear-signature", signature.parse().expect("header value"));

    linear_webhook(
        State(state.provider_handles.linear.clone()),
        headers,
        Bytes::from(body),
    )
    .await
}

/// The Linear connection the envelope's `organizationId` resolves to. The only
/// fixture a delivery needs in order to be recorded.
async fn seed_connection(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'linear', 'org', $2, $3, 'user', $3)",
    )
    .bind(CONNECTION)
    .bind(ORG)
    .bind(USER_1)
    .execute(pool)
    .await
    .expect("seed Linear connection");
}

/// Every admitted record, as `(kind, event_id, status)`.
async fn records(pool: &PgPool) -> Vec<(String, String, String)> {
    sqlx::query("SELECT kind, event_id, status FROM adapter_delivery ORDER BY kind, event_id")
        .fetch_all(pool)
        .await
        .expect("read adapter_delivery")
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("kind"),
                row.get::<String, _>("event_id"),
                row.get::<String, _>("status"),
            )
        })
        .collect()
}

/// Run the real worker until nothing is left unfinished, then stop it.
async fn drain_admitted_work(pool: &PgPool) {
    drain_linear_adapter_delivery(pool, replica(pool)).await;
}

/// A workspace notification carrying `action`.
fn notification(action: &str, kind: &str, id: &str) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "type": "AppUserNotification",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "notification": {
            "id": id,
            "type": kind,
            "issueId": ISSUE,
            "issue": {
                "id": ISSUE,
                "identifier": "ENG-7",
                "title": "Fix it",
                "teamId": TEAM,
                "state": {"id": "s1", "name": "Todo", "type": "unstarted"},
            },
        },
    })
}

/// An `AppUserNotification` action this adapter does not act on is still a
/// delivery that arrived, so it still gets a record.
///
/// The action filter used to sit at the route, which meant a whole class of
/// Linear delivery was acknowledged with nothing durable behind it — leaving an
/// operator no way to answer "did it arrive?". The decision belongs behind
/// admission, and the no-op belongs in the worker.
#[sqlx::test]
async fn an_unhandled_notification_action_is_still_recorded_and_no_ops(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    assert_eq!(
        deliver(
            &state,
            &notification("issueMention", "issueMention", "notif-2")
        )
        .await
        .expect("Linear is acknowledged"),
        StatusCode::OK
    );

    // Before any worker runs: the record exists and nothing has executed it.
    // That is the whole of "nothing runs inline" — the response came back with
    // the work still waiting.
    assert_eq!(
        records(&pool).await,
        vec![(
            linear::delivery::NOTIFICATION.to_string(),
            "AppUserNotification:issueMention:notif-2".to_string(),
            "pending".to_string(),
        )],
        "an unhandled action is admitted like any other delivery"
    );

    drain_admitted_work(&pool).await;

    let records = records(&pool).await;
    assert_eq!(records.len(), 1, "one delivery, one record: {records:?}");
    assert_eq!(
        records[0].2, "succeeded",
        "nothing to do is a successful no-op, not a failure an operator has to triage"
    );
}

/// The handled action takes the same route to the same kind — so the record
/// above is not a special case for unhandled actions.
#[sqlx::test]
async fn a_handled_notification_action_admits_under_the_same_kind(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    assert_eq!(
        deliver(
            &state,
            &notification("issueStatusChanged", "issueStatusChanged", "notif-1")
        )
        .await
        .expect("Linear is acknowledged"),
        StatusCode::OK
    );

    assert_eq!(
        records(&pool).await,
        vec![(
            linear::delivery::NOTIFICATION.to_string(),
            "AppUserNotification:issueStatusChanged:notif-1".to_string(),
            "pending".to_string(),
        )]
    );
}

/// An issue update Linear delivered without a `webhookTimestamp` has nothing
/// that could tell it apart from the next update to the same issue.
///
/// Admitting it would not lose this delivery — it would lose the *next* one,
/// silently and for the whole retention period, because the durable unique
/// index would recognise a genuinely different state change as a duplicate of
/// this one. So the route refuses instead, and withholds the acknowledgment
/// rather than dropping the delivery quietly behind a `200`.
#[sqlx::test]
async fn an_issue_update_without_a_delivery_timestamp_is_refused_rather_than_collapsed(
    pool: PgPool,
) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let untimed = |to_state: &str| {
        serde_json::json!({
            "action": "update",
            "type": "Issue",
            "organizationId": ORG,
            "webhookId": "wh-stable",
            // No `webhookTimestamp` — the field the identity is built from.
            "updatedFrom": {"stateId": "todo"},
            "data": {
                "id": ISSUE,
                "identifier": "ENG-7",
                "title": "Fix it",
                "teamId": TEAM,
                "state": {"id": to_state, "name": to_state, "type": "started"},
            },
        })
    };

    let refused = deliver(&state, &untimed("in-progress"))
        .await
        .expect_err("the delivery must not be acknowledged");
    assert!(
        matches!(
            refused,
            AdapterError::Shared(SharedAdapterError::BadRequest(_))
        ),
        "the delivery is malformed, not the backend unwell: {refused:?}"
    );
    assert!(
        records(&pool).await.is_empty(),
        "nothing may be recorded under an identity that would collapse"
    );

    // A second, genuinely different update — same issue, different destination
    // state — is refused the same way on the same replica. The refusal is
    // stateless: nothing on this replica remembers the first delivery, so the
    // second cannot come back as a swallowed `200`, which is the
    // acknowledgment lie the refusal exists to avoid.
    let refused_again = deliver(&state, &untimed("in-review"))
        .await
        .expect_err("the second delivery must not be acknowledged either");
    assert!(
        matches!(
            refused_again,
            AdapterError::Shared(SharedAdapterError::BadRequest(_))
        ),
        "a refusal must leave nothing behind that could swallow the next attempt: \
         {refused_again:?}"
    );
    assert!(records(&pool).await.is_empty());
}

/// The same issue update *with* a timestamp is admitted — so the refusal above
/// is about the missing field, not about issue updates.
#[sqlx::test]
async fn an_issue_update_with_a_delivery_timestamp_is_admitted(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let timed = serde_json::json!({
        "action": "update",
        "type": "Issue",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "updatedFrom": {"stateId": "todo"},
        "data": {
            "id": ISSUE,
            "identifier": "ENG-7",
            "title": "Fix it",
            "teamId": TEAM,
            "state": {"id": "in-progress", "name": "In Progress", "type": "started"},
        },
    });

    assert_eq!(
        deliver(&state, &timed)
            .await
            .expect("Linear is acknowledged"),
        StatusCode::OK
    );
    assert_eq!(
        records(&pool).await,
        vec![(
            linear::delivery::ISSUE_UPDATE.to_string(),
            format!("Issue:update:{ISSUE}:1700000000000"),
            "pending".to_string(),
        )]
    );
}

/// When the durable write itself fails, the sender is asked to send the
/// delivery again — and nothing on this replica stands in the way of the
/// redelivery.
///
/// This is the property the deleted in-process cache put at risk. It was
/// marked *before* the dispatch ran, so every failed admission needed a
/// compensating eviction, or the same replica would have dismissed Linear's
/// next attempt as a duplicate and answered `200` — losing the delivery behind
/// the acknowledgment the failure had just refused to give. With the cache
/// gone there is nothing left to compensate for, and this asserts the outcome
/// rather than the correction: same replica, same delivery, admitted the
/// moment the store is back.
#[sqlx::test]
async fn a_delivery_whose_admission_failed_is_admitted_when_it_is_sent_again(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;
    let delivery = notification("issueStatusChanged", "issueStatusChanged", "notif-1");

    // The store, unavailable — which is what the route sees when the durable
    // write cannot commit.
    sqlx::query("ALTER TABLE adapter_delivery RENAME TO adapter_delivery_unavailable")
        .execute(&pool)
        .await
        .expect("take the Adapter delivery store away");

    let refused = deliver(&state, &delivery)
        .await
        .expect_err("a delivery that was not recorded must not be acknowledged");
    assert!(
        matches!(refused, AdapterError::Shared(SharedAdapterError::Db(_))),
        "the store failed, so the backend is unwell rather than the delivery malformed: \
         {refused:?}"
    );

    sqlx::query("ALTER TABLE adapter_delivery_unavailable RENAME TO adapter_delivery")
        .execute(&pool)
        .await
        .expect("give the Adapter delivery store back");

    assert_eq!(
        deliver(&state, &delivery)
            .await
            .expect("Linear is acknowledged once the delivery is durably owned"),
        StatusCode::OK
    );
    assert_eq!(
        records(&pool).await,
        vec![(
            linear::delivery::NOTIFICATION.to_string(),
            "AppUserNotification:issueStatusChanged:notif-1".to_string(),
            "pending".to_string(),
        )],
        "the redelivery Linear was asked for is admitted, not swallowed"
    );
}
