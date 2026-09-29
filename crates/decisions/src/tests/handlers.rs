use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use moatless_core::config::{DecisionConfig, DecisionProviderConfig};
use moatless_core::http_auth::UserTokenVerifier;
use moatless_core::sandbox_jwt::JwtUserTokenVerifier;
use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::stub::{JevStub, Reply};

fn router(config: &DecisionConfig) -> axum::Router {
    let verifier: Arc<dyn UserTokenVerifier> = Arc::new(JwtUserTokenVerifier::new(None));
    crate::compose::decisions_router(config, verifier).0
}

fn configured(stub: &JevStub) -> DecisionConfig {
    DecisionConfig {
        provider: Some(DecisionProviderConfig::Jev(stub.config())),
    }
}

fn body(options: &[&str]) -> Value {
    json!({
        "question": "Bug or feature?",
        "options": options.iter().map(|name| json!({"name": name})).collect::<Vec<_>>(),
        "state": "The login button does nothing",
        "taskId": "task-1",
    })
}

async fn post(router: axum::Router, body: &Value, authenticated: bool) -> (StatusCode, Value) {
    let mut request = Request::post("/api/v1/decisions/pick")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    if authenticated {
        request.extensions_mut().insert(AuthIdentity {
            user_id: Uuid::new_v4(),
            login: "alice".to_string(),
            role: UserRole::User,
            provider: "github".to_string(),
            group_id: None,
        });
    }
    let response = router.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn an_unconfigured_server_answers_503_naming_the_setting() {
    let (status, body) = post(
        router(&DecisionConfig { provider: None }),
        &body(&["bug", "feature"]),
        true,
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let message = body["error"].as_str().unwrap_or_default();
    assert!(message.contains("DECISION_PROVIDER"), "{body}");
}

#[tokio::test]
async fn an_anonymous_caller_is_refused_before_the_provider_is_called() {
    let stub = JevStub::start(vec![Reply::answer("bug", 0.9, &[])]).await;

    let (status, _) = post(
        router(&configured(&stub)),
        &body(&["bug", "feature"]),
        false,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(stub.hits(), 0);
}

#[tokio::test]
async fn an_invalid_request_is_400_without_a_provider_call() {
    let stub = JevStub::start(vec![Reply::answer("bug", 0.9, &[])]).await;

    let (status, body) = post(router(&configured(&stub)), &body(&["bug"]), true).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(stub.hits(), 0);
}

#[tokio::test]
async fn a_confident_answer_is_returned_with_provider_model_and_band() {
    let stub = JevStub::start(vec![Reply::answer(
        "bug",
        0.93,
        &[("feature", 0.05), ("bug", 0.95)],
    )])
    .await;

    let (status, body) = post(router(&configured(&stub)), &body(&["bug", "feature"]), true).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "choice": "bug",
            "confidence": 0.93,
            "band": "act",
            "probabilities": [
                {"name": "bug", "probability": 0.95},
                {"name": "feature", "probability": 0.05},
            ],
            "provider": "jev",
            "model": "jev-1.13.0",
            "usage": {"inputTokens": 812, "outputTokens": 40},
        })
    );
}

// The CLI discards the caller's session on a 401, so the provider's own 401
// must surface as something else.
#[tokio::test]
async fn provider_failures_map_to_statuses_that_never_log_the_caller_out() {
    for (reply, expected) in [
        (Reply::status(401), StatusCode::BAD_GATEWAY),
        (Reply::status(403), StatusCode::BAD_GATEWAY),
        (Reply::status(422), StatusCode::BAD_REQUEST),
        (
            Reply::status(429).retry_after(0),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (Reply::status(500), StatusCode::BAD_GATEWAY),
        (Reply::answer("neither", 0.9, &[]), StatusCode::BAD_GATEWAY),
    ] {
        let stub = JevStub::start(vec![reply]).await;

        let (status, body) =
            post(router(&configured(&stub)), &body(&["bug", "feature"]), true).await;

        assert_eq!(status, expected, "{body}");
        let message = body["error"].as_str().unwrap_or_default();
        assert!(!message.contains("stub"), "provider body leaked: {message}");
    }
}
