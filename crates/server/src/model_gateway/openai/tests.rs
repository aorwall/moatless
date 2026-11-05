//! Upstream choice, and the gateway against a fake ChatGPT backend.

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::routing::{get, post};
use moatless_core::config::ModelGatewayConfig;
use moatless_core::sandbox_jwt::{StaticTokenGenerations, generate_user_auth_token};
use moatless_ports::identity::{MockHarnessCredentials, MockSecretResolver};
use moatless_workspace_ports::{MockWorkspaceCredentialReader, MockWorkspaceResolver};

use super::*;
use crate::model_gateway::fakes::{FixedCodex, UnusedCodex};
use moatless_ports::sandbox::CodexCredentials;

const HUB: &str = "http://hub.moatless.svc.cluster.local:8317";
const HUB_V1: &str = "http://hub.moatless.svc.cluster.local:8317/v1";
const SECRET: &str = "model-gateway-test-secret";
const CHATGPT_BLOB: &str = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,
    "tokens":{"access_token":"chatgpt-access","refresh_token":"r","account_id":"acct-1"}}"#;

fn credentials(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn bases() -> ProviderBases {
    ProviderBases::default()
}

fn bearer(base: &str, token: &str) -> Upstream {
    Upstream {
        base: base.to_string(),
        credential: Credential::Bearer(token.to_string()),
    }
}

fn chatgpt() -> Upstream {
    Upstream {
        base: bases().chatgpt,
        credential: Credential::ChatGpt {
            access_token: "chatgpt-access".to_string(),
            account_id: Some("acct-1".to_string()),
        },
    }
}

#[test]
fn a_codex_login_outranks_an_api_key_which_outranks_the_hub() {
    let both = credentials(&[(CODEX_AUTH_JSON, CHATGPT_BLOB), (OPENAI_API_KEY, "sk-key")]);
    assert_eq!(
        resolve_upstream(&both, &bases(), Some(HUB), Some("hub-key")).unwrap(),
        chatgpt()
    );
    let key = credentials(&[(OPENAI_API_KEY, "sk-key")]);
    assert_eq!(
        resolve_upstream(&key, &bases(), Some(HUB), Some("hub-key")).unwrap(),
        bearer(&bases().openai, "sk-key")
    );
    assert_eq!(
        resolve_upstream(&HashMap::new(), &bases(), Some(HUB), Some("hub-key")).unwrap(),
        bearer(HUB_V1, "hub-key")
    );
    assert!(matches!(
        resolve_upstream(&HashMap::new(), &bases(), None, None),
        Err(GatewayError::NoCredential(_))
    ));
}

#[test]
fn an_api_key_auth_json_presents_its_key_to_openai() {
    for blob in [
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-blob","tokens":{"access_token":"x"}}"#,
        // No `auth_mode`: Codex's legacy reading takes the key when present.
        r#"{"OPENAI_API_KEY":"sk-blob","tokens":null}"#,
    ] {
        assert_eq!(
            resolve_upstream(
                &credentials(&[(CODEX_AUTH_JSON, blob)]),
                &bases(),
                None,
                None
            )
            .unwrap(),
            bearer(&bases().openai, "sk-blob"),
            "{blob}"
        );
    }
}

#[test]
fn an_unusable_auth_json_falls_back_to_the_api_key() {
    for blob in ["{invalid", r#"{"tokens":{}}"#] {
        let pairs = credentials(&[(CODEX_AUTH_JSON, blob), (OPENAI_API_KEY, "sk-key")]);
        assert_eq!(
            resolve_upstream(&pairs, &bases(), None, None).unwrap(),
            bearer(&bases().openai, "sk-key"),
            "{blob}"
        );
    }
}

#[test]
fn a_task_naming_the_hub_keeps_its_own_key_and_never_sends_its_login_there() {
    let own_key = credentials(&[
        (OPENAI_BASE_URL, &format!("{HUB_V1}/")),
        (OPENAI_API_KEY, "task-hub-key"),
    ]);
    assert_eq!(
        resolve_upstream(&own_key, &bases(), Some(HUB), Some("hub-key")).unwrap(),
        bearer(HUB_V1, "task-hub-key")
    );
    let login_only = credentials(&[(OPENAI_BASE_URL, HUB_V1), (CODEX_AUTH_JSON, CHATGPT_BLOB)]);
    assert_eq!(
        resolve_upstream(&login_only, &bases(), Some(HUB), Some("hub-key")).unwrap(),
        bearer(HUB_V1, "hub-key")
    );
}

#[test]
fn a_base_url_that_is_neither_openai_nor_the_hub_is_refused() {
    let elsewhere = credentials(&[
        (OPENAI_BASE_URL, "http://169.254.169.254/v1"),
        (OPENAI_API_KEY, "key"),
    ]);
    assert!(matches!(
        resolve_upstream(&elsewhere, &bases(), Some(HUB), Some("hub-key")),
        Err(GatewayError::NoCredential(_))
    ));
    // The hub's root without `/v1` is not where its OpenAI API is served.
    let hub_root = credentials(&[(OPENAI_BASE_URL, HUB), (OPENAI_API_KEY, "key")]);
    assert!(resolve_upstream(&hub_root, &bases(), Some(HUB), Some("hub-key")).is_err());
}

/// A gateway whose credential lookups resolve `codex` as the actor's login,
/// presenting a ChatGPT login to `chatgpt_base`.
fn gateway(codex: Option<&'static str>, chatgpt_base: &str) -> ModelGateway {
    gateway_with(Arc::new(FixedCodex(codex)), &[], chatgpt_base)
}

/// [`gateway`], with `codex` as the refresher and `scoped` as the actor's
/// secrets.
fn gateway_with(
    codex: Arc<dyn CodexCredentials>,
    scoped: &[(&str, &str)],
    chatgpt_base: &str,
) -> ModelGateway {
    let scoped = credentials(scoped);
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    let mut workspaces = MockWorkspaceResolver::new();
    workspaces.expect_resolve().returning(|_| Ok(None));
    let mut workspace_credentials = MockWorkspaceCredentialReader::new();
    workspace_credentials
        .expect_resolve_env()
        .returning(|_| Ok(HashMap::new()));
    let mut secrets = MockSecretResolver::new();
    secrets
        .expect_plaintext()
        .returning(move |_| Ok(scoped.clone()));
    let mut provider_tokens = MockHarnessCredentials::new();
    provider_tokens
        .expect_provider_token()
        .returning(|_, _| Ok(None));
    ModelGateway::new(
        Arc::new(crate::proxy::caller::SandboxCallers::new(
            moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress(),
            Arc::new(StaticTokenGenerations::one("sandbox-1", 2, "user-1")),
            Some(SECRET.to_string()),
        )),
        Arc::new(workspaces),
        Arc::new(workspace_credentials),
        Arc::new(secrets),
        Arc::new(provider_tokens),
        codex,
        ModelGatewayConfig::default(),
        ProviderBases {
            chatgpt: chatgpt_base.to_string(),
            ..ProviderBases::default()
        },
        moatless_core::config::HttpConfig::default().model_gateway,
    )
}

fn scope() -> super::super::service::SandboxScope {
    super::super::service::SandboxScope {
        sandbox: "sandbox-1".to_string(),
        token_user: "user-1".to_string(),
        user: "user-1".to_string(),
        workspace: "workspace-1".to_string(),
        run_as: None,
    }
}

#[tokio::test]
async fn the_refreshed_codex_login_is_what_the_gateway_presents() {
    let gateway = gateway(Some(CHATGPT_BLOB), &bases().chatgpt);
    let resolved = gateway
        .openai_credentials(&scope(), crate::model_gateway::auth_mode::AuthMode::Default)
        .await
        .unwrap();
    assert_eq!(
        resolve_upstream(&resolved, &gateway.bases, None, None).unwrap(),
        chatgpt()
    );
}

#[tokio::test]
async fn a_task_routed_to_the_hub_never_refreshes_its_codex_login() {
    let gateway = gateway_with(
        Arc::new(UnusedCodex),
        &[(OPENAI_BASE_URL, HUB_V1), (OPENAI_API_KEY, "hub-key")],
        &bases().chatgpt,
    );
    let resolved = gateway
        .openai_credentials(&scope(), crate::model_gateway::auth_mode::AuthMode::Default)
        .await
        .unwrap();
    assert_eq!(
        resolve_upstream(&resolved, &gateway.bases, Some(HUB), None).unwrap(),
        bearer(HUB_V1, "hub-key")
    );
}

#[tokio::test]
async fn api_key_only_requests_never_refresh_a_codex_login() {
    let gateway = gateway_with(Arc::new(UnusedCodex), &[], &bases().chatgpt);
    let resolved = gateway
        .openai_credentials(
            &scope(),
            crate::model_gateway::auth_mode::AuthMode::ApiKeyOrHub,
        )
        .await
        .unwrap();
    assert!(!resolved.contains_key(CODEX_AUTH_JSON));
}

#[test]
fn only_openais_own_base_url_still_reads_the_codex_login() {
    assert!(reads_codex_login(&credentials(&[]), &bases()));
    assert!(reads_codex_login(
        &credentials(&[(OPENAI_BASE_URL, "https://api.openai.com/v1/")]),
        &bases()
    ));
    assert!(!reads_codex_login(
        &credentials(&[(OPENAI_BASE_URL, HUB_V1)]),
        &bases()
    ));
}

async fn error_of(response: Response) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

/// What Codex sees after a mid-session generation bump, and reruns its auth
/// command on.
#[tokio::test]
async fn a_token_from_an_older_generation_is_a_401_in_openais_shape() {
    let stale = generate_user_auth_token("user-1", "user", "sandbox-1", 1, SECRET, 3600).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {stale}")).unwrap(),
    );
    let uri: Uri = "/llm/openai/v1/responses".parse().unwrap();

    let (status, body) = error_of(
        forward(
            &gateway(None, &bases().chatgpt),
            Method::POST,
            "/responses",
            &uri,
            &headers,
            Body::empty(),
        )
        .await,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"]["message"].is_string(), "{body}");
}

/// A ChatGPT backend that streams two SSE events to the login it expects and
/// answers anything else with a usage-limit 429 and its rate-limit header.
async fn fake_chatgpt() -> String {
    let app = Router::new()
        .route(
            "/backend-api/codex/responses",
            post(|headers: HeaderMap, body: String| async move {
                let presented = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
                let authorized = presented("authorization") == Some("Bearer chatgpt-access")
                    && presented("chatgpt-account-id") == Some("acct-1")
                    && presented("x-api-key").is_none()
                    && presented("originator") == Some("codex_cli_rs")
                    && body == r#"{"model":"gpt-5"}"#;
                if !authorized {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("x-codex-primary-used-percent", "100")],
                        "{\"error\":{\"type\":\"usage_limit_reached\"}}".to_string(),
                    )
                        .into_response();
                }
                let events = futures::stream::iter([
                    Ok::<_, std::io::Error>("event: response.created\ndata: {}\n\n"),
                    Ok("event: response.completed\ndata: {}\n\n"),
                ]);
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(events),
                )
                    .into_response()
            }),
        )
        .route(
            "/backend-api/codex/models",
            get(|headers: HeaderMap, uri: Uri| async move {
                assert_eq!(headers["authorization"], "Bearer chatgpt-access");
                assert!(headers.get(header::TRANSFER_ENCODING).is_none());
                axum::Json(serde_json::json!({ "query": uri.query() }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/backend-api/codex")
}

/// What a pod's Codex sends: the task token as its bearer, and an account
/// header it has no business choosing.
fn codex_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer task-token"),
    );
    headers.insert("x-api-key", HeaderValue::from_static("task-token"));
    headers.insert(
        "chatgpt-account-id",
        HeaderValue::from_static("someone-else"),
    );
    headers.insert("originator", HeaderValue::from_static("codex_cli_rs"));
    headers
}

/// What `forward` sends upstream for `access_token`'s ChatGPT login.
fn chatgpt_request(
    gateway: &ModelGateway,
    method: Method,
    url: &str,
    access_token: &str,
    body: &'static str,
) -> reqwest::RequestBuilder {
    let request = stream::upstream_request(
        &gateway.http,
        method,
        url,
        &codex_headers(),
        Body::from(body),
    );
    let credential = Credential::ChatGpt {
        access_token: access_token.to_string(),
        account_id: Some("acct-1".to_string()),
    };
    let (request, kind) = authorized_request(request, credential);
    assert_eq!(kind, "chatgpt");
    request
}

#[tokio::test]
async fn a_chatgpt_login_replaces_the_task_token_and_the_stream_passes_through() {
    let base = fake_chatgpt().await;
    let gateway = gateway(None, &base);
    let request = chatgpt_request(
        &gateway,
        Method::POST,
        &format!("{base}/responses"),
        "chatgpt-access",
        r#"{"model":"gpt-5"}"#,
    );

    let response = stream::relay(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        &body[..],
        b"event: response.created\ndata: {}\n\nevent: response.completed\ndata: {}\n\n"
    );
}

#[tokio::test]
async fn an_upstream_usage_limit_reaches_codex_unchanged() {
    let base = fake_chatgpt().await;
    let gateway = gateway(None, &base);
    let request = chatgpt_request(
        &gateway,
        Method::POST,
        &format!("{base}/responses"),
        "expired",
        r#"{"model":"gpt-5"}"#,
    );

    let response = stream::relay(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["x-codex-primary-used-percent"], "100");
}

#[tokio::test]
async fn the_model_list_is_a_bodiless_get_with_its_query() {
    let base = fake_chatgpt().await;
    let gateway = gateway(None, &base);
    let request = chatgpt_request(
        &gateway,
        Method::GET,
        &format!("{base}/models?client_version=0.164.0"),
        "chatgpt-access",
        "",
    );

    let response = stream::relay(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], br#"{"query":"client_version=0.164.0"}"#);
}

#[test]
fn api_only_ignores_a_subscription_and_uses_the_key_or_hub() {
    let mut values = credentials(&[(CODEX_AUTH_JSON, CHATGPT_BLOB), (OPENAI_API_KEY, "key")]);
    crate::model_gateway::auth_mode::AuthMode::ApiKeyOrHub.filter(&mut values);
    assert_eq!(
        resolve_upstream(&values, &bases(), Some(HUB), Some("hub-key")).unwrap(),
        bearer(&bases().openai, "key")
    );
    values.remove(OPENAI_API_KEY);
    assert_eq!(
        resolve_upstream(&values, &bases(), Some(HUB), Some("hub-key")).unwrap(),
        bearer(HUB_V1, "hub-key")
    );
    assert!(resolve_upstream(&values, &bases(), None, None).is_err());
}
