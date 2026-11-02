//! Upstream choice, and the gateway against a fake Anthropic.

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::routing::post;
use moatless_core::config::ModelGatewayConfig;
use moatless_core::sandbox_jwt::{StaticTokenGenerations, generate_user_auth_token};
use moatless_ports::identity::{MockHarnessCredentials, MockSecretResolver};
use moatless_workspace_ports::{MockWorkspaceCredentialReader, MockWorkspaceResolver};

use super::*;
use crate::model_gateway::ProviderBases;
use crate::model_gateway::fakes::FixedCodex;

const ANTHROPIC: &str = "https://api.anthropic.com";
const HUB: &str = "http://hub.moatless.svc.cluster.local:8317";
const SECRET: &str = "model-gateway-test-secret";

fn credentials(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn upstream(base: &str, credential: Credential) -> Upstream {
    Upstream {
        base: base.to_string(),
        credential,
    }
}

#[test]
fn an_auth_token_outranks_an_api_key_which_outranks_oauth() {
    let all = credentials(&[
        (ANTHROPIC_AUTH_TOKEN, "bearer"),
        (ANTHROPIC_API_KEY, "key"),
        (CLAUDE_CODE_OAUTH_TOKEN, "oauth"),
    ]);
    assert_eq!(
        resolve_upstream(&all, ANTHROPIC, None, None).unwrap(),
        upstream(ANTHROPIC, Credential::Bearer("bearer".into()))
    );
    let key = credentials(&[
        (ANTHROPIC_API_KEY, "key"),
        (CLAUDE_CODE_OAUTH_TOKEN, "oauth"),
    ]);
    assert_eq!(
        resolve_upstream(&key, ANTHROPIC, Some(HUB), Some("hub-key")).unwrap(),
        upstream(ANTHROPIC, Credential::ApiKey("key".into()))
    );
    let oauth = credentials(&[(CLAUDE_CODE_OAUTH_TOKEN, "oauth")]);
    assert_eq!(
        resolve_upstream(&oauth, ANTHROPIC, Some(HUB), Some("hub-key")).unwrap(),
        upstream(ANTHROPIC, Credential::OAuth("oauth".into()))
    );
}

#[test]
fn a_task_with_no_credential_falls_back_to_the_hub() {
    assert_eq!(
        resolve_upstream(&HashMap::new(), ANTHROPIC, Some(HUB), Some("hub-key")).unwrap(),
        upstream(HUB, Credential::Bearer("hub-key".into()))
    );
    assert!(matches!(
        resolve_upstream(&HashMap::new(), ANTHROPIC, None, None),
        Err(GatewayError::NoCredential(_))
    ));
}

#[test]
fn a_task_naming_the_hub_keeps_its_own_key_and_never_sends_oauth_there() {
    let own_key = credentials(&[
        (ANTHROPIC_BASE_URL, &format!("{HUB}/")),
        (ANTHROPIC_API_KEY, "task-hub-key"),
    ]);
    assert_eq!(
        resolve_upstream(&own_key, ANTHROPIC, Some(HUB), Some("hub-key")).unwrap(),
        upstream(HUB, Credential::ApiKey("task-hub-key".into()))
    );
    let oauth_only = credentials(&[
        (ANTHROPIC_BASE_URL, HUB),
        (CLAUDE_CODE_OAUTH_TOKEN, "oauth"),
    ]);
    assert_eq!(
        resolve_upstream(&oauth_only, ANTHROPIC, Some(HUB), Some("hub-key")).unwrap(),
        upstream(HUB, Credential::Bearer("hub-key".into()))
    );
    let configured_with_slash = format!("{HUB}/");
    assert_eq!(
        resolve_upstream(&own_key, ANTHROPIC, Some(&configured_with_slash), None).unwrap(),
        upstream(HUB, Credential::ApiKey("task-hub-key".into()))
    );
    assert!(matches!(
        resolve_upstream(&oauth_only, ANTHROPIC, Some(HUB), None),
        Err(GatewayError::NoCredential(_))
    ));
}

#[test]
fn a_base_url_that_is_neither_anthropic_nor_the_hub_is_refused() {
    let elsewhere = credentials(&[
        (ANTHROPIC_BASE_URL, "http://169.254.169.254"),
        (ANTHROPIC_API_KEY, "key"),
    ]);
    assert!(matches!(
        resolve_upstream(&elsewhere, ANTHROPIC, Some(HUB), Some("hub-key")),
        Err(GatewayError::NoCredential(_))
    ));
}

#[test]
fn the_oauth_beta_joins_the_callers_betas_once() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "anthropic-beta",
        HeaderValue::from_static("claude-code-20250219, interleaved-thinking-2025-05-14"),
    );
    assert_eq!(
        with_oauth_beta(&headers),
        "claude-code-20250219,interleaved-thinking-2025-05-14,oauth-2025-04-20"
    );
    headers.insert("anthropic-beta", HeaderValue::from_static(OAUTH_BETA));
    assert_eq!(with_oauth_beta(&headers), OAUTH_BETA);
}

