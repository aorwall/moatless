//! The project half of the listing: which workspaces a viewer is grouped under,
//! and what a project row says about its scripts.

use super::contract::assert_decodable_thread;
use super::harness::{admin, mounted_router, project, project_ids, ui_rpc_authed_request, viewer};
use super::seeding::{seed_task, seed_workspaces};
use crate::common::USER_1;
use crate::common::body_json;
use crate::common::make_workspace_row;
use crate::common::run_config;
use axum::http::StatusCode;
use moatless_workspaces::compose;
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

#[sqlx::test]
async fn a_viewer_sees_the_projects_they_may_read(pool: PgPool) {
    seed_workspaces(&pool).await;

    // Own and global — including the global one they do not own. Someone
    // else's private workspace and a deleted one are absent.
    assert_eq!(
        project_ids(&pool, &viewer("user-1")).await,
        vec!["ws-global".to_string(), "ws-own".to_string()]
    );
}

#[sqlx::test]
async fn an_administrator_sees_every_project_in_the_deployment(pool: PgPool) {
    seed_workspaces(&pool).await;

    assert_eq!(
        project_ids(&pool, &admin("root")).await,
        vec![
            "ws-global".to_string(),
            "ws-other".to_string(),
            "ws-own".to_string()
        ]
    );
}

#[sqlx::test]
async fn deleted_workspaces_are_absent_even_for_an_administrator(pool: PgPool) {
    seed_workspaces(&pool).await;

    assert!(
        !project_ids(&pool, &admin("root"))
            .await
            .contains(&"ws-deleted".to_string())
    );
}

#[sqlx::test]
async fn two_workspaces_are_two_groups_not_one(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    repo.insert_workspace(&make_workspace_row("ws-b", "user-1"))
        .await
        .unwrap();

    let viewer = viewer("user-1");
    let a = project(&pool, &viewer, "ws-a").await;
    let b = project(&pool, &viewer, "ws-b").await;

    // The client groups by the project's root, so two workspaces reporting one
    // root would collapse into a single sidebar group.
    assert_ne!(a["workspaceRoot"], b["workspaceRoot"]);
    assert_eq!(a["title"], "ws-ws-a");
    assert_eq!(b["title"], "ws-ws-b");
}

/// The listing's widening half: an unreadable workspace is included once
/// something in the listing references it, absent otherwise.
///
/// Asserted one level down, at the query the listing is built from, since
/// nothing here references a workspace yet.
#[sqlx::test]
async fn an_unreadable_workspace_is_included_only_when_referenced(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-other", "user-2"))
        .await
        .unwrap();

    let unreferenced = repo
        .list_readable_including(&viewer("user-1"), &[])
        .await
        .unwrap();
    assert!(unreferenced.is_empty());

    let referenced = repo
        .list_readable_including(&viewer("user-1"), &["ws-other".to_string()])
        .await
        .unwrap();
    assert_eq!(
        referenced
            .iter()
            .map(|listed| &listed.workspace.id)
            .collect::<Vec<_>>(),
        vec!["ws-other"]
    );
    assert!(
        !referenced[0].readable,
        "the widened half must say it is the widened half, or the listing \
         above it cannot project it any differently"
    );
}

/// A deleted workspace is off the readable half but still on the widened one,
/// since tasks naming it are open work a listing must not drop.
///
/// Readable by this viewer, so it is deletion, not access, that keeps it off
/// the first half.
#[sqlx::test]
async fn a_referenced_workspace_that_was_deleted_is_still_included(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let mut deleted = make_workspace_row("ws-gone", "user-1");
    deleted.deleted = true;
    repo.insert_workspace(&deleted).await.unwrap();

    let unreferenced = repo
        .list_readable_including(&viewer("user-1"), &[])
        .await
        .unwrap();
    assert!(unreferenced.is_empty());

    let referenced = repo
        .list_readable_including(&viewer("user-1"), &["ws-gone".to_string()])
        .await
        .unwrap();
    assert_eq!(
        referenced
            .iter()
            .map(|listed| &listed.workspace.id)
            .collect::<Vec<_>>(),
        vec!["ws-gone"]
    );
}

