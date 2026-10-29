#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions; the lints only fire when the db-tests feature is enabled"
)]
//! Tests for the Telegram webhook route: which Subscription a delivery routes
//! under, whose `secret_token` is checked, and whether the response
//! acknowledges work nothing durable was written for.
//!
//! Connection and Subscription must resolve once, not twice under different
//! keys, else an ignored update's metric double-counts or one Subscription's
//! secret is checked against a different one's route; no update here reaches
//! api.telegram.org.

use crate::common;
use moatless_types::adapter::AdapterKind;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use moatless_backend::adapters::routes::adapter_router;
use moatless_backend::state::AppState;
use moatless_types::access_control::Scope;
use moatless_types::loops::RoutingMode;
use moatless_types::loops::SubscriptionMatch;
use moatless_workspaces::compose;
use sqlx::{PgPool, Row};
use tower::ServiceExt;

use common::{
    USER_1, make_repository_row, seed_team, seed_telegram_app, test_app_state_with_adapters,
    test_config,
};

const CONNECTION: &str = "conn-telegram-1";
const TRIGGER: &str = "tt-telegram-1";
const CHAT: &str = "-100123";
const SECRET: &str = "telegram-secret";
const WEBHOOK_PATH: &str = "/api/v1/adapters/telegram/webhook";

/// A backend replica with the Telegram Adapter installed, as startup builds it.
///
/// Must seed the `adapter_apps` row: ingress resolves the DB-authoritative
/// client before normalizing, so a missing row yields `400` instead of the
/// ignored/admitted outcome under test.
async fn replica(pool: &PgPool) -> AppState {
    let mut config = test_config();
    config.telegram.bot_token = Some("fake-token".to_string());
    seed_telegram_app(pool, &config, "fake-token").await;
    test_app_state_with_adapters(pool.clone(), config)
}

/// Deliver one update the way Telegram does, and return the response status.
async fn deliver(
    state: &AppState,
    update: &serde_json::Value,
    secret_header: Option<&str>,
) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri(WEBHOOK_PATH)
        .header("content-type", "application/json");
    if let Some(secret) = secret_header {
        request = request.header("x-telegram-bot-api-secret-token", secret);
    }
    let request = request
        .body(Body::from(serde_json::to_vec(update).unwrap()))
        .unwrap();

    adapter_router(state.clone())
        .0
        .oneshot(request)
        .await
        .expect("route answered")
        .status()
}

/// A group text message in [`CHAT`].
fn group_update(update_id: i64, text: &str) -> serde_json::Value {
    serde_json::json!({
        "update_id": update_id,
        "message": {
            "message_id": 42,
            "from": { "id": 999, "first_name": "Ada" },
            "chat": { "id": -100_123, "type": "supergroup" },
            "text": text,
        }
    })
}

/// A private-chat text message. Private chats are the shape whose two
/// resolutions used *different* keys before Decision 5.
fn private_update(update_id: i64, text: &str) -> serde_json::Value {
    serde_json::json!({
        "update_id": update_id,
        "message": {
            "message_id": 42,
            "from": { "id": 999, "first_name": "Ada" },
            "chat": { "id": 555, "type": "private" },
            "text": text,
        }
    })
}

// =========================================================================
// Fixtures
// =========================================================================

async fn seed_connection(pool: &PgPool) {
    seed_team(pool, &format!("personal-{USER_1}")).await;
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'telegram', 'env', 'default', $2, 'user', $2)",
    )
    .bind(CONNECTION)
    .bind(USER_1)
    .execute(pool)
    .await
    .expect("seed Telegram connection");
}

