#![cfg(feature = "db-tests")]
#![expect(clippy::unwrap_used, reason = "tests fail by panicking")]
//! HTTP handler integration tests for task endpoints.

use crate::common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_tasks::message::test_support as message_fixtures;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::relations::TaskRelation;
use moatless_tasks::task::relations::TaskRelationType;
use moatless_tasks::task::routes::task_router;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_workspaces::compose;

use common::{
    USER_1, authed_json_request, authed_request, body_json, make_repository_row, make_task_row,
    test_app_state, user_scope,
};

fn build_router(pool: PgPool) -> axum::Router {
    let state = test_app_state(pool);
    let (router, _api) = task_router(state);
    router
}

async fn seed_repo(pool: &PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    compose::seed_repository(pool, &repo).await.unwrap();
}

#[sqlx::test]
async fn get_task_200(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-get", USER_1);
    task_fixtures::seed_task(&pool, &row).await.unwrap();

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

// Regression: utoipa-axum's `routes!()` groups every handler in one call
// onto one path, so mixing handlers with different paths (e.g. GET
// /{task_id} and GET /{task_id}/statistics) panics at startup with
// "Overlapping method route". Each distinct path needs its own
// `.routes(routes!(...))` call.
#[sqlx::test]
async fn router_builds_and_statistics_route_is_registered(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-stats", USER_1);
    task_fixtures::seed_task(&pool, &row).await.unwrap();

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
        task_fixtures::seed_task(&pool, &row).await.unwrap();
    }

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 3);
    assert_eq!(body["pagination"]["total"], 3);
}

/// DB-1: the total is a property of the filtered relation, not of the page, so
/// a page past the end still tells the client how many tasks there are.
#[sqlx::test]
async fn list_tasks_offset_past_end_reports_true_total(pool: PgPool) {
    seed_repo(&pool).await;
    for i in 0..3 {
        let row = make_task_row(&format!("task-off-{i}"), USER_1);
        task_fixtures::seed_task(&pool, &row).await.unwrap();
    }

    let app = build_router(pool);
    let req = authed_request("GET", "/api/v1/tasks?limit=2&offset=10", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["pagination"]["total"], 3);
    assert_eq!(body["pagination"]["hasMore"], false);
}

/// DB-1: tags must not fan a task out into several counted rows.
#[sqlx::test]
async fn list_tasks_multiple_tags_counted_once(pool: PgPool) {
    seed_repo(&pool).await;
    for id in ["task-tagged", "task-plain"] {
        let row = make_task_row(id, USER_1);
        task_fixtures::seed_task(&pool, &row).await.unwrap();
    }

    let names = ["bug".to_string(), "urgent".to_string(), "p1".to_string()];
    let tags = moatless_tasks::tag::test_support::find_or_create_tags(&pool, &names)
        .await
        .unwrap();
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    moatless_tasks::tag::test_support::set_task_tags(&pool, "task-tagged", &tag_ids)
        .await
        .unwrap();

    let app = build_router(pool);

    let req = authed_request("GET", "/api/v1/tasks", USER_1);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
    assert_eq!(body["pagination"]["total"], 2);

    let req = authed_request("GET", "/api/v1/tasks?tag=urgent", USER_1);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["taskId"], "task-tagged");
    assert_eq!(body["pagination"]["total"], 1);
}

