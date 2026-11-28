#![cfg(feature = "db-tests")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests fail by panicking"
)]
//! HTTP handler integration tests for `POST /api/v1/feedback`'s best-effort
//! scoping: a caller that names no message lands on the task's latest activity,
//! not on the task as a whole.

use crate::common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_tasks::activities::mapping::{
    ActivityKind, ActivityRow, ActivityStatus, ConversationWrite,
};
use moatless_tasks::feedback::routes::feedback_router;
use moatless_tasks::message::test_support as message_fixtures;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_workspaces::compose;

use common::{
    USER_1, USER_2, authed_json_request, body_json, make_message_row, make_repository_row,
    make_task_row, test_app_state,
};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = feedback_router(state);
    router
}

async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    let task = make_task_row(task_id, user);
    task_fixtures::seed_task(pool, &task)
        .await
        .expect("seed task");
}

async fn seed_activity(
    pool: &PgPool,
    task_id: &str,
    ordinal: i32,
    message_uuid: &str,
    tool_use_id: &str,
) {
    let row = ActivityRow {
        activity_id: format!("{message_uuid}:{ordinal}"),
        task_id: task_id.to_string(),
        ordinal,
        message_uuid: Some(message_uuid.to_string()),
        parent_uuid: None,
        kind: ActivityKind::ToolCall,
        status: ActivityStatus::Completed,
        tool_use_id: Some(tool_use_id.to_string()),
        summary: "Bash".to_string(),
        payload: serde_json::json!({}),
        is_error: false,
    };
    let conversation = ConversationWrite {
        activities: vec![row],
        ..ConversationWrite::default()
    };
    message_fixtures::seed_activities(pool, task_id, message_uuid, &conversation)
        .await
        .expect("seed activity");
}

fn unscoped_task_feedback(comment: &str) -> serde_json::Value {
    serde_json::json!({
        "taskId": "task-1",
        "surface": "task",
        "comment": comment,
    })
}

#[sqlx::test]
async fn a_message_with_no_scope_lands_on_the_latest_tool_call(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    let msg = make_message_row("task-1", "msg-1");
    message_fixtures::seed_message(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .expect("seed message");
    seed_activity(&pool, "task-1", 1, "msg-1", "tool-1").await;

    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        unscoped_task_feedback("cwd defaults to the wrong directory"),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["surface"], "tool_call");
    assert_eq!(
        body["messageUuid"],
        message_fixtures::seeded_uuid("msg-1").to_string()
    );
    assert_eq!(body["toolUseId"], "tool-1");
}

#[sqlx::test]
async fn a_message_with_no_scope_falls_back_to_the_latest_message(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    let msg = make_message_row("task-1", "msg-1");
    message_fixtures::seed_message(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .expect("seed message");

    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        unscoped_task_feedback("the skill did not trigger"),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["surface"], "message");
    assert_eq!(
        body["messageUuid"],
        message_fixtures::seeded_uuid("msg-1").to_string()
    );
    assert!(body.get("toolUseId").is_none());
}

#[sqlx::test]
async fn a_message_with_no_scope_stays_task_level_on_a_task_with_no_messages(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        unscoped_task_feedback("no activity to attach to yet"),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["surface"], "task");
    assert!(body.get("messageUuid").is_none());
}

/// An explicit `message_uuid` is never overridden by the latest-activity
/// lookup, even when a later activity exists on the task.
#[sqlx::test]
async fn an_explicit_message_scope_is_not_overridden(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    let earlier = make_message_row("task-1", "msg-earlier");
    message_fixtures::seed_message(&pool, &earlier, &[], &ConversationWrite::default())
        .await
        .expect("seed earlier message");
    let later = make_message_row("task-1", "msg-later");
    message_fixtures::seed_message(&pool, &later, &[], &ConversationWrite::default())
        .await
        .expect("seed later message");

    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        serde_json::json!({
            "taskId": "task-1",
            "surface": "message",
            "messageUuid": "msg-earlier",
            "comment": "about this specific reply",
        }),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["messageUuid"], "msg-earlier");
}

