#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr)]
//! HTTP handler integration tests for task endpoints.

mod common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::task::dao as task_dao;
use moatless_vibe_backend::task::routes::task_router;

use common::{
    authed_json_request, authed_request, body_json, make_repository_row, make_task_row,
    test_app_state,
};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = task_router(state);
    router
}

async fn seed_repo(pool: &PgPool) {
    let repo = make_repository_row("repo-1", "user-1");
    repository_dao::insert(pool, &repo).await.unwrap();
}

#[sqlx::test]
async fn get_task_200(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-get", "user-1");
    task_dao::insert_task(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/task-get", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["taskId"], "task-get");
    assert_eq!(body["status"], "open");
}

#[sqlx::test]
async fn get_task_404(pool: PgPool) {
    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/nonexistent", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn list_tasks_empty(pool: PgPool) {
    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks", "user-1");
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
        let row = make_task_row(&format!("task-list-{i}"), "user-1");
        task_dao::insert_task(&pool, &row).await.unwrap();
    }

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 3);
    assert_eq!(body["pagination"]["total"], 3);
}

#[sqlx::test]
async fn update_task_metadata(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-upd", "user-1");
    task_dao::insert_task(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_json_request(
        "PATCH",
        "/api/v1/tasks/task-upd",
        "user-1",
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
    let row = make_task_row("task-del-h", "user-1");
    task_dao::insert_task(&pool, &row).await.unwrap();

    let app = build_router(pool.clone());
    let req = authed_request("DELETE", "/api/v1/tasks/task-del-h", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify deleted
    let found = task_dao::find_by_id(&pool, "task-del-h", "user-1")
        .await
        .unwrap();
    assert!(found.is_none());
}