#[tokio::test]
async fn an_oauth_request_sends_one_merged_beta_header_and_no_task_token() {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", HeaderValue::from_static("task-token"));
    headers.insert(
        "anthropic-beta",
        HeaderValue::from_static("claude-code-20250219"),
    );
    let (request, kind) = authorized_request(
        &gateway().http,
        "https://api.anthropic.com/v1/messages",
        &headers,
        Body::empty(),
        Credential::OAuth("oauth".into()),
    );
    let request = request.build().unwrap();

    assert_eq!(kind, "oauth");
    let betas: Vec<_> = request.headers().get_all("anthropic-beta").iter().collect();
    assert_eq!(betas, ["claude-code-20250219,oauth-2025-04-20"]);
    assert_eq!(request.headers()["authorization"], "Bearer oauth");
    assert!(request.headers().get("x-api-key").is_none());
}

#[tokio::test]
async fn the_callers_session_and_network_path_stay_at_the_backend() {
    let mut headers = HeaderMap::new();
    for name in ["cookie", "x-forwarded-for", "forwarded", "authorization"] {
        headers.insert(name, HeaderValue::from_static("caller"));
    }
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

    let request = stream::upstream_request(
        &gateway().http,
        Method::POST,
        "https://api.anthropic.com/v1/messages",
        &headers,
        Body::empty(),
    )
    .build()
    .unwrap();

    let sent: Vec<_> = request.headers().keys().map(HeaderName::as_str).collect();
    assert_eq!(sent, ["anthropic-version"]);
}

/// A gateway whose sandbox lookups never connect: the tests below fail before
/// reaching the sandbox row.
fn gateway() -> ModelGateway {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    ModelGateway::new(
        moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress(),
        Arc::new(StaticTokenGenerations::one("sandbox-1", 2, "user-1")),
        Arc::new(MockWorkspaceResolver::new()),
        Arc::new(MockWorkspaceCredentialReader::new()),
        Arc::new(MockSecretResolver::new()),
        Arc::new(MockHarnessCredentials::new()),
        Arc::new(FixedCodex(None)),
        Some(SECRET.to_string()),
        ModelGatewayConfig::default(),
        ProviderBases::default(),
        moatless_core::config::HttpConfig::default().model_gateway,
    )
}

async fn error_of(response: Response) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn a_token_from_an_older_generation_is_refused_in_anthropics_shape() {
    let stale = generate_user_auth_token("user-1", "user", "sandbox-1", 1, SECRET, 3600).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", HeaderValue::from_str(&stale).unwrap());
    let uri: Uri = "/llm/anthropic/v1/messages?beta=true".parse().unwrap();

    let (status, body) =
        error_of(forward(&gateway(), "/v1/messages", &uri, &headers, Body::empty()).await).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "authentication_error");
}

#[tokio::test]
async fn a_request_with_no_token_is_refused() {
    let uri: Uri = "/llm/anthropic/v1/messages".parse().unwrap();
    let (status, _) = error_of(
        forward(
            &gateway(),
            "/v1/messages",
            &uri,
            &HeaderMap::new(),
            Body::empty(),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// An Anthropic that rejects with a usage limit, the way the pod's classifier
/// expects to see it, and checks what the gateway sent.
async fn fake_anthropic() -> String {
    let app = Router::new().route(
        "/v1/messages",
        post(|headers: HeaderMap, body: String| async move {
            assert_eq!(headers.get("x-api-key").unwrap(), "upstream-key");
            assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
            assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
            assert_eq!(body, "{\"model\":\"claude\"}");
            (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    ("anthropic-ratelimit-unified-status", "rejected"),
                    ("retry-after", "120"),
                    ("content-type", "text/event-stream"),
                ],
                "event: error\ndata: {\"type\":\"error\"}\n\n",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn the_upstream_status_headers_and_stream_pass_through() {
    let base = fake_anthropic().await;
    let gateway = gateway();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", HeaderValue::from_static("task-token"));
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    let request = stream::upstream_request(
        &gateway.http,
        Method::POST,
        &format!("{base}/v1/messages"),
        &headers,
        Body::from("{\"model\":\"claude\"}"),
    )
    .header("x-api-key", "upstream-key");

    let response = stream::relay(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers()["anthropic-ratelimit-unified-status"],
        "rejected"
    );
    assert_eq!(response.headers()["retry-after"], "120");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"event: error\ndata: {\"type\":\"error\"}\n\n");
}

#[test]
fn api_only_ignores_oauth_and_bearer_credentials() {
    let mut values = credentials(&[
        (CLAUDE_CODE_OAUTH_TOKEN, "oauth"),
        (ANTHROPIC_AUTH_TOKEN, "bearer"),
        (ANTHROPIC_API_KEY, "key"),
    ]);
    crate::model_gateway::auth_mode::AuthMode::ApiKeyOrHub.filter(&mut values);
    assert_eq!(
        resolve_upstream(&values, ANTHROPIC, None, None).unwrap(),
        upstream(ANTHROPIC, Credential::ApiKey("key".into()))
    );
    values.remove(ANTHROPIC_API_KEY);
    assert!(resolve_upstream(&values, ANTHROPIC, None, None).is_err());
    assert_eq!(
        resolve_upstream(&values, ANTHROPIC, Some(HUB), Some("hub-key")).unwrap(),
        upstream(HUB, Credential::Bearer("hub-key".into()))
    );
}
