//! The proxy against a fake GitHub that accepts only the token currently live.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::any;
use moatless_core::sandbox_jwt::generate_user_auth_token;
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer, MockGitHostTokenIssuer};
use moatless_types::access_control::Scope;
use moatless_workspace_ports::{
    MockWorkspaceCredentialReader, MockWorkspaceResolver, RepoPlacement, WorkspaceCredentialReader,
    WorkspaceResolver,
};
use moatless_workspace_types::repository::{
    GitHostRepo, Repository, RepositoryKind, RepositoryProvider, RepositoryRemote,
};

use super::*;

const SECRET: &str = "github-proxy-test-secret";

/// The token the fake GitHub accepts, and the issuer hands out, at the moment.
type LiveToken = Arc<Mutex<String>>;

fn repository(provider: RepositoryProvider, host_repo: GitHostRepo) -> Repository {
    Repository {
        id: "repo-1".to_string(),
        name: "repo".to_string(),
        kind: RepositoryKind::Git,
        remote: Some(RepositoryRemote {
            url: format!("https://{}/owner/repo.git", host_repo.host()),
            uses_ssh: false,
            host_repo: Some(host_repo),
        }),
        default_branch: None,
        description: None,
        provider: Some(provider),
        scope: Scope::User,
        owner_user_id: None,
        created_by: None,
        deleted: false,
        created_at: None,
        updated_at: None,
    }
}

fn github_repository() -> Repository {
    repository(
        RepositoryProvider::Github,
        GitHostRepo::github("owner", "repo"),
    )
}

/// A workspace placing `primary` and pinning `pinned`.
fn workspaces(
    primary: Repository,
    pinned: HashMap<String, String>,
) -> (
    Arc<dyn WorkspaceResolver>,
    Arc<dyn WorkspaceCredentialReader>,
) {
    let mut resolver = MockWorkspaceResolver::new();
    let placement = RepoPlacement {
        repository_id: primary.id.clone(),
        branch: None,
        mount_name: None,
        is_primary: true,
        position: 0,
        repository: Some(primary),
    };
    resolver
        .expect_placements()
        .returning(move |_| Ok(vec![placement.clone()]));
    resolver.expect_resolve().returning(|_| Ok(None));
    let mut credentials = MockWorkspaceCredentialReader::new();
    credentials
        .expect_resolve_env()
        .returning(move |_| Ok(pinned.clone()));
    (Arc::new(resolver), Arc::new(credentials))
}

/// An issuer answering whatever token is live when it is asked.
fn issuer(live: &LiveToken) -> Arc<dyn GitHostTokenIssuer> {
    let live = Arc::clone(live);
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens
        .expect_token()
        .returning(move |_, _| Ok(live.lock().unwrap().clone()));
    Arc::new(tokens)
}

/// A proxy whose sandbox lookups never connect: the tests below either fail
/// before reaching the sandbox row or start from a resolved [`SandboxScope`].
fn proxy(
    (workspaces, credentials): (
        Arc<dyn WorkspaceResolver>,
        Arc<dyn WorkspaceCredentialReader>,
    ),
    tokens: Arc<dyn GitHostTokenIssuer>,
    api_base: &str,
) -> GitHubProxy {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    GitHubProxy::new(
        moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress(),
        workspaces,
        credentials,
        tokens,
        Some(SECRET.to_string()),
        api_base,
        api_base,
        moatless_core::config::HttpConfig::default().git_proxy,
    )
}

fn scope() -> SandboxScope {
    SandboxScope {
        sandbox: "sandbox-1".to_string(),
        user: "user-1".to_string(),
        workspace: "workspace-1".to_string(),
        run_as: None,
    }
}

