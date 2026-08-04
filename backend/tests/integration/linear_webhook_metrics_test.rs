#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on, here \
              and in the shared `common` module compiled into this binary; both lints only \
              fire when the db-tests feature is enabled"
)]
//! What the `linear` webhook metric says, row by row.
//!
//! This file was written to be *wrong* later, and stage 15 of
//! `.plans/linear-adapter-breakup.md` is where it became so. Every assertion
//! below was first recorded against the pre-change route (#191), so the diff
//! that introduced the `ignored` outcome shows exactly which rows moved and
//! nothing else — a row that changed without anyone meaning to change it fails
//! here.
//!
//! Eight benign rejections that used to meter `success` now meter `ignored`
//! with one of the seven reasons Decision 4 names (defect D1: the dashboard
//! could not tell "arrived and admitted" apart from "arrived and dropped").
//! The three error rows now carry the `event_type` they never had (D2), except
//! the one delivery that has no event type to carry.
//!
//! Observed at the real route, because the metric is emitted by the route: the
//! outcome depends on which `IngressError` variant reached it, and that
//! mapping is precisely what a unit test below the route cannot see. What each
//! reason *means* is asserted one level down, in `linear_ingress_test.rs`.

use crate::common;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use sha2::Sha256;
use sqlx::PgPool;

use moatless_vibe_backend::adapters::error::AdapterError;
use moatless_vibe_backend::adapters::linear::adapter::LinearAdapter;
use moatless_vibe_backend::adapters::linear::handlers::linear_webhook;
use moatless_vibe_backend::adapters::Adapters;
use moatless_vibe_backend::state::AppState;

use common::{linear_config, test_app_state_with_config, test_config, USER_1};

const SECRET: &str = "linear-webhook-secret";
const ORG: &str = "org-1";
const TEAM: &str = "team-1";
const ISSUE: &str = "issue-1";
const CONNECTION: &str = "conn-linear-1";
const WEBHOOK_TOTAL: &str = "moatless_adapter_webhook_total";

type HmacSha256 = Hmac<Sha256>;

// =========================================================================
// Fixtures
// =========================================================================

/// A replica whose Linear clients point at an address nothing listens on.
///
/// The created-session path posts a heartbeat activity before it answers, and
/// that call must not leave the machine. Pointing it at a dead port also makes
/// the point the metric assertions rest on: the outcome label does not depend
/// on whether Linear answered, because the heartbeat is best-effort.
fn replica(pool: &PgPool) -> AppState {
    let mut adapters = Adapters::new();
    let mut config = test_config();
    config.linear = linear_config(
        Some(SECRET),
        Some(moatless_vibe_backend::adapters::test_support::UNREACHABLE_BASE),
    );
    adapters.register_linear(Arc::new(LinearAdapter::new(&config.linear, pool.clone())));
    test_app_state_with_config(pool.clone(), Some(Arc::new(adapters)), config)
}

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

/// Deliver `envelope` the way Linear does. `sign` false stands in for the
/// unsigned delivery — the header is absent, not wrong.
async fn deliver(
    state: &AppState,
    envelope: &serde_json::Value,
    sign: bool,
) -> Result<StatusCode, AdapterError> {
    deliver_bytes(
        state,
        serde_json::to_vec(envelope).expect("serialize"),
        sign,
    )
    .await
}

async fn deliver_bytes(
    state: &AppState,
    body: Vec<u8>,
    sign: bool,
) -> Result<StatusCode, AdapterError> {
    let mut headers = HeaderMap::new();
    if sign {
        let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).expect("hmac key");
        mac.update(&body);
        let signature = hex::encode(mac.finalize().into_bytes());
        headers.insert("linear-signature", signature.parse().expect("header value"));
    }
    linear_webhook(State(state.clone()), headers, Bytes::from(body)).await
}

// =========================================================================
// Capture
// =========================================================================

