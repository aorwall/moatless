//! HTTP handler integration tests for command endpoints.

mod common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use sandbox_controller::db::command_dao;
use sandbox_controller::command::routes::command_router;

use common::{
    authed_json_request, authed_request, body_json, make_command_row, test_app_state,
};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = command_router(state);
    router
}

#[sqlx::test]
async fn list_commands(pool: PgPool) {
    // Insert commands directly via DAO
    for name in ["cmd-x", "cmd-y"] {
        command_dao::insert(&pool, &make_command_row(name))
            .await
            .unwrap();
    }

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/commands", "user-1");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body.as_array().unwrap();
    // May include default-seeded commands + our 2
    assert!(items.len() >= 2);
}

#[sqlx::test]
async fn update_command(pool: PgPool) {
    command_dao::insert(&pool, &make_command_row("upd-http"))
        .await
        .unwrap();

    let app = build_router(pool);
    let req = authed_json_request(
        "PATCH",
        "/api/v1/commands/upd-http",
        "user-1",
        serde_json::json!({"description": "patched desc", "icon": "star"}),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["description"], "patched desc");
    assert_eq!(body["icon"], "star");
}

#[sqlx::test]
async fn missing_auth_401(pool: PgPool) {
    let app = build_router(pool);

    // Request without x-user-id header
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/commands")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