/// The whole path a client takes: the router as it is mounted in production,
/// the session's identity, the database, and the JSON body.
#[sqlx::test]
async fn the_mounted_route_returns_the_viewers_projects(pool: PgPool) {
    let mut ws = make_workspace_row("ws-http", USER_1);
    ws.name = "Checkout".into();
    compose::workspaces(pool.clone())
        .insert_workspace(&ws)
        .await
        .unwrap();
    seed_task(&pool, "t-http", USER_1, Some("ws-http")).await;

    let router = mounted_router(&pool).await;

    let response = router
        .oneshot(ui_rpc_authed_request(
            "GET",
            "/api/orchestration/shell",
            USER_1,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    let projects = body["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["title"], "Checkout");
    assert_eq!(projects[0]["workspaceRoot"], "/moatless/workspaces/ws-http");

    let threads = body["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "the creator follows what they created");
    assert_eq!(threads[0]["id"], "t-http");
    assert_eq!(threads[0]["projectId"], "ws-http");
    assert_eq!(threads[0]["title"], "Test task t-http");
    assert_decodable_thread(&threads[0]);
}

#[sqlx::test]
async fn a_projects_scripts_are_separate_from_servers_and_setup(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let mut ws = make_workspace_row("ws-a", "user-1");
    ws.setup_commands = Some(run_config(serde_json::json!(["bun install"])));
    ws.servers = Some(run_config(serde_json::json!([{
        "name": "dev",
        "port": 3000,
        "startCommands": ["bun run dev"],
        "default": true
    }])));
    ws.scripts = Some(run_config(
        json!([{ "name": "dev", "command": "bun run dev" }]),
    ));
    repo.insert_workspace(&ws).await.unwrap();

    let scripts = project(&pool, &viewer("user-1"), "ws-a").await["scripts"]
        .as_array()
        .unwrap()
        .clone();

    assert_eq!(scripts.len(), 1, "setup commands are not scripts");
    assert_eq!(scripts[0]["id"], "dev");
    assert_eq!(scripts[0]["name"], "dev");
    assert_eq!(scripts[0]["command"], "bun run dev");
    assert_eq!(scripts[0]["icon"], "play");
    // Scripts run only on request unless their declaration opts in.
    assert_eq!(scripts[0]["runOnWorktreeCreate"], false);
    assert!(
        scripts[0].get("previewUrl").is_none(),
        "a stored port is not a preview URL"
    );
}

/// `runOnWorktreeCreate` is the client's name for `run_on_start`, and the
/// backend acts on it: a flagged Script starts when the Sandbox reports Ready.
#[sqlx::test]
async fn a_script_that_runs_on_sandbox_start_is_projected_as_flagged(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let mut ws = make_workspace_row("ws-a", "user-1");
    ws.scripts = Some(run_config(json!([
        { "name": "dev", "command": "bun run dev", "runOnStart": true },
        { "name": "test", "command": "bun test" }
    ])));
    repo.insert_workspace(&ws).await.unwrap();

    let scripts = project(&pool, &viewer("user-1"), "ws-a").await["scripts"]
        .as_array()
        .unwrap()
        .clone();

    assert_eq!(scripts[0]["id"], "dev");
    assert_eq!(scripts[0]["runOnWorktreeCreate"], true);
    assert_eq!(scripts[1]["id"], "test");
    assert_eq!(scripts[1]["runOnWorktreeCreate"], false);
}

/// Server and setup declarations do not become client-run Scripts.
#[sqlx::test]
async fn the_repositorys_server_config_does_not_create_scripts(pool: PgPool) {
    let mut ws = make_workspace_row("ws-vibe", "user-1");
    ws.name = "Moatless".into();
    ws.setup_commands = Some(run_config(serde_json::json!([
        "curl -fsSL https://bun.sh/install | bash",
        "bun install"
    ])));
    ws.servers = Some(run_config(serde_json::json!([
        {
            "name": "dev",
            "port": 3000,
            "default": true,
            "autoReload": true,
            "startCommands": ["bun run dev -- --host 0.0.0.0"],
            "startupTimeoutSeconds": 120
        },
        {
            "name": "backend",
            "label": "Shadow Backend",
            "port": 8080,
            "startCommands": ["bash scripts/shadow/up.sh"],
            "startupTimeoutSeconds": 600
        }
    ])));
    compose::workspaces(pool.clone())
        .insert_workspace(&ws)
        .await
        .unwrap();

    let scripts = project(&pool, &viewer("user-1"), "ws-vibe").await["scripts"]
        .as_array()
        .unwrap()
        .clone();

    assert!(
        scripts.is_empty(),
        "servers and setup commands are not scripts"
    );
}
