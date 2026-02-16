#![cfg(feature = "db-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout, clippy::print_stderr)]
//! Integration tests for command_dao — requires `DATABASE_URL` and `--features db-tests`.

mod common;

use moatless_vibe_backend::command::dao as command_dao;
use sqlx::PgPool;

use common::make_command_row;

#[sqlx::test]
async fn insert_and_find_by_name(pool: PgPool) {
    let cmd = make_command_row("test-cmd");
    command_dao::insert(&pool, &cmd).await.unwrap();

    let found = command_dao::find_by_name(&pool, "test-cmd").await.unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.name, "test-cmd");
    assert_eq!(found.description, "Test command test-cmd");
    assert_eq!(found.prompt, "Do test-cmd");
    assert_eq!(found.active, Some(true));
}

#[sqlx::test]
async fn insert_duplicate_ignored(pool: PgPool) {
    let cmd = make_command_row("dup-cmd");
    command_dao::insert(&pool, &cmd).await.unwrap();

    // Insert again — ON CONFLICT DO NOTHING
    let mut cmd2 = make_command_row("dup-cmd");
    cmd2.description = "should be ignored".to_string();
    command_dao::insert(&pool, &cmd2).await.unwrap();

    let found = command_dao::find_by_name(&pool, "dup-cmd")
        .await
        .unwrap()
        .unwrap();
    // Original description preserved
    assert_eq!(found.description, "Test command dup-cmd");
}

#[sqlx::test]
async fn list_all(pool: PgPool) {
    for name in ["cmd-c", "cmd-a", "cmd-b"] {
        command_dao::insert(&pool, &make_command_row(name))
            .await
            .unwrap();
    }

    let all = command_dao::list_all(&pool).await.unwrap();
    assert_eq!(all.len(), 3);
    // Ordered by name
    assert_eq!(all[0].name, "cmd-a");
    assert_eq!(all[1].name, "cmd-b");
    assert_eq!(all[2].name, "cmd-c");
}

#[sqlx::test]
async fn update_by_name(pool: PgPool) {
    command_dao::insert(&pool, &make_command_row("upd-cmd"))
        .await
        .unwrap();

    let updated = command_dao::update_by_name(
        &pool,
        "upd-cmd",
        Some("new desc"),
        None, // prompt unchanged
        None,
        Some("rocket"),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(updated);

    let found = command_dao::find_by_name(&pool, "upd-cmd")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.description, "new desc");
    assert_eq!(found.icon.as_deref(), Some("rocket"));
    assert_eq!(found.prompt, "Do upd-cmd"); // unchanged
}

#[sqlx::test]
async fn delete_by_name(pool: PgPool) {
    command_dao::insert(&pool, &make_command_row("del-cmd"))
        .await
        .unwrap();

    let deleted = command_dao::delete_by_name(&pool, "del-cmd")
        .await
        .unwrap();
    assert!(deleted);

    let found = command_dao::find_by_name(&pool, "del-cmd").await.unwrap();
    assert!(found.is_none());

    // Double delete returns false
    let deleted_again = command_dao::delete_by_name(&pool, "del-cmd")
        .await
        .unwrap();
    assert!(!deleted_again);
}

#[sqlx::test]
async fn count(pool: PgPool) {
    assert_eq!(command_dao::count(&pool).await.unwrap(), 0);

    for name in ["c1", "c2", "c3"] {
        command_dao::insert(&pool, &make_command_row(name))
            .await
            .unwrap();
    }

    assert_eq!(command_dao::count(&pool).await.unwrap(), 3);
}
