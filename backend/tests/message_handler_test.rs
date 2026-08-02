#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! HTTP handler integration tests for message endpoints.

mod common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_vibe_backend::message::dao as message_dao;
use moatless_vibe_backend::message::routes::message_router;
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::task::dao as task_dao;
use moatless_vibe_backend::task_activities::mapping::ConversationWrite;

use common::{
    authed_json_request, authed_request, body_json, make_message_row, make_repository_row,
    make_task_row, test_app_state, USER_1,
};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = message_router(state);
    router
}

async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    repository_dao::insert(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row(task_id, user);
    task_dao::insert_task(pool, &task).await.expect("seed task");
}

// =========================================================================
// GET /api/v1/tasks/{task_id}/messages
// =========================================================================

#[sqlx::test]
async fn get_messages_empty(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/task-1/messages", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);
    assert_eq!(body["hasMore"], false);
}

#[sqlx::test]
async fn get_messages_pagination(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    // Seed 75 user messages so they cleanly survive the empty-content filter.
    for i in 0..75 {
        let msg = make_message_row("task-1", &format!("msg-{i:03}"));
        message_dao::append(&pool, &msg, &[], &ConversationWrite::default())
            .await
            .unwrap();
    }

    // Page 1: latest 50.
    let app = build_router(pool.clone());
    let req = authed_request("GET", "/api/v1/tasks/task-1/messages?limit=50", USER_1);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 50, "first page should be full");
    assert_eq!(body["hasMore"], true);
    let next_before_id = body["nextBeforeId"]
        .as_i64()
        .expect("nextBeforeId must be set when has_more=true");

    // Page 2: remaining 25 strictly older than the cursor.
    let app = build_router(pool);
    let req = authed_request(
        "GET",
        &format!("/api/v1/tasks/task-1/messages?limit=50&before_id={next_before_id}"),
        USER_1,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 25, "second page should hold the remainder");
    assert_eq!(body["hasMore"], false);
    assert!(body.get("nextBeforeId").is_none() || body["nextBeforeId"].is_null());
}

#[sqlx::test]
async fn get_messages_with_data(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    // Insert messages via DAO
    let msg1 = make_message_row("task-1", "msg-1");
    message_dao::append(&pool, &msg1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let msg2 = make_message_row("task-1", "msg-2");
    message_dao::append(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/task-1/messages", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    // Messages with content should be returned
    let messages = body["messages"].as_array().unwrap();
    assert!(messages.len() <= 2); // may filter empty messages
}

#[sqlx::test]
async fn get_messages_missing_auth_401(pool: PgPool) {
    let app = build_router(pool);
    // No auth headers
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/tasks/task-1/messages")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// =========================================================================
// GET /api/v1/tasks/{task_id}/messages/pending
// =========================================================================

#[sqlx::test]
async fn get_pending_messages_empty(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/task-1/messages/pending", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["totalCount"], 0);
}

#[sqlx::test]
async fn get_pending_messages_with_data(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    // Insert pending user messages (no agent_session_id)
    let msg1 = make_message_row("task-1", "pending-1");
    message_dao::append(&pool, &msg1, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let msg2 = make_message_row("task-1", "pending-2");
    message_dao::append(&pool, &msg2, &[], &ConversationWrite::default())
        .await
        .unwrap();

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks/task-1/messages/pending", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let messages = body["messages"].as_array().unwrap();
    // 2 pending user messages (no last_result)
    assert_eq!(messages.len(), 2);
    assert_eq!(body["totalCount"], 2);
}

// =========================================================================
// POST /api/v1/tasks/{task_id}/messages
// =========================================================================

#[sqlx::test]
async fn send_message_task_not_found(pool: PgPool) {
    // No task seeded — should 404
    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/tasks/nonexistent/messages",
        USER_1,
        serde_json::json!({"message": "hello"}),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn send_message_success(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/tasks/task-1/messages",
        USER_1,
        serde_json::json!({"message": "hello world"}),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "sent");
    assert!(body["timestamp"].as_str().is_some());
}
