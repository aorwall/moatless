//! PostgreSQL tests for the task store.

use crate::task::dao::{TaskDao, store_over};
use crate::task::models::{SandboxPlacement, TaskFilter, TaskPatch};
use crate::task::models::{TaskStatus, Visibility};
use crate::task::pin_dao::OrderSlot;
use chrono::{DateTime, Duration, Utc};
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentProfile;
use sqlx::PgPool;

use crate::task::test_support::{
    find_task, list_tasks, make_task_row, patch_task, recompute_total_cost, remove_task, seed_task,
    short_id_exists,
};
use moatless_workspaces::test_support::make_repository_row;

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext::user(user_id)
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
    moatless_workspaces::compose::seed_repository(pool, &row)
        .await
        .expect("seed repo");
}

#[sqlx::test]
async fn insert_and_find_by_id(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let row = make_task_row("task-aaa", user);
    seed_task(&pool, &row).await.unwrap();

    let found = find_task(&pool, "task-aaa", &scope_ctx(user))
        .await
        .unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.task_id, "task-aaa");
    assert_eq!(found.status, TaskStatus::Open);
    assert_eq!(found.created_by.as_deref(), Some(user));
}

#[sqlx::test]
async fn loop_attribution_and_reporting_do_not_subscribe_personal_feeds(pool: PgPool) {
    sqlx::query("INSERT INTO loops (id, name, kind, scope, created_by) VALUES ('loop-1', 'Test', 'manual', 'global', 'loop-user')")
        .execute(&pool)
        .await
        .unwrap();
    let mut task = make_task_row("task-loop", "loop-user");
    task.loop_id = Some("loop-1".into());
    task.reporter_user_id = Some("reporter".into());
    task.owner_user_id = Some("reporter".into());
    seed_task(&pool, &task).await.unwrap();
    let commands = store_over(&pool);

    let found = find_task(&pool, "task-loop", &scope_ctx("reporter"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.created_by.as_deref(), Some("loop-user"));
    assert_eq!(found.reporter_user_id.as_deref(), Some("reporter"));
    let (listed, _) = list_tasks(&pool, &base_filter("reporter")).await.unwrap();
    assert_eq!(listed[0].reporter_user_id.as_deref(), Some("reporter"));
    for user in ["loop-user", "reporter"] {
        assert!(
            commands
                .get_followed(user, std::slice::from_ref(&task.task_id))
                .await
                .unwrap()
                .is_empty()
        );
    }

    commands
        .follow_task("reporter", &task.task_id)
        .await
        .unwrap();
    assert_eq!(
        commands
            .get_followed("reporter", &[task.task_id])
            .await
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test]
async fn directly_created_tasks_still_follow_the_creator(pool: PgPool) {
    let task = make_task_row("task-direct", "creator");
    seed_task(&pool, &task).await.unwrap();
    assert_eq!(
        store_over(&pool)
            .get_followed("creator", &[task.task_id])
            .await
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test]
async fn find_by_id_access_control_private(pool: PgPool) {
    let owner = "user-1";
    let other = "user-2";
    seed_repo(&pool, "repo-1", owner).await;

    let row = make_task_row("task-private", owner);
    seed_task(&pool, &row).await.unwrap();

    let found = find_task(&pool, "task-private", &scope_ctx(other))
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
    seed_task(&pool, &row).await.unwrap();

    let found = find_task(&pool, "task-public", &scope_ctx(other))
        .await
        .unwrap();
    assert!(found.is_some());
}

#[sqlx::test]
async fn system_context_reads_any_task(pool: PgPool) {
    let owner = "user-1";
    seed_repo(&pool, "repo-1", owner).await;

    let row = make_task_row("task-internal", owner);
    seed_task(&pool, &row).await.unwrap();

    let found = find_task(&pool, "task-internal", &scope_ctx("user-2"))
        .await
        .unwrap();
    assert!(found.is_none());

    // ...but background/system callers read through the same scoped method.
    // reason: exercising the system-context path replacing find_by_id_internal
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let found = find_task(&pool, "task-internal", &sys).await.unwrap();
    assert!(found.is_some());
}

#[sqlx::test]
async fn list_tasks_pagination(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    for i in 0..5 {
        let row = make_task_row(&format!("task-page-{i}"), user);
        seed_task(&pool, &row).await.unwrap();
    }

    let filter = TaskFilter {
        limit: 2,
        ..base_filter(user)
    };
    let (rows, total) = list_tasks(&pool, &filter).await.unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(total, 5);
}

/// Seeds `task-retired` with a stored `agent_profile` `AgentProfile` rejects.
async fn seed_retired_profile_task(pool: &PgPool, user: &str, profile: &str) {
    seed_repo(pool, "repo-1", user).await;
    seed_task(pool, &make_task_row("task-retired", user))
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET agent_profile = $1 WHERE task_id = 'task-retired'")
        .bind(profile)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn list_tasks_error_names_the_undecodable_task(pool: PgPool) {
    let user = "user-1";
    seed_retired_profile_task(&pool, user, "claude-code-tmux").await;

    let err = list_tasks(&pool, &base_filter(user)).await.unwrap_err();

    assert!(err.to_string().contains("task-retired"), "{err}");
}

#[sqlx::test]
async fn retired_agent_profiles_migration_makes_tasks_listable(pool: PgPool) {
    let user = "user-1";
    seed_retired_profile_task(&pool, user, "claude-code-tmux").await;
    seed_task(&pool, &make_task_row("task-e2e", user))
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET agent_profile = 'not-an-agent' WHERE task_id = 'task-e2e'")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::raw_sql(include_str!(
        "../../../core/migrations/0197_retired_agent_profiles.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let (rows, _) = list_tasks(&pool, &base_filter(user)).await.unwrap();
    let profile = |id: &str| rows.iter().find(|t| t.task_id == id).unwrap().agent_profile;
    assert_eq!(profile("task-retired"), AgentProfile::ClaudeCodeCli);
    assert_eq!(profile("task-e2e"), AgentProfile::ClaudeCode);
}

#[sqlx::test]
async fn list_tasks_status_filter(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let open_task = make_task_row("task-open", user);
    seed_task(&pool, &open_task).await.unwrap();

    let mut closed_task = make_task_row("task-closed", user);
    closed_task.status = TaskStatus::Closed;
    seed_task(&pool, &closed_task).await.unwrap();

    let filter = TaskFilter {
        status: Some(TaskStatus::Open),
        ..base_filter(user)
    };
    let (rows, total) = list_tasks(&pool, &filter).await.unwrap();

    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-open");
}

#[sqlx::test]
async fn list_tasks_exclude_status_filter(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let open_task = make_task_row("task-open", user);
    seed_task(&pool, &open_task).await.unwrap();

    let mut closed_task = make_task_row("task-closed", user);
    closed_task.status = TaskStatus::Closed;
    seed_task(&pool, &closed_task).await.unwrap();

    let mut error_task = make_task_row("task-error", user);
    error_task.status = TaskStatus::Error;
    seed_task(&pool, &error_task).await.unwrap();

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
    let (rows, total) = store_over(&pool).list_tasks(&filter).await.unwrap();

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
    seed_task(&pool, &task_a).await.unwrap();

    let task_b = make_task_row("task-untagged", user);
    seed_task(&pool, &task_b).await.unwrap();

    let tags = crate::tag::test_support::find_or_create_tags(&pool, &["bug".to_string()])
        .await
        .unwrap();
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    crate::tag::test_support::set_task_tags(&pool, "task-tagged", &tag_ids)
        .await
        .unwrap();

    let filter = TaskFilter {
        tag: Some("bug".to_string()),
        ..base_filter(user)
    };
    let (rows, total) = list_tasks(&pool, &filter).await.unwrap();

    assert_eq!(total, 1);
    assert_eq!(rows[0].task_id, "task-tagged");
}

#[sqlx::test]
async fn update_task_partial(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let row = make_task_row("task-update", user);
    seed_task(&pool, &row).await.unwrap();

    patch_task(
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

    let found = find_task(&pool, "task-update", &scope_ctx(user))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name.as_deref(), Some("Updated name"));
    assert_eq!(found.status, TaskStatus::Open); // unchanged
}

#[sqlx::test]
async fn recompute_total_cost_sums_priced_turns(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    // Start from a non-zero stored total to prove the recompute overwrites
    // rather than adds to it — the idempotency property a blind increment lacks.
    let mut row = make_task_row("task-cost", user);
    row.total_cost_usd = 99.0;
    seed_task(&pool, &row).await.unwrap();

    // Two priced turns and one that never priced (JSON null): the null turn is
    // skipped, not summed as zero-or-error.
    for (turn, metrics) in [
        (1, serde_json::json!({ "totalCostUsd": 0.25 })),
        (2, serde_json::json!({ "totalCostUsd": 1.5 })),
        (
            3,
            serde_json::json!({ "totalCostUsd": serde_json::Value::Null }),
        ),
    ] {
        sqlx::query(
            "INSERT INTO task_turns (task_id, turn_number, status, metrics) \
             VALUES ($1, $2, 'active', $3)",
        )
        .bind("task-cost")
        .bind(turn)
        .bind(&metrics)
        .execute(&pool)
        .await
        .unwrap();
    }

    recompute_total_cost(&pool, "task-cost").await.unwrap();

    let found = find_task(&pool, "task-cost", &scope_ctx(user))
        .await
        .unwrap()
        .unwrap();
    assert!((found.total_cost_usd - 1.75).abs() < 0.001);

    // Idempotent: running it again yields the same total, not a doubled one.
    recompute_total_cost(&pool, "task-cost").await.unwrap();
    let again = find_task(&pool, "task-cost", &scope_ctx(user))
        .await
        .unwrap()
        .unwrap();
    assert!((again.total_cost_usd - 1.75).abs() < 0.001);
}

#[sqlx::test]
async fn delete_task(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let row = make_task_row("task-del", user);
    seed_task(&pool, &row).await.unwrap();

    remove_task(&pool, "task-del", &scope_ctx(user))
        .await
        .unwrap();

    let found = find_task(&pool, "task-del", &scope_ctx(user))
        .await
        .unwrap();
    assert!(found.is_none());
}

#[sqlx::test]
async fn deleting_a_child_records_a_change_for_its_parent(pool: PgPool) {
    use crate::task::relations::{TaskRelation, TaskRelationType};
    use crate::task::relations_dao::{PgTaskRelationDao, TaskRelationDao};

    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;
    for id in ["task-parent", "task-child"] {
        seed_task(&pool, &make_task_row(id, user)).await.unwrap();
    }
    PgTaskRelationDao(pool.clone())
        .insert(&TaskRelation::new(
            "task-child",
            "task-parent",
            TaskRelationType::CreatedBy,
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    let (before,): (i64,) =
        sqlx::query_as("SELECT COALESCE(max(position), 0) FROM task_listing_changes")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        remove_task(&pool, "task-child", &scope_ctx(user))
            .await
            .unwrap()
    );

    let changed: Vec<String> = sqlx::query_scalar(
        "SELECT task_id FROM task_listing_changes WHERE position > $1 ORDER BY position",
    )
    .bind(before)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(changed, ["task-child", "task-parent"]);
}

#[sqlx::test]
async fn exists_by_short_id(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let mut row = make_task_row("task-short", user);
    row.short_id = Some("abcd1234".to_string());
    seed_task(&pool, &row).await.unwrap();

    assert!(short_id_exists(&pool, "abcd1234").await.unwrap());
    assert!(!short_id_exists(&pool, "xxxxxxxx").await.unwrap());
}

#[sqlx::test]
async fn follow_unfollow_and_list_followed(pool: PgPool) {
    let owner = "user-1";
    let follower = "user-2";
    seed_repo(&pool, "repo-1", owner).await;

    // A public task by `owner` that `follower` can see and therefore follow.
    let mut row = make_task_row("task-followed", owner);
    row.visibility = Visibility::Public;
    seed_task(&pool, &row).await.unwrap();

    let repo = store_over(&pool);

    let followed = repo
        .get_followed(follower, &["task-followed".to_string()])
        .await
        .unwrap();
    assert!(followed.is_empty());

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
    seed_task(&pool, &row).await.unwrap();

    let repo = store_over(&pool);
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

    assert!(
        follow_state(&pool, follower, "task-state-delete")
            .await
            .is_none()
    );
}

#[sqlx::test]
async fn refollow_after_settle_and_unfollow_starts_with_null_state(pool: PgPool) {
    let owner = "user-1";
    let follower = "user-2";
    seed_repo(&pool, "repo-1", owner).await;
    let mut row = make_task_row("task-state-refollow", owner);
    row.visibility = Visibility::Public;
    seed_task(&pool, &row).await.unwrap();

    let repo = store_over(&pool);
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

async fn pin_state(
    pool: &PgPool,
    user_id: &str,
    task_id: &str,
) -> (Option<DateTime<Utc>>, Option<String>, Option<String>) {
    sqlx::query_as(
        r#"SELECT pinned_at, pin_order_key, active_order_key
           FROM task_follows
           WHERE user_id = $1 AND task_id = $2"#,
    )
    .bind(user_id)
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Pinning puts the thread on top now, so a settle and a snooze it outranks
/// are spent; a repeated pin keeps the slot the user already arranged.
#[sqlx::test]
async fn pin_unsettles_and_unsnoozes_and_a_repin_keeps_its_slot(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;
    seed_task(&pool, &make_task_row("task-pin", user))
        .await
        .unwrap();
    let repo = store_over(&pool);
    repo.follow_task(user, "task-pin").await.unwrap();
    repo.settle_task(user, "task-pin", Utc::now())
        .await
        .unwrap();
    repo.snooze_task(
        user,
        "task-pin",
        Utc::now() + Duration::hours(1),
        Utc::now(),
    )
    .await
    .unwrap();

    repo.pin_task(user, "task-pin", Some("a0".to_string()), Utc::now())
        .await
        .unwrap();
    let (settled_override, settled_at, snoozed_until, snoozed_at) =
        follow_state(&pool, user, "task-pin").await.unwrap();
    assert_eq!(settled_override.as_deref(), Some("active"));
    assert_eq!((settled_at, snoozed_until, snoozed_at), (None, None, None));
    let (pinned_at, pin_order_key, _) = pin_state(&pool, user, "task-pin").await;
    assert!(pinned_at.is_some());
    assert_eq!(pin_order_key.as_deref(), Some("a0"));

    repo.pin_task(user, "task-pin", Some("b0".to_string()), Utc::now())
        .await
        .unwrap();
    assert_eq!(
        pin_state(&pool, user, "task-pin").await,
        (pinned_at, Some("a0".to_string()), None)
    );

    repo.set_order_key(user, "task-pin", OrderSlot::Pinned, "a5")
        .await
        .unwrap();
    repo.set_order_key(user, "task-pin", OrderSlot::Active, "Zz")
        .await
        .unwrap();
    assert_eq!(
        pin_state(&pool, user, "task-pin").await,
        (pinned_at, Some("a5".to_string()), Some("Zz".to_string()))
    );

    repo.unpin_task(user, "task-pin").await.unwrap();
    assert_eq!(
        pin_state(&pool, user, "task-pin").await,
        (None, None, Some("Zz".to_string()))
    );
}

#[sqlx::test]
async fn settle_drops_the_pin_and_both_slots(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;
    seed_task(&pool, &make_task_row("task-pin-settle", user))
        .await
        .unwrap();
    let repo = store_over(&pool);
    repo.follow_task(user, "task-pin-settle").await.unwrap();
    repo.pin_task(user, "task-pin-settle", Some("a0".to_string()), Utc::now())
        .await
        .unwrap();
    repo.set_order_key(user, "task-pin-settle", OrderSlot::Active, "Zz")
        .await
        .unwrap();

    repo.settle_task(user, "task-pin-settle", Utc::now())
        .await
        .unwrap();

    assert_eq!(
        pin_state(&pool, user, "task-pin-settle").await,
        (None, None, None)
    );
    let (settled_override, ..) = follow_state(&pool, user, "task-pin-settle").await.unwrap();
    assert_eq!(settled_override.as_deref(), Some("settled"));
}

// Pagination invariants (DB-1): the page and the total must share one
// filtered relation — tags can't fan a task into several counted rows,
// every filter/access predicate constrains both, and an offset past the
// end still reports the true total.

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
    let tags = crate::tag::test_support::find_or_create_tags(pool, &names)
        .await
        .expect("create tags");
    let tag_ids: Vec<String> = tags.iter().map(|t| t.tag_id.clone()).collect();
    crate::tag::test_support::set_task_tags(pool, task_id, &tag_ids)
        .await
        .expect("set task tags");
}

#[sqlx::test]
async fn list_tasks_multiple_tags_contribute_one_row_and_one_count(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    seed_task(&pool, &make_task_row("task-many-tags", user))
        .await
        .unwrap();
    seed_task(&pool, &make_task_row("task-no-tags", user))
        .await
        .unwrap();
    tag_task(&pool, "task-many-tags", &["bug", "urgent", "backend"]).await;

    let repo = store_over(&pool);

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
        seed_task(&pool, &make_task_row(&format!("task-off-{i}"), user))
            .await
            .unwrap();
    }
    // Tags on one task would inflate a join-fanned count.
    tag_task(&pool, "task-off-0", &["bug", "urgent"]).await;

    let repo = store_over(&pool);

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
        seed_task(&pool, &make_task_row(&format!("task-private-{i}"), owner))
            .await
            .unwrap();
    }
    let mut public = make_task_row("task-public", owner);
    public.visibility = Visibility::Public;
    seed_task(&pool, &public).await.unwrap();

    // Tags on the hidden tasks must not leak into the other user's total.
    tag_task(&pool, "task-private-0", &["bug", "urgent"]).await;
    tag_task(&pool, "task-public", &["bug", "urgent"]).await;

    seed_task(&pool, &make_task_row("task-own", other))
        .await
        .unwrap();

    let repo = store_over(&pool);

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
    use crate::task::relations::{TaskRelation, TaskRelationType};
    use crate::task::relations_dao::{PgTaskRelationDao, TaskRelationDao};

    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    for id in ["task-parent", "task-child-a", "task-child-b", "task-orphan"] {
        seed_task(&pool, &make_task_row(id, user)).await.unwrap();
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

    let repo = store_over(&pool);

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

    seed_task(&pool, &make_task_row("task-followed-open", user))
        .await
        .unwrap();
    let mut closed = make_task_row("task-followed-closed", user);
    closed.status = TaskStatus::Closed;
    seed_task(&pool, &closed).await.unwrap();
    seed_task(&pool, &make_task_row("task-unfollowed", user))
        .await
        .unwrap();

    tag_task(&pool, "task-followed-open", &["bug", "urgent", "backend"]).await;
    tag_task(&pool, "task-followed-closed", &["bug", "urgent"]).await;

    let repo = store_over(&pool);
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

// =========================================================================
// insert_task — every Task is created in a Workspace that exists
// =========================================================================

#[sqlx::test]
async fn insert_task_refuses_a_task_with_no_workspace(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let mut task = make_task_row("task-1", user);
    task.workspace_id = None;

    let error = store_over(&pool)
        .insert_task(&task, &SandboxPlacement::default())
        .await
        .expect_err("a task with no workspace is refused");
    assert!(
        error.to_string().contains("has no workspace_id"),
        "unexpected error: {error}"
    );
}

#[sqlx::test]
async fn insert_task_refuses_a_workspace_that_does_not_exist(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let mut task = make_task_row("task-1", user);
    task.workspace_id = Some("ws-absent".to_string());

    let error = store_over(&pool)
        .insert_task(&task, &SandboxPlacement::default())
        .await
        .expect_err("a task naming an absent workspace is refused");
    assert!(
        error.to_string().contains("does not exist"),
        "unexpected error: {error}"
    );
}

#[sqlx::test]
async fn insert_task_persists_the_workspace_it_was_given(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;

    let mut task = make_task_row("task-2", user);
    task.workspace_id = Some("ws-explicit".to_string());
    seed_task(&pool, &task).await.unwrap();

    let stored = find_task(&pool, "task-2", &scope_ctx(user))
        .await
        .unwrap()
        .expect("task present");
    assert_eq!(stored.workspace_id.as_deref(), Some("ws-explicit"));
}

async fn read_receipt(pool: &PgPool, user_id: &str, task_id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar(
        "SELECT last_read_at FROM task_read_receipts WHERE user_id = $1 AND task_id = $2",
    )
    .bind(user_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn listing_changes(pool: &PgPool, task_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM task_listing_changes WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn mark_unread_rewinds_the_receipt_before_the_last_ended_turn(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;
    seed_task(&pool, &make_task_row("task-read", user))
        .await
        .unwrap();
    let ended = Utc::now() - Duration::minutes(5);
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status, ended_at) \
         VALUES ('task-read', 1, 'completed', $1)",
    )
    .bind(ended)
    .execute(&pool)
    .await
    .unwrap();
    let repo = store_over(&pool);

    repo.mark_task_read(user, "task-read", Utc::now())
        .await
        .unwrap();
    let before = listing_changes(&pool, "task-read").await;
    repo.mark_task_unread(user, "task-read").await.unwrap();

    let receipt = read_receipt(&pool, user, "task-read").await.unwrap();
    assert!(receipt < ended);
    assert_eq!(listing_changes(&pool, "task-read").await, before + 1);

    // Already before the last turn: nothing moves and no change is recorded.
    repo.mark_task_unread(user, "task-read").await.unwrap();
    assert_eq!(read_receipt(&pool, user, "task-read").await, Some(receipt));
    assert_eq!(listing_changes(&pool, "task-read").await, before + 1);
}

#[sqlx::test]
async fn mark_unread_drops_the_receipt_when_no_turn_has_ended(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;
    seed_task(&pool, &make_task_row("task-fresh", user))
        .await
        .unwrap();
    let repo = store_over(&pool);

    repo.mark_task_read(user, "task-fresh", Utc::now())
        .await
        .unwrap();
    repo.mark_task_unread(user, "task-fresh").await.unwrap();

    assert_eq!(read_receipt(&pool, user, "task-fresh").await, None);
}

#[sqlx::test]
async fn delete_task_removes_its_tags_follows_and_receipts(pool: PgPool) {
    let user = "user-1";
    seed_repo(&pool, "repo-1", user).await;
    seed_task(&pool, &make_task_row("task-gone", user))
        .await
        .unwrap();
    tag_task(&pool, "task-gone", &["bug"]).await;
    let repo = store_over(&pool);
    repo.follow_task(user, "task-gone").await.unwrap();
    repo.mark_task_read(user, "task-gone", Utc::now())
        .await
        .unwrap();

    remove_task(&pool, "task-gone", &scope_ctx(user))
        .await
        .unwrap();

    for table in ["task_tags", "task_follows", "task_read_receipts"] {
        let left: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE task_id = 'task-gone'"
        )))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(left, 0, "{table} kept a row for the deleted task");
    }
}

/// A public Task reads and is written by a stranger, never owned by one; a
/// private Task is none of the three to a stranger, and all three to its owner.
#[sqlx::test]
async fn write_and_ownership_narrow_the_read_rule(pool: PgPool) {
    let owner = "user-1";
    let stranger = "user-2";
    seed_repo(&pool, "repo-1", owner).await;
    seed_task(&pool, &make_task_row("task-private", owner))
        .await
        .unwrap();
    let mut public = make_task_row("task-public", owner);
    public.visibility = Visibility::Public;
    seed_task(&pool, &public).await.unwrap();

    let dao = store_over(&pool);
    let reach = |task_id: &'static str, user: &'static str| {
        let dao = &dao;
        async move {
            let ctx = scope_ctx(user);
            (
                dao.find_by_id(task_id, &ctx).await.unwrap().is_some(),
                dao.find_written_by_id(task_id, &ctx)
                    .await
                    .unwrap()
                    .is_some(),
                dao.find_owned_by_id(task_id, &ctx).await.unwrap().is_some(),
            )
        }
    };
    assert_eq!(reach("task-private", owner).await, (true, true, true));
    assert_eq!(reach("task-private", stranger).await, (false, false, false));
    assert_eq!(reach("task-public", stranger).await, (true, true, false));
    assert_eq!(reach("task-public", owner).await, (true, true, true));
}

/// Each Task action under its grant, for the four callers that differ: read
/// and write follow visibility, the owner-only actions never do.
#[sqlx::test]
async fn each_action_answers_to_its_grant(pool: PgPool) {
    let owner = "user-1";
    seed_repo(&pool, "repo-1", owner).await;
    let dao = store_over(&pool);

    let callers = [
        ("owner", Visibility::Private, scope_ctx(owner)),
        ("stranger-public", Visibility::Public, scope_ctx("user-2")),
        ("stranger-private", Visibility::Private, scope_ctx("user-2")),
        ("admin", Visibility::Private, ScopeContext::admin("root")),
    ];
    let actions = ["read", "rename", "close", "visibility", "delete"];
    let mut lines = Vec::new();
    for (name, visibility, ctx) in &callers {
        let mut verdicts = Vec::new();
        for action in actions {
            let task_id = format!("{name}-{action}");
            let mut row = make_task_row(&task_id, owner);
            row.visibility = *visibility;
            seed_task(&pool, &row).await.unwrap();
            let allowed = match action {
                "read" => dao.find_by_id(&task_id, ctx).await.unwrap().is_some(),
                "rename" => {
                    let patch = TaskPatch {
                        name: Some("renamed".to_string()),
                        ..Default::default()
                    };
                    dao.update_task(&task_id, &patch, ctx).await.unwrap()
                }
                "close" => dao
                    .update_status_if_not_closed(&task_id, TaskStatus::Closed, ctx)
                    .await
                    .unwrap(),
                "visibility" => {
                    let patch = TaskPatch {
                        visibility: Some(Visibility::Private),
                        ..Default::default()
                    };
                    dao.update_task(&task_id, &patch, ctx).await.unwrap()
                }
                _ => dao.delete_task(&task_id, ctx).await.unwrap(),
            };
            verdicts.push(format!(
                "{action}={}",
                if allowed { "allow" } else { "deny" }
            ));
        }
        lines.push(format!("{name:<16} {}", verdicts.join(" ")));
    }

    let expected = "\
owner            read=allow rename=allow close=allow visibility=allow delete=allow
stranger-public  read=allow rename=allow close=allow visibility=deny delete=deny
stranger-private read=deny rename=deny close=deny visibility=deny delete=deny
admin            read=allow rename=allow close=allow visibility=allow delete=allow";
    assert_eq!(lines.join("\n"), expected);
}

/// A patch carrying an owner-only field lands none of its fields for a
/// visitor, so a rename cannot ride along with a refused visibility change.
#[sqlx::test]
async fn a_mixed_patch_from_a_visitor_lands_nothing(pool: PgPool) {
    let owner = "user-1";
    seed_repo(&pool, "repo-1", owner).await;
    let mut public = make_task_row("task-public", owner);
    public.visibility = Visibility::Public;
    public.name = Some("before".to_string());
    seed_task(&pool, &public).await.unwrap();

    let patch = TaskPatch {
        name: Some("after".to_string()),
        visibility: Some(Visibility::Private),
        ..Default::default()
    };
    let dao = store_over(&pool);
    assert!(
        !dao.update_task("task-public", &patch, &scope_ctx("user-2"))
            .await
            .unwrap()
    );

    let row = find_task(&pool, "task-public", &scope_ctx(owner))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.name.as_deref(), Some("before"));
    assert_eq!(row.visibility, Visibility::Public);
}

/// The status-only read answers for exactly the Tasks the full read returns.
#[sqlx::test]
async fn the_readable_status_follows_the_read_rule(pool: PgPool) {
    let owner = "user-1";
    seed_repo(&pool, "repo-1", owner).await;
    seed_task(&pool, &make_task_row("task-private", owner))
        .await
        .unwrap();

    let dao = store_over(&pool);
    let status = |user: &'static str| {
        let dao = &dao;
        async move {
            dao.find_readable_status("task-private", &scope_ctx(user))
                .await
                .unwrap()
        }
    };
    let row = dao
        .find_by_id("task-private", &scope_ctx(owner))
        .await
        .unwrap()
        .expect("the owner reads the Task");
    assert_eq!(status(owner).await, Some(row.status));
    assert_eq!(status("user-2").await, None);
}

async fn seed_public_task(pool: &PgPool, task_id: &str) {
    seed_repo(pool, "repo-1", "user-1").await;
    let mut row = make_task_row(task_id, "user-1");
    row.visibility = Visibility::Public;
    seed_task(pool, &row).await.unwrap();
    // `insert_task` auto-follows the creator; left in, `user-1` is an unsettled
    // follower that keeps every task here from counting as settled.
    store_over(pool)
        .unfollow_task("user-1", task_id)
        .await
        .unwrap();
}

#[sqlx::test]
async fn a_task_nobody_follows_is_settled_only_once_the_reaper_settles_it(pool: PgPool) {
    seed_public_task(&pool, "task-unfollowed").await;
    let repo = store_over(&pool);
    let ids = vec!["task-unfollowed".to_string()];
    assert!(repo.get_settled_since_map(&ids).await.unwrap().is_empty());

    let at = Utc::now() - Duration::hours(2);
    assert!(
        repo.auto_settle_task("task-unfollowed", at, at)
            .await
            .unwrap()
    );
    assert!(
        !repo
            .auto_settle_task("task-unfollowed", Utc::now(), at)
            .await
            .unwrap(),
        "a second settle moves no row and keeps the first stamp"
    );

    let settled = repo.get_settled_since_map(&ids).await.unwrap();
    assert_eq!(
        settled.get("task-unfollowed").map(DateTime::timestamp),
        Some(at.timestamp())
    );
}

#[sqlx::test]
async fn a_followed_task_is_settled_from_when_its_last_follower_settled_it(pool: PgPool) {
    seed_public_task(&pool, "task-two-followers").await;
    let repo = store_over(&pool);
    let ids = vec!["task-two-followers".to_string()];
    repo.follow_task("user-2", "task-two-followers")
        .await
        .unwrap();
    repo.follow_task("user-3", "task-two-followers")
        .await
        .unwrap();

    let first = Utc::now() - Duration::days(3);
    repo.settle_task("user-2", "task-two-followers", first)
        .await
        .unwrap();
    assert!(
        repo.get_settled_since_map(&ids).await.unwrap().is_empty(),
        "user-3 still has it open"
    );

    let last = Utc::now() - Duration::days(1);
    repo.settle_task("user-3", "task-two-followers", last)
        .await
        .unwrap();
    let settled = repo.get_settled_since_map(&ids).await.unwrap();
    assert_eq!(
        settled.get("task-two-followers").map(DateTime::timestamp),
        Some(last.timestamp())
    );
}

#[sqlx::test]
async fn auto_settle_leaves_a_hand_unsettled_follower_and_the_task_open(pool: PgPool) {
    seed_public_task(&pool, "task-kept").await;
    let repo = store_over(&pool);
    let ids = vec!["task-kept".to_string()];
    repo.follow_task("user-2", "task-kept").await.unwrap();
    repo.follow_task("user-3", "task-kept").await.unwrap();
    repo.unsettle_task("user-3", "task-kept").await.unwrap();

    repo.auto_settle_task("task-kept", Utc::now(), Utc::now())
        .await
        .unwrap();

    let (user2, _, _, _) = follow_state(&pool, "user-2", "task-kept").await.unwrap();
    let (user3, _, _, _) = follow_state(&pool, "user-3", "task-kept").await.unwrap();
    assert_eq!(user2.as_deref(), Some("settled"));
    assert_eq!(user3.as_deref(), Some("active"), "a hand unsettle stands");
    assert!(repo.get_settled_since_map(&ids).await.unwrap().is_empty());
}

#[sqlx::test]
async fn auto_settle_skips_an_opted_out_follower_until_they_opt_back_in(pool: PgPool) {
    seed_public_task(&pool, "task-opted-out").await;
    let repo = store_over(&pool);
    repo.follow_task("user-2", "task-opted-out").await.unwrap();
    let opted_out_at = Utc::now() - Duration::hours(3);
    repo.set_auto_settle("user-2", "task-opted-out", false, opted_out_at)
        .await
        .unwrap();
    repo.set_auto_settle("user-2", "task-opted-out", false, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        auto_settle_disabled_at(&pool, "user-2", "task-opted-out")
            .await
            .map(|at| at.timestamp()),
        Some(opted_out_at.timestamp()),
        "a repeated opt-out keeps the first stamp"
    );

    repo.auto_settle_task("task-opted-out", Utc::now(), Utc::now())
        .await
        .unwrap();
    let (state, _, _, _) = follow_state(&pool, "user-2", "task-opted-out")
        .await
        .unwrap();
    assert_eq!(state, None);

    repo.set_auto_settle("user-2", "task-opted-out", true, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        auto_settle_disabled_at(&pool, "user-2", "task-opted-out").await,
        None
    );
    repo.auto_settle_task("task-opted-out", Utc::now(), Utc::now())
        .await
        .unwrap();
    let (state, _, _, _) = follow_state(&pool, "user-2", "task-opted-out")
        .await
        .unwrap();
    assert_eq!(state.as_deref(), Some("settled"));
}

#[sqlx::test]
async fn auto_settle_skips_a_snooze_that_has_not_been_awake_for_the_idle_window(pool: PgPool) {
    seed_public_task(&pool, "task-snoozed").await;
    let repo = store_over(&pool);
    let now = Utc::now();
    let idle_since = now - Duration::hours(24);
    for (user, until) in [
        ("user-asleep", now + Duration::hours(2)),
        ("user-just-woke", now - Duration::hours(1)),
        ("user-woke-long-ago", now - Duration::hours(30)),
    ] {
        repo.follow_task(user, "task-snoozed").await.unwrap();
        repo.snooze_task(user, "task-snoozed", until, now - Duration::days(3))
            .await
            .unwrap();
    }
    // Woken by hand an hour ago; cleared after a recent timer wake; cleared
    // long after its timer had woken it.
    for (user, until, woke) in [
        (
            "user-woke-by-hand",
            now + Duration::hours(5),
            now - Duration::hours(1),
        ),
        (
            "user-unsnoozed-after-timer",
            now - Duration::hours(2),
            now - Duration::hours(1),
        ),
        (
            "user-woke-late-by-hand",
            now - Duration::hours(30),
            now - Duration::hours(1),
        ),
    ] {
        repo.follow_task(user, "task-snoozed").await.unwrap();
        repo.snooze_task(user, "task-snoozed", until, now - Duration::days(3))
            .await
            .unwrap();
        repo.unsnooze_task(user, "task-snoozed", woke)
            .await
            .unwrap();
    }

    repo.auto_settle_task("task-snoozed", now, idle_since)
        .await
        .unwrap();

    for (user, expected) in [
        ("user-asleep", None),
        ("user-just-woke", None),
        ("user-woke-long-ago", Some("settled")),
        ("user-woke-by-hand", None),
        ("user-unsnoozed-after-timer", None),
        ("user-woke-late-by-hand", Some("settled")),
    ] {
        let (state, _, _, _) = follow_state(&pool, user, "task-snoozed").await.unwrap();
        assert_eq!(state.as_deref(), expected, "{user}");
    }
}

async fn auto_settle_disabled_at(
    pool: &PgPool,
    user_id: &str,
    task_id: &str,
) -> Option<DateTime<Utc>> {
    sqlx::query_scalar(
        r#"SELECT auto_settle_disabled_at FROM task_follows
            WHERE user_id = $1 AND task_id = $2"#,
    )
    .bind(user_id)
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn new_activity_clears_the_reaper_settle(pool: PgPool) {
    seed_public_task(&pool, "task-reactivated").await;
    let repo = store_over(&pool);
    let ids = vec!["task-reactivated".to_string()];
    repo.auto_settle_task("task-reactivated", Utc::now(), Utc::now())
        .await
        .unwrap();

    let mut conn = pool.acquire().await.unwrap();
    crate::task::dao::clear_settled_for_all_followers(&mut conn, "task-reactivated")
        .await
        .unwrap();

    assert!(repo.get_settled_since_map(&ids).await.unwrap().is_empty());
}
