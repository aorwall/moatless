#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for task_dao.

mod common;

use moatless_vibe_backend::scope::ScopeContext;
use moatless_vibe_backend::task::dao::{
    self as task_dao, PgTaskRepository, TaskFilter, TaskPatch, TaskRepository,
};
use moatless_vibe_backend::task::models::{TaskStatus, Visibility};
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        team_ids: vec![],
        is_admin: false,
    }
}

/// Helper: insert the personal team a repository references, satisfying the
/// `repositories_team_id_fkey` FK. Idempotent so repeated calls within a test
/// are harmless. `make_repository_row` sets `team_id = personal-{user_id}`.
async fn seed_team(pool: &PgPool, user_id: &str) {
    sqlx::query(
        r#"INSERT INTO teams (id, name, slug)
           VALUES ($1, $2, $1)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(format!("personal-{user_id}"))
    .bind(format!("{user_id} Personal"))
    .execute(pool)
    .await
    .expect("seed team");
}

/// Helper: insert a repository so task FK references are satisfied.
async fn seed_repo(pool: &PgPool, id: &str, user_id: &str) {
    seed_team(pool, user_id).await;
    let row = make_repository_row(id, user_id);
    moatless_vibe_backend::repository::dao::insert(pool, &row)
        .await
        .expect("seed repo");
}

#[sqlx::test]
async fn insert_and_find_by_id(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let row = make_task_row("task-aaa", user);
    task_dao::insert_task(&pool, &row).await.unwrap();

    let found = task_dao::find_by_id(&pool, "task-aaa", &scope_ctx(user))
        .await
        .unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.task_id, "task-aaa");
    assert_eq!(found.status, TaskStatus::Open);
    assert_eq!(found.created_by.as_deref(), Some(user));
}

