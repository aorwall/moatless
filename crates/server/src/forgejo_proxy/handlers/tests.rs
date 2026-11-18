//! The proxy against a fake Forgejo that records what reached it.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::any;
use moatless_core::sandbox_jwt::StaticTokenGenerations;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer, MockGitHostTokenIssuer};
use moatless_types::attribution::Attribution;
use moatless_types::git_host::{GitHostAccount, RepositoryProvider};
use moatless_workspace_ports::{MockWorkspaceResolver, WorkspaceResolver};

use super::*;
use crate::proxy::caller::SandboxCallers;

const HOST: &str = "git.example.com";

/// What the fake Forgejo saw: method, path and query, headers and body.
type Seen = Arc<Mutex<Vec<(String, String, HeaderMap, String)>>>;

fn workspaces() -> Arc<dyn WorkspaceResolver> {
    let mut resolver = MockWorkspaceResolver::new();
    resolver.expect_placements().returning(|_| Ok(vec![]));
    resolver.expect_resolve().returning(|_| Ok(None));
    Arc::new(resolver)
}

/// An issuer holding `forgejo_pat` for [`HOST`] only.
fn issuer() -> Arc<dyn GitHostTokenIssuer> {
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens
        .expect_token()
        .returning(|actor, account| match account {
            GitHostAccount::Forgejo { host } if host == HOST => Ok("forgejo_pat".to_string()),
            _ => Err(GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider: RepositoryProvider::Forgejo,
                host: None,
            }),
        });
    Arc::new(tokens)
}

fn never_issues() -> Arc<dyn GitHostTokenIssuer> {
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens.expect_token().never();
    Arc::new(tokens)
}

/// A proxy whose sandbox lookups never connect: every test starts from a
/// resolved [`SandboxScope`].
fn proxy(tokens: Arc<dyn GitHostTokenIssuer>, base: &str) -> ForgejoProxy {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    ForgejoProxy::new(
        Arc::new(SandboxCallers::new(
            moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress(),
            Arc::new(StaticTokenGenerations::one("sandbox-1", 1, "user-1")),
            Some("forgejo-proxy-test-secret".to_string()),
        )),
        workspaces(),
        tokens,
        moatless_core::config::HttpConfig::default().git_proxy,
    )
    .with_base(base)
}

fn scope() -> SandboxScope {
    SandboxScope {
        attribution: Attribution {
            owner: Some("user-1".to_string()),
            ..Attribution::default()
        },
        sandbox: "sandbox-1".to_string(),
        token_user: "user-1".to_string(),
        user: "user-1".to_string(),
        workspace: "workspace-1".to_string(),
        run_as: None,
        service_account: None,
        task: None,
    }
}

async fn fake_forgejo(seen: Seen) -> String {
    let app = Router::new().fallback(any(
        move |method: Method, uri: Uri, headers: HeaderMap, body: String| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock()
                    .unwrap()
                    .push((method.to_string(), uri.to_string(), headers, body));
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({ "ok": true })),
                )
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn send(
    proxy: &ForgejoProxy,
    host: &str,
    method: Method,
    path: &str,
    headers: HeaderMap,
) -> Response {
    forward(
        proxy,
        &scope(),
        host,
        method,
        path,
        &headers,
        Body::from(r#"{"has_actions":true}"#),
    )
    .await
}

async fn message(response: Response) -> String {
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    json["message"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn a_request_reaches_forgejo_with_the_run_as_token_and_none_of_the_pods_credentials() {
    let seen: Seen = Arc::default();
    let base = fake_forgejo(Arc::clone(&seen)).await;
    let proxy = proxy(issuer(), &base);
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, "Bearer task-token".parse().unwrap());
    headers.insert(header::COOKIE, "i_like_gitea=abc".parse().unwrap());
    headers.insert("sudo", "root".parse().unwrap());
    headers.insert(USER_TOKEN, "task-token".parse().unwrap());
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());

    let response = send(
        &proxy,
        HOST,
        Method::PATCH,
        "/api/v1/repos/o/r?x=1",
        headers,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let seen = seen.lock().unwrap();
    let (method, uri, headers, body) = &seen[0];
    assert_eq!(method, "PATCH");
    assert_eq!(uri, "/api/v1/repos/o/r?x=1");
    assert_eq!(body, r#"{"has_actions":true}"#);
    assert_eq!(headers[header::AUTHORIZATION], "token forgejo_pat");
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert!(headers.get(header::COOKIE).is_none());
    assert!(headers.get("sudo").is_none());
    assert!(headers.get(USER_TOKEN).is_none());
}

#[tokio::test]
async fn a_refused_path_is_answered_before_any_token_is_resolved() {
    let seen: Seen = Arc::default();
    let base = fake_forgejo(Arc::clone(&seen)).await;
    let proxy = proxy(never_issues(), &base);

    for path in [
        "/api/v1/admin/users",
        "/api/v1/user?sudo=root",
        "/o/r.git/info/refs",
    ] {
        let response = send(&proxy, HOST, Method::GET, path, HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_host_the_run_as_identity_has_not_connected_says_what_to_connect() {
    let seen: Seen = Arc::default();
    let base = fake_forgejo(Arc::clone(&seen)).await;
    let proxy = proxy(issuer(), &base);

    let response = send(
        &proxy,
        "git.other.example",
        Method::GET,
        "/api/v1/user",
        HeaderMap::new(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        message(response)
            .await
            .contains("Connect Forgejo at git.other.example")
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn the_task_token_is_read_from_a_bearer_header_else_x_user_token() {
    let mut headers = HeaderMap::new();
    headers.insert(USER_TOKEN, "from-moat-api".parse().unwrap());
    assert_eq!(task_token(&headers), Some("from-moat-api"));
    headers.insert(header::AUTHORIZATION, "Bearer from-relay".parse().unwrap());
    assert_eq!(task_token(&headers), Some("from-relay"));
}

#[test]
fn the_upstream_path_is_everything_after_the_host() {
    let uri: Uri = "/forgejo/git.example.com/api/v1/repos/o/r?limit=1"
        .parse()
        .unwrap();
    assert_eq!(upstream_path(&uri).unwrap(), "/api/v1/repos/o/r?limit=1");
    let bare: Uri = "/forgejo/git.example.com".parse().unwrap();
    assert!(upstream_path(&bare).is_none());
}
