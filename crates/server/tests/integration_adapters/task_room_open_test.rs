#![cfg(feature = "db-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `POST /api/v1/tasks/{task_id}/room` admits the Task's one `room.open`
//! delivery, on a connection resolved against the caller's scope.

use crate::common;

use chrono::Utc;
use sqlx::PgPool;
use tower::ServiceExt;

use moatless_backend::adapters::routes::adapter_router;
use moatless_backend::state::AppState;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_types::access_control::Scope;
use moatless_types::adapter::{AdapterConnection, AdapterKind};

use common::{
    USER_1, USER_2, authed_json_request, body_json, make_task_row, test_app_state_with_adapters,
    test_config, user_scope,
};

const TASK: &str = "task-room-cli";

async fn harness(pool: &PgPool) -> AppState {
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    task_fixtures::seed_task(pool, &make_task_row(TASK, USER_1))
        .await
        .expect("seed task");
    save_connection(&state, "pc_slack_room", "T1").await;
    state
}

async fn save_connection(state: &AppState, id: &str, team: &str) {
    let now = Utc::now();
    state
        .adapter_connections
        .save(
            &user_scope(USER_1),
            &AdapterConnection {
                id: id.into(),
                adapter_kind: AdapterKind::Slack,
                connection_kind: "workspace".into(),
                scope_value: team.into(),
                credential_ref: None,
                webhook_secret: None,
                transport_config: serde_json::json!({}),
                created_by: USER_1.into(),
                scope: Scope::User,
                owner_user_id: Some(USER_1.into()),
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .expect("seed slack connection");
}

async fn open(state: &AppState, user: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
    let (router, _) = adapter_router(state.clone());
    let response = router
        .oneshot(authed_json_request(
            "POST",
            &format!("/api/v1/tasks/{TASK}/room"),
            user,
            body,
        ))
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

async fn room_payload(pool: &PgPool) -> Option<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT payload FROM adapter_delivery WHERE kind = 'room.open' AND event_id = $1",
    )
    .bind(TASK)
    .fetch_optional(pool)
    .await
    .expect("read room delivery")
}

#[sqlx::test]
async fn a_room_is_requested_once_on_the_callers_connection(pool: PgPool) {
    let state = harness(&pool).await;
    let body = serde_json::json!({
        "via": "slack",
        "anchor": "channel:C0FACTORY",
        "invite": ["requester"],
        "private": true,
    });

    let (status, first) = open(&state, USER_1, body.clone()).await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["requested"], true);
    let (_, second) = open(&state, USER_1, body).await;
    assert_eq!(second["requested"], false, "a Task has one room");

    let payload = room_payload(&pool).await.expect("room.open admitted");
    assert_eq!(payload["room"]["connection"], "pc_slack_room");
    assert_eq!(payload["room"]["in"], "channel:C0FACTORY");
    assert_eq!(payload["requester"], USER_1);
}

#[sqlx::test]
async fn a_room_without_an_anchor_stays_on_its_threads_connection(pool: PgPool) {
    let state = harness(&pool).await;
    save_connection(&state, "pc_slack_thread", "T2").await;
    sqlx::query(
        r#"INSERT INTO task_bindings
               (id, task_id, adapter_kind, subject_kind, subject_key, connection_id,
                destination_config, origin)
           VALUES ('binding-thread', $1, 'slack', 'slack_thread', 'C0THREAD:1.2',
                   'pc_slack_thread', '{}', 'connected_thread')"#,
    )
    .bind(TASK)
    .execute(&pool)
    .await
    .expect("seed thread binding");

    let (status, body) = open(&state, USER_1, serde_json::json!({ "via": "slack" })).await;

    assert_eq!(status, 200, "{body}");
    let payload = room_payload(&pool).await.expect("room.open admitted");
    assert_eq!(payload["room"]["connection"], "pc_slack_thread");
}

#[sqlx::test]
async fn a_room_with_nothing_to_open_from_is_refused(pool: PgPool) {
    let state = harness(&pool).await;

    let (status, body) = open(&state, USER_1, serde_json::json!({ "via": "slack" })).await;

    assert_eq!(status, 400, "{body}");
    assert!(room_payload(&pool).await.is_none());
}

#[sqlx::test]
async fn another_users_task_is_not_there(pool: PgPool) {
    let state = harness(&pool).await;

    let (status, _) = open(
        &state,
        USER_2,
        serde_json::json!({ "via": "slack", "anchor": "channel:C0FACTORY" }),
    )
    .await;

    assert_eq!(status, 404);
    assert!(room_payload(&pool).await.is_none());
}
