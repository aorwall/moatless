#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! HTTP handler integration tests for repository endpoints.

mod common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::repository::routes::repository_router;
use moatless_vibe_backend::scope::ScopeContext;

use common::{authed_json_request, authed_request, body_json, make_repository_row, test_app_state};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = repository_router(state);
    router
}

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        team_ids: vec![],
        is_admin: false,
    }
}

#[sqlx::test]
async fn create_repository(pool: PgPool) {
    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/repositories",
        "user-1",
        serde_json::json!({
            "name": "my-repo",
            "kind": "git",
            "remoteUrl": "https://github.com/test/repo.git"
        }),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "my-repo");
    assert_eq!(body["kind"], "git");
    assert!(!body["id"].as_str().unwrap().is_empty());
}

#[sqlx::test]
async fn get_repository(pool: PgPool) {
    let row = make_repository_row("repo-get", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/repositories/repo-get", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["id"], "repo-get");
}

#[sqlx::test]
async fn list_repositories(pool: PgPool) {
    for id in ["repo-a", "repo-b"] {
        let row = make_repository_row(id, "user-1");
        repository_dao::insert(&pool, &row).await.unwrap();
    }

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/repositories", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[sqlx::test]
async fn update_repository(pool: PgPool) {
    let row = make_repository_row("repo-upd", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_json_request(
        "PUT",
        "/api/v1/repositories/repo-upd",
        "user-1",
        serde_json::json!({"name": "updated-name"}),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "updated-name");
}

#[sqlx::test]
async fn delete_repository(pool: PgPool) {
    let row = make_repository_row("repo-del", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let app = build_router(pool.clone());
    let req = authed_request("DELETE", "/api/v1/repositories/repo-del", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify soft-deleted
    let found = repository_dao::find_by_id(&pool, "repo-del", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(found.is_none());
}
