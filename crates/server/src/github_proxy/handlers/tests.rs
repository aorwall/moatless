//! The proxy against a fake GitHub that accepts only the token currently live.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::any;
use moatless_core::sandbox_jwt::{StaticTokenGenerations, generate_user_auth_token};
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_ports::identity::{
    AppWriteToken, GitHostTokenError, GitHostTokenIssuer, MockGitHostTokenIssuer,
};
use moatless_types::access_control::Scope;
use moatless_types::attribution::Attribution;
use moatless_types::git_host::GitHostAccount;
use moatless_workspace_ports::{
    MockWorkspaceCredentialReader, MockWorkspaceResolver, RepoPlacement, WorkspaceCredentialReader,
    WorkspaceResolver,
};
use moatless_workspace_types::repository::{
    GitHostRepo, Repository, RepositoryKind, RepositoryProvider, RepositoryRemote,
};

use moatless_core::proxy_rules::ProxyPolicyMode;
use moatless_core::workspace_identity_grants::{
    GitHubCapability, GitHubPolicyGrant, WorkspaceIdentityGrant,
};

use super::*;
use crate::proxy::caller::SandboxCallers;

const SECRET: &str = "github-proxy-test-secret";

/// The generation `sandbox-1`'s tokens are at in these tests.
const GENERATION: i64 = 2;

/// The token the fake GitHub accepts, and the issuer hands out, at the moment.
type LiveToken = Arc<Mutex<String>>;