/// An active Telegram Subscription matching `source_matcher`, optionally
/// carrying a `secret_token`.
async fn seed_subscription(
    state: &AppState,
    pool: &PgPool,
    source_matcher: &str,
    secret: Option<&str>,
) {
    let now = Utc::now();
    compose::seed_repository(pool, &make_repository_row("repo-1", USER_1))
        .await
        .expect("seed repository");
    common::seed_workspace_placing(pool, common::FIXTURE_WORKSPACE, "repo-1", USER_1).await;
    moatless_loops::loops::test_support::upsert_subscription(
        &state.loops,
        &SubscriptionMatch {
            name: "test-loop".to_string(),
            id: TRIGGER.to_string(),
            adapter_kind: AdapterKind::Telegram,
            connection_id: CONNECTION.to_string(),
            source_matcher: source_matcher.to_string(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            workspace_id: "ws-1".to_string(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            memory_preload: Vec::new(),
            skills: Vec::new(),
            run_as_user_id: Some(USER_1.to_string()),
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            close_after_settled_days: None,
            settle_after_hours: None,
            node_selector: None,
            stop_when_done: None,
            room: None,
            message_buffer_seconds: None,
            allow_direct_send: false,
            adapter_config: secret
                .map(|secret| serde_json::json!({ "secret_token": secret }))
                .or_else(|| Some(serde_json::json!({}))),
            ongoing_task_id: None,
            close_task_on_close: true,
            notify_on_success: false,
            active: true,
            created_by: USER_1.to_string(),
            scope: Scope::User,
            owner_user_id: Some(USER_1.to_string()),
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .expect("seed Telegram subscription");
}

/// Every admitted delivery, as `(kind, event_id)`.
async fn admitted(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query("SELECT kind, event_id FROM adapter_delivery ORDER BY event_id")
        .fetch_all(pool)
        .await
        .expect("read adapter_delivery")
        .into_iter()
        .map(|row| (row.get("kind"), row.get("event_id")))
        .collect()
}

// =========================================================================
// Metric capture
// =========================================================================

/// One counter increment, flattened to the labels these tests assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sample {
    reason: String,
    count: u64,
}

/// Runs `f` under a thread-local recorder and returns every increment of
/// `metric` it produced.
///
/// Relies on `sqlx::test` using a current-thread runtime, so awaits inside `f`
/// stay on the recorder's thread.
async fn captured<F, T>(metric: &str, f: F) -> (T, Vec<Sample>)
where
    F: std::future::Future<Output = T>,
{
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let guard = metrics::set_default_local_recorder(&recorder);
    let value = f.await;
    drop(guard);

    let samples = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, _, _, _)| key.key().name() == metric)
        .map(|(key, _, _, value)| Sample {
            reason: key
                .key()
                .labels()
                .find(|label| label.key() == "reason")
                .map_or_else(String::new, |label| label.value().to_string()),
            count: match value {
                DebugValue::Counter(count) => count,
                _ => 0,
            },
        })
        .collect();
    (value, samples)
}

const INBOUND_TOTAL: &str = "moatless_binding_inbound_total";
const WEBHOOK_TOTAL: &str = "moatless_adapter_webhook_total";

// =========================================================================
// One resolution (Decision 5)
// =========================================================================

/// An update with no Telegram connection at all resolves once, so it is metered
/// once: a second resolution to find a secret would meter its own miss too,
/// double-counting one ignored update.
#[sqlx::test]
async fn an_ignored_update_is_metered_once(pool: PgPool) {
    let state = replica(&pool).await;

    let (status, samples) = captured(INBOUND_TOTAL, async {
        deliver(&state, &group_update(1, "do the thing"), None).await
    })
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        samples,
        vec![Sample {
            reason: "no_connection".to_string(),
            count: 1,
        }],
        "one ignored update, one resolution, one increment"
    );
    assert!(admitted(&pool).await.is_empty());
}

/// The secret checked must belong to the Subscription the delivery routes
/// under.
///
/// A second resolution under `"*"` for the secret would check a chat-keyed
/// Subscription's `secret_token` against a different Subscription's route,
/// letting any header pass.
#[sqlx::test]
async fn a_private_update_is_verified_against_the_subscription_it_routes_under(pool: PgPool) {
    let state = replica(&pool).await;
    seed_connection(&pool).await;
    seed_subscription(&state, &pool, "555", Some(SECRET)).await;

    assert_eq!(
        deliver(&state, &private_update(2, "do the thing"), Some("wrong")).await,
        StatusCode::UNAUTHORIZED,
        "the routing Subscription's secret is the one that must match"
    );
    assert!(admitted(&pool).await.is_empty());

    assert_eq!(
        deliver(&state, &private_update(3, "do the thing"), Some(SECRET)).await,
        StatusCode::OK
    );
    assert_eq!(
        admitted(&pool).await,
        vec![("telegram.inbound_message".to_string(), "3".to_string())],
    );
}

/// Resolution comes first, so an update nothing routes is ignored — and a
/// header that matches no Subscription cannot make it a `401`, because there is
/// no Subscription whose secret to check.
#[sqlx::test]
async fn an_update_with_no_subscription_is_ignored_rather_than_refused(pool: PgPool) {
    let state = replica(&pool).await;
    seed_connection(&pool).await;

    assert_eq!(
        deliver(&state, &group_update(4, "do the thing"), Some("wrong")).await,
        StatusCode::OK
    );
    assert!(admitted(&pool).await.is_empty());
}

