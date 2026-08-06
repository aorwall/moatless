#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! HTTP handler integration tests for repository endpoints.

use crate::common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::repository::routes::repository_router;
use moatless_core::scope::ScopeContext;

use common::{
    authed_json_request, authed_request, body_json, make_repository_row, seed_team, test_app_state,
    USER_1,
};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = repository_router(state);
    router
}

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

#[sqlx::test]
async fn create_repository(pool: PgPool) {
    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/repositories",
        USER_1,
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
async fn create_allows_same_name_with_different_urls(pool: PgPool) {
    let app = build_router(pool);
    for remote_url in [
        "https://gitness.invalid/acme/name-a",
        "https://gitness.invalid/acme/name-b",
    ] {
        let req = authed_json_request(
            "POST",
            "/api/v1/repositories",
            USER_1,
            serde_json::json!({
                "name": "same-name",
                "kind": "git",
                "provider": "gitness",
                "remoteUrl": remote_url
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }
}

#[sqlx::test]
async fn create_rejects_canonical_duplicate_url(pool: PgPool) {
    let app = build_router(pool);
    for (remote_url, expected_status) in [
        (
            "https://gitness.invalid/acme/duplicate",
            StatusCode::CREATED,
        ),
        (
            "  HTTPS://GITNESS.INVALID/ACME/DUPLICATE.git/  ",
            StatusCode::CONFLICT,
        ),
    ] {
        let req = authed_json_request(
            "POST",
            "/api/v1/repositories",
            USER_1,
            serde_json::json!({
                "name": format!("repo-{expected_status}"),
                "kind": "git",
                "provider": "gitness",
                "remoteUrl": remote_url
            }),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected_status);
    }
}

#[sqlx::test]
async fn get_repository(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    let row = make_repository_row("repo-get", USER_1);
    repository_dao::insert(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/repositories/repo-get", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["id"], "repo-get");
}

#[sqlx::test]
async fn list_repositories(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    for id in ["repo-a", "repo-b"] {
        let row = make_repository_row(id, USER_1);
        repository_dao::insert(&pool, &row).await.unwrap();
    }

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/repositories", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[sqlx::test]
async fn update_repository(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    let row = make_repository_row("repo-upd", USER_1);
    repository_dao::insert(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_json_request(
        "PUT",
        "/api/v1/repositories/repo-upd",
        USER_1,
        serde_json::json!({"name": "updated-name"}),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "updated-name");
}

#[sqlx::test]
async fn update_rejects_canonical_duplicate_url(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    let first = make_repository_row("repo-update-url-a", USER_1);
    let second = make_repository_row("repo-update-url-b", USER_1);
    repository_dao::insert(&pool, &first).await.unwrap();
    repository_dao::insert(&pool, &second).await.unwrap();

    let app = build_router(pool);
    let req = authed_json_request(
        "PUT",
        "/api/v1/repositories/repo-update-url-b",
        USER_1,
        serde_json::json!({
            "remoteUrl": " HTTPS://GITHUB.COM/TEST/REPO-UPDATE-URL-A.git/ "
        }),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test]
async fn delete_repository(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    let row = make_repository_row("repo-del", USER_1);
    repository_dao::insert(&pool, &row).await.unwrap();

    let app = build_router(pool.clone());
    let req = authed_request("DELETE", "/api/v1/repositories/repo-del", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify soft-deleted
    let found = repository_dao::find_by_id(&pool, "repo-del", &scope_ctx(USER_1))
        .await
        .unwrap();
    assert!(found.is_none());
}
