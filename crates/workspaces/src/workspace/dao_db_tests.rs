//! Integration tests for the workspace DAO (Phase B1a).
//!
//! Covers the invariants the CRUD/composition handlers rely on:
//! - `find_by_id` / `list` access control (owner vs other vs global).
//! - `set_primary` — exactly one primary per workspace, transactionally.
//! - `add_repo` / `remove_repo` + the partial-unique mount-name guard.
//! - `refresh_kind` — `blank` / `single` / `multi` derived from placement count.
//! - `find_repos_for_workspaces` — batched placement fetch (no N+1).

use std::collections::HashMap;

use moatless_types::access_control::{Scope, ScopeContext};
use moatless_workspace_types::workspace::{WorkspaceKind, WorkspaceSource};
use sqlx::PgPool;

use crate::test_support::{make_workspace_repo_row, make_workspace_row};
use crate::workspace::dao::{PgWorkspaceDao, WorkspaceDao};
use crate::workspace::models::{UpdateOutcome, WorkspacePatch};

/// Non-admin `ScopeContext` for a given user.
fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext::user(user_id)
}

// =========================================================================
// find_by_id_scoped / insert — access control
// =========================================================================

#[sqlx::test]
async fn insert_and_find_by_id(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    let ws = make_workspace_row("ws-1", "user-1");
    repo.insert_workspace(&ws).await.unwrap();

    let found = repo.find_by_id("ws-1", &scope_ctx("user-1")).await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, "ws-1");
    assert_eq!(found.name, "ws-ws-1");
    assert_eq!(found.kind, WorkspaceKind::Blank);

    // A different user cannot read another user's user-scoped workspace
    // (out-of-scope reads are indistinguishable from missing → None).
    let hidden = repo.find_by_id("ws-1", &scope_ctx("user-2")).await.unwrap();
    assert!(hidden.is_none());
}

#[sqlx::test]
async fn find_by_id_scoped_hides_plain_soft_deleted(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-del", "user-1"))
        .await
        .unwrap();

    assert!(
        repo.soft_delete("ws-del", &scope_ctx("user-1"))
            .await
            .unwrap()
    );

    // Soft-deleted rows are filtered out even for the owner.
    let found = repo
        .find_by_id("ws-del", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn find_and_list_keep_git_origin_tombstones_visible(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    let mut ws = make_workspace_row("ws-git-del", "user-1");
    ws.source = WorkspaceSource::Git;
    ws.source_repository_id = Some("repo-1".to_string());
    ws.source_key = Some("api".to_string());
    ws.source_config_path = Some(".moatless/workspaces.json".to_string());
    repo.insert_workspace(&ws).await.unwrap();

    assert!(
        repo.soft_delete_git_origin("ws-git-del", &scope_ctx("user-1"))
            .await
            .unwrap()
    );

    let found = repo
        .find_by_id("ws-git-del", &scope_ctx("user-1"))
        .await
        .unwrap()
        .expect("git-origin tombstone should stay addressable");
    assert_eq!(found.source, WorkspaceSource::Manual);
    assert!(found.deleted);
    assert_eq!(found.source_repository_id.as_deref(), Some("repo-1"));
    assert_eq!(found.source_key.as_deref(), Some("api"));

    let ids: Vec<String> = repo
        .list(&scope_ctx("user-1"))
        .await
        .unwrap()
        .into_iter()
        .map(|workspace| workspace.id)
        .collect();
    assert_eq!(ids, vec!["ws-git-del".to_string()]);
}

#[sqlx::test]
async fn list_scope_filtering(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());

    repo.insert_workspace(&make_workspace_row("ws-user", "user-1"))
        .await
        .unwrap();

    let mut global = make_workspace_row("ws-global", "system");
    global.scope = Scope::Global;
    global.owner_user_id = None;
    repo.insert_workspace(&global).await.unwrap();

    repo.insert_workspace(&make_workspace_row("ws-other", "user-2"))
        .await
        .unwrap();

    // user-1 sees their own workspace + the global one, but not user-2's.
    let mut ids: Vec<String> = repo
        .list(&scope_ctx("user-1"))
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["ws-global".to_string(), "ws-user".to_string()]);
}

