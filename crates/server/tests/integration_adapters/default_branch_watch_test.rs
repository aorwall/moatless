#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! The automatic Loop sync a moved default branch starts
//! (`repository_sync::watch`), against a GitHub stub serving `.moatless/`.

use crate::common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use moatless_backend::repository_sync::watch::DefaultBranchWatch;
use moatless_backend::state::AppState;
use moatless_loops::loops::test_support::activate_loop;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::compose;
use sqlx::{PgPool, Row};
use tokio_util::sync::CancellationToken;

use common::{make_repository_row, test_app_state_with_config, test_config};

const REPO: &str = "repo-watch";

const NIGHTLY: &str = "---\non: schedule\nname: Nightly\nschedule:\n  cron: \"0 0 3 * * * *\"\n  timezone: UTC\n---\n\nRun nightly.\n";
const WEEKLY: &str = "---\non: schedule\nname: Weekly\nschedule:\n  cron: \"0 0 3 * * MON *\"\n  timezone: UTC\n---\n\nRun weekly.\n";

/// What the stub's default branch holds.
struct Branch {
    head: String,
    docker_image: String,
    loops: BTreeMap<String, String>,
    /// Answers the `.moatless/loops` listing instead of `loops` when set.
    listing_status: Option<StatusCode>,
    /// Answers `.moatless/workspaces.json` instead of the declaration when set.
    workspaces_status: Option<StatusCode>,
    repo_visible: bool,
    head_reads: usize,
}

type Shared = Arc<Mutex<Branch>>;

