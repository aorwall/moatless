#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for the handler-facing task-binding and adapter-connection
//! ports on [`AppState`].
//!
//! These prove the two ports production composition supplies are wired to the
//! same database the handlers used when they constructed `Pg*` repositories
//! themselves.

use crate::common;
use moatless_types::adapter::AdapterKind;

use chrono::Utc;
use sqlx::PgPool;

use moatless_backend::state::AppState;
use moatless_loops::bindings::test_support::upsert_binding;
use moatless_tasks::task::test_support as task_fixtures;
use moatless_types::access_control::Scope;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_workspaces::compose;

use common::{USER_1, make_repository_row, make_task_row, seed_team, test_app_state, user_scope};

/// Seed the team + repository a task's FKs require, then the task itself.
async fn seed_task(pool: &PgPool, task_id: &str, user_id: &str) {
    seed_team(pool, &format!("personal-{user_id}")).await;
    let repo = make_repository_row("repo-1", user_id);
    // Repeated calls across tasks in one test share the single repository row.
    let _ = compose::seed_repository(pool, &repo).await;
    let task = make_task_row(task_id, user_id);
    task_fixtures::seed_task(pool, &task)
        .await
        .expect("seed task row");
}

/// Write an `adapter_connections` row through the injected port and return
/// its id. `task_bindings.connection_id` has an FK onto this table.
async fn seed_connection(state: &AppState, id: &str, user_id: &str) -> String {
    let now = Utc::now();
    let row = AdapterConnection {
        id: id.to_string(),
        adapter_kind: AdapterKind::Github,
        connection_kind: "org_webhook".to_string(),
        scope_value: "acme".to_string(),
        credential_ref: None,
        webhook_secret: None,
        transport_config: serde_json::json!({}),
        created_by: user_id.to_string(),
        scope: Scope::User,
        owner_user_id: Some(user_id.to_string()),
        created_at: now,
        updated_at: now,
    };
    state
        .adapter_connections
        .save(&user_scope(user_id), &row)
        .await
        .expect("upsert adapter connection through the port")
        .id
}

fn binding(id: &str, task_id: &str, connection_id: &str, subject_key: &str) -> TaskBinding {
    let now = Utc::now();
    TaskBinding {
        id: id.to_string(),
        task_id: task_id.to_string(),
        adapter_kind: AdapterKind::Github,
        subject_kind: "github_pr".to_string(),
        subject_key: subject_key.to_string(),
        connection_id: connection_id.to_string(),
        loop_id: None,
        destination_config: serde_json::json!({"owner": "acme", "repo": "web", "pr_number": 7}),
        metadata: serde_json::json!({}),
        last_external_event_id: None,
        origin: "ingress".to_string(),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    }
}

/// Whether a binding row is still routing inbound events.
async fn is_active(pool: &PgPool, binding_id: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT active FROM task_bindings WHERE id = $1")
        .bind(binding_id)
        .fetch_one(pool)
        .await
        .expect("read binding active flag")
}

/// `deactivate_for_task` is scoped to one task and idempotent.
#[sqlx::test]
async fn deactivate_for_task_through_the_operation_is_task_scoped_and_idempotent(pool: PgPool) {
    seed_task(&pool, "task-keep", USER_1).await;
    seed_task(&pool, "task-close", USER_1).await;
    let state = test_app_state(pool.clone());
    let connection_id = seed_connection(&state, "pc-1", USER_1).await;

    for (id, task_id, subject) in [
        ("tb-keep", "task-keep", "acme/web#1"),
        ("tb-close-a", "task-close", "acme/web#2"),
        ("tb-close-b", "task-close", "acme/web#3"),
    ] {
        upsert_binding(
            &state.task_bindings,
            &binding(id, task_id, &connection_id, subject),
        )
        .await
        .expect("upsert binding through the port");
    }

    let deactivated = state
        .task_bindings
        .deactivate_for_task("task-close")
        .await
        .expect("deactivate through the port");
    assert_eq!(deactivated, 2);

    // Only the closed task's listeners stopped.
    assert!(!is_active(&pool, "tb-close-a").await);
    assert!(!is_active(&pool, "tb-close-b").await);
    assert!(is_active(&pool, "tb-keep").await);

    // A second close/delete of the same task is a no-op, not a re-stamp.
    let again = state
        .task_bindings
        .deactivate_for_task("task-close")
        .await
        .expect("deactivate through the port");
    assert_eq!(again, 0);
}

/// The adapter-connection port resolves the row it just wrote by both of the
/// lookups handlers use (`get` under a viewer, and `by_external_account`).
#[sqlx::test]
async fn adapter_connection_port_round_trips_a_connection(pool: PgPool) {
    let state = test_app_state(pool.clone());
    let connection_id = seed_connection(&state, "pc-1", USER_1).await;

    let by_id = state
        .adapter_connections
        .get(&user_scope(USER_1), &connection_id)
        .await
        .expect("get through the port")
        .expect("connection row");
    assert_eq!(by_id.adapter_kind, AdapterKind::Github);
    assert_eq!(by_id.scope_value, "acme");

    let by_account = state
        .connection_routing
        .by_external_account("github", "acme")
        .await
        .expect("by_external_account through the routing handle")
        .expect("connection row");
    assert_eq!(by_account.id, connection_id);
}