fn repository(provider: RepositoryProvider, host_repo: GitHostRepo) -> Repository {
    Repository {
        id: "repo-1".to_string(),
        name: "repo".to_string(),
        kind: RepositoryKind::Git,
        remote: Some(RepositoryRemote {
            url: format!("https://{}/owner/repo.git", host_repo.host()),
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
    tokens
        .expect_github_app_write()
        .returning(|_, _, _| Ok(None));
    tokens
        .expect_credited_github_login()
        .returning(|_| Ok(None));
    Arc::new(tokens)
}

/// A proxy whose sandbox lookups never connect: the tests below either fail
/// before reaching the sandbox row or start from a resolved [`SandboxScope`].
fn proxy(
    workspaces: (
        Arc<dyn WorkspaceResolver>,
        Arc<dyn WorkspaceCredentialReader>,
    ),
    tokens: Arc<dyn GitHostTokenIssuer>,
    api_base: &str,
) -> GitHubProxy {
    proxy_under(ProxyPolicyMode::Log, None, workspaces, tokens, api_base)
}

/// [`proxy`] under `mode`, with `grant` as every Workspace's grant.
fn proxy_under(
    mode: ProxyPolicyMode,
    grant: Option<WorkspaceIdentityGrant>,
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
        Arc::new(SandboxCallers::new(
            moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress(),
            Arc::new(StaticTokenGenerations::one(
                "sandbox-1",
                GENERATION,
                "user-1",
            )),
            Some(SECRET.to_string()),
        )),
        workspaces.clone(),
        credentials,
        tokens,
        api_base,
        api_base,
        crate::github_policy::test_support::policy(mode, workspaces, grant),
        moatless_core::config::HttpConfig::default().git_proxy,
    )
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
        task: None,
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
    let forged = generate_user_auth_token(
        "user-1",
        "user",
        "sandbox-1",
        GENERATION,
        "another-secret",
        600,
    )
    .unwrap();

    let refused = proxy.authenticate(Some(&forged)).await.unwrap_err();
    assert!(
        matches!(refused, GitHubProxyError::Unauthorized(_)),
        "{refused}"
    );

    let superseded =
        generate_user_auth_token("user-1", "user", "sandbox-1", GENERATION - 1, SECRET, 600)
            .unwrap();
    let refused = proxy.authenticate(Some(&superseded)).await.unwrap_err();
    assert!(
        matches!(refused, GitHubProxyError::Unauthorized(_)),
        "{refused}"
    );
    // A current one passes the check and reaches the sandbox row, which this
    // proxy's pool cannot read.
    let current =
        generate_user_auth_token("user-1", "user", "sandbox-1", GENERATION, SECRET, 600).unwrap();
    let past_the_check = proxy.authenticate(Some(&current)).await.unwrap_err();
    assert!(
        matches!(past_the_check, GitHubProxyError::Upstream(_)),
        "{past_the_check}"
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

/// An issuer minting `ghs_{repository}` for every repository but
/// `refused`, which the identity has no role on.
fn per_repository_issuer(refused: &'static str) -> Arc<dyn GitHostTokenIssuer> {
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens.expect_token().returning(move |actor, account| {
        let GitHostAccount::Github {
            repository: Some(repository),
            ..
        } = account
        else {
            panic!("a GitHub token is always minted for one repository: {account:?}");
        };
        if repository == refused {
            return Err(GitHostTokenError::Forbidden {
                actor: actor.clone(),
                account: Box::new(account.clone()),
                source: "no role".into(),
            });
        }
        Ok(format!("ghs_{repository}"))
    });
    tokens
        .expect_github_app_write()
        .returning(|_, _, _| Ok(None));
    tokens
        .expect_credited_github_login()
        .returning(|_| Ok(None));
    Arc::new(tokens)
}

/// A GitHub answering every request with the token it was presented.
async fn echoing_github() -> String {
    serve(Router::new().fallback(any(|headers: HeaderMap| async move {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        axum::Json(serde_json::json!({ "message": presented }))
    })))
    .await
}

async fn presented(proxy: &GitHubProxy, method: Method, path: &str, body: &str) -> String {
    message(
        forward(
            proxy,
            &scope(),
            GitHubUpstream::Api,
            method,
            path,
            &HeaderMap::new(),
            Body::from(body.to_string()),
        )
        .await,
    )
    .await
}

/// The 404s on sibling repositories from 2026-10-26: a capped identity's
/// token for the primary repository cannot see any other.
#[tokio::test]
async fn a_request_naming_a_repository_presents_that_repositorys_token() {
    let base = echoing_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        per_repository_issuer("private"),
        &base,
    );

    assert_eq!(
        presented(
            &proxy,
            Method::GET,
            "/repos/owner/sibling/actions/runs?x=1",
            ""
        )
        .await,
        "Bearer ghs_sibling"
    );
    let query = r#"{"query":"q","variables":{"owner":"owner","repo":"sibling","pr_number":1}}"#;
    assert_eq!(
        presented(&proxy, Method::POST, "/graphql", query).await,
        "Bearer ghs_sibling"
    );
    let mutation = r#"{"query":"m","variables":{"input":{"pullRequestId":"PR_kw"}}}"#;
    assert_eq!(
        presented(&proxy, Method::POST, "/graphql", mutation).await,
        "Bearer ghs_repo",
        "a request naming no repository presents the primary one's token"
    );
    assert_eq!(
        presented(&proxy, Method::GET, "/repos/owner/private", "").await,
        "Bearer ghs_repo",
        "a repository the identity has no role on falls back to the primary one's token"
    );
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

async fn send(proxy: &GitHubProxy, method: Method, path: &str, body: &str) -> Response {
    forward(
        proxy,
        &scope(),
        GitHubUpstream::Api,
        method,
        path,
        &HeaderMap::new(),
        Body::from(body.to_string()),
    )
    .await
}

fn approving_grant() -> WorkspaceIdentityGrant {
    WorkspaceIdentityGrant {
        repository: "owner/repo".to_string(),
        workspace: "default".to_string(),
        github_policy: Some(GitHubPolicyGrant {
            capabilities: vec![GitHubCapability::Approve],
            extra_repos: Vec::new(),
        }),
        ..WorkspaceIdentityGrant::default()
    }
}

/// The fake answers 404 to anything but `/user`, so a 404 means the request
/// reached it and a 403 that the policy stopped it.
#[tokio::test]
async fn an_approving_review_is_refused_under_enforce_without_the_capability() {
    let live: LiveToken = Arc::new(Mutex::new("ghs_first".to_string()));
    let base = fake_github(Arc::clone(&live)).await;
    let review = r#"{"event":"APPROVE","body":"lgtm"}"#;
    let path = "/repos/owner/repo/pulls/1/reviews";

    let enforcing = proxy_under(
        ProxyPolicyMode::Enforce,
        None,
        workspaces(github_repository(), HashMap::new()),
        issuer(&live),
        &base,
    );
    let refused = send(&enforcing, Method::POST, path, review).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(message(refused).await.contains("`approve` capability"));
    let comment = send(&enforcing, Method::POST, path, r#"{"event":"COMMENT"}"#).await;
    assert_eq!(comment.status(), StatusCode::NOT_FOUND);

    let granted = proxy_under(
        ProxyPolicyMode::Enforce,
        Some(approving_grant()),
        workspaces(github_repository(), HashMap::new()),
        issuer(&live),
        &base,
    );
    assert_eq!(
        send(&granted, Method::POST, path, review).await.status(),
        StatusCode::NOT_FOUND
    );

    let logging = proxy(
        workspaces(github_repository(), HashMap::new()),
        issuer(&live),
        &base,
    );
    assert_eq!(
        send(&logging, Method::POST, path, review).await.status(),
        StatusCode::NOT_FOUND
    );
    let oversized = format!(
        r#"{{"event":"COMMENT","body":"{}"}}"#,
        "x".repeat(1024 * 1024)
    );
    assert_eq!(
        send(&logging, Method::POST, path, &oversized)
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "under log a review too large to read still goes on"
    );
    assert_eq!(
        send(&enforcing, Method::POST, path, &oversized)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_write_elsewhere_or_an_unlisted_mutation_is_refused_under_enforce() {
    let live: LiveToken = Arc::new(Mutex::new("ghs_first".to_string()));
    let base = fake_github(Arc::clone(&live)).await;
    let enforcing = proxy_under(
        ProxyPolicyMode::Enforce,
        None,
        workspaces(github_repository(), HashMap::new()),
        issuer(&live),
        &base,
    );

    let elsewhere = send(&enforcing, Method::POST, "/repos/other/repo/issues", "{}").await;
    assert_eq!(elsewhere.status(), StatusCode::FORBIDDEN);
    let contents = send(
        &enforcing,
        Method::PUT,
        "/repos/owner/repo/contents/README.md",
        "{}",
    )
    .await;
    assert_eq!(contents.status(), StatusCode::FORBIDDEN);

    let mutation = serde_json::json!({
        "query": "mutation { deleteRef(input: {refId: \"R\"}) { clientMutationId } }"
    })
    .to_string();
    let graphql = send(&enforcing, Method::POST, "/graphql", &mutation).await;
    assert_eq!(graphql.status(), StatusCode::FORBIDDEN);

    let read = send(
        &enforcing,
        Method::GET,
        "/repos/other/repo/contents/README.md",
        "",
    )
    .await;
    assert_eq!(read.status(), StatusCode::NOT_FOUND);
}

/// An issuer for a driver `ada` signed in through the App: the App writes on
/// every repository but `refused`, where `ada` lacks write access.
fn app_issuer(refused: &'static str) -> Arc<dyn GitHostTokenIssuer> {
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens
        .expect_token()
        .returning(|_, _| Ok("ghu_ada".to_string()));
    tokens
        .expect_github_app_write()
        .returning(move |actor, owner, repository| {
            if repository == refused {
                return Err(GitHostTokenError::Forbidden {
                    actor: actor.clone(),
                    account: Box::new(GitHostAccount::Github {
                        owner: owner.to_string(),
                        repository: Some(repository.to_string()),
                    }),
                    source: "ada needs write access".into(),
                });
            }
            Ok(Some(AppWriteToken {
                token: format!("ghs_app_{repository}"),
            }))
        });
    tokens
        .expect_credited_github_login()
        .returning(|_| Ok(Some(("user-1".to_string(), "ada".to_string()))));
    Arc::new(tokens)
}

/// A GitHub answering a node lookup with `owner/sibling`, and every other
/// request with the token and the body it was sent. It refuses the App a
/// REST `.../hooks` write and a GraphQL `addLabelsToLabelable`, as an
/// installation without that permission is refused, and the driver a pull
/// request on `owner/locked`.
async fn recording_github() -> String {
    serve(
        Router::new().fallback(any(|uri: Uri, headers: HeaderMap, body: Bytes| async move {
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let query = body["query"].as_str().unwrap_or_default();
            if query.contains("node(id") {
                return axum::Json(serde_json::json!({
                    "data": { "node": { "repository": { "nameWithOwner": "owner/sibling" } } }
                }))
                .into_response();
            }
            let presented = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            if presented == "Bearer ghu_ada" && uri.path() == "/repos/owner/locked/pulls" {
                return (
                    StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({ "message": "Resource not accessible by integration" })),
                )
                    .into_response();
            }
            if presented.starts_with("Bearer ghs_app") {
                if uri.path().ends_with("/rate-limited") {
                    return (
                        StatusCode::FORBIDDEN,
                        [("x-ratelimit-remaining", "0")],
                        axum::Json(serde_json::json!({ "message": "API rate limit exceeded" })),
                    )
                        .into_response();
                }
                if uri.path().ends_with("/hooks") {
                    return (
                        StatusCode::FORBIDDEN,
                        axum::Json(serde_json::json!({ "message": "Resource not accessible by integration" })),
                    )
                        .into_response();
                }
                if query.contains("addLabelsToLabelable") {
                    return axum::Json(serde_json::json!({
                        "data": { "addLabelsToLabelable": null },
                        "errors": [{ "type": "FORBIDDEN", "message": "Resource not accessible by integration" }]
                    }))
                    .into_response();
                }
            }
            axum::Json(serde_json::json!({ "message": presented, "sent": body })).into_response()
        })),
    )
    .await
}

async fn sent(proxy: &GitHubProxy, method: Method, path: &str, body: &str) -> serde_json::Value {
    let response = send(proxy, method, path, body).await;
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// The comment on soap-backend#1693 from 2026-11-05: `gh api` posted it with
/// the driver's own token after `moat gh pr reply` failed, so it showed the
/// driver as its author.
#[tokio::test]
async fn an_app_signed_drivers_comment_is_sent_as_the_app_and_credits_them() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );
    let credit = "hi\n\nDriven by [@ada](https://github.com/ada)";

    let rest = sent(
        &proxy,
        Method::POST,
        "/repos/owner/repo/pulls/1/comments/2/replies",
        r#"{"body":"hi"}"#,
    )
    .await;
    assert_eq!(rest["message"], "Bearer ghs_app_repo");
    assert_eq!(rest["sent"]["body"], credit);

    let mutation = serde_json::json!({
        "query": "mutation($input: AddCommentInput!) { addComment(input: $input) { clientMutationId } }",
        "variables": { "input": { "subjectId": "PR_kw", "body": "hi" } }
    })
    .to_string();
    let graphql = sent(&proxy, Method::POST, "/graphql", &mutation).await;
    assert_eq!(
        graphql["message"], "Bearer ghs_app_sibling",
        "a mutation naming only a node is sent for the node's repository"
    );
    assert_eq!(graphql["sent"]["variables"]["input"]["body"], credit);

    let mixed = serde_json::json!({
        "query": "mutation($input: AddCommentInput!, $merge: MergePullRequestInput!) { \
            addComment(input: $input) { clientMutationId } \
            mergePullRequest(input: $merge) { clientMutationId } }",
        "variables": { "input": { "subjectId": "PR_kw", "body": "hi" } }
    })
    .to_string();
    let mixed = sent(&proxy, Method::POST, "/graphql", &mixed).await;
    assert_eq!(mixed["message"], "Bearer ghs_app_sibling");
    assert_eq!(
        mixed["sent"]["variables"]["input"]["body"], "hi",
        "a document running a mutation that posts no text is not credited"
    );

    let label = sent(
        &proxy,
        Method::POST,
        "/repos/owner/repo/issues/1/labels",
        r#"{"labels":["bug"]}"#,
    )
    .await;
    assert_eq!(label["message"], "Bearer ghs_app_repo");
    assert_eq!(
        label["sent"],
        serde_json::json!({ "labels": ["bug"] }),
        "a write that posts no text is sent as the App, uncredited"
    );
}

/// An App installation lacking a permission the driver's own token has: the
/// write still lands, as the driver.
#[tokio::test]
async fn a_write_github_refuses_the_app_is_resent_as_the_driver() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );

    let hook = sent(
        &proxy,
        Method::POST,
        "/repos/owner/repo/hooks",
        r#"{"name":"web"}"#,
    )
    .await;
    assert_eq!(hook["message"], "Bearer ghu_ada");
    assert_eq!(hook["sent"], serde_json::json!({ "name": "web" }));

    let label = serde_json::json!({
        "query": "mutation($input: AddLabelsToLabelableInput!) { addLabelsToLabelable(input: $input) { clientMutationId } }",
        "variables": { "input": { "labelableId": "PR_kw", "labelIds": ["L_1"] } }
    })
    .to_string();
    assert_eq!(
        sent(&proxy, Method::POST, "/graphql", &label).await["message"],
        "Bearer ghu_ada"
    );
}

/// Every pull request a driver signed in through the App opened showed the
/// App as its author: it is opened with the driver's own token first.
#[tokio::test]
async fn an_app_signed_drivers_pull_request_is_opened_as_the_driver() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );

    let rest = sent(
        &proxy,
        Method::POST,
        "/repos/owner/repo/pulls",
        r#"{"title":"t","head":"h","base":"main","body":"hi"}"#,
    )
    .await;
    assert_eq!(rest["message"], "Bearer ghu_ada");
    assert_eq!(
        rest["sent"]["body"], "hi",
        "GitHub already shows the driver as its author"
    );

    let mutation = serde_json::json!({
        "query": "mutation($input: CreatePullRequestInput!) { createPullRequest(input: $input) { clientMutationId } }",
        "variables": { "input": { "repositoryId": "R_kw", "headRefName": "h", "baseRefName": "main", "title": "t", "body": "hi" } }
    })
    .to_string();
    let graphql = sent(&proxy, Method::POST, "/graphql", &mutation).await;
    assert_eq!(graphql["message"], "Bearer ghu_ada");
}

#[tokio::test]
async fn a_pull_request_github_refuses_the_driver_is_opened_as_the_app() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );

    let opened = sent(
        &proxy,
        Method::POST,
        "/repos/owner/locked/pulls",
        r#"{"title":"t","head":"h","base":"main","body":"hi"}"#,
    )
    .await;
    assert_eq!(opened["message"], "Bearer ghs_app_locked");
    assert_eq!(
        opened["sent"]["body"],
        "hi\n\nDriven by [@ada](https://github.com/ada)"
    );
}