async fn github_stub(branch: Shared) -> String {
    async fn handle(
        axum::extract::State(branch): axum::extract::State<Shared>,
        request: axum::extract::Request,
    ) -> Response {
        let mut branch = branch.lock().unwrap();
        let path = request.uri().path().to_string();
        let prefix = format!("/repos/test/{REPO}");
        let Some(rest) = path.strip_prefix(&prefix) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        match rest {
            "" if branch.repo_visible => Json(serde_json::json!({"default_branch": "main"})).into_response(),
            "/commits/main" => {
                branch.head_reads += 1;
                branch.head.clone().into_response()
            }
            "/contents/.moatless/workspaces.json" if branch.workspaces_status.is_some() => {
                branch.workspaces_status.unwrap().into_response()
            }
            "/contents/.moatless/workspaces.json" => serde_json::json!({
                "workspaces": [{
                    "key": "app",
                    "name": "App",
                    "dockerImage": branch.docker_image,
                    "repos": [{"url": format!("https://github.com/test/{REPO}.git"), "primary": true}]
                }]
            })
            .to_string()
            .into_response(),
            "/contents/.moatless/loops" => match branch.listing_status {
                Some(status) => status.into_response(),
                None => Json(
                    branch
                        .loops
                        .keys()
                        .map(|name| serde_json::json!({"name": name, "type": "dir"}))
                        .collect::<Vec<_>>(),
                )
                .into_response(),
            },
            other => other
                .strip_prefix("/contents/.moatless/loops/")
                .and_then(|file| file.strip_suffix("/LOOP.md"))
                .and_then(|dir| branch.loops.get(dir))
                .map_or_else(
                    || StatusCode::NOT_FOUND.into_response(),
                    |content| content.clone().into_response(),
                ),
        }
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(any(handle)).with_state(branch);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn seed_team(pool: &PgPool, user_id: &str) {
    sqlx::query(
        "INSERT INTO teams (id, name, slug) VALUES ($1, $1, $1) ON CONFLICT (id) DO NOTHING",
    )
    .bind(format!("personal-{user_id}"))
    .execute(pool)
    .await
    .unwrap();
}

/// A backend whose GitHub calls reach a stub holding `branch`, and the watch
/// composed from it as `app.rs` composes it. The repository's creator holds a
/// GitHub credential.
async fn watched(pool: &PgPool, branch: Branch) -> (AppState, DefaultBranchWatch, Shared) {
    common::seed_github_token(pool, "user-1").await;
    watched_without_credential(pool, branch).await
}

async fn watched_without_credential(
    pool: &PgPool,
    branch: Branch,
) -> (AppState, DefaultBranchWatch, Shared) {
    seed_team(pool, "user-1").await;
    compose::seed_repository(pool, &make_repository_row(REPO, "user-1"))
        .await
        .unwrap();
    let branch = Arc::new(Mutex::new(branch));
    let mut config = test_config();
    config.github_api_base = github_stub(branch.clone()).await;
    let state = test_app_state_with_config(pool.clone(), config, false);
    let watch = DefaultBranchWatch::new(
        state.repositories.clone(),
        state.loop_sync.clone(),
        state.git_sync.clone(),
    );
    (state, watch, branch)
}

fn nightly_only() -> Branch {
    Branch {
        head: "sha-1".into(),
        docker_image: "app:1".into(),
        loops: BTreeMap::from([("nightly".to_string(), NIGHTLY.to_string())]),
        listing_status: None,
        workspaces_status: None,
        repo_visible: true,
        head_reads: 0,
    }
}

/// `(active, run_as_user_id)` of the Loop synced from `source_key`.
async fn loop_approval(pool: &PgPool, source_key: &str) -> Option<(bool, Option<String>)> {
    sqlx::query(
        "SELECT t.active, r.run_as_user_id FROM loops t JOIN loop_configs r ON r.loop_id = t.id \
         WHERE t.source_repository_id = $1 AND t.source_key = $2",
    )
    .bind(REPO)
    .bind(source_key)
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|row| (row.get("active"), row.get("run_as_user_id")))
}

async fn loop_id(pool: &PgPool, source_key: &str) -> String {
    sqlx::query_scalar("SELECT id FROM loops WHERE source_repository_id = $1 AND source_key = $2")
        .bind(REPO)
        .bind(source_key)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn a_moved_default_branch_syncs_new_loops_and_keeps_approval_and_overrides(pool: PgPool) {
    let (state, watch, branch) = watched(&pool, nightly_only()).await;

    watch.tick(&CancellationToken::new()).await;
    assert_eq!(
        loop_approval(&pool, "nightly").await,
        Some((false, None)),
        "a Loop synced without an admin lands inactive with no run-as user"
    );

    // An admin approves the Loop and overrides the git-declared workspace.
    let nightly = loop_id(&pool, "nightly").await;
    activate_loop(
        &state.loops,
        &ScopeContext::admin("admin"),
        &nightly,
        "approved-user",
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE workspaces SET source = 'manual', docker_image = 'override:1' \
         WHERE source_repository_id = $1 AND source_key = 'app'",
    )
    .bind(REPO)
    .execute(&pool)
    .await
    .unwrap();

    // A merge adds a Loop and changes the declared workspace.
    {
        let mut branch = branch.lock().unwrap();
        branch.head = "sha-2".into();
        branch.docker_image = "app:2".into();
        branch.loops.insert("weekly".into(), WEEKLY.into());
    }
    watch.tick(&CancellationToken::new()).await;

    assert_eq!(loop_approval(&pool, "weekly").await, Some((false, None)));
    assert_eq!(
        loop_approval(&pool, "nightly").await,
        Some((true, Some("approved-user".to_string()))),
        "an approved Loop keeps running as its approved user"
    );
    let (source, image): (String, Option<String>) = sqlx::query_as(
        "SELECT source, docker_image FROM workspaces WHERE source_repository_id = $1 AND source_key = 'app'",
    )
    .bind(REPO)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (source.as_str(), image.as_deref()),
        ("manual", Some("override:1")),
        "the automatic sync never reclaims an overridden workspace"
    );
}

#[sqlx::test]
async fn an_unmoved_head_is_not_synced_again(pool: PgPool) {
    let (_state, watch, branch) = watched(&pool, nightly_only()).await;
    watch.tick(&CancellationToken::new()).await;

    branch
        .lock()
        .unwrap()
        .loops
        .insert("weekly".into(), WEEKLY.into());
    watch.tick(&CancellationToken::new()).await;
    assert_eq!(loop_approval(&pool, "weekly").await, None);

    branch.lock().unwrap().head = "sha-2".into();
    watch.tick(&CancellationToken::new()).await;
    assert_eq!(loop_approval(&pool, "weekly").await, Some((false, None)));
}

#[sqlx::test]
async fn a_failed_fetch_keeps_every_loop_and_retries(pool: PgPool) {
    let (_state, watch, branch) = watched(&pool, nightly_only()).await;
    watch.tick(&CancellationToken::new()).await;

    {
        let mut branch = branch.lock().unwrap();
        branch.head = "sha-2".into();
        branch.listing_status = Some(StatusCode::INTERNAL_SERVER_ERROR);
    }
    watch.tick(&CancellationToken::new()).await;
    assert_eq!(loop_approval(&pool, "nightly").await, Some((false, None)));

    // A 404 from a repository the credential cannot see is no confirmed removal.
    {
        let mut branch = branch.lock().unwrap();
        branch.listing_status = Some(StatusCode::NOT_FOUND);
        branch.repo_visible = false;
    }
    watch.tick(&CancellationToken::new()).await;
    assert_eq!(loop_approval(&pool, "nightly").await, Some((false, None)));

    // The failed head was never recorded, so the same head syncs once it reads.
    {
        let mut branch = branch.lock().unwrap();
        branch.listing_status = None;
        branch.repo_visible = true;
        branch.loops.insert("weekly".into(), WEEKLY.into());
    }
    watch.tick(&CancellationToken::new()).await;
    assert_eq!(loop_approval(&pool, "weekly").await, Some((false, None)));
}

#[sqlx::test]
async fn a_failed_workspace_sync_retries_the_same_head(pool: PgPool) {
    const NAMES_APP: &str = "---\non: schedule\nname: Uses App\nworkspace: App\nschedule:\n  cron: \"0 0 3 * * * *\"\n  timezone: UTC\n---\n\nRun.\n";
    let mut branch = nightly_only();
    branch.loops = BTreeMap::from([("uses-app".to_string(), NAMES_APP.to_string())]);
    branch.workspaces_status = Some(StatusCode::INTERNAL_SERVER_ERROR);
    let (_state, watch, branch) = watched(&pool, branch).await;

    watch.tick(&CancellationToken::new()).await;
    assert_eq!(
        loop_approval(&pool, "uses-app").await,
        None,
        "the Loop names a workspace the failed sync never created"
    );

    branch.lock().unwrap().workspaces_status = None;
    watch.tick(&CancellationToken::new()).await;
    assert_eq!(loop_approval(&pool, "uses-app").await, Some((false, None)));
}

#[sqlx::test]
async fn a_repository_read_without_a_credential_is_read_less_often(pool: PgPool) {
    let (_state, watch, branch) = watched_without_credential(&pool, nightly_only()).await;

    watch.tick(&CancellationToken::new()).await;
    assert_eq!(loop_approval(&pool, "nightly").await, Some((false, None)));
    branch.lock().unwrap().head = "sha-2".into();
    watch.tick(&CancellationToken::new()).await;
    assert_eq!(
        branch.lock().unwrap().head_reads,
        1,
        "an unauthenticated read waits out `UNAUTHENTICATED_POLL_INTERVAL`"
    );
}