/// One increment of the webhook counter, flattened to every label the route
/// sets — the whole row, because the point is to catch a label that changes
/// without anyone meaning to change it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    adapter: String,
    outcome: String,
    reason: String,
    event_type: String,
    loop_id: String,
    connection_id: String,
    count: u64,
}

fn row(outcome: &str, reason: &str, event_type: &str, loop_id: &str, connection_id: &str) -> Row {
    Row {
        adapter: "linear".to_string(),
        outcome: outcome.to_string(),
        reason: reason.to_string(),
        event_type: event_type.to_string(),
        loop_id: loop_id.to_string(),
        connection_id: connection_id.to_string(),
        count: 1,
    }
}

/// Run `f` against a thread-local recorder and return every webhook-counter row
/// it produced.
///
/// `sqlx::test` drives the body on a current-thread runtime, so awaiting inside
/// the guard stays on the thread the recorder is installed for.
async fn captured<F, T>(f: F) -> (T, Vec<Row>)
where
    F: std::future::Future<Output = T>,
{
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let guard = metrics::set_default_local_recorder(&recorder);
    let value = f.await;
    drop(guard);

    let label = |key: &metrics_util::CompositeKey, name: &str| {
        key.key()
            .labels()
            .find(|l| l.key() == name)
            .map_or_else(String::new, |l| l.value().to_string())
    };
    let rows = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, _, _, _)| key.key().name() == WEBHOOK_TOTAL)
        .map(|(key, _, _, value)| Row {
            adapter: label(&key, "adapter"),
            outcome: label(&key, "outcome"),
            reason: label(&key, "reason"),
            event_type: label(&key, "event_type"),
            loop_id: label(&key, "loop_id"),
            connection_id: label(&key, "connection_id"),
            count: match value {
                DebugValue::Counter(count) => count,
                _ => 0,
            },
        })
        .collect();
    (value, rows)
}

// =========================================================================
// Envelopes
// =========================================================================

fn comment_envelope(action: &str, comment_id: &str) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "type": "Comment",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "data": {
            "id": comment_id,
            "body": "please look at this",
            "issueId": ISSUE,
            "issue": {"id": ISSUE, "identifier": "ENG-7", "title": "Fix it", "teamId": TEAM},
        },
    })
}

/// An `Issue.update`, with `webhookTimestamp` present unless `timestamped` is
/// false — its absence is the one delivery the route refuses outright.
fn issue_envelope(timestamped: bool) -> serde_json::Value {
    let mut envelope = serde_json::json!({
        "action": "update",
        "type": "Issue",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "updatedFrom": {"stateId": "s0"},
        "data": {
            "id": ISSUE,
            "identifier": "ENG-7",
            "title": "Fix it",
            "teamId": TEAM,
            "state": {"id": "s1", "name": "Done", "type": "completed"},
        },
    });
    if timestamped {
        envelope["webhookTimestamp"] = serde_json::json!(1_700_000_000_000i64);
    }
    envelope
}

fn session_envelope(action: &str) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "type": "AgentSessionEvent",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "agentSession": {
            "id": "session-1",
            "issue": {
                "id": ISSUE,
                "identifier": "ENG-7",
                "title": "Fix it",
                "teamId": TEAM,
            },
        },
    })
}

// =========================================================================
// The admitted row
// =========================================================================

