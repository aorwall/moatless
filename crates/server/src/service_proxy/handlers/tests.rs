//! The proxy against a fake upstream that accepts only the brokered secret.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::any;
use moatless_core::sandbox_jwt::StaticTokenGenerations;
use moatless_core::service_proxies::ServiceProxies;
use moatless_core::workspace_identity_grants::WorkspaceIdentityGrant;
use moatless_ports::identity::MockSecretResolver;
use moatless_workspace_ports::{
    MockWorkspaceCredentialReader, MockWorkspaceResolver, ResolvedWorkspace,
};

use super::*;
use crate::proxy::caller::SandboxCallers;
use crate::service_proxy::access::{GrantReader, ServiceAccess};

const SECRET: &str = "service-proxy-test-secret";
const GRAFANA_SECRET: &str = "glsa-test-value";

struct NoGrant;

#[async_trait::async_trait]
impl GrantReader for NoGrant {
    async fn grant_for(&self, _: &str) -> Result<Option<WorkspaceIdentityGrant>, String> {
        Ok(None)
    }
}

/// What the fake upstream saw: path and query, and the `Authorization` header.
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

async fn fake_upstream(seen: Seen) -> String {
    let app = Router::new()
        .route(
            "/gateway-timeout",
            any(|| async { StatusCode::GATEWAY_TIMEOUT }),
        )
        .route(
            "/redirect",
            any(|| async {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "https://elsewhere.example.com/x")],
                )
            }),
        )
        .fallback(any(move |uri: Uri, headers: HeaderMap| {
            let seen = Arc::clone(&seen);
            async move {
                let authorization = headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                seen.lock().unwrap().push((uri.to_string(), authorization));
                "streamed answer"
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A `grafana` (availability `all`, bearer), a `keyed` (query-injected) and a
/// `gated` (availability `granted`) service, all pointed at `upstream`.
fn services(upstream: &str) -> Arc<ServiceProxies> {
    let json = serde_json::json!({
        "grafana": {
            "upstream": "https://grafana.example.com",
            "credential": { "secret": "GRAFANA_TOKEN", "inject": { "style": "bearer" } },
            "availability": "all",
            "limits": { "requestBodyBytes": 64 },
            "rules": [
                { "methods": ["GET"], "path": "/api/health" },
                { "methods": ["GET"], "path": "/redirect" },
                { "methods": ["GET"], "path": "/gateway-timeout" },
                { "methods": ["GET"], "path": "/api/prod/**", "serviceAccounts": ["moatless-ops"] },
                { "methods": ["POST"], "path": "/api/ds/query",
                  "body": { "pointer": "/kind", "oneOf": ["read"] } }
            ]
        },
        "keyed": {
            "upstream": "https://keyed.example.com",
            "credential": { "secret": "GRAFANA_TOKEN", "inject": { "style": "query", "param": "key" } },
            "availability": "all",
            "rules": [{ "methods": ["GET"], "path": "/**" }]
        },
        "gated": {
            "upstream": "https://gated.example.com",
            "credential": { "secret": "GATED_TOKEN", "inject": { "style": "bearer" } },
            "availability": "granted",
            "rules": [{ "methods": ["GET"], "path": "/**" }]
        }
    });
    // Two services sharing a secret is refused by the parser, so each is parsed
    // on its own before both are pointed at the local upstream.
    let parsed: Vec<_> = ["grafana", "keyed", "gated"]
        .iter()
        .flat_map(|name| {
            let one = serde_json::json!({ *name: json[*name].clone() });
            ServiceProxies::parse(Some(&one.to_string()))
                .unwrap()
                .iter()
                .cloned()
                .collect::<Vec<_>>()
        })
        .map(|mut service| {
            service.upstream = upstream.to_string();
            service
        })
        .collect();
    Arc::new(parsed.into_iter().collect())
}

fn gateway(upstream: &str, opted_in: &[&str], secrets: &[(&str, &str)]) -> ServiceProxyGateway {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    let opted_in: Vec<String> = opted_in.iter().map(|s| (*s).to_string()).collect();
    let mut workspaces = MockWorkspaceResolver::new();
    workspaces.expect_resolve().returning(move |id| {
        Ok(Some(ResolvedWorkspace {
            id: id.to_string(),
            name: "ws".to_string(),
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            resources: None,
            language_servers: None,
            network_profile: None,
            node_selector: None,
            services: opted_in.clone(),
            run_as_user_id: None,
            deleted: false,
        }))
    });
    let workspaces: Arc<dyn moatless_workspace_ports::WorkspaceResolver> = Arc::new(workspaces);
    let mut credentials = MockWorkspaceCredentialReader::new();
    credentials
        .expect_resolve_env()
        .returning(|_| Ok(HashMap::new()));
    let scoped: HashMap<String, String> = secrets
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let mut resolver = MockSecretResolver::new();
    resolver
        .expect_plaintext()
        .returning(move |_| Ok(scoped.clone()));
    ServiceProxyGateway::new(
        Arc::new(SandboxCallers::new(
            moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress(),
            Arc::new(StaticTokenGenerations::one("sandbox-1", 1, "user-1")),
            Some(SECRET.to_string()),
        )),
        Arc::new(ServiceAccess::new(
            services(upstream),
            false,
            workspaces.clone(),
            Arc::new(NoGrant),
        )),
        workspaces,
        Arc::new(credentials),
        Arc::new(resolver),
        moatless_core::config::HttpConfig::default().github_proxy,
    )
}

fn scope() -> SandboxScope {
    SandboxScope {
        attribution: Default::default(),
        sandbox: "sandbox-1".to_string(),
        token_user: "user-1".to_string(),
        user: "user-1".to_string(),
        workspace: "workspace-1".to_string(),
        run_as: None,
        service_account: None,
        task: None,
    }
}

async fn send(
    gateway: &ServiceProxyGateway,
    name: &str,
    method: Method,
    path: &str,
    body: &str,
) -> Response {
    send_as(gateway, &scope(), name, method, path, body).await
}

async fn send_as(
    gateway: &ServiceProxyGateway,
    scope: &SandboxScope,
    name: &str,
    method: Method,
    path: &str,
    body: &str,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        "Bearer moatless-brokered".parse().unwrap(),
    );
    match forward(
        gateway,
        scope,
        name,
        method,
        path,
        &headers,
        Body::from(body.to_string()),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn text(response: Response) -> String {
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn an_allowed_request_reaches_the_upstream_with_the_secret_in_place_of_the_placeholder() {
    let seen = Seen::default();
    let upstream = fake_upstream(Arc::clone(&seen)).await;
    let gateway = gateway(
        &upstream,
        &["grafana"],
        &[("GRAFANA_TOKEN", GRAFANA_SECRET)],
    );

    let response = send(&gateway, "grafana", Method::GET, "/api/health", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert_eq!(body, "streamed answer");
    assert!(!body.contains(GRAFANA_SECRET));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            "/api/health".to_string(),
            Some(format!("Bearer {GRAFANA_SECRET}"))
        )]
    );
}

#[tokio::test]
async fn a_query_style_secret_replaces_the_callers_value() {
    let seen = Seen::default();
    let upstream = fake_upstream(Arc::clone(&seen)).await;
    let gateway = gateway(&upstream, &["keyed"], &[("GRAFANA_TOKEN", GRAFANA_SECRET)]);

    let response = send(&gateway, "keyed", Method::GET, "/x?key=placeholder&q=1", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    let (path, authorization) = seen.lock().unwrap()[0].clone();
    assert_eq!(path, format!("/x?q=1&key={GRAFANA_SECRET}"));
    assert_eq!(authorization, None);
}

#[tokio::test]
async fn a_request_no_rule_allows_is_refused_before_the_upstream() {
    let seen = Seen::default();
    let upstream = fake_upstream(Arc::clone(&seen)).await;
    let gateway = gateway(
        &upstream,
        &["grafana"],
        &[("GRAFANA_TOKEN", GRAFANA_SECRET)],
    );

    for (method, path, body) in [
        (Method::DELETE, "/api/health", ""),
        (Method::GET, "/api/admin/users", ""),
        (Method::POST, "/api/ds/query", r#"{"kind":"write"}"#),
    ] {
        let response = send(&gateway, "grafana", method.clone(), path, body).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }
    let allowed = send(
        &gateway,
        "grafana",
        Method::POST,
        "/api/ds/query",
        r#"{"kind":"read"}"#,
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_rule_limited_to_service_accounts_forwards_only_for_their_pods() {
    let seen = Seen::default();
    let upstream = fake_upstream(Arc::clone(&seen)).await;
    let gateway = gateway(
        &upstream,
        &["grafana"],
        &[("GRAFANA_TOKEN", GRAFANA_SECRET)],
    );
    let running_as = |account: Option<&str>| SandboxScope {
        service_account: account.map(str::to_string),
        ..scope()
    };

    for account in [None, Some("moatless-user")] {
        let response = send_as(
            &gateway,
            &running_as(account),
            "grafana",
            Method::GET,
            "/api/prod/x",
            "",
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{account:?}");
    }
    let ops = send_as(
        &gateway,
        &running_as(Some("moatless-ops")),
        "grafana",
        Method::GET,
        "/api/prod/x",
        "",
    )
    .await;
    assert_eq!(ops.status(), StatusCode::OK);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_encoded_traversal_is_a_400() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(
        &upstream,
        &["grafana"],
        &[("GRAFANA_TOKEN", GRAFANA_SECRET)],
    );
    let response = send(&gateway, "grafana", Method::GET, "/api/%2e%2e/admin", "").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_body_over_the_limit_is_a_413() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(
        &upstream,
        &["grafana"],
        &[("GRAFANA_TOKEN", GRAFANA_SECRET)],
    );
    let response = send(
        &gateway,
        "grafana",
        Method::POST,
        "/api/ds/query",
        &"x".repeat(65),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn an_ungranted_workspace_is_told_the_setting() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(&upstream, &["gated"], &[("GATED_TOKEN", GRAFANA_SECRET)]);
    let response = send(&gateway, "gated", Method::GET, "/api/health", "").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = text(response).await;
    assert!(
        body.contains("workspaces.json") && body.contains("WORKSPACE_IDENTITY_GRANTS"),
        "{body}"
    );
}

#[tokio::test]
async fn an_all_service_needs_no_opt_in() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(&upstream, &[], &[("GRAFANA_TOKEN", GRAFANA_SECRET)]);
    let response = send(&gateway, "grafana", Method::GET, "/api/health", "").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn no_resolvable_secret_is_a_424_naming_the_key_and_the_identity() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(&upstream, &["grafana"], &[]);
    let response = send(&gateway, "grafana", Method::GET, "/api/health", "").await;
    assert_eq!(response.status(), StatusCode::FAILED_DEPENDENCY);
    let body = text(response).await;
    assert!(
        body.contains("GRAFANA_TOKEN") && body.contains("user-1"),
        "{body}"
    );
}

#[tokio::test]
async fn an_upstream_5xx_is_relayed_marked_as_the_upstreams_failure() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(
        &upstream,
        &["grafana"],
        &[("GRAFANA_TOKEN", GRAFANA_SECRET)],
    );
    let failed = send(&gateway, "grafana", Method::GET, "/gateway-timeout", "").await;
    assert_eq!(failed.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(
        failed
            .extensions()
            .get::<RelayedUpstreamFailure>()
            .is_some()
    );

    let ok = send(&gateway, "grafana", Method::GET, "/api/health", "").await;
    assert!(ok.extensions().get::<RelayedUpstreamFailure>().is_none());
}

#[tokio::test]
async fn a_redirect_is_returned_rather_than_followed() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(
        &upstream,
        &["grafana"],
        &[("GRAFANA_TOKEN", GRAFANA_SECRET)],
    );
    let response = send(&gateway, "grafana", Method::GET, "/redirect", "").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()[header::LOCATION],
        "https://elsewhere.example.com/x"
    );
}

#[tokio::test]
async fn an_unknown_service_is_a_404_and_a_bad_task_token_a_401() {
    let upstream = fake_upstream(Seen::default()).await;
    let gateway = gateway(&upstream, &["grafana"], &[]);
    let unknown = send(&gateway, "jira", Method::GET, "/", "").await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let uri: Uri = "/proxy/grafana/api/health".parse().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, "Bearer not-a-jwt".parse().unwrap());
    let refused = proxy(
        State(Arc::new(gateway)),
        Method::GET,
        uri,
        headers,
        Body::empty(),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn a_proxy_uri_splits_into_the_service_and_its_path() {
    let uri: Uri = "/proxy/grafana/api/ds/query?from=now-1h".parse().unwrap();
    assert_eq!(
        split(&uri),
        (
            "grafana".to_string(),
            "/api/ds/query?from=now-1h".to_string()
        )
    );
    let bare: Uri = "/proxy/grafana".parse().unwrap();
    assert_eq!(split(&bare), ("grafana".to_string(), "/".to_string()));
}
