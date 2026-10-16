//! PostgreSQL tests for the task repository store.

use sqlx::PgPool;

use super::{PgTaskRepositoryDao, TaskRepositoryDao};
use crate::task::repositories::TaskRepositorySource;
use crate::task::test_support::seed_bare_task;
use moatless_types::access_control::ScopeContext;

async fn seed_repositories(pool: &PgPool, ids: &[&str]) {
    for id in ids {
        sqlx::query(
            "INSERT INTO repositories (id, kind, name, scope) \
             VALUES ($1, 'git', $1, 'global')",
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("seed repository");
    }
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

#[sqlx::test]
async fn a_repository_keeps_the_source_it_was_first_recorded_with(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    seed_repositories(&pool, &["repo-a", "repo-b"]).await;
    let dao = PgTaskRepositoryDao(pool);

    dao.record("task-1", &ids(&["repo-a"]), TaskRepositorySource::Workspace)
        .await
        .unwrap();
    dao.record(
        "task-1",
        &ids(&["repo-a", "repo-b"]),
        TaskRepositorySource::Cloned,
    )
    .await
    .unwrap();

    let mut listed: Vec<(String, TaskRepositorySource)> = dao
        .list("task-1")
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.repository_id, row.source))
        .collect();
    listed.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        listed,
        [
            ("repo-a".to_string(), TaskRepositorySource::Workspace),
            ("repo-b".to_string(), TaskRepositorySource::Cloned),
        ]
    );
}

#[sqlx::test]
async fn only_the_repositories_of_tasks_the_viewer_reads_in_the_workspace_count(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    seed_bare_task(&pool, "task-2", "error").await;
    seed_repositories(&pool, &["repo-a", "repo-b"]).await;
    let dao = PgTaskRepositoryDao(pool);
    dao.record("task-1", &ids(&["repo-a"]), TaskRepositorySource::Cloned)
        .await
        .unwrap();
    dao.record("task-2", &ids(&["repo-b"]), TaskRepositorySource::Cloned)
        .await
        .unwrap();

    let owner = ScopeContext::user("user-1".to_string());
    assert_eq!(
        dao.readable_in_workspace(&owner, "ws_repo-x")
            .await
            .unwrap(),
        ["repo-a"],
        "an errored Task is not listed, so its repository does not count"
    );
    assert!(
        dao.readable_in_workspace(&owner, "ws_other")
            .await
            .unwrap()
            .is_empty()
    );
    let stranger = ScopeContext::user("user-2".to_string());
    assert!(
        dao.readable_in_workspace(&stranger, "ws_repo-x")
            .await
            .unwrap()
            .is_empty()
    );
}
