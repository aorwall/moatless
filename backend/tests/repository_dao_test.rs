//! Integration tests for repository_dao.

mod common;

use moatless_vibe_backend::db::repository_dao::{self, RepositoryPatch};
use sqlx::PgPool;

use common::make_repository_row;

#[sqlx::test]
async fn insert_and_find_by_id(pool: PgPool) {
    let user = "user-1";
    let row = make_repository_row("repo-abc", user);
    repository_dao::insert(&pool, &row).await.unwrap();

    let found = repository_dao::find_by_id(&pool, "repo-abc", user)
        .await
        .unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.id, "repo-abc");
    assert_eq!(found.name, "repo-repo-abc");
    assert_eq!(found.kind, "git");
}

#[sqlx::test]
async fn find_by_id_access_control(pool: PgPool) {
    let mut row = make_repository_row("repo-priv", "user-1");
    row.visibility = Some("private".to_string());
    repository_dao::insert(&pool, &row).await.unwrap();

    // Owner can see it
    let found = repository_dao::find_by_id(&pool, "repo-priv", "user-1")
        .await
        .unwrap();
    assert!(found.is_some());

    // Other user cannot
    let found = repository_dao::find_by_id(&pool, "repo-priv", "user-2")
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn list_all_visibility_scoping(pool: PgPool) {
    let mut private_repo = make_repository_row("repo-priv", "user-1");
    private_repo.visibility = Some("private".to_string());
    repository_dao::insert(&pool, &private_repo).await.unwrap();

    let public_repo = make_repository_row("repo-pub", "user-2");
    // Default visibility is "public" from make_repository_row
    repository_dao::insert(&pool, &public_repo).await.unwrap();

    // user-1 sees own private + other's public
    let list = repository_dao::list_all(&pool, "user-1").await.unwrap();
    assert_eq!(list.len(), 2);

    // user-2 sees own public but not user-1's private
    let list = repository_dao::list_all(&pool, "user-2").await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "repo-pub");
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

    let found = repository_dao::find_by_id(&pool, "repo-upd", "user-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name, "new-name");
    assert_eq!(found.description.as_deref(), Some("new desc"));
    assert_eq!(found.kind, "git"); // unchanged
}

#[sqlx::test]
async fn soft_delete(pool: PgPool) {
    let row = make_repository_row("repo-del", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let deleted = repository_dao::soft_delete(&pool, "repo-del").await.unwrap();
    assert!(deleted);

    // Should not be findable anymore
    let found = repository_dao::find_by_id(&pool, "repo-del", "user-1")
        .await
        .unwrap();
    assert!(found.is_none());

    // Double delete should be no-op
    let deleted_again = repository_dao::soft_delete(&pool, "repo-del").await.unwrap();
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

#[sqlx::test]
async fn update_jsonb_fields(pool: PgPool) {
    let row = make_repository_row("repo-json", "user-1");
    repository_dao::insert(&pool, &row).await.unwrap();

    let servers = serde_json::json!([{"name": "web", "port": 3000}]);
    let env_vars = serde_json::json!({"NODE_ENV": "test"});

    repository_dao::update(
        &pool,
        "repo-json",
        &RepositoryPatch {
            servers: Some(Some(servers.clone())),
            env_vars: Some(Some(env_vars.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let found = repository_dao::find_by_id(&pool, "repo-json", "user-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(found.servers.unwrap(), servers);
    assert_eq!(found.env_vars.unwrap(), env_vars);
}
