#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for the Phase B1b task-creation shadow write.
//!
//! Covers:
//! - `workspace::dao::ensure_shadow_workspace` — creates the deterministic
//!   `ws_<repo>` workspace + primary `wsr_<repo>` placement, copies repo columns
//!   1:1, and is idempotent + race-safe (`ON CONFLICT DO NOTHING`).
//! - `task::dao::insert_task` persists `workspace_id`, deriving the shadow when
//!   unset and respecting an explicitly-set value.

use crate::common;

use moatless_backend::repository::dao as repository_dao;
use moatless_backend::task::dao as task_dao;
use moatless_backend::workspace::dao::{PgWorkspaceDao, WorkspaceDao, ensure_shadow_workspace};
use moatless_core::scope::{Scope, ScopeContext};
use moatless_backend::task_listing::current_position;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

// =========================================================================
// ensure_shadow_workspace
// =========================================================================

#[sqlx::test]
async fn ensure_shadow_workspace_creates_shadow_and_primary_placement(pool: PgPool) {
    let repo_row = make_repository_row("repo-shadow", "user-1");
    repository_dao::insert(&pool, &repo_row).await.unwrap();

    let ws_id = ensure_shadow_workspace(&pool, "repo-shadow").await.unwrap();
    assert_eq!(ws_id, "ws_repo-shadow");

    // Workspace copies the repository's identity/ACL columns 1:1 with kind=single.
    let ws_repo = PgWorkspaceDao(pool.clone());
    let ws = ws_repo
        .find_by_id(&ws_id, &scope_ctx("user-1"))
        .await
        .unwrap()
        .expect("shadow workspace present");
    assert_eq!(ws.name, repo_row.name);
    assert_eq!(ws.kind, "single");
    assert_eq!(ws.scope, Scope::User);
    assert_eq!(ws.owner_user_id.as_deref(), Some("user-1"));

    // Exactly one primary placement pointing back at the repository.
    let placements = ws_repo.find_repos(&ws_id).await.unwrap();
    assert_eq!(placements.len(), 1);
    let p = &placements[0];
    assert_eq!(p.id, "wsr_repo-shadow");
    assert_eq!(p.repository_id, "repo-shadow");
    assert!(p.is_primary);
    assert_eq!(p.position, 0);
    assert_eq!(
        current_position(&pool).await.unwrap(),
        0,
        "creating a shadow Workspace advanced the Task listing"
    );
}

#[sqlx::test]
async fn ensure_shadow_workspace_is_idempotent(pool: PgPool) {
    let repo_row = make_repository_row("repo-idem", "user-1");
    repository_dao::insert(&pool, &repo_row).await.unwrap();

    let first = ensure_shadow_workspace(&pool, "repo-idem").await.unwrap();
    let second = ensure_shadow_workspace(&pool, "repo-idem").await.unwrap();
    assert_eq!(first, second);

    // No duplicate workspace or placement rows after a second call.
    let ws_repo = PgWorkspaceDao(pool.clone());
    let placements = ws_repo.find_repos("ws_repo-idem").await.unwrap();
    assert_eq!(placements.len(), 1, "placement is not duplicated");
}

// =========================================================================
// insert_task persists workspace_id
// =========================================================================

#[sqlx::test]
async fn insert_task_derives_shadow_workspace_id_when_unset(pool: PgPool) {
    // make_task_row uses repository_id "repo-1" and workspace_id None.
    let repo_row = make_repository_row("repo-1", "user-1");
    repository_dao::insert(&pool, &repo_row).await.unwrap();

    let task = make_task_row("task-1", "user-1");
    assert!(task.workspace_id.is_none());
    task_dao::insert_task(&pool, &task).await.unwrap();

    // The task now points at its repository's shadow workspace...
    let stored = task_dao::find_by_id(&pool, "task-1", &scope_ctx("user-1"))
        .await
        .unwrap()
        .expect("task present");
    assert_eq!(stored.workspace_id.as_deref(), Some("ws_repo-1"));

    // ...and that shadow workspace was actually created.
    let ws_repo = PgWorkspaceDao(pool.clone());
    assert!(ws_repo
        .find_by_id("ws_repo-1", &scope_ctx("user-1"))
        .await
        .unwrap()
        .is_some());
}

#[sqlx::test]
async fn insert_task_respects_explicit_workspace_id(pool: PgPool) {
    let repo_row = make_repository_row("repo-1", "user-1");
    repository_dao::insert(&pool, &repo_row).await.unwrap();

    // A caller that already resolved a (real, multi-repo) workspace keeps it —
    // insert_task must not overwrite an explicit workspace_id with the shadow.
    let ws_repo = PgWorkspaceDao(pool.clone());
    ws_repo
        .insert_workspace(&common::make_workspace_row("ws-explicit", "user-1"))
        .await
        .unwrap();

    let mut task = make_task_row("task-2", "user-1");
    task.workspace_id = Some("ws-explicit".to_string());
    task_dao::insert_task(&pool, &task).await.unwrap();

    let stored = task_dao::find_by_id(&pool, "task-2", &scope_ctx("user-1"))
        .await
        .unwrap()
        .expect("task present");
    assert_eq!(stored.workspace_id.as_deref(), Some("ws-explicit"));

    // The repository's shadow workspace was NOT force-created for this path.
    assert!(ws_repo
        .find_by_id("ws_repo-1", &scope_ctx("user-1"))
        .await
        .unwrap()
        .is_none());
}
