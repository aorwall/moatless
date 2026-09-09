//! PostgreSQL tests for the repository store.

use moatless_types::access_control::{Scope, ScopeContext};
use sqlx::PgPool;

use crate::repository::dao::{PgRepositoryDao, RepositoryDao};
use crate::repository::models::RepositoryPatch;
use crate::test_support::{make_repository_row, seed_team};

/// Helper to create a non-admin `ScopeContext` for a given user.
fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

#[sqlx::test]
async fn insert_and_find_by_id(pool: PgPool) {
    let user = "user-1";
    seed_team(&pool, "personal-user-1").await;
    let row = make_repository_row("repo-abc", user);
    PgRepositoryDao(pool.clone()).insert(&row).await.unwrap();

    let found = PgRepositoryDao(pool.clone())
        .find_by_id("repo-abc", &scope_ctx(user))
        .await
        .unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, "repo-abc");
    assert_eq!(found.name, "repo-repo-abc");
    assert_eq!(found.kind, crate::repository::models::RepositoryKind::Git);
}

#[sqlx::test]
async fn find_by_id_access_control(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let row = make_repository_row("repo-priv", "user-1");
    PgRepositoryDao(pool.clone()).insert(&row).await.unwrap();

    // Owner can see a user-scoped repository.
    let found = PgRepositoryDao(pool.clone())
        .find_by_id("repo-priv", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(found.is_some());

    // Other users cannot read another user's user-scoped repository.
    let found = PgRepositoryDao(pool.clone())
        .find_by_id("repo-priv", &scope_ctx("user-2"))
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn list_all_canonical_scope_filtering(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let user_repo = make_repository_row("repo-user", "user-1");
    PgRepositoryDao(pool.clone())
        .insert(&user_repo)
        .await
        .unwrap();

    let mut global_repo = make_repository_row("repo-global", "system");
    global_repo.scope = Scope::Global;
    global_repo.owner_user_id = None;
    global_repo.created_by = Some("system".to_string());
    PgRepositoryDao(pool.clone())
        .insert(&global_repo)
        .await
        .unwrap();

    // user-1 sees own user-scoped repository plus global repositories.
    let list = PgRepositoryDao(pool.clone())
        .list_all(&scope_ctx("user-1"))
        .await
        .unwrap();
    assert_eq!(list.len(), 2);

    // user-2 sees global repositories but not user-1's user-scoped repository.
    let list = PgRepositoryDao(pool.clone())
        .list_all(&scope_ctx("user-2"))
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "repo-global");
}

#[sqlx::test]
async fn update_partial(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let row = make_repository_row("repo-upd", "user-1");
    PgRepositoryDao(pool.clone()).insert(&row).await.unwrap();

    let updated = PgRepositoryDao(pool.clone())
        .update(
            "repo-upd",
            &RepositoryPatch {
                name: Some("new-name".to_string()),
                description: Some(Some("new desc".to_string())),
                ..Default::default()
            },
            &scope_ctx("user-1"),
        )
        .await
        .unwrap();
    assert!(updated);

    let found = PgRepositoryDao(pool.clone())
        .find_by_id("repo-upd", &scope_ctx("user-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name, "new-name");
    assert_eq!(found.description.as_deref(), Some("new desc"));
    assert_eq!(found.kind, crate::repository::models::RepositoryKind::Git); // unchanged
}

#[sqlx::test]
async fn soft_delete(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let row = make_repository_row("repo-del", "user-1");
    PgRepositoryDao(pool.clone()).insert(&row).await.unwrap();

    let deleted = PgRepositoryDao(pool.clone())
        .soft_delete("repo-del", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(deleted);

    // Should not be findable anymore
    let found = PgRepositoryDao(pool.clone())
        .find_by_id("repo-del", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(found.is_none());

    // Double delete should be no-op
    let deleted_again = PgRepositoryDao(pool.clone())
        .soft_delete("repo-del", &scope_ctx("user-1"))
        .await
        .unwrap();
    assert!(!deleted_again);
}

#[sqlx::test]
async fn same_name_with_different_urls_is_allowed(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let first = make_repository_row("repo-name-a", "user-1");
    let mut second = make_repository_row("repo-name-b", "user-1");
    second.name = first.name.clone();

    PgRepositoryDao(pool.clone()).insert(&first).await.unwrap();
    PgRepositoryDao(pool.clone()).insert(&second).await.unwrap();
}

#[sqlx::test]
async fn canonical_url_is_unique_within_access_owner(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let first = make_repository_row("repo-url-a", "user-1");
    let mut second = make_repository_row("repo-url-b", "user-1");
    second.remote_url = Some("  HTTPS://GITHUB.COM/TEST/REPO-URL-A.git/  ".to_string());

    PgRepositoryDao(pool.clone()).insert(&first).await.unwrap();
    let error = PgRepositoryDao(pool.clone())
        .insert(&second)
        .await
        .expect_err("canonical duplicate URL must be rejected");

    assert_eq!(
        error
            .as_database_error()
            .and_then(|database_error| database_error.constraint()),
        Some("repositories_active_owner_remote_url_uq")
    );
}

#[sqlx::test]
async fn canonical_url_can_be_reused_by_a_different_access_owner(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let user_repo = make_repository_row("repo-owner-user", "user-1");
    let mut global_repo = make_repository_row("repo-owner-global", "system");
    global_repo.scope = Scope::Global;
    global_repo.owner_user_id = None;
    global_repo.created_by = Some("system".to_string());
    global_repo.remote_url = user_repo.remote_url.clone();

    PgRepositoryDao(pool.clone())
        .insert(&user_repo)
        .await
        .unwrap();
    PgRepositoryDao(pool.clone())
        .insert(&global_repo)
        .await
        .unwrap();
}

#[sqlx::test]
async fn soft_delete_releases_canonical_url(pool: PgPool) {
    seed_team(&pool, "personal-user-1").await;
    let first = make_repository_row("repo-release-a", "user-1");
    let mut replacement = make_repository_row("repo-release-b", "user-1");
    replacement.remote_url = first.remote_url.clone();

    PgRepositoryDao(pool.clone()).insert(&first).await.unwrap();
    PgRepositoryDao(pool.clone())
        .soft_delete(&first.id, &scope_ctx("user-1"))
        .await
        .unwrap();
    PgRepositoryDao(pool.clone())
        .insert(&replacement)
        .await
        .unwrap();
}
