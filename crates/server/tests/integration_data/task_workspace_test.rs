#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Every Task is created in a Workspace that exists.
//!
//! Covers `task::dao::insert_task`'s refusals, and the Workspace a Loop firing
//! resolves through `TaskOpener::open`.

use crate::common;

use moatless_ports::task::{NewTask, TaskOpenError, TaskOpener};
use moatless_tasks::task::dao::{self as task_dao, TaskDao};
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentType;
use moatless_types::task::Visibility;
use moatless_workspaces::repository::dao as repository_dao;
use moatless_workspaces::workspace::dao::{PgWorkspaceDao, WorkspaceDao};
use sqlx::PgPool;

use common::{make_repository_row, make_task_row, make_workspace_repo_row, make_workspace_row};

const USER: &str = "user-1";

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

/// A repository that needs no GitHub preflight, so opening a task against it
/// reaches persistence.
async fn seed_local_repository(pool: &PgPool, repository_id: &str) {
    let mut repo = make_repository_row(repository_id, common::USER_1);
    repo.provider = None;
    repo.remote_url = None;
    repository_dao::insert(pool, &repo).await.unwrap();
}

/// A live Workspace placing `repository_id` as its primary repo.
async fn seed_placed_workspace(pool: &PgPool, workspace_id: &str, repository_id: &str) {
    let workspaces = PgWorkspaceDao(pool.clone());
    workspaces
        .insert_workspace(&make_workspace_row(workspace_id, common::USER_1))
        .await
        .unwrap();
    workspaces
        .add_repo(
            &make_workspace_repo_row(
                &format!("wsr-{workspace_id}"),
                workspace_id,
                repository_id,
                true,
                0,
            ),
            &scope_ctx(common::USER_1),
        )
        .await
        .unwrap();
}

fn loop_task(workspace_id: &str) -> NewTask {
    NewTask {
        workspace_id: workspace_id.to_string(),
        agent_type: AgentType::from("claude-code"),
        model: None,
        name: Some("loop task".to_string()),
        description: None,
        created_by: Some(common::USER_1.to_string()),
        visibility: Visibility::Private,
        loop_id: "loop-1".to_string(),
        append_system_prompt: None,
        ttl_minutes: 15,
        inactive_ttl_days: 7,
        tag_ids: Vec::new(),
        skills: Vec::new(),
        related_to: None,
        event_metadata: serde_json::json!({}),
    }
}

// =========================================================================
// insert_task
// =========================================================================

#[sqlx::test]
async fn insert_task_refuses_a_task_with_no_workspace(pool: PgPool) {
    repository_dao::insert(&pool, &make_repository_row("repo-1", USER))
        .await
        .unwrap();

    let mut task = make_task_row("task-1", USER);
    task.workspace_id = None;

    let error = task_dao::store_over(&pool)
        .insert_task(&task)
        .await
        .expect_err("a task with no workspace is refused");
    assert!(
        error.to_string().contains("has no workspace_id"),
        "unexpected error: {error}"
    );
}

#[sqlx::test]
async fn insert_task_refuses_a_workspace_that_does_not_exist(pool: PgPool) {
    repository_dao::insert(&pool, &make_repository_row("repo-1", USER))
        .await
        .unwrap();

    let mut task = make_task_row("task-1", USER);
    task.workspace_id = Some("ws-absent".to_string());

    let error = task_dao::store_over(&pool)
        .insert_task(&task)
        .await
        .expect_err("a task naming an absent workspace is refused");
    assert!(
        error.to_string().contains("does not exist"),
        "unexpected error: {error}"
    );
}

#[sqlx::test]
async fn insert_task_persists_the_workspace_it_was_given(pool: PgPool) {
    repository_dao::insert(&pool, &make_repository_row("repo-1", USER))
        .await
        .unwrap();
    common::seed_workspace(&pool, "ws-explicit", USER).await;

    let mut task = make_task_row("task-2", USER);
    task.workspace_id = Some("ws-explicit".to_string());
    task_dao::store_over(&pool).insert_task(&task).await.unwrap();

    let stored = task_dao::find_by_id(&pool, "task-2", &scope_ctx(USER))
        .await
        .unwrap()
        .expect("task present");
    assert_eq!(stored.workspace_id.as_deref(), Some("ws-explicit"));
}