#[sqlx::test]
async fn find_by_id_access_control_private(pool: PgPool) {
    let owner = "user-1";
    let other = "user-2";
    seed_repo(&pool, "repo-1", owner).await;

    let row = make_task_row("task-private", owner);
    task_dao::insert_task(&pool, &row).await.unwrap();

    // Other user cannot see private task
    let found = task_dao::find_by_id(&pool, "task-private", &scope_ctx(other))
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn find_by_id_access_control_public(pool: PgPool) {
    let owner = "user-1";
    let other = "user-2";
    seed_repo(&pool, "repo-1", owner).await;

    let mut row = make_task_row("task-public", owner);
    row.visibility = Visibility::Public;
    task_dao::insert_task(&pool, &row).await.unwrap();

    // Other user can see public task
    let found = task_dao::find_by_id(&pool, "task-public", &scope_ctx(other))
        .await
        .unwrap();
    assert!(found.is_some());
}

#[sqlx::test]
async fn find_by_id_internal_bypasses_visibility(pool: PgPool) {
    let owner = "user-1";
    seed_repo(&pool, "repo-1", owner).await;

    let row = make_task_row("task-internal", owner);
    task_dao::insert_task(&pool, &row).await.unwrap();

    // Internal find ignores visibility
    let found = task_dao::find_by_id_internal(&pool, "task-internal")
        .await
        .unwrap();
    assert!(found.is_some());
}

#[sqlx::test]
async fn list_tasks_pagination(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    for i in 0..5 {
        let row = make_task_row(&format!("task-page-{i}"), user);
        task_dao::insert_task(&pool, &row).await.unwrap();
    }

    let (rows, total) =
        task_dao::list_tasks(&pool, &scope_ctx(user), 2, 0, None, None, None, None, None)
            .await
            .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(total, 5);
}

#[sqlx::test]
async fn list_tasks_status_filter(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let open_task = make_task_row("task-open", user);
    task_dao::insert_task(&pool, &open_task).await.unwrap();

    let mut closed_task = make_task_row("task-closed", user);
    closed_task.status = TaskStatus::Closed;
    task_dao::insert_task(&pool, &closed_task).await.unwrap();

    let (rows, total) = task_dao::list_tasks(
        &pool,
        &scope_ctx(user),
        50,
        0,
        Some(TaskStatus::Open),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-open");
}

#[sqlx::test]
async fn list_tasks_exclude_status_filter(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let open_task = make_task_row("task-open", user);
    task_dao::insert_task(&pool, &open_task).await.unwrap();

    let mut closed_task = make_task_row("task-closed", user);
    closed_task.status = TaskStatus::Closed;
    task_dao::insert_task(&pool, &closed_task).await.unwrap();

    let mut error_task = make_task_row("task-error", user);
    error_task.status = TaskStatus::Error;
    task_dao::insert_task(&pool, &error_task).await.unwrap();

    let filter = TaskFilter {
        scope: scope_ctx(user),
        limit: 50,
        offset: 0,
        status: None,
        exclude_status: Some(TaskStatus::Closed),
        repository_id: None,
        created_by: None,
        tag: None,
        parent_task_id: None,
        followed_by: None,
    };
    let (rows, total) = PgTaskRepository(pool).list_tasks(&filter).await.unwrap();

    assert_eq!(total, 2);
    let ids: Vec<&str> = rows.iter().map(|r| r.task_id.as_str()).collect();
    assert!(ids.contains(&"task-open"));
    assert!(ids.contains(&"task-error"));
    assert!(!ids.contains(&"task-closed"));
}

#[sqlx::test]
async fn list_tasks_tag_filter(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let task_a = make_task_row("task-tagged", user);
    task_dao::insert_task(&pool, &task_a).await.unwrap();

    let task_b = make_task_row("task-untagged", user);
    task_dao::insert_task(&pool, &task_b).await.unwrap();

    // Create tag and assign
    let tags = moatless_vibe_backend::tag::dao::find_or_create_tags(&pool, &["bug".to_string()])
        .await
        .unwrap();
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    moatless_vibe_backend::tag::dao::set_task_tags(&pool, "task-tagged", &tag_ids)
        .await
        .unwrap();

    let (rows, total) = task_dao::list_tasks(
        &pool,
        &scope_ctx(user),
        50,
        0,
        None,
        None,
        None,
        Some("bug"),
        None,
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-tagged");
}

#[sqlx::test]
async fn update_task_partial(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let row = make_task_row("task-update", user);
    task_dao::insert_task(&pool, &row).await.unwrap();

    task_dao::update_task(
        &pool,
        "task-update",
        &TaskPatch {
            name: Some("Updated name".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let found = task_dao::find_by_id(&pool, "task-update", &scope_ctx(user))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name.as_deref(), Some("Updated name"));
    assert_eq!(found.status, TaskStatus::Open); // unchanged
}

#[sqlx::test]
async fn update_task_cost_increment(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let mut row = make_task_row("task-cost", user);
    row.total_cost_usd = 1.0;
    task_dao::insert_task(&pool, &row).await.unwrap();

    task_dao::update_task(
        &pool,
        "task-cost",
        &TaskPatch {
            total_cost_usd_increment: Some(0.5),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let found = task_dao::find_by_id(&pool, "task-cost", &scope_ctx(user))
        .await
        .unwrap()
        .unwrap();
    assert!((found.total_cost_usd - 1.5).abs() < 0.001);
}

#[sqlx::test]
async fn delete_task(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let row = make_task_row("task-del", user);
    task_dao::insert_task(&pool, &row).await.unwrap();

    task_dao::delete_task(&pool, "task-del").await.unwrap();

    let found = task_dao::find_by_id(&pool, "task-del", &scope_ctx(user))
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn exists_by_short_id(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let mut row = make_task_row("task-short", user);
    row.short_id = Some("abcd1234".to_string());
    task_dao::insert_task(&pool, &row).await.unwrap();

    assert!(task_dao::exists_by_short_id(&pool, "abcd1234")
        .await
        .unwrap());
    assert!(!task_dao::exists_by_short_id(&pool, "xxxxxxxx")
        .await
        .unwrap());
}

#[sqlx::test]
async fn follow_unfollow_and_list_followed(pool: PgPool) {
    let owner = "user-1";
    let follower = "user-2";
    seed_repo(&pool, "repo-1", owner).await;

    // A public task by `owner` that `follower` can see and therefore follow.
    let mut row = make_task_row("task-followed", owner);
    row.visibility = Visibility::Public;
    task_dao::insert_task(&pool, &row).await.unwrap();

    let repo = PgTaskRepository(pool.clone());

    // Initially the follower follows nothing.
    let followed = repo
        .get_followed(follower, &["task-followed".to_string()])
        .await
        .unwrap();
    assert!(followed.is_empty());

    // Follow, then the followed_by filter surfaces the task.
    repo.follow_task(follower, "task-followed").await.unwrap();

    let followed = repo
        .get_followed(follower, &["task-followed".to_string()])
        .await
        .unwrap();
    assert!(followed.contains("task-followed"));

    let filter = TaskFilter {
        scope: scope_ctx(follower),
        limit: 50,
        offset: 0,
        status: None,
        exclude_status: None,
        repository_id: None,
        created_by: None,
        tag: None,
        parent_task_id: None,
        followed_by: Some(follower.to_string()),
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-followed");

    // Following is idempotent — a second follow does not duplicate the row.
    repo.follow_task(follower, "task-followed").await.unwrap();
    let (_, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(total, 1);

    // Unfollow removes it from the followed_by filter again.
    repo.unfollow_task(follower, "task-followed").await.unwrap();
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(total, 0);
    assert!(rows.is_empty());

    let followed = repo
        .get_followed(follower, &["task-followed".to_string()])
        .await
        .unwrap();
    assert!(followed.is_empty());
}
