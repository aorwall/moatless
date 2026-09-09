//! PostgreSQL tests for the tag store.

use crate::tag::dao::{PgTagDao, TagDao};
use sqlx::PgPool;

use crate::task::test_support::{make_task_row, seed_task as seed_task_row};
use moatless_workspaces::test_support::make_repository_row;

async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");

    let task = make_task_row(task_id, user);
    seed_task_row(pool, &task).await.expect("seed task");
}

#[sqlx::test]
async fn find_or_create_new_tags(pool: PgPool) {
    let names = vec!["bug".to_string(), "feature".to_string()];
    let tags = PgTagDao(pool.clone())
        .find_or_create_tags(&names)
        .await
        .unwrap();

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
    let first = PgTagDao(pool.clone())
        .find_or_create_tags(&names)
        .await
        .unwrap();
    let first_id = first[0].tag_id.clone();

    // Create again — should return existing, not duplicate
    let second = PgTagDao(pool.clone())
        .find_or_create_tags(&names)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].tag_id, first_id);
}

#[sqlx::test]
async fn set_task_tags(pool: PgPool) {
    seed_task(&pool, "task-tags", "user-1").await;

    let tags = PgTagDao(pool.clone())
        .find_or_create_tags(&["alpha".to_string(), "beta".to_string()])
        .await
        .unwrap();
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();

    PgTagDao(pool.clone())
        .set_task_tags("task-tags", &tag_ids)
        .await
        .unwrap();

    let task_tags = PgTagDao(pool.clone())
        .get_tags_for_task("task-tags")
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
    let tags_ab = PgTagDao(pool.clone())
        .find_or_create_tags(&["A".to_string(), "B".to_string()])
        .await
        .unwrap();
    let ids_ab: Vec<String> = tags_ab.iter().map(|t| t.tag_id.clone()).collect();
    PgTagDao(pool.clone())
        .set_task_tags("task-replace", &ids_ab)
        .await
        .unwrap();

    // Replace with B, C
    let tags_bc = PgTagDao(pool.clone())
        .find_or_create_tags(&["B".to_string(), "C".to_string()])
        .await
        .unwrap();
    let ids_bc: Vec<String> = tags_bc.iter().map(|t| t.tag_id.clone()).collect();
    PgTagDao(pool.clone())
        .set_task_tags("task-replace", &ids_bc)
        .await
        .unwrap();

    let result = PgTagDao(pool.clone())
        .get_tags_for_task("task-replace")
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
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .unwrap();

    let t1 = make_task_row("task-batch-1", user);
    seed_task_row(&pool, &t1).await.unwrap();
    let t2 = make_task_row("task-batch-2", user);
    seed_task_row(&pool, &t2).await.unwrap();

    let tags = PgTagDao(pool.clone())
        .find_or_create_tags(&["x".to_string(), "y".to_string()])
        .await
        .unwrap();
    let x_id = &tags.iter().find(|t| t.name == "x").unwrap().tag_id;
    let y_id = &tags.iter().find(|t| t.name == "y").unwrap().tag_id;

    PgTagDao(pool.clone())
        .set_task_tags("task-batch-1", std::slice::from_ref(x_id))
        .await
        .unwrap();
    PgTagDao(pool.clone())
        .set_task_tags("task-batch-2", std::slice::from_ref(y_id))
        .await
        .unwrap();

    let all = PgTagDao(pool.clone())
        .get_tags_for_tasks(&["task-batch-1".to_string(), "task-batch-2".to_string()])
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
    PgTagDao(pool.clone())
        .find_or_create_tags(&["zulu".to_string(), "alpha".to_string(), "mike".to_string()])
        .await
        .unwrap();

    let all = PgTagDao(pool.clone()).list_all().await.unwrap();
    assert_eq!(all.len(), 3);
    // Should be ordered by name
    assert_eq!(all[0].name, "alpha");
    assert_eq!(all[1].name, "mike");
    assert_eq!(all[2].name, "zulu");
}