#[tokio::test]
async fn a_rate_limited_app_write_is_answered_not_resent() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );

    let response = send(&proxy, Method::POST, "/repos/owner/repo/rate-limited", "{}").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(message(response).await, "API rate limit exceeded");
}

#[tokio::test]
async fn a_write_naming_no_repository_goes_as_the_driver() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );

    let gist = sent(&proxy, Method::POST, "/gists", r#"{"files":{}}"#).await;
    assert_eq!(gist["message"], "Bearer ghu_ada");
}

#[tokio::test]
async fn a_driver_below_write_access_is_told_rather_than_posting_as_themselves() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );

    let response = send(
        &proxy,
        Method::POST,
        "/repos/owner/private/issues/1/comments",
        r#"{"body":"hi"}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(message(response).await, "ada needs write access");
}

/// A bot run-as with no App sign-in: GitHub shows the bot as the author, so
/// the text names the driver it works for.
#[tokio::test]
async fn a_bot_run_as_comment_credits_the_driver_it_works_for() {
    let base = recording_github().await;
    let mut tokens = MockGitHostTokenIssuer::new();
    tokens
        .expect_token()
        .returning(|_, _| Ok("ghs_bot".to_string()));
    tokens
        .expect_github_app_write()
        .returning(|_, _, _| Ok(None));
    tokens
        .expect_credited_github_login()
        .withf(|attribution| attribution.driver.as_deref() == Some("driver"))
        .returning(|_| Ok(Some(("driver".to_string(), "ada".to_string()))));
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        Arc::new(tokens),
        &base,
    );
    let scope = SandboxScope {
        run_as: Some("bot".to_string()),
        user: "driver".to_string(),
        attribution: Attribution {
            driver: Some("driver".to_string()),
            ..Attribution::default()
        },
        ..scope()
    };

    let response = forward(
        &proxy,
        &scope,
        GitHubUpstream::Api,
        Method::POST,
        "/repos/owner/repo/issues/1/comments",
        &HeaderMap::new(),
        Body::from(r#"{"body":"hi"}"#),
    )
    .await;
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(sent["message"], "Bearer ghs_bot");
    assert_eq!(
        sent["sent"]["body"],
        "hi\n\nDriven by [@ada](https://github.com/ada)"
    );
}