#[sqlx::test]
async fn update_task_metadata(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-upd", USER_1);
    task_fixtures::seed_task(&pool, &row).await.unwrap();

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

/// The model a later turn falls back to is the task row's, so a patch has to
/// reach it — and has to be checked against the agent the way creation is.
#[sqlx::test]
async fn update_task_metadata_sets_the_model_and_refuses_another_agents(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-model", USER_1);
    task_fixtures::seed_task(&pool, &row).await.unwrap();

    let app = build_router(pool);
    let req = authed_json_request(
        "PATCH",
        "/api/v1/tasks/task-model",
        USER_1,
        serde_json::json!({"model": "opus"}),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["model"], "opus");

    // `gpt-6-astra` is a Codex model, and this task runs claude-code.
    let req = authed_json_request(
        "PATCH",
        "/api/v1/tasks/task-model",
        USER_1,
        serde_json::json!({"model": "gpt-6-astra"}),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let req = authed_request("GET", "/api/v1/tasks/task-model", USER_1);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(body_json(resp).await["model"], "opus");
}

#[sqlx::test]
async fn delete_task_handler(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-del-h", USER_1);
    task_fixtures::seed_task(&pool, &row).await.unwrap();

    let app = build_router(pool.clone());
    let req = authed_request("DELETE", "/api/v1/tasks/task-del-h", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let found = task_fixtures::find_task(&pool, "task-del-h", &user_scope(USER_1))
        .await
        .unwrap();
    assert!(found.is_none());
}

/// Set an existing task to `Closed` (mirrors the close handler's DB effect).
async fn set_closed(pool: &PgPool, task_id: &str) {
    moatless_tasks::task::test_support::patch_task(
        pool,
        task_id,
        &TaskPatch {
            status: Some(TaskStatus::Closed),
            ..Default::default()
        },
        &user_scope(USER_1),
    )
    .await
    .unwrap();
}

async fn seed_parent_and_child(pool: &PgPool) {
    seed_repo(pool).await;
    for id in ["task-parent", "task-child"] {
        task_fixtures::seed_task(pool, &make_task_row(id, USER_1))
            .await
            .unwrap();
    }
    moatless_tasks::task::test_support::seed_relation(
        pool,
        &TaskRelation::new(
            "task-child",
            "task-parent",
            TaskRelationType::CreatedBy,
            serde_json::json!({}),
        ),
    )
    .await
    .unwrap();
}

#[sqlx::test]
async fn close_parent_requires_explicit_child_choice(pool: PgPool) {
    seed_parent_and_child(&pool).await;
    let app = build_router(pool);
    let req = authed_json_request(
        "POST",
        "/api/v1/tasks/task-parent/close",
        USER_1,
        serde_json::json!({}),
    );

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn close_parent_without_children_leaves_child_open(pool: PgPool) {
    seed_parent_and_child(&pool).await;
    let app = build_router(pool.clone());
    let req = authed_json_request(
        "POST",
        "/api/v1/tasks/task-parent/close",
        USER_1,
        serde_json::json!({"closeChildren": false}),
    );

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let child = task_fixtures::find_task(&pool, "task-child", &user_scope(USER_1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.status, TaskStatus::Open);
}

#[sqlx::test]
async fn close_parent_with_children_closes_descendants(pool: PgPool) {
    seed_parent_and_child(&pool).await;
    let app = build_router(pool.clone());
    let req = authed_json_request(
        "POST",
        "/api/v1/tasks/task-parent/close",
        USER_1,
        serde_json::json!({"closeChildren": true}),
    );

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let child = task_fixtures::find_task(&pool, "task-child", &user_scope(USER_1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.status, TaskStatus::Closed);
}

/// A close cancels what was still waiting for each closed Task's agent: no
/// later turn can deliver it, so leaving it `pending` would strand it.
#[sqlx::test]
async fn close_cancels_the_pending_messages_of_every_closed_task(pool: PgPool) {
    seed_parent_and_child(&pool).await;
    for task_id in ["task-parent", "task-child"] {
        message_fixtures::queue_named(&pool, task_id, task_id, "## Pull Request Closed").await;
    }
    let app = build_router(pool.clone());
    let req = authed_json_request(
        "POST",
        "/api/v1/tasks/task-parent/close",
        USER_1,
        serde_json::json!({"closeChildren": true}),
    );

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let statuses: Vec<(String, String)> =
        sqlx::query_as("SELECT task_id, status FROM task_inbox ORDER BY task_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        statuses,
        [
            ("task-child".to_string(), "cancelled".to_string()),
            ("task-parent".to_string(), "cancelled".to_string()),
        ]
    );
}

#[sqlx::test]
async fn reopen_closed_task_200(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-reopen", USER_1);
    task_fixtures::seed_task(&pool, &row).await.unwrap();
    set_closed(&pool, "task-reopen").await;

    let app = build_router(pool.clone());
    let req = authed_request("POST", "/api/v1/tasks/task-reopen/reopen", USER_1);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["taskId"], "task-reopen");
    assert_eq!(body["status"], "open");

    // Persisted, not just echoed.
    let found = task_fixtures::find_task(&pool, "task-reopen", &user_scope(USER_1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.status, TaskStatus::Open);
    assert_eq!(found.close_after_settled_days, 0);
}

#[sqlx::test]
async fn reopen_open_task_400(pool: PgPool) {
    seed_repo(&pool).await;
    let row = make_task_row("task-open-reopen", USER_1);
    task_fixtures::seed_task(&pool, &row).await.unwrap();

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
