#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on, here \
              and in the shared `common` module compiled into this binary; both lints only \
              fire when the db-tests feature is enabled"
)]
//! Where Slack's signature check sits relative to routing resolution.
//!
//! An event naming a channel no Subscription matches is dropped as `no_trigger`
//! and acknowledged before its signature is checked — hoisting that check above
//! the drop would turn it into a `401`.
//!
//! Asserted here because `ingress::receive` needs a real `AppState`; the unit
//! test beside it only reaches the seam's two halves.

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use tower::ServiceExt as _;

use moatless_backend::adapters::routes::adapter_router;
use moatless_backend::state::AppState;

use common::{test_app_state_with_adapters, test_config};

const SIGNING_SECRET: &str = "8f742231b10e8888abcd99yez67676hg";
const UNROUTED_CHANNEL: &str = "C-NOBODY-LISTENS";

type HmacSha256 = Hmac<Sha256>;

/// A backend with the Slack adapter installed via the composition root `main`
/// uses.
///
/// `bot_user_id` is set so `resolve_bot_user_id` doesn't call `auth.test` on
/// slack.com.
async fn replica(pool: &PgPool) -> AppState {
    let mut config = test_config();
    config.slack.bot_token = Some("xoxb-test".to_string());
    config.slack.signing_secret = Some(SIGNING_SECRET.to_string());
    config.slack.bot_user_id = Some("U0BOT".to_string());

    test_app_state_with_adapters(pool.clone(), config)
}

/// One message event for a channel this database has never heard of.
fn event(channel: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "type": "event_callback",
        "event_id": "Ev-unrouted",
        "team_id": "T1",
        "event": {
            "type": "message",
            "user": "U1HUMAN",
            "channel": channel,
            "text": "<@U0BOT> anyone home?",
            "ts": "1700000000.000100",
        },
    }))
    .expect("serialize envelope")
}

/// The signature Slack would send for `body`, or a plausible-looking forgery.
fn signature(body: &[u8], valid: bool) -> (String, String) {
    let timestamp = chrono::Utc::now().timestamp().to_string();
    if !valid {
        return (timestamp, "v0=deadbeef".to_string());
    }
    let mut mac = HmacSha256::new_from_slice(SIGNING_SECRET.as_bytes()).expect("hmac key");
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    let signature = format!("v0={}", hex::encode(mac.finalize().into_bytes()));
    (timestamp, signature)
}

async fn deliver(state: &AppState, body: Vec<u8>, signed: bool) -> StatusCode {
    let (timestamp, signature) = signature(&body, signed);
    let (router, _) = adapter_router(state.clone());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/adapters/slack/webhook")
        .header("x-slack-request-timestamp", timestamp)
        .header("x-slack-signature", signature)
        .body(Body::from(body))
        .expect("build request");

    router
        .oneshot(request)
        .await
        .expect("route request succeeds")
        .status()
}

async fn admitted(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery")
        .fetch_one(pool)
        .await
        .expect("count admissions")
}

/// Pins the ordering: resolution runs before the signature check, so an
/// unrouted event is acknowledged whether or not its signature holds.
#[sqlx::test]
async fn an_unrouted_event_is_acknowledged_whatever_its_signature_says(pool: PgPool) {
    let state = replica(&pool).await;

    for signed in [false, true] {
        assert_eq!(
            deliver(&state, event(UNROUTED_CHANNEL), signed).await,
            StatusCode::OK,
            "an event no Subscription matches is dropped before its signature is read \
             (signed: {signed})"
        );
    }

    assert_eq!(
        admitted(&pool).await,
        0,
        "a drop admits no work, so nothing durable exists to retry"
    );
}
