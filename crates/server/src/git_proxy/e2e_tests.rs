//! A real clone, through the real routes, from a real Forgejo.
//!
//! Ignored by default. The proxy's own dependencies are ports, so only the
//! `sandboxes` row is real here: the run reads one existing row and writes
//! nothing. Give it a Postgres holding that row, a Forgejo remote, and a token
//! for it:
//!
//! ```text
//! FORGEJO_E2E_SANDBOX_ID=… FORGEJO_E2E_REMOTE=https://git.example.com/owner/repo.git \
//!   FORGEJO_TOKEN=… DATABASE_URL=… \
//!   cargo test -p moatless-backend git_proxy::e2e -- --ignored --nocapture
//! ```

use std::process::Command;
use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use base64::Engine as _;
use moatless_core::sandbox_jwt::generate_user_auth_token;
use moatless_ports::identity::{MockCommitIdentity, MockGitHostTokenIssuer};
use moatless_types::access_control::Scope;
use moatless_workspace_ports::{MockRepositoryReader, MockWorkspaceResolver, RepoPlacement};
use moatless_workspace_types::repository::{Repository, RepositoryKind, RepositoryProvider};

use super::handlers;
use crate::git_proxy::service::GitProxy;

const SECRET: &str = "git-proxy-e2e-secret";

struct Env {
    sandbox_id: String,
    remote: String,
    token: String,
    database_url: String,
}

/// The four values the run needs, or `None` when it was not configured.
fn env() -> Option<Env> {
    Some(Env {
        sandbox_id: std::env::var("FORGEJO_E2E_SANDBOX_ID").ok()?,
        remote: std::env::var("FORGEJO_E2E_REMOTE").ok()?,
        token: std::env::var("FORGEJO_TOKEN").ok()?,
        database_url: std::env::var("DATABASE_URL").ok()?,
    })
}

fn repository(id: &str, remote: &str) -> Repository {
    Repository {
        id: id.to_string(),
        name: "e2e".to_string(),
        kind: RepositoryKind::Git,
        remote: Some(moatless_workspaces::test_support::repository_remote(
            Some(RepositoryProvider::Forgejo),
            remote,
        )),
        default_branch: None,
        description: None,
        provider: Some(RepositoryProvider::Forgejo),
        scope: Scope::User,
        owner_user_id: None,
        created_by: None,
        deleted: false,
        created_at: None,
        updated_at: None,
    }
}

/// The proxy over a real `sandboxes` row, with the workspace and the credential
/// supplied by their ports.
async fn proxy(env: &Env, repository_id: &str) -> GitProxy {
    let pool = moatless_core::db::create_pool(&env.database_url, 2)
        .await
        .expect("connect to the database holding the sandboxes row");
    let ingress = moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress();

    let mut workspaces = MockWorkspaceResolver::new();
    let placement = RepoPlacement {
        repository_id: repository_id.to_string(),
        branch: None,
        mount_name: None,
        is_primary: true,
        position: 0,
        repository: Some(repository(repository_id, &env.remote)),
    };
    workspaces
        .expect_placements()
        .returning(move |_| Ok(vec![placement.clone()]));
    workspaces.expect_resolve().returning(|_| Ok(None));

    let mut tokens = MockGitHostTokenIssuer::new();
    let token = env.token.clone();
    tokens
        .expect_token()
        .returning(move |_, _| Ok(token.clone()));

    // Nothing outside the placement is reachable here, so an id the workspace
    // does not place stays a 404 rather than falling through to a real read.
    let mut repositories = MockRepositoryReader::new();
    repositories.expect_describe().returning(|_, _| Ok(None));
    let mut users = MockCommitIdentity::new();
    users.expect_is_platform_admin().returning(|_| Ok(false));

    GitProxy::new(
        ingress,
        Arc::new(workspaces),
        Arc::new(repositories),
        Arc::new(users),
        Arc::new(tokens),
        Some(SECRET.to_string()),
    )
}

/// The three routes as [`super::routes::git_proxy_router`] mounts them, over a
/// state this test can build.
async fn serve(proxy: GitProxy) -> String {
    let app = Router::new()
        .route("/git/{repository_id}/info/refs", get(handlers::info_refs))
        .route(
            "/git/{repository_id}/git-upload-pack",
            post(handlers::upload_pack),
        )
        .route(
            "/git/{repository_id}/git-receive-pack",
            post(handlers::receive_pack),
        )
        .with_state(Arc::new(proxy));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A `git -c` value presenting `password` the way a pod's git does, without a
/// credential helper or a prompt.
fn basic(password: &str) -> String {
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{password}"));
    format!("http.extraHeader=Authorization: Basic {encoded}")
}

// `git` is driven to completion by a blocking call, which would starve the
// proxy's own task on the single-threaded runtime `#[tokio::test]` gives.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a Forgejo instance, a token for it, and the database holding the sandbox row"]
async fn a_pod_clones_a_forgejo_repository_through_the_proxy() {
    let Some(env) = env() else {
        eprintln!(
            "skipped: set FORGEJO_E2E_SANDBOX_ID, FORGEJO_E2E_REMOTE, FORGEJO_TOKEN and DATABASE_URL"
        );
        return;
    };
    let repository_id = "e2e-forgejo-repository";
    let base = serve(proxy(&env, repository_id).await).await;
    let token = generate_user_auth_token("e2e-user", "e2e", &env.sandbox_id, SECRET, 600).unwrap();
    let into = tempfile::tempdir().unwrap();
    let target = into.path().join("checkout");

    let output = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            &basic(&token),
            "clone",
            &format!("{base}/git/{repository_id}"),
        ])
        .arg(&target)
        .output()
        .expect("run git");

    assert!(
        output.status.success(),
        "clone failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(target.join(".git").is_dir());

    // The clone is only evidence the proxy streamed a packfile if the working
    // tree it produced is non-empty.
    let entries = std::fs::read_dir(&target).unwrap().count();
    assert!(entries > 1, "the checkout holds only .git");
}

#[tokio::test]
#[ignore = "needs a Forgejo instance, a token for it, and the database holding the sandbox row"]
async fn a_repository_outside_the_workspace_is_not_found() {
    let Some(env) = env() else {
        return;
    };
    let base = serve(proxy(&env, "e2e-forgejo-repository").await).await;
    let token = generate_user_auth_token("e2e-user", "e2e", &env.sandbox_id, SECRET, 600).unwrap();

    let response = reqwest::Client::new()
        .get(format!(
            "{base}/git/some-other-repository/info/refs?service=git-upload-pack"
        ))
        .basic_auth("x-access-token", Some(&token))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "needs a Forgejo instance, a token for it, and the database holding the sandbox row"]
async fn a_token_this_deployment_did_not_sign_is_refused() {
    let Some(env) = env() else {
        return;
    };
    let repository_id = "e2e-forgejo-repository";
    let base = serve(proxy(&env, repository_id).await).await;
    let forged =
        generate_user_auth_token("e2e-user", "e2e", &env.sandbox_id, "another-secret", 600)
            .unwrap();

    let response = reqwest::Client::new()
        .get(format!(
            "{base}/git/{repository_id}/info/refs?service=git-upload-pack"
        ))
        .basic_auth("x-access-token", Some(&forged))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    // git retries with its credential helper only when challenged.
    assert!(response.headers().contains_key("www-authenticate"));
}
