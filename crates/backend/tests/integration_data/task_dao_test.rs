#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for task_dao.

use crate::common;

use chrono::{DateTime, Duration, Utc};
use moatless_types::access_control::ScopeContext;
use moatless_backend::task::dao::{
    self as task_dao, PgTaskDao, TaskDao, TaskFilter, TaskPatch,
};
use moatless_backend::task::models::{TaskStatus, Visibility};
use sqlx::PgPool;

use common::{make_repository_row, make_task_row};

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
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
    moatless_backend::repository::dao::insert(pool, &row)
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
async fn system_context_reads_any_task(pool: PgPool) {
    let owner = "user-1";
    seed_repo(&pool, "repo-1", owner).await;

    let row = make_task_row("task-internal", owner);
    task_dao::insert_task(&pool, &row).await.unwrap();

    // Another user cannot read the private user-scoped task...
    let found = task_dao::find_by_id(&pool, "task-internal", &scope_ctx("user-2"))
        .await
        .unwrap();
    assert!(found.is_none());

    // ...but background/system callers read through the same scoped method.
    // reason: exercising the system-context path replacing find_by_id_internal
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let found = task_dao::find_by_id(&pool, "task-internal", &sys)
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
    let (rows, total) = PgTaskDao(pool).list_tasks(&filter).await.unwrap();

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
    let tags = moatless_backend::tag::dao::find_or_create_tags(&pool, &["bug".to_string()])
        .await
        .unwrap();
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    moatless_backend::tag::dao::set_task_tags(&pool, "task-tagged", &tag_ids)
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
        &scope_ctx(user),
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
        &scope_ctx(user),
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

    task_dao::delete_task(&pool, "task-del", &scope_ctx(user))
        .await
        .unwrap();

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

    let repo = PgTaskDao(pool.clone());

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

async fn follow_state(
    pool: &PgPool,
    user_id: &str,
    task_id: &str,
) -> Option<(
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
)> {
    sqlx::query_as(
        r#"SELECT settled_override, settled_at, snoozed_until, snoozed_at
           FROM task_follows
           WHERE user_id = $1 AND task_id = $2"#,
    )
    .bind(user_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn unfollow_removes_settlement_and_snooze_with_the_follow_row(pool: PgPool) {
    let owner = "user-1";
    let follower = "user-2";
    seed_repo(&pool, "repo-1", owner).await;
    let mut row = make_task_row("task-state-delete", owner);
    row.visibility = Visibility::Public;
    task_dao::insert_task(&pool, &row).await.unwrap();

    let repo = PgTaskDao(pool.clone());
    repo.follow_task(follower, "task-state-delete")
        .await
        .unwrap();
    repo.settle_task(follower, "task-state-delete", Utc::now())
        .await
        .unwrap();
    repo.snooze_task(
        follower,
        "task-state-delete",
        Utc::now() + Duration::hours(1),
        Utc::now(),
    )
    .await
    .unwrap();

    repo.unfollow_task(follower, "task-state-delete")
        .await
        .unwrap();

    assert!(follow_state(&pool, follower, "task-state-delete")
        .await
        .is_none());
}

#[sqlx::test]
async fn refollow_after_settle_and_unfollow_starts_with_null_state(pool: PgPool) {
    let owner = "user-1";
    let follower = "user-2";
    seed_repo(&pool, "repo-1", owner).await;
    let mut row = make_task_row("task-state-refollow", owner);
    row.visibility = Visibility::Public;
    task_dao::insert_task(&pool, &row).await.unwrap();

    let repo = PgTaskDao(pool.clone());
    repo.follow_task(follower, "task-state-refollow")
        .await
        .unwrap();
    repo.settle_task(follower, "task-state-refollow", Utc::now())
        .await
        .unwrap();
    repo.snooze_task(
        follower,
        "task-state-refollow",
        Utc::now() + Duration::hours(1),
        Utc::now(),
    )
    .await
    .unwrap();
    repo.unfollow_task(follower, "task-state-refollow")
        .await
        .unwrap();

    repo.follow_task(follower, "task-state-refollow")
        .await
        .unwrap();

    assert_eq!(
        follow_state(&pool, follower, "task-state-refollow").await,
        Some((None, None, None, None))
    );
}

// =========================================================================
// Pagination invariants (DB-1)
//
// The page and the total must come from the same distinct, filtered task
// relation: tags must not fan a task out into several counted rows, every
// filter and the access-control predicate must constrain both, and an offset
// past the end must still report the true total.
// =========================================================================

/// A `TaskFilter` with no filters set, so each test only names what it varies.
fn base_filter(user_id: &str) -> TaskFilter {
    TaskFilter {
        scope: scope_ctx(user_id),
        limit: 50,
        offset: 0,
        status: None,
        exclude_status: None,
        repository_id: None,
        created_by: None,
        tag: None,
        parent_task_id: None,
        followed_by: None,
    }
}

/// Attach the named tags to a task.
async fn tag_task(pool: &PgPool, task_id: &str, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
    let tags = moatless_backend::tag::dao::find_or_create_tags(pool, &names)
        .await
        .expect("create tags");
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    moatless_backend::tag::dao::set_task_tags(pool, task_id, &tag_ids)
        .await
        .expect("set task tags");
}

#[sqlx::test]
async fn list_tasks_multiple_tags_contribute_one_row_and_one_count(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    task_dao::insert_task(&pool, &make_task_row("task-many-tags", user))
        .await
        .unwrap();
    task_dao::insert_task(&pool, &make_task_row("task-no-tags", user))
        .await
        .unwrap();
    tag_task(&pool, "task-many-tags", &["bug", "urgent", "backend"]).await;

    let repo = PgTaskDao(pool.clone());

    // Unfiltered: the three tags must not fan the task out into three counted
    // rows — two tasks exist, so the total is two.
    let (rows, total) = repo.list_tasks(&base_filter(user)).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(total, 2);

    // The total is the size of the filtered relation, not of the page.
    let filter = TaskFilter {
        limit: 1,
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(total, 2);

    // Filtering on one of the three tags still yields exactly one row/count.
    let filter = TaskFilter {
        tag: Some("urgent".to_string()),
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].task_id, "task-many-tags");
    assert_eq!(total, 1);
}

#[sqlx::test]
async fn list_tasks_offset_past_end_returns_empty_page_with_true_total(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    for i in 0..3 {
        task_dao::insert_task(&pool, &make_task_row(&format!("task-off-{i}"), user))
            .await
            .unwrap();
    }
    // Tags on one task would inflate a join-fanned count.
    tag_task(&pool, "task-off-0", &["bug", "urgent"]).await;

    let repo = PgTaskDao(pool.clone());

    // Offset exactly at the end.
    let filter = TaskFilter {
        limit: 2,
        offset: 3,
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 3);

    // Offset far beyond the end.
    let filter = TaskFilter {
        limit: 2,
        offset: 1_000,
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 3);

    // Offset past the end of a filtered relation reports that relation's total.
    let filter = TaskFilter {
        limit: 2,
        offset: 50,
        tag: Some("bug".to_string()),
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 1);

    // An empty relation still reports a total, and it is zero.
    let filter = TaskFilter {
        tag: Some("nonexistent".to_string()),
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 0);
}

#[sqlx::test]
async fn list_tasks_acl_predicate_constrains_page_and_total(pool: PgPool) {
    let owner = "user-1";
    let other = "user-2";
    seed_repo(&pool, "repo-1", owner).await;
    seed_team(&pool, other).await;

    // Two private tasks the other user must never see, plus one public one.
    for i in 0..2 {
        task_dao::insert_task(&pool, &make_task_row(&format!("task-private-{i}"), owner))
            .await
            .unwrap();
    }
    let mut public = make_task_row("task-public", owner);
    public.visibility = Visibility::Public;
    task_dao::insert_task(&pool, &public).await.unwrap();

    // Tags on the hidden tasks must not leak into the other user's total.
    tag_task(&pool, "task-private-0", &["bug", "urgent"]).await;
    tag_task(&pool, "task-public", &["bug", "urgent"]).await;

    // The other user's own task.
    task_dao::insert_task(&pool, &make_task_row("task-own", other))
        .await
        .unwrap();

    let repo = PgTaskDao(pool.clone());

    let (rows, total) = repo.list_tasks(&base_filter(other)).await.unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.task_id.as_str()).collect();
    assert_eq!(total, 2, "only the public task and the caller's own task");
    assert!(ids.contains(&"task-own"));
    assert!(ids.contains(&"task-public"));
    assert!(!ids.contains(&"task-private-0"));
    assert!(!ids.contains(&"task-private-1"));

    // The owner sees everything they own plus their public task.
    let (_, total) = repo.list_tasks(&base_filter(owner)).await.unwrap();
    assert_eq!(total, 3);

    // The access-control predicate still bounds the total on an empty page.
    let filter = TaskFilter {
        offset: 100,
        ..base_filter(other)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 2);

    // Combined with a tag filter, the ACL still applies to both.
    let filter = TaskFilter {
        tag: Some("bug".to_string()),
        ..base_filter(other)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-public");
}

#[sqlx::test]
async fn list_tasks_parent_relation_filter_counts_tagged_child_once(pool: PgPool) {
    use moatless_backend::task::relations::{
        PgTaskRelationDao, TaskRelation, TaskRelationDao, TaskRelationType,
    };

    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    for id in ["task-parent", "task-child-a", "task-child-b", "task-orphan"] {
        task_dao::insert_task(&pool, &make_task_row(id, user))
            .await
            .unwrap();
    }
    tag_task(&pool, "task-child-a", &["bug", "urgent", "backend"]).await;

    let relations = PgTaskRelationDao(pool.clone());
    for child in ["task-child-a", "task-child-b"] {
        relations
            .insert(&TaskRelation::new(
                child,
                "task-parent",
                TaskRelationType::CreatedBy,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
    }

    let repo = PgTaskDao(pool.clone());

    let filter = TaskFilter {
        parent_task_id: Some("task-parent".to_string()),
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.task_id.as_str()).collect();
    assert_eq!(total, 2);
    assert_eq!(rows.len(), 2);
    assert!(ids.contains(&"task-child-a"));
    assert!(ids.contains(&"task-child-b"));

    // Offset past the end of the children relation keeps the true total.
    let filter = TaskFilter {
        parent_task_id: Some("task-parent".to_string()),
        offset: 10,
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 2);
}

#[sqlx::test]
async fn list_tasks_followed_by_and_status_filters_count_tagged_task_once(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    task_dao::insert_task(&pool, &make_task_row("task-followed-open", user))
        .await
        .unwrap();
    let mut closed = make_task_row("task-followed-closed", user);
    closed.status = TaskStatus::Closed;
    task_dao::insert_task(&pool, &closed).await.unwrap();
    task_dao::insert_task(&pool, &make_task_row("task-unfollowed", user))
        .await
        .unwrap();

    tag_task(&pool, "task-followed-open", &["bug", "urgent", "backend"]).await;
    tag_task(&pool, "task-followed-closed", &["bug", "urgent"]).await;

    let repo = PgTaskDao(pool.clone());
    for id in ["task-followed-open", "task-followed-closed"] {
        repo.follow_task(user, id).await.unwrap();
    }
    // Insertion auto-follows the creator, so the third task has to be
    // *unfollowed* to be the one this filter must leave out.
    repo.unfollow_task(user, "task-unfollowed").await.unwrap();

    // followed_by alone: two followed tasks, each counted once despite tags.
    let filter = TaskFilter {
        followed_by: Some(user.to_string()),
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(total, 2);

    // followed_by + status.
    let filter = TaskFilter {
        followed_by: Some(user.to_string()),
        status: Some(TaskStatus::Open),
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-followed-open");

    // followed_by + status exclusion + tag, then the same past the end.
    let filter = TaskFilter {
        followed_by: Some(user.to_string()),
        exclude_status: Some(TaskStatus::Closed),
        tag: Some("bug".to_string()),
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-followed-open");

    let filter = TaskFilter {
        followed_by: Some(user.to_string()),
        exclude_status: Some(TaskStatus::Closed),
        tag: Some("bug".to_string()),
        offset: 25,
        ..base_filter(user)
    };
    let (rows, total) = repo.list_tasks(&filter).await.unwrap();
    assert!(rows.is_empty());
    assert_eq!(total, 1);
}
