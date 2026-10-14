//! The Task-name read against a real database: what access hides.

use moatless_types::access_control::ScopeContext;
use moatless_workspaces::test_support::make_repository_row;
use sqlx::PgPool;

use super::TaskListing;
use crate::task::test_support::{make_task_row, seed_task};

const USER_1: &str = "11111111-1111-1111-1111-111111111111";
const USER_2: &str = "22222222-2222-2222-2222-222222222222";

/// A Task the viewer may not read has no name entry, whether or not it exists.
#[sqlx::test(migrations = "../core/migrations")]
async fn readable_names_omit_what_the_viewer_cannot_read(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    for (task_id, owner) in [("mine", USER_1), ("theirs", USER_2)] {
        seed_task(&pool, &make_task_row(task_id, owner))
            .await
            .expect("seed task");
    }

    let listing = TaskListing::new(pool.clone());
    let names = listing
        .readable_names(
            &ScopeContext::user(USER_1.to_string()),
            &["mine".to_string(), "theirs".to_string(), "gone".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(names.len(), 1);
    assert_eq!(names["mine"], "Test task mine");
}