/// A GitHub that answers `/user` only for the live token, and redirects
/// `/logs` to signed storage the way a run-log download does.
async fn fake_github(live: LiveToken) -> String {
    let app = Router::new()
        .route(
            "/user",
            any(move |headers: HeaderMap| {
                let live = Arc::clone(&live);
                async move {
                    let expected = format!("Bearer {}", live.lock().unwrap());
                    let presented = headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok());
                    if presented == Some(expected.as_str()) {
                        (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({ "login": "bot" })),
                        )
                    } else {
                        (
                            StatusCode::UNAUTHORIZED,
                            axum::Json(serde_json::json!({ "message": "Bad credentials" })),
                        )
                    }
                }
            }),
        )
        .route(
            "/logs",
            any(|| async {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "https://blob.example.com/logs.zip?sig=1")],
                )
            }),
        );
    serve(app).await
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn get(proxy: &GitHubProxy, path: &str) -> Response {
    forward(
        proxy,
        &scope(),
        GitHubUpstream::Api,
        Method::GET,
        path,
        &HeaderMap::new(),
        Body::empty(),
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

/// The 401 from 2026-10-02: an installation token written into the pod once
/// was still presented after it expired. Resolving per request presents the
/// one the issuer refreshed it to.
#[tokio::test]
async fn a_request_after_the_token_expired_presents_the_refreshed_one() {
    let live: LiveToken = Arc::new(Mutex::new("ghs_first".to_string()));
    let base = fake_github(Arc::clone(&live)).await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        issuer(&live),
        &base,
    );

    assert_eq!(get(&proxy, "/user").await.status(), StatusCode::OK);

    // The first token expires; GitHub refuses it and the issuer refreshes.
    *live.lock().unwrap() = "ghs_second".to_string();

    assert_eq!(get(&proxy, "/user").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_workspace_pinned_token_outranks_the_run_as_identity() {
    let live: LiveToken = Arc::new(Mutex::new("ghp_pinned".to_string()));
    let base = fake_github(Arc::clone(&live)).await;
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens.expect_token().never();
    let pinned = HashMap::from([(GITHUB_TOKEN.to_string(), "ghp_pinned".to_string())]);
    let proxy = proxy(
        workspaces(github_repository(), pinned),
        Arc::new(tokens),
        &base,
    );

    assert_eq!(get(&proxy, "/user").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_identity_that_has_not_connected_github_gets_a_message_not_a_502() {
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens.expect_token().returning(|actor, _| {
        Err(GitHostTokenError::NoCredential {
            actor: actor.clone(),
            provider: RepositoryProvider::Github,
            host: None,
        })
    });
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        Arc::new(tokens),
        "http://127.0.0.1:1",
    );

    let response = get(&proxy, "/user").await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        message(response).await.contains("no GitHub identity"),
        "the message gh prints must name the cause"
    );
}

#[tokio::test]
async fn a_workspace_with_no_github_repository_gets_a_message_not_a_502() {
    let forgejo = repository(
        RepositoryProvider::Forgejo,
        GitHostRepo::new(
            RepositoryProvider::Forgejo,
            "git.example.com",
            "owner",
            "repo",
        ),
    );
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens.expect_token().never();
    let proxy = proxy(
        workspaces(forgejo, HashMap::new()),
        Arc::new(tokens),
        "http://127.0.0.1:1",
    );

    let response = get(&proxy, "/user").await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(message(response).await.contains("no GitHub identity"));
}

/// The signed URL a log download redirects to is fetched by the pod, so the
/// token is never sent to a host this proxy did not choose.
#[tokio::test]
async fn a_redirect_is_returned_to_gh_rather_than_followed() {
    let live: LiveToken = Arc::new(Mutex::new("ghs_first".to_string()));
    let base = fake_github(Arc::clone(&live)).await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        issuer(&live),
        &base,
    );

    let response = get(&proxy, "/logs").await;

    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()[header::LOCATION],
        "https://blob.example.com/logs.zip?sig=1"
    );
}

#[tokio::test]
async fn a_token_this_deployment_did_not_sign_is_refused() {
    let live: LiveToken = Arc::new(Mutex::new("ghs_first".to_string()));
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        issuer(&live),
        "http://127.0.0.1:1",
    );
    let forged =
        generate_user_auth_token("user-1", "user", "sandbox-1", "another-secret", 600).unwrap();

    let refused = proxy.authenticate(Some(&forged)).await.unwrap_err();
    assert!(
        matches!(refused, GitHubProxyError::Unauthorized(_)),
        "{refused}"
    );

    let missing = proxy.authenticate(None).await.unwrap_err();
    assert!(
        matches!(missing, GitHubProxyError::Unauthorized(_)),
        "{missing}"
    );
}

/// Through the route: a refused token answers 401 before any GitHub token is
/// resolved.
#[tokio::test]
async fn the_route_refuses_a_request_without_a_valid_task_token() {
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens.expect_token().never();
    let proxy = Arc::new(proxy(
        workspaces(github_repository(), HashMap::new()),
        Arc::new(tokens),
        "http://127.0.0.1:1",
    ));
    let base = serve(
        Router::new()
            .route("/github/{*rest}", any(super::proxy))
            .with_state(proxy),
    )
    .await;

    let response = client()
        .get(format!("{base}/github/api/user"))
        .bearer_auth("not-a-task-token")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[test]
fn the_first_segment_names_the_upstream_and_the_rest_is_forwarded() {
    let split = |uri: &str| split_upstream(&uri.parse::<Uri>().unwrap());

    assert_eq!(
        split("/github/api/repos/o/r/pulls?state=open"),
        Some((
            GitHubUpstream::Api,
            "/repos/o/r/pulls?state=open".to_string()
        ))
    );
    assert_eq!(
        split("/github/api/"),
        Some((GitHubUpstream::Api, "/".to_string()))
    );
    assert_eq!(
        split("/github/api"),
        Some((GitHubUpstream::Api, "/".to_string()))
    );
    assert_eq!(
        split("/github/uploads/repos/o/r/releases/1/assets?name=a.zip"),
        Some((
            GitHubUpstream::Uploads,
            "/repos/o/r/releases/1/assets?name=a.zip".to_string()
        ))
    );
    assert_eq!(split("/github/evil.example.com/steal"), None);
}

#[test]
fn neither_the_task_token_nor_a_hop_by_hop_header_is_forwarded() {
    assert!(!forwards(&header::HOST));
    assert!(!forwards(&header::TRANSFER_ENCODING));
    assert!(forwards(&header::ACCEPT));
    assert!(forwards(&HeaderName::from_static("x-github-api-version")));
    assert_eq!(
        bearer_token(&HeaderMap::from_iter([(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer task-jwt"),
        )])),
        Some("task-jwt")
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "a test client with a short bound"
)]
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
}
