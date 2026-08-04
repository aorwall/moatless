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
//! A Slack event naming a channel no Subscription matches is dropped as
//! `no_trigger` and acknowledged, and it always has been — including when the
//! signature is junk, because the drop happens before the signature is
//! consulted. That ordering is the whole of the behaviour: an endpoint that
//! answered `401` here would stop attributing unrouted traffic to a channel and
//! would ask Slack to re-sign events nobody is listening for.
//!
//! It is asserted at the real route because that is the only place it exists.
//! `ingress::receive` needs an `AppState` to resolve anything, so the unit
//! characterization next to it can only reach the halves either side of the
//! resolution; this file covers the seam between them.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use tower::ServiceExt as _;

use moatless_vibe_backend::adapters::routes::adapter_router;
use moatless_vibe_backend::adapters::Adapters;
use moatless_vibe_backend::compose::compose;
use moatless_vibe_backend::state::AppState;

use common::{test_app_state_with_adapters, test_config};

const SIGNING_SECRET: &str = "8f742231b10e8888abcd99yez67676hg";
const UNROUTED_CHANNEL: &str = "C-NOBODY-LISTENS";

type HmacSha256 = Hmac<Sha256>;

/// A backend with the Slack adapter installed, built through the composition
/// root so the registration is the one `main` performs. `bot_user_id` is set
/// because an absent one sends `resolve_bot_user_id` to `auth.test`, and a test
/// does not call slack.com.
async fn replica(pool: &PgPool) -> AppState {
    let mut config = test_config();
    config.slack.bot_token = Some("xoxb-test".to_string());
    config.slack.signing_secret = Some(SIGNING_SECRET.to_string());
    config.slack.bot_user_id = Some("U0BOT".to_string());

    let adapters = Adapters::from_config(&config, &compose(pool.clone(), &config))
        .await
        .expect("Slack adapter configured");
    test_app_state_with_adapters(pool.clone(), Some(Arc::new(adapters)))
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

/// The regression this pins: resolution runs first, so an unrouted event is
/// acknowledged whether or not the signature holds. Hoisting the signature
/// check above the `no_trigger` return turns the first case into a `401` with
/// no `ignored`/`no_trigger` metric behind it — which is exactly the change
/// this asserts against.
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
