#![cfg(feature = "db-tests")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests fail by panicking"
)]
//! The Workspace a Loop firing resolves through `TaskOpener::open`.
//!
//! `insert_task`'s own refusals are covered in the tasks band's store tests.

use crate::common;

use moatless_loops::loops::test_support::insert_manual_loop;
use moatless_ports::task::{NewTask, TaskOpenError, TaskOpener};
use moatless_tasks::task::test_support::find_task;
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentProfile;
use moatless_types::task::Visibility;
use sqlx::PgPool;

use common::make_repository_row;

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext::user(user_id)
}

/// A repository that needs no GitHub preflight, so opening a task against it
/// reaches persistence.
async fn seed_local_repository(pool: &PgPool, repository_id: &str) {
    let mut repo = make_repository_row(repository_id, common::USER_1);
    repo.provider = None;
    repo.remote = None;
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .unwrap();
}

fn loop_task(workspace_id: &str) -> NewTask {
    NewTask {
        workspace_id: workspace_id.to_string(),
        agent_profile: AgentProfile::ClaudeCode,
        model: None,
        name: Some("loop task".to_string()),
        description: None,
        created_by: Some(common::USER_1.to_string()),
        reporter_user_id: None,
        run_as_user_id: None,
        visibility: Visibility::Private,
        loop_id: "loop-1".to_string(),
        append_system_prompt: None,
        ttl_minutes: 15,
        close_after_settled_days: 7,
        settle_after_hours: None,
        node_selector: None,
        stop_when_done: false,
        room: None,
        tag_ids: Vec::new(),
        memory_preload: Vec::new(),
        agent_properties: Default::default(),
        skills: Vec::new(),
        related_to: None,
        event_metadata: serde_json::json!({}),
    }
}

#[sqlx::test]
async fn a_loop_opens_its_task_in_the_workspace_it_names(pool: PgPool) {
    seed_local_repository(&pool, "repo-1").await;
    common::seed_workspace_placing(&pool, "ws-other", "repo-1", common::USER_1).await;
    common::seed_workspace_placing(&pool, "ws-named", "repo-1", common::USER_1).await;
    insert_manual_loop(&pool, "loop-1", "loop", "ws-named")
        .await
        .unwrap();
    let state = common::test_app_state(pool.clone());

    let opened = state
        .task_creation
        .open(loop_task("ws-named"))
        .await
        .expect("loop opens a task");

    let stored = find_task(&pool, &opened.task_id, &scope_ctx(common::USER_1))
        .await
        .unwrap()
        .expect("task present");
    assert_eq!(stored.workspace_id.as_deref(), Some("ws-named"));
    assert_eq!(stored.repository_id.as_deref(), Some("repo-1"));
}

#[sqlx::test]
async fn a_loop_naming_a_git_tombstone_is_refused(pool: PgPool) {
    seed_local_repository(&pool, "repo-1").await;
    common::seed_workspace_placing(&pool, "ws-gone", "repo-1", common::USER_1).await;
    // The provenance columns are what make this row reach the `deleted`
    // refusal: `ACTIVE_OR_GIT_TOMBSTONE` admits one kind of deleted Workspace,
    // and `seed_workspace_placing` leaves both of them `NULL`.
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
    common::seed_workspace_placing(&pool, "ws-gone", "repo-1", common::USER_1).await;
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
async fn a_workspace_that_places_no_repository_opens_a_task_with_none(pool: PgPool) {
    common::seed_workspace(&pool, "ws-empty", common::USER_1).await;
    insert_manual_loop(&pool, "loop-1", "loop", "ws-empty")
        .await
        .unwrap();
    let state = common::test_app_state(pool.clone());

    let opened = state
        .task_creation
        .open(loop_task("ws-empty"))
        .await
        .expect("a workspace with no repository opens a task");

    let stored = find_task(&pool, &opened.task_id, &scope_ctx(common::USER_1))
        .await
        .unwrap()
        .expect("task present");
    assert_eq!(stored.workspace_id.as_deref(), Some("ws-empty"));
    assert_eq!(stored.repository_id, None);
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM task_repositories WHERE task_id = $1")
            .bind(&opened.task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recorded, 0);
}
