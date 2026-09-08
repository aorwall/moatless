#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Reconcile-delete only runs against a desired set git actually declared.

use crate::common;

use moatless_types::access_control::ScopeContext;
use moatless_types::repository::RepositoryProvider;
use moatless_types::workspace::WorkspaceSource;
use moatless_workspaces::repository::dao as repository_dao;
use moatless_workspaces::workspace::dao::{PgWorkspaceDao, WorkspaceDao};
use moatless_workspaces::workspace::git_sync::SyncForce;
use sqlx::PgPool;

use common::{make_repository_row, make_workspace_row};

const USER: &str = "user-1";

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

#[sqlx::test]
async fn a_repository_shipping_no_workspaces_json_removes_nothing(pool: PgPool) {
    // Gitness resolves no config file, so the sync reaches the "no file" return
    // without a provider call.
    let mut repo = make_repository_row("repo-1", USER);
    repo.provider = Some(RepositoryProvider::Gitness);
    repo.remote_url = Some("https://gitness.example/acme/repo.git".to_string());
    repository_dao::insert(&pool, &repo).await.unwrap();

    let workspaces = PgWorkspaceDao(pool.clone());
    let mut declared = make_workspace_row("ws-git", USER);
    declared.source = WorkspaceSource::Git;
    declared.source_repository_id = Some("repo-1".to_string());
    declared.source_key = Some("ws-git".to_string());
    workspaces.insert_workspace(&declared).await.unwrap();

    let state = common::test_app_state(pool.clone());
    let summary = state
        .git_sync
        .sync_repo_workspaces_for(&repo, &scope_ctx(USER), &SyncForce::RespectOverrides)
        .await
        .expect("sync succeeds");

    assert_eq!(summary.removed, 0);
    let survivor = workspaces
        .find_by_id("ws-git", &scope_ctx(USER))
        .await
        .unwrap()
        .expect("workspace still present");
    assert!(
        !survivor.deleted,
        "an undeclared desired set tombstoned the repository's workspace"
    );
}
