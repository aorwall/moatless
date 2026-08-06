#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for tag_dao.

use crate::common;

use moatless_core::tag::dao as tag_dao;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    moatless_vibe_backend::repository::dao::insert(pool, &repo)
        .await
        .expect("seed repo");

    let task = make_task_row(task_id, user);
    moatless_vibe_backend::task::dao::insert_task(pool, &task)
        .await
        .expect("seed task");
}

#[sqlx::test]
async fn find_or_create_new_tags(pool: PgPool) {
    let names = vec!["bug".to_string(), "feature".to_string()];
    let tags = tag_dao::find_or_create_tags(&pool, &names).await.unwrap();

    assert_eq!(tags.len(), 2);
    assert!(!tags[0].tag_id.is_empty());
    assert!(!tags[1].tag_id.is_empty());
    // Names should match
    let mut tag_names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
    tag_names.sort_unstable();
    assert_eq!(tag_names, vec!["bug", "feature"]);
}

#[sqlx::test]
async fn find_or_create_existing_tags(pool: PgPool) {
    let names = vec!["existing-tag".to_string()];
    let first = tag_dao::find_or_create_tags(&pool, &names).await.unwrap();
    let first_id = first[0].tag_id.clone();

    // Create again — should return existing, not duplicate
    let second = tag_dao::find_or_create_tags(&pool, &names).await.unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].tag_id, first_id);
}

#[sqlx::test]
async fn set_task_tags(pool: PgPool) {
    seed_task(&pool, "task-tags", "user-1").await;

    let tags = tag_dao::find_or_create_tags(&pool, &["alpha".to_string(), "beta".to_string()])
        .await
        .unwrap();
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();

    tag_dao::set_task_tags(&pool, "task-tags", &tag_ids)
        .await
        .unwrap();

    let task_tags = tag_dao::get_tags_for_task(&pool, "task-tags")
        .await
        .unwrap();
    assert_eq!(task_tags.len(), 2);
    let mut names: Vec<&str> = task_tags.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["alpha", "beta"]);
}

#[sqlx::test]
async fn set_task_tags_replaces(pool: PgPool) {
    seed_task(&pool, "task-replace", "user-1").await;

    // Set tags A, B
    let tags_ab = tag_dao::find_or_create_tags(&pool, &["A".to_string(), "B".to_string()])
        .await
        .unwrap();
    let ids_ab: Vec<String> = tags_ab.iter().map(|t| t.tag_id.clone()).collect();
    tag_dao::set_task_tags(&pool, "task-replace", &ids_ab)
        .await
        .unwrap();

    // Replace with B, C
    let tags_bc = tag_dao::find_or_create_tags(&pool, &["B".to_string(), "C".to_string()])
        .await
        .unwrap();
    let ids_bc: Vec<String> = tags_bc.iter().map(|t| t.tag_id.clone()).collect();
    tag_dao::set_task_tags(&pool, "task-replace", &ids_bc)
        .await
        .unwrap();

    let result = tag_dao::get_tags_for_task(&pool, "task-replace")
        .await
        .unwrap();
    assert_eq!(result.len(), 2);
    let mut names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["B", "C"]);
}

#[sqlx::test]
async fn get_tags_for_tasks_batch(pool: PgPool) {
    let user = "user-1";
    let repo = make_repository_row("repo-1", user);
    moatless_vibe_backend::repository::dao::insert(&pool, &repo)
        .await
        .unwrap();

    let t1 = make_task_row("task-batch-1", user);
    moatless_vibe_backend::task::dao::insert_task(&pool, &t1)
        .await
        .unwrap();
    let t2 = make_task_row("task-batch-2", user);
    moatless_vibe_backend::task::dao::insert_task(&pool, &t2)
        .await
        .unwrap();

    let tags = tag_dao::find_or_create_tags(&pool, &["x".to_string(), "y".to_string()])
        .await
        .unwrap();
    let x_id = &tags.iter().find(|t| t.name == "x").unwrap().tag_id;
    let y_id = &tags.iter().find(|t| t.name == "y").unwrap().tag_id;

    tag_dao::set_task_tags(&pool, "task-batch-1", std::slice::from_ref(x_id))
        .await
        .unwrap();
    tag_dao::set_task_tags(&pool, "task-batch-2", std::slice::from_ref(y_id))
        .await
        .unwrap();

    let all = tag_dao::get_tags_for_tasks(
        &pool,
        &["task-batch-1".to_string(), "task-batch-2".to_string()],
    )
    .await
    .unwrap();

    assert_eq!(all.len(), 2);
    let t1_tags: Vec<&str> = all
        .iter()
        .filter(|t| t.task_id == "task-batch-1")
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(t1_tags, vec!["x"]);
}

#[sqlx::test]
async fn list_all(pool: PgPool) {
    tag_dao::find_or_create_tags(
        &pool,
        &["zulu".to_string(), "alpha".to_string(), "mike".to_string()],
    )
    .await
    .unwrap();

    let all = tag_dao::list_all(&pool).await.unwrap();
    assert_eq!(all.len(), 3);
    // Should be ordered by name
    assert_eq!(all[0].name, "alpha");
    assert_eq!(all[1].name, "mike");
    assert_eq!(all[2].name, "zulu");
}
