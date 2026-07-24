#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for repository_dao.

mod common;

use moatless_vibe_backend::repository::dao::{self as repository_dao, RepositoryPatch};
use moatless_vibe_backend::scope::{Scope, ScopeContext};
use sqlx::PgPool;

use common::make_repository_row;

/// Helper to create a non-admin ScopeContext for a given user.
fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        team_ids: vec![],
        is_admin: false,
    }
}

#[sqlx::test]
async fn insert_and_find_by_id(pool: PgPool) {
    let user = "user-1";
    let row = make_repository_row("repo-abc", user);
    repository_dao::insert(&pool, &row).await.unwrap();

    let found = repository_dao::find_by_id(&pool, "repo-abc", &scope_ctx(user))
        .await
        .unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, "repo-abc");
    assert_eq!(found.name, "repo-repo-abc");
    assert_eq!(
        found.kind,
        moatless_vibe_backend::repository::models::RepositoryKind::Git
    );
}

#[sqlx::test]
async fn find_by_id_access_control(pool: PgPool) {
    let row = make_repository_row("repo-priv", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    // Owner can see a user-scoped repository.
    let found = repository_dao::find_by_id(&pool, "repo-priv", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(found.is_some());

    // Other users cannot read another user's user-scoped repository.
    let found = repository_dao::find_by_id(&pool, "repo-priv", &scope_ctx("user-2"))
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn list_all_canonical_scope_filtering(pool: PgPool) {
    let user_repo = make_repository_row("repo-user", "user-1");
    repository_dao::insert(&pool, &user_repo).await.unwrap();

    let mut global_repo = make_repository_row("repo-global", "system");
    global_repo.scope = Scope::Global;
    global_repo.team_id = Some("system".to_string());
    global_repo.owner_user_id = None;
    global_repo.created_by = Some("system".to_string());
    repository_dao::insert(&pool, &global_repo).await.unwrap();

    // user-1 sees own user-scoped repository plus global repositories.
    let list = repository_dao::list_all(&pool, &scope_ctx("user-1"))
        .await
        .unwrap();
    assert_eq!(list.len(), 2);

    // user-2 sees global repositories but not user-1's user-scoped repository.
    let list = repository_dao::list_all(&pool, &scope_ctx("user-2"))
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "repo-global");
}

#[sqlx::test]
async fn update_partial(pool: PgPool) {
    let row = make_repository_row("repo-upd", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let updated = repository_dao::update(
        &pool,
        "repo-upd",
        &RepositoryPatch {
            name: Some("new-name".to_string()),
            description: Some(Some("new desc".to_string())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(updated);

    let found = repository_dao::find_by_id(&pool, "repo-upd", &scope_ctx("user-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name, "new-name");
    assert_eq!(found.description.as_deref(), Some("new desc"));
    assert_eq!(
        found.kind,
        moatless_vibe_backend::repository::models::RepositoryKind::Git
    ); // unchanged
}

#[sqlx::test]
async fn soft_delete(pool: PgPool) {
    let row = make_repository_row("repo-del", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let deleted = repository_dao::soft_delete(&pool, "repo-del")
        .await
        .unwrap();
    assert!(deleted);

    // Should not be findable anymore
    let found = repository_dao::find_by_id(&pool, "repo-del", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(found.is_none());

    // Double delete should be no-op
    let deleted_again = repository_dao::soft_delete(&pool, "repo-del")
        .await
        .unwrap();
    assert!(!deleted_again);
}

#[sqlx::test]
async fn find_by_name_and_user(pool: PgPool) {
    let row = make_repository_row("repo-named", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let found = repository_dao::find_by_name_and_user(&pool, "repo-repo-named", "user-1")
        .await
        .unwrap();
    assert!(found.is_some());

    let not_found = repository_dao::find_by_name_and_user(&pool, "repo-repo-named", "user-2")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

// Run-config JSONB columns (servers/env_vars/plugins) moved off `repositories`
// onto `workspaces`; their round-trip coverage now lives in
// `workspace_dao_test::update_jsonb_run_config_round_trips`. Repositories keep
// the provider-reachability columns
// (`access_status`/`access_checked_at`/`access_error`), so this exercises the
// update + read-back path against those.
#[sqlx::test]
async fn update_access_fields(pool: PgPool) {
    let row = make_repository_row("repo-access", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let checked_at = chrono::Utc::now();
    let updated = repository_dao::update(
        &pool,
        "repo-access",
        &RepositoryPatch {
            access_status: Some("not_found".to_string()),
            access_checked_at: Some(Some(checked_at)),
            access_error: Some(Some("remote returned 404".to_string())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(updated);

    let found = repository_dao::find_by_id(&pool, "repo-access", &scope_ctx("user-1"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(found.access_status, "not_found");
    assert!(found.access_checked_at.is_some());
    assert_eq!(found.access_error.as_deref(), Some("remote returned 404"));
}
