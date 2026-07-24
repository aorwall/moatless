#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! HTTP handler integration tests for task endpoints.

mod common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::task::dao::{
    self as task_dao, PgTaskRepository, TaskPatch, TaskRepository,
};
use moatless_vibe_backend::task::models::TaskStatus;
use moatless_vibe_backend::task::routes::task_router;

use common::{
    authed_json_request, authed_request, body_json, make_repository_row, make_task_row,
    test_app_state, user_scope, USER_1,
};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = task_router(state);
    router
}

async fn seed_repo(pool: &PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    repository_dao::insert(pool, &repo).await.unwrap();
}

#[sqlx::test]
async fn get_task_200(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-get", USER_1);
    task_dao::insert_task(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/task-get", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["taskId"], "task-get");
    assert_eq!(body["status"], "open");
}

#[sqlx::test]
async fn get_task_404(pool: PgPool) {
    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/nonexistent", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// Regression: `task_router` must build without panicking. utoipa-axum's
// `routes!()` groups every handler in a single call onto one path, so mixing
// handlers with different paths (e.g. GET /{task_id} and GET
// /{task_id}/statistics) registers two GET method-routes on the same path and
// panics at startup with "Overlapping method route". Each distinct path must
// get its own `.routes(routes!(...))` call. See backend/src/task/routes.rs.
#[sqlx::test]
async fn router_builds_and_statistics_route_is_registered(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-stats", USER_1);
    task_dao::insert_task(&pool, &row).await.unwrap();

    // Panics here if the router has overlapping method routes.
    let app = build_router(pool);

    // The statistics path must resolve to its own handler, not be shadowed by
    // GET /{task_id} — a routed request returns a real status, never 404.
    let req = authed_request("GET", "/api/v1/tasks/task-stats/statistics", USER_1);
    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn list_tasks_empty(pool: PgPool) {
    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["pagination"]["total"], 0);
}

#[sqlx::test]
async fn list_tasks_with_data(pool: PgPool) {
    seed_repo(&pool).await;
    for i in 0..3 {
        let row = make_task_row(&format!("task-list-{i}"), USER_1);
        task_dao::insert_task(&pool, &row).await.unwrap();
    }

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 3);
    assert_eq!(body["pagination"]["total"], 3);
}

#[sqlx::test]
async fn update_task_metadata(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-upd", USER_1);
    task_dao::insert_task(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_json_request(
        "PATCH",
        "/api/v1/tasks/task-upd",
        USER_1,
        serde_json::json!({"name": "Updated Title"}),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "Updated Title");
}

#[sqlx::test]
async fn delete_task_handler(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-del-h", USER_1);
    task_dao::insert_task(&pool, &row).await.unwrap();

    let app = build_router(pool.clone());
    let req = authed_request("DELETE", "/api/v1/tasks/task-del-h", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify deleted
    let found = task_dao::find_by_id(&pool, "task-del-h", &user_scope(USER_1))
        .await
        .unwrap();
    assert!(found.is_none());
}

/// Set an existing task to `Closed` (mirrors the close handler's DB effect).
async fn set_closed(pool: &PgPool, task_id: &str) {
    PgTaskRepository(pool.clone())
        .update_task(
            task_id,
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

#[sqlx::test]
async fn reopen_closed_task_200(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-reopen", USER_1);
    task_dao::insert_task(&pool, &row).await.unwrap();
    set_closed(&pool, "task-reopen").await;

    let app = build_router(pool.clone());
    let req = authed_request("POST", "/api/v1/tasks/task-reopen/reopen", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["taskId"], "task-reopen");
    assert_eq!(body["status"], "open");

    // Persisted, not just echoed.
    let found = task_dao::find_by_id(&pool, "task-reopen", &user_scope(USER_1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.status, TaskStatus::Open);
}

#[sqlx::test]
async fn reopen_open_task_400(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-open-reopen", USER_1);
    task_dao::insert_task(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_request("POST", "/api/v1/tasks/task-open-reopen/reopen", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    // Only closed tasks can be reopened.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn reopen_nonexistent_404(pool: PgPool) {
    let app = build_router(pool);
    let req = authed_request("POST", "/api/v1/tasks/nope/reopen", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