// =========================================================================
// update / soft_delete
// =========================================================================

#[sqlx::test]
async fn update_mutates_present_fields_only(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-u", "user-1"))
        .await
        .unwrap();

    let patch = WorkspacePatch {
        name: Some("renamed".to_string()),
        description: Some(Some("a description".to_string())),
        ..Default::default()
    };
    assert_eq!(
        repo.update("ws-u", &patch, &scope_ctx("user-1"))
            .await
            .unwrap(),
        UpdateOutcome::Applied
    );

    let found = repo
        .find_by_id("ws-u", &scope_ctx("user-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name, "renamed");
    assert_eq!(found.description.as_deref(), Some("a description"));

    // An empty patch is a no-op (no columns to set), which is not a denial.
    assert_eq!(
        repo.update("ws-u", &WorkspacePatch::default(), &scope_ctx("user-1"))
            .await
            .unwrap(),
        UpdateOutcome::NoFields
    );

    // A caller outside the workspace's control scope matches no row, and says
    // so distinctly from the empty patch above — the two used to share `false`.
    assert_eq!(
        repo.update("ws-u", &patch, &scope_ctx("user-2"))
            .await
            .unwrap(),
        UpdateOutcome::NoMatch
    );
}

/// The run-config JSONB column `env_vars` lives on `workspaces`,
/// not `repositories`; this covers their patch + read-back round-trip at the
/// workspace layer.
#[sqlx::test]
async fn update_jsonb_run_config_round_trips(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-json", "user-1"))
        .await
        .unwrap();

    let env_vars = HashMap::from([("NODE_ENV".to_string(), "test".to_string())]);
    let patch = WorkspacePatch {
        env_vars: Some(Some(env_vars.clone())),
        ..Default::default()
    };
    assert_eq!(
        repo.update("ws-json", &patch, &scope_ctx("user-1"))
            .await
            .unwrap(),
        UpdateOutcome::Applied
    );

    let found = repo
        .find_by_id("ws-json", &scope_ctx("user-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.env_vars.unwrap(), env_vars);
}

// =========================================================================
// placements — add / set_primary invariant / remove / refresh_kind
// =========================================================================

#[sqlx::test]
async fn set_primary_keeps_exactly_one_primary(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-p", "user-1"))
        .await
        .unwrap();

    // Two placements; the first is elected primary at insert time.
    repo.add_repo(
        &make_workspace_repo_row("wsr-a", "ws-p", "repo-a", true, 0),
        &scope_ctx("user-1"),
    )
    .await
    .unwrap();
    repo.add_repo(
        &make_workspace_repo_row("wsr-b", "ws-p", "repo-b", false, 1),
        &scope_ctx("user-1"),
    )
    .await
    .unwrap();

    // Promote B — A must be demoted in the same transaction.
    assert!(
        repo.set_primary("ws-p", "wsr-b", &scope_ctx("user-1"))
            .await
            .unwrap()
    );

    let placements = repo.find_repos("ws-p").await.unwrap();
    let primaries: Vec<&str> = placements
        .iter()
        .filter(|p| p.is_primary)
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(primaries, vec!["wsr-b"], "exactly one primary, and it's B");

    // set_primary on a placement that doesn't belong to the workspace is a no-op.
    assert!(
        !repo
            .set_primary("ws-p", "wsr-missing", &scope_ctx("user-1"))
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn add_repo_rejects_duplicate_mount_name(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-m", "user-1"))
        .await
        .unwrap();

    let mut first = make_workspace_repo_row("wsr-1", "ws-m", "repo-1", true, 0);
    first.mount_name = Some("frontend".to_string());
    repo.add_repo(&first, &scope_ctx("user-1")).await.unwrap();

    // Same mount name in the same workspace violates the partial unique index.
    let mut dup = make_workspace_repo_row("wsr-2", "ws-m", "repo-2", false, 1);
    dup.mount_name = Some("frontend".to_string());
    assert!(repo.add_repo(&dup, &scope_ctx("user-1")).await.is_err());

    let mut ok = make_workspace_repo_row("wsr-3", "ws-m", "repo-3", false, 2);
    ok.mount_name = Some("backend".to_string());
    repo.add_repo(&ok, &scope_ctx("user-1")).await.unwrap();
}

#[sqlx::test]
async fn refresh_kind_derives_label_and_remove_repo(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-k", "user-1"))
        .await
        .unwrap();

    // 0 placements → blank.
    repo.refresh_kind("ws-k", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert_eq!(kind_of(&repo, "ws-k").await, WorkspaceKind::Blank);

    // 1 placement → single.
    repo.add_repo(
        &make_workspace_repo_row("wsr-1", "ws-k", "repo-1", true, 0),
        &scope_ctx("user-1"),
    )
    .await
    .unwrap();
    repo.refresh_kind("ws-k", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert_eq!(kind_of(&repo, "ws-k").await, WorkspaceKind::Single);

    // 2 placements → multi.
    repo.add_repo(
        &make_workspace_repo_row("wsr-2", "ws-k", "repo-2", false, 1),
        &scope_ctx("user-1"),
    )
    .await
    .unwrap();
    repo.refresh_kind("ws-k", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert_eq!(kind_of(&repo, "ws-k").await, WorkspaceKind::Multi);

    // Remove one → back to single. remove_repo is scoped to the workspace.
    assert!(
        repo.remove_repo("ws-k", "wsr-2", &scope_ctx("user-1"))
            .await
            .unwrap()
    );
    repo.refresh_kind("ws-k", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert_eq!(kind_of(&repo, "ws-k").await, WorkspaceKind::Single);

    // Removing a placement that isn't there is a no-op.
    assert!(
        !repo
            .remove_repo("ws-k", "wsr-2", &scope_ctx("user-1"))
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn find_repos_for_workspaces_batches_by_id(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-1", "user-1"))
        .await
        .unwrap();
    repo.insert_workspace(&make_workspace_row("ws-2", "user-1"))
        .await
        .unwrap();

    repo.add_repo(
        &make_workspace_repo_row("wsr-1a", "ws-1", "repo-a", true, 0),
        &scope_ctx("user-1"),
    )
    .await
    .unwrap();
    repo.add_repo(
        &make_workspace_repo_row("wsr-1b", "ws-1", "repo-b", false, 1),
        &scope_ctx("user-1"),
    )
    .await
    .unwrap();
    repo.add_repo(
        &make_workspace_repo_row("wsr-2a", "ws-2", "repo-c", true, 0),
        &scope_ctx("user-1"),
    )
    .await
    .unwrap();

    let ids = vec!["ws-1".to_string(), "ws-2".to_string()];
    let placements = repo.find_repos_for_workspaces(&ids).await.unwrap();
    assert_eq!(placements.len(), 3);

    let ws1_count = placements
        .iter()
        .filter(|p| p.workspace_id == "ws-1")
        .count();
    let ws2_count = placements
        .iter()
        .filter(|p| p.workspace_id == "ws-2")
        .count();
    assert_eq!(ws1_count, 2);
    assert_eq!(ws2_count, 1);

    // An empty id list returns nothing (and doesn't error).
    assert!(
        repo.find_repos_for_workspaces(&[])
            .await
            .unwrap()
            .is_empty()
    );
}

/// Read a workspace's derived `kind` label via the scoped finder.
async fn kind_of(repo: &PgWorkspaceDao, id: &str) -> WorkspaceKind {
    repo.find_by_id(id, &scope_ctx("user-1"))
        .await
        .unwrap()
        .unwrap()
        .kind
}
