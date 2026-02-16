#![cfg(feature = "db-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr)]
//! HTTP handler integration tests for tag endpoints.

mod common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::tag::dao as tag_dao;
use moatless_vibe_backend::tag::routes::tag_router;
use moatless_vibe_backend::task::dao as task_dao;

use common::{body_json, make_repository_row, make_task_row, test_app_state};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = tag_router(state);
    router
}

// The tag list endpoint does not require auth (no AuthUser extractor).

#[sqlx::test]
async fn list_tags_empty(pool: PgPool) {
    let app = build_router(pool);
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/tags")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[sqlx::test]
async fn list_tags_with_data(pool: PgPool) {
    // Create some tags
    tag_dao::find_or_create_tags(
        &pool,
        &["bug".to_string(), "feature".to_string(), "urgent".to_string()],
    )
    .await
    .unwrap();

    let app = build_router(pool);
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/tags")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_json(resp).await;
    let tags = body.as_array().unwrap();
    assert_eq!(tags.len(), 3);
    // Should be ordered alphabetically by name
    assert_eq!(tags[0]["name"], "bug");
    assert_eq!(tags[1]["name"], "feature");
    assert_eq!(tags[2]["name"], "urgent");
    // Each tag has tagId and name
    assert!(!tags[0]["tagId"].as_str().unwrap().is_empty());
}

#[sqlx::test]
async fn list_tags_response_shape(pool: PgPool) {
    tag_dao::find_or_create_tags(&pool, &["alpha".to_string()])
        .await
        .unwrap();

    let app = build_router(pool);
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/tags")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = body_json(resp).await;
    let tag = &body[0];

    // Verify camelCase JSON serialization
    assert!(tag.get("tagId").is_some());
    assert!(tag.get("name").is_some());
    // description is optional, should be null for newly created tags
    assert!(tag.get("description").is_some());
}

#[sqlx::test]
async fn list_tags_with_task_associations(pool: PgPool) {
    // Seed repo + task + tags, then associate
    let repo = make_repository_row("repo-1", "user-1");
    repository_dao::insert(&pool, &repo).await.unwrap();
    let task = make_task_row("task-tag-h", "user-1");
    task_dao::insert_task(&pool, &task).await.unwrap();

    let tags = tag_dao::find_or_create_tags(&pool, &["deploy".to_string(), "hotfix".to_string()])
        .await
        .unwrap();
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    tag_dao::set_task_tags(&pool, "task-tag-h", &tag_ids)
        .await
        .unwrap();

    // Tags are still listed globally
    let app = build_router(pool);
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/tags")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 2);
}