/// The one row that means what it says: a signed delivery on a known
/// organization, admitted, acknowledged.
///
/// It is also the row every `success` below is indistinguishable from, which is
/// the whole of D1.
#[sqlx::test]
async fn an_admitted_delivery_is_success_with_its_event_type(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (status, rows) = captured(deliver(&state, &comment_envelope("create", "c-1"), true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(rows, vec![row("success", "", "Comment", "", "")]);
}

// =========================================================================
// The eight benign rejections that used to meter `success` (D1)
//
// One test per row of the plan's Behaviour table, and between them every one
// of the seven reasons Decision 4 names.
// =========================================================================

/// An unsigned delivery. Nothing is verified and nothing is admitted.
#[sqlx::test]
async fn an_unsigned_delivery_is_ignored_as_no_signature(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (status, rows) = captured(deliver(&state, &comment_envelope("create", "c-2"), false)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row("ignored", "no_signature", "Comment", "", "")],
        "an unsigned delivery is a drop, not a success"
    );
}

/// An organization with no `adapter_connections` row. Resolution misses
/// and the delivery is dropped.
#[sqlx::test]
async fn an_unknown_organization_is_ignored_as_no_connection(pool: PgPool) {
    let state = replica(&pool);
    // Deliberately no connection seeded.

    let (status, rows) = captured(deliver(&state, &comment_envelope("create", "c-3"), true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row("ignored", "no_connection", "Comment", "", "")]
    );
}

/// An event type this Adapter does not handle.
#[sqlx::test]
async fn an_unsupported_event_type_is_ignored_as_unsupported_event(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let envelope = serde_json::json!({
        "action": "create",
        "type": "Project",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "data": {"id": "proj-1"},
    });
    let (status, rows) = captured(deliver(&state, &envelope, true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row("ignored", "unsupported_event", "Project", "", "")],
        "the event type reaches the label even when nothing handles it"
    );
}

/// A `Comment` action other than `create`.
#[sqlx::test]
async fn a_non_create_comment_action_is_ignored_as_unsupported_action(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (status, rows) = captured(deliver(&state, &comment_envelope("update", "c-4"), true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row("ignored", "unsupported_action", "Comment", "", "")]
    );
}

/// An inner payload that does not parse into the type the handler expects.
#[sqlx::test]
async fn an_unparseable_inner_payload_is_ignored_as_malformed_payload(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let envelope = serde_json::json!({
        "action": "create",
        "type": "Comment",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "data": {"id": 17},
    });
    let (status, rows) = captured(deliver(&state, &envelope, true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row("ignored", "malformed_payload", "Comment", "", "")]
    );
}

/// An `AgentSessionEvent` action this Adapter does not act on.
#[sqlx::test]
async fn an_unsupported_session_action_is_ignored_as_unsupported_action(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (status, rows) = captured(deliver(&state, &session_envelope("archived"), true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row(
            "ignored",
            "unsupported_action",
            "AgentSessionEvent",
            "",
            ""
        )]
    );
}

/// A created session on a team with no active Subscription. The heartbeat is
/// attempted and nothing durable is written.
///
/// The heartbeat surviving the drop is asserted where it is observable — the
/// `AgentSessionEvent.created` budget in `linear_workflow_test.rs`, which
/// spends its one `agentActivityCreate` with a rule and without one. Open
/// question 2 in the plan is about whether the drop should also become durable;
/// `no_subscription` is the interim answer, and it is this row.
#[sqlx::test]
async fn a_created_session_with_no_rule_is_ignored_as_no_subscription(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (status, rows) = captured(deliver(&state, &session_envelope("created"), true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row(
            "ignored",
            "no_subscription",
            "AgentSessionEvent",
            "",
            ""
        )]
    );

    let admitted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery")
        .fetch_one(&pool)
        .await
        .expect("count admitted records");
    assert_eq!(
        admitted, 0,
        "no rule means nothing durable records that Linear asked — Open question 2"
    );
}

/// A prompted session carrying an empty body.
#[sqlx::test]
async fn an_empty_prompt_body_is_ignored_as_empty_body(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let mut envelope = session_envelope("prompted");
    envelope["agentActivity"] = serde_json::json!({"id": "act-1", "body": ""});
    let (status, rows) = captured(deliver(&state, &envelope, true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row("ignored", "empty_body", "AgentSessionEvent", "", "")]
    );
}

// =========================================================================
// The three error rows, and the `event_type` they now carry (D2)
// =========================================================================

/// A delivery whose signature does not verify. Acknowledged — no redelivery of
/// the same body will ever verify — but metered as an error.
///
/// The `event_type` label is D2's fix: the one metric that fires when something
/// breaks used to pass `""` here, even though the envelope had parsed and the
/// type was in hand.
#[sqlx::test]
async fn a_bad_signature_is_error_delivery_failed_with_its_event_type(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let body = serde_json::to_vec(&comment_envelope("create", "c-5")).expect("serialize");
    let deliver_wrong_signature = async {
        let mut headers = HeaderMap::new();
        headers.insert("linear-signature", "00ff".parse().expect("header value"));
        linear_webhook(State(state.clone()), headers, Bytes::from(body)).await
    };
    let (status, rows) = captured(deliver_wrong_signature).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(
        rows,
        vec![row("error", "delivery_failed", "Comment", "", "")],
        "the error row names the kind of delivery that broke — D2"
    );
}

/// A body that is not JSON at all. Same treatment: acknowledged, metered as an
/// error, no event type — the one error row that still carries none, because
/// there genuinely is not one to carry.
#[sqlx::test]
async fn an_unparseable_envelope_is_error_delivery_failed(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (status, rows) = captured(deliver_bytes(&state, b"not json".to_vec(), true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(rows, vec![row("error", "delivery_failed", "", "", "")]);
}

/// An `Issue.update` with no `webhookTimestamp`. The delivery carries work but
/// no field that tells that work apart from the next update to the same issue,
/// so it is refused with a `400` rather than admitted under a colliding
/// identity — and metered as `unidentifiable`.
///
/// The `400` is the contract: a refusal the sender can see, rather than a
/// silent collision that loses the *next* delivery.
#[sqlx::test]
async fn a_delivery_with_no_identity_is_error_unidentifiable_and_a_400(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (result, rows) = captured(deliver(&state, &issue_envelope(false), true)).await;

    assert!(
        matches!(result, Err(AdapterError::BadRequest(ref message))
            if message.contains("webhookTimestamp")),
        "the refusal names the missing field, got {result:?}"
    );
    assert_eq!(
        rows,
        vec![row("error", "unidentifiable", "Issue", "", "")],
        "the refusal names the kind of delivery it refused — D2"
    );

    let admitted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery")
        .fetch_one(&pool)
        .await
        .expect("count admitted records");
    assert_eq!(admitted, 0, "a refused delivery writes nothing");
}

/// The same issue update *with* its timestamp is admitted and metered
/// `success` — so the row above is about the missing field, not about issues.
#[sqlx::test]
async fn an_identified_issue_update_is_success(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let (status, rows) = captured(deliver(&state, &issue_envelope(true), true)).await;

    assert_eq!(status.expect("acknowledged"), StatusCode::OK);
    assert_eq!(rows, vec![row("success", "", "Issue", "", "")]);
}

/// Every row this route can emit sets `loop_id` and `connection_id` to the
/// empty string, on every outcome.
///
/// Asserted once, on purpose: the two labels are on the metric's signature and
/// nothing fills them, so a stage that starts filling one changes the metric's
/// cardinality. That is a decision, and it should fail a test rather than
/// appear in a dashboard.
#[sqlx::test]
async fn no_row_carries_a_trigger_or_connection_label(pool: PgPool) {
    let state = replica(&pool);
    seed_connection(&pool).await;

    let ((), rows) = captured(async {
        let _ = deliver(&state, &comment_envelope("create", "c-6"), true).await;
        let _ = deliver(&state, &issue_envelope(false), true).await;
        let _ = deliver_bytes(&state, b"not json".to_vec(), true).await;
    })
    .await;

    assert_eq!(rows.len(), 3, "one row per delivery: {rows:?}");
    for row in &rows {
        assert_eq!(row.adapter, "linear");
        assert_eq!(row.loop_id, "", "loop_id is never filled: {row:?}");
        assert_eq!(
            row.connection_id, "",
            "connection_id is never filled: {row:?}"
        );
    }
}
