#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on"
)]
//! `POST /api/v1/loops/{id}/run`: an administrator's firing of a Loop opens a
//! Task as the Loop would, so loop-scoped memory can be seeded from it
//! (feedback 047647d2).

use crate::common;

use axum::http::StatusCode;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_backend::state::AppState;
use moatless_loops::loops::test_support::activate_loop;
use moatless_types::access_control::ScopeContext;
use moatless_types::user::UserRole;

use common::{FIXTURE_WORKSPACE, USER_1, USER_2, body_json, make_repository_row};

const LOOP: &str = "loop-run-now";

fn request(
    method: &str,
    uri: &str,
    user_id: &str,
    role: UserRole,
    body: serde_json::Value,
) -> axum::http::Request<axum::body::Body> {
    let mut req = common::authed_json_request(method, uri, user_id, body);
    req.extensions_mut()
        .get_mut::<moatless_types::auth::AuthIdentity>()
        .expect("authed_json_request sets an identity")
        .role = role;
    req
}

async fn send(
    state: &AppState,
    router: fn(AppState) -> (axum::Router, utoipa::openapi::OpenApi),
    req: axum::http::Request<axum::body::Body>,
) -> (StatusCode, serde_json::Value) {
    let (app, _api) = router(state.clone());
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn run(
    state: &AppState,
    role: UserRole,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send(
        state,
        moatless_loops::loops::http::loops_router::<AppState>,
        request(
            "POST",
            &format!("/api/v1/loops/{LOOP}/run"),
            USER_2,
            role,
            body,
        ),
    )
    .await
}

/// A paused `kind` Loop (`schedule` or `manual`) with no run-as user, on a
/// workspace whose repository needs no GitHub preflight.
async fn seeded(pool: &PgPool, kind: &str) -> AppState {
    let mut repo = make_repository_row("repo-1", USER_1);
    repo.provider = None;
    repo.remote = None;
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repository");
    common::seed_workspace_placing(pool, FIXTURE_WORKSPACE, "repo-1", USER_1).await;
    sqlx::query(
        "INSERT INTO loops (id, name, kind, active, scope, created_by, owner_user_id) \
         VALUES ($1, 'Run now', $2, false, 'user', $3, $3)",
    )
    .bind(LOOP)
    .bind(kind)
    .bind(USER_1)
    .execute(pool)
    .await
    .expect("seed loop");
    sqlx::query(
        "INSERT INTO loop_configs (loop_id, workspace_id, routing_mode, agent_profile) \
         VALUES ($1, $2, 'by_subject', 'claude-code')",
    )
    .bind(LOOP)
    .bind(FIXTURE_WORKSPACE)
    .execute(pool)
    .await
    .expect("seed loop config");
    if kind == "schedule" {
        sqlx::query(
            "INSERT INTO schedules (loop_id, cron_expression, message_template, next_fire_at) \
             VALUES ($1, '0 0 3 * * * *', 'Run nightly.', now() + interval '1 day')",
        )
        .bind(LOOP)
        .execute(pool)
        .await
        .expect("seed schedule");
    }
    common::test_app_state(pool.clone())
}

#[sqlx::test]
async fn a_run_opens_a_loop_task_whose_loop_memory_is_the_loops(pool: PgPool) {
    let state = seeded(&pool, "schedule").await;
    activate_loop(&state.loops, &ScopeContext::admin("admin"), LOOP, USER_1)
        .await
        .unwrap();

    let (status, fired) = run(&state, UserRole::Admin, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK, "{fired}");
    let task_id = fired["taskId"].as_str().expect("a task id").to_string();

    let (loop_id, owner): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT loop_id, owner_user_id FROM tasks WHERE task_id = $1")
            .bind(&task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(loop_id.as_deref(), Some(LOOP));
    assert_eq!(
        owner.as_deref(),
        Some(USER_1),
        "a person run-as owns the Task, not the admin who ran it"
    );

    let (status, entry) = send(
        &state,
        moatless_tasks::memory::routes::memory_router::<AppState>,
        request(
            "PUT",
            &format!("/api/v1/tasks/{task_id}/memory/entry"),
            USER_1,
            UserRole::User,
            serde_json::json!({"key": "seed:k", "summary": "seeded", "scope": "loop"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{entry}");
    let (scope_kind, scope_id): (String, String) =
        sqlx::query_as("SELECT scope_kind, scope_id FROM memory_entries WHERE key = 'seed:k'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((scope_kind.as_str(), scope_id.as_str()), ("loop", LOOP));

    let (status, listed) = send(
        &state,
        moatless_tasks::memory::routes::memory_router::<AppState>,
        request(
            "GET",
            &format!("/api/v1/tasks/{task_id}/memory?scope=loop"),
            USER_1,
            UserRole::User,
            serde_json::json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed["items"][0]["expiresAt"].is_string(),
        "a listing carries the expiry: {listed}"
    );
}

#[sqlx::test]
async fn only_an_administrator_may_run_a_loop(pool: PgPool) {
    let state = seeded(&pool, "schedule").await;
    activate_loop(&state.loops, &ScopeContext::admin("admin"), LOOP, USER_1)
        .await
        .unwrap();

    let (status, _) = run(
        &state,
        UserRole::User,
        serde_json::json!({"message": "seed"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tasks, 0);
}

#[sqlx::test]
async fn a_loop_without_a_run_as_user_is_refused(pool: PgPool) {
    let state = seeded(&pool, "schedule").await;
    let (status, body) = run(&state, UserRole::Admin, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[sqlx::test]
async fn only_a_schedule_loop_runs_without_a_message(pool: PgPool) {
    let state = seeded(&pool, "manual").await;
    activate_loop(&state.loops, &ScopeContext::admin("admin"), LOOP, USER_1)
        .await
        .unwrap();
    let (status, body) = run(&state, UserRole::Admin, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