// =========================================================================
// TaskOpener — the Loop firing path
// =========================================================================

#[sqlx::test]
async fn a_loop_opens_its_task_in_the_workspace_it_names(pool: PgPool) {
    seed_local_repository(&pool, "repo-1").await;
    seed_placed_workspace(&pool, "ws-other", "repo-1").await;
    seed_placed_workspace(&pool, "ws-named", "repo-1").await;
    let state = common::test_app_state(pool.clone());

    let opened = state
        .task_creation
        .open(loop_task("ws-named"))
        .await
        .expect("loop opens a task");

    let stored = task_dao::find_by_id(&pool, &opened.task_id, &scope_ctx(common::USER_1))
        .await
        .unwrap()
        .expect("task present");
    assert_eq!(stored.workspace_id.as_deref(), Some("ws-named"));
    assert_eq!(stored.repository_id, "repo-1");
}

#[sqlx::test]
async fn a_loop_naming_a_git_tombstone_is_refused(pool: PgPool) {
    seed_local_repository(&pool, "repo-1").await;
    seed_placed_workspace(&pool, "ws-gone", "repo-1").await;
    // The provenance columns are what make this row reach the `deleted`
    // refusal: `ACTIVE_OR_GIT_TOMBSTONE` admits one kind of deleted Workspace,
    // and `make_workspace_row` leaves both of them `None`.
    sqlx::query(
        "UPDATE workspaces SET deleted = true, source_repository_id = $2, source_key = $3 \
         WHERE id = $1",
    )
    .bind("ws-gone")
    .bind("repo-1")
    .bind("ws-gone")
    .execute(&pool)
    .await
    .unwrap();
    let state = common::test_app_state(pool.clone());

    let error = state
        .task_creation
        .open(loop_task("ws-gone"))
        .await
        .expect_err("a deleted workspace cannot open a task");
    assert!(
        matches!(&error, TaskOpenError::BadRequest(message) if message.contains("has been deleted")),
        "unexpected error: {error}"
    );
}

#[sqlx::test]
async fn a_loop_naming_a_workspace_it_cannot_read_is_refused(pool: PgPool) {
    seed_local_repository(&pool, "repo-1").await;
    seed_placed_workspace(&pool, "ws-gone", "repo-1").await;
    sqlx::query("UPDATE workspaces SET deleted = true WHERE id = $1")
        .bind("ws-gone")
        .execute(&pool)
        .await
        .unwrap();
    let state = common::test_app_state(pool.clone());

    // An ordinary soft-deleted Workspace is unreadable rather than deleted:
    // `find_by_id` returns it to nobody, so this is the arm every non-git
    // deletion takes, and an id that never existed takes it too.
    for workspace_id in ["ws-gone", "ws-never-existed"] {
        let error = state
            .task_creation
            .open(loop_task(workspace_id))
            .await
            .expect_err("an unreadable workspace cannot open a task");
        assert!(
            matches!(&error, TaskOpenError::BadRequest(message) if message.contains("does not exist")),
            "unexpected error for {workspace_id}: {error}"
        );
    }
}

#[sqlx::test]
async fn a_workspace_that_places_no_repository_is_refused(pool: PgPool) {
    common::seed_workspace(&pool, "ws-empty", common::USER_1).await;
    let state = common::test_app_state(pool.clone());

    let error = state
        .task_creation
        .open(loop_task("ws-empty"))
        .await
        .expect_err("a workspace with no repository cannot open a task");
    assert!(
        matches!(&error, TaskOpenError::BadRequest(message) if message.contains("places no repository")),
        "unexpected error: {error}"
    );
}