/// A category survives the round trip and narrows the listing. It is orthogonal
/// to `surface`: the same note is still auto-scoped to the latest activity.
#[sqlx::test]
async fn a_category_round_trips_and_filters_the_listing(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        serde_json::json!({
            "taskId": "task-1",
            "surface": "task",
            "category": "environment",
            "comment": "cargo build fails on a missing system library",
        }),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["category"], "environment");

    let matching = authed_json_request(
        "GET",
        "/api/v1/feedback?category=environment",
        USER_1,
        serde_json::Value::Null,
    );
    let resp = app.clone().oneshot(matching).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["total"], 1);

    let other = authed_json_request(
        "GET",
        "/api/v1/feedback?category=codebase",
        USER_1,
        serde_json::Value::Null,
    );
    let resp = app.oneshot(other).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["total"], 0);
}

/// `GET /api/v1/feedback` resolves `repositoryName` through `RepositoryReader`
/// rather than a DAO-level join — `repositories` is owned by
/// `crates/workspaces`, not `crates/tasks`.
#[sqlx::test]
async fn list_all_feedback_resolves_repository_name(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;

    let app = build_router(pool);
    let create_req = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        unscoped_task_feedback("no repo picker in the create-task flow"),
    );
    let create_resp = app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(create_resp.status(), StatusCode::OK);

    let list_req = authed_json_request("GET", "/api/v1/feedback", USER_1, serde_json::Value::Null);
    let list_resp = app.oneshot(list_req).await.unwrap();

    assert_eq!(list_resp.status(), StatusCode::OK);
    let body = body_json(list_resp).await;
    assert_eq!(body["items"][0]["repositoryId"], "repo-1");
    assert_eq!(body["items"][0]["repositoryName"], "repo-repo-1");
}

/// A note is filed `open`; a status patch moves it and stamps the reader.
#[sqlx::test]
async fn set_feedback_status_moves_the_note_and_records_who(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    let app = build_router(pool);

    let create = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        unscoped_task_feedback("the skill did not trigger"),
    );
    let created = body_json(app.clone().oneshot(create).await.unwrap()).await;
    assert_eq!(created["status"], "open");
    let id = created["feedbackId"].as_str().unwrap();

    let patch = authed_json_request(
        "PATCH",
        &format!("/api/v1/feedback/{id}/status"),
        USER_1,
        serde_json::json!({ "status": "fixed", "note": "fixed in #700" }),
    );
    let resp = app.oneshot(patch).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "fixed");
    assert_eq!(body["statusNote"], "fixed in #700");
    assert_eq!(body["statusUpdatedBy"], USER_1);
    assert!(body["statusUpdatedAt"].is_string());
}

/// A stranger to the note's task gets the same 404 as a missing id, so the
/// response never confirms the note exists.
#[sqlx::test]
async fn set_feedback_status_hides_a_note_on_an_unreadable_task(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    let app = build_router(pool);

    let create = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        unscoped_task_feedback("owner-only note"),
    );
    let created = body_json(app.clone().oneshot(create).await.unwrap()).await;
    let id = created["feedbackId"].as_str().unwrap();

    let patch = authed_json_request(
        "PATCH",
        &format!("/api/v1/feedback/{id}/status"),
        USER_2,
        serde_json::json!({ "status": "fixed" }),
    );
    let resp = app.oneshot(patch).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn set_feedback_status_404s_for_an_unknown_id(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    let app = build_router(pool);

    let patch = authed_json_request(
        "PATCH",
        "/api/v1/feedback/nope/status",
        USER_1,
        serde_json::json!({ "status": "fixed" }),
    );
    let resp = app.oneshot(patch).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// `preview_failure` feedback names no message by design; the latest-activity
/// lookup must not attach one to it.
#[sqlx::test]
async fn preview_failure_feedback_is_not_scoped_to_the_latest_activity(pool: PgPool) {
    seed_task(&pool, "task-1", USER_1).await;
    let msg = make_message_row("task-1", "msg-1");
    message_fixtures::seed_message(&pool, &msg, &[], &ConversationWrite::default())
        .await
        .expect("seed message");

    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/feedback",
        USER_1,
        serde_json::json!({
            "taskId": "task-1",
            "surface": "preview_failure",
            "comment": "preview server 502s",
        }),
    );
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["surface"], "preview_failure");
    assert!(body.get("messageUuid").is_none());
}