// =========================================================================
// Properties the breakup must preserve
// =========================================================================

/// The update id is the durable identity: a re-delivery collapses onto the
/// record the first one admitted.
#[sqlx::test]
async fn the_update_id_is_the_admission_identity(pool: PgPool) {
    let state = replica(&pool).await;
    seed_connection(&pool).await;
    seed_subscription(&state, &pool, CHAT, None).await;

    assert_eq!(
        deliver(&state, &group_update(7, "do the thing"), None).await,
        StatusCode::OK
    );
    assert_eq!(
        deliver(&state, &group_update(7, "do the thing"), None).await,
        StatusCode::OK,
        "a re-delivery is still acknowledged"
    );

    assert_eq!(
        admitted(&pool).await,
        vec![("telegram.inbound_message".to_string(), "7".to_string())],
        "one unit of work, not two"
    );
}

/// The payload the Adapter admits is what the worker reads back after a
/// restart, so its shape is a wire contract.
#[sqlx::test]
async fn the_admitted_payload_names_the_routing_subscription(pool: PgPool) {
    let state = replica(&pool).await;
    seed_connection(&pool).await;
    seed_subscription(&state, &pool, CHAT, None).await;

    deliver(&state, &group_update(8, "do the thing"), None).await;

    let payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM adapter_delivery WHERE event_id = '8'")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(payload["connection_id"], CONNECTION);
    assert_eq!(payload["loop_id"], TRIGGER);
    assert_eq!(payload["subject_kind"], "telegram_thread");
    assert_eq!(payload["subject_key"], "-100123:thread:42");
    assert_eq!(payload["event_type"], "", "Telegram labels no event type");
    assert_eq!(payload["message"]["adapter_kind"], "telegram");
    assert_eq!(payload["message"]["source_id"], CHAT);
    assert_eq!(payload["message"]["external_event_id"], "tg:m:42");
    assert_eq!(
        payload["message"]["audience"],
        serde_json::json!({ "kind": "everyone" })
    );
    assert_eq!(payload["destination_config"]["chat_id"], CHAT);
}

/// A matching secret passes, a mismatched one is refused with `401`, and a
/// Subscription with no secret configured accepts a request that carries none.
#[sqlx::test]
async fn the_secret_policy_is_matching_header_or_no_secret_at_all(pool: PgPool) {
    let state = replica(&pool).await;
    seed_connection(&pool).await;
    seed_subscription(&state, &pool, CHAT, Some(SECRET)).await;

    assert_eq!(
        deliver(&state, &group_update(10, "do the thing"), Some(SECRET)).await,
        StatusCode::OK
    );
    assert_eq!(
        deliver(&state, &group_update(11, "do the thing"), Some("wrong")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        deliver(&state, &group_update(12, "do the thing"), None).await,
        StatusCode::UNAUTHORIZED,
        "an absent header is not a matching one"
    );

    assert_eq!(
        admitted(&pool).await,
        vec![("telegram.inbound_message".to_string(), "10".to_string())],
        "only the verified delivery was admitted"
    );
}

/// An envelope with no `update_id` cannot be given a durable identity, and no
/// re-delivery supplies one.
#[sqlx::test]
async fn an_update_without_an_id_is_refused_rather_than_acknowledged(pool: PgPool) {
    let state = replica(&pool).await;
    seed_connection(&pool).await;
    seed_subscription(&state, &pool, CHAT, None).await;

    let status = deliver(
        &state,
        &serde_json::json!({
            "message": {
                "message_id": 42,
                "from": { "id": 999 },
                "chat": { "id": -100_123, "type": "supergroup" },
                "text": "do the thing",
            }
        }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(admitted(&pool).await.is_empty());
}

/// An update nothing routes is acknowledged — there is no work left to lose —
/// and metered under the one reason Telegram's dashboards query.
#[sqlx::test]
async fn an_unroutable_update_is_acknowledged_and_metered_as_not_routable(pool: PgPool) {
    let state = replica(&pool).await;
    seed_connection(&pool).await;

    let (status, samples) = captured(WEBHOOK_TOTAL, async {
        deliver(&state, &group_update(20, "do the thing"), None).await
    })
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        samples,
        vec![Sample {
            reason: "not_routable".to_string(),
            count: 1,
        }]
    );
    assert!(admitted(&pool).await.is_empty());
}