/// soaplabs/t3code#282 from 2026-11-12: `gh pr merge` in a sandbox merged it
/// with the driver's token, so GitHub showed the driver as merging it.
#[tokio::test]
async fn an_app_signed_drivers_merge_is_sent_as_the_app_uncredited() {
    let base = recording_github().await;
    let proxy = proxy(
        workspaces(github_repository(), HashMap::new()),
        app_issuer("private"),
        &base,
    );

    let merge = serde_json::json!({
        "query": "mutation($input: MergePullRequestInput!) { mergePullRequest(input: $input) { clientMutationId } }",
        "variables": { "input": { "pullRequestId": "PR_kw", "mergeMethod": "MERGE" } }
    })
    .to_string();
    let graphql = sent(&proxy, Method::POST, "/graphql", &merge).await;
    assert_eq!(graphql["message"], "Bearer ghs_app_sibling");
    assert!(
        graphql["sent"]["variables"]["input"].get("body").is_none(),
        "a merge's body would replace the merge commit message GitHub writes"
    );

    let rest = sent(
        &proxy,
        Method::PUT,
        "/repos/owner/repo/pulls/282/merge",
        r#"{"merge_method":"merge"}"#,
    )
    .await;
    assert_eq!(rest["message"], "Bearer ghs_app_repo");
    assert_eq!(rest["sent"], serde_json::json!({ "merge_method": "merge" }));
}

#[tokio::test]
async fn a_write_answer_that_breaks_off_is_handed_on_rather_than_replaced() {
    let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
        Ok(Bytes::from_static(b"{\"id\":")),
        Err(std::io::Error::other("reset")),
    ];
    let Answer::Partial(body) = read_answer(Body::from_stream(futures::stream::iter(chunks))).await
    else {
        panic!("a broken answer is not whole");
    };
    let mut stream = body.into_data_stream();
    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        Bytes::from_static(b"{\"id\":")
    );
    assert!(stream.next().await.unwrap().is_err());
}

#[tokio::test]
async fn a_whole_write_answer_is_read() {
    let Answer::Whole(bytes) = read_answer(Body::from("{\"id\":42}")).await else {
        panic!("a small answer is read whole");
    };
    assert_eq!(bytes, Bytes::from_static(b"{\"id\":42}"));
}
