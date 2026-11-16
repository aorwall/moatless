//! The Task listing against a real database: who a browse reaches, which
//! Turn's error a row reports, and the parent and children a row names.

use moatless_types::access_control::ScopeContext;
use moatless_types::task::{TaskStatus, Visibility};
use moatless_workspaces::test_support::make_repository_row;
use sqlx::PgPool;

use super::{FOLLOWED_LIMIT, ListedTask, ListingFilter, TaskListing};
use crate::task::test_support::{make_task_row, seed_task};

const TASK: &str = "task-listing";
const USER_1: &str = "11111111-1111-1111-1111-111111111111";
const USER_2: &str = "22222222-2222-2222-2222-222222222222";
/// The browsing viewer, who owns none of the Tasks below and follows none.
const ADMIN: &str = "33333333-3333-3333-3333-333333333333";

fn listing_over(pool: &PgPool) -> TaskListing {
    TaskListing::new(pool.clone())
}

async fn seed_owned_task(pool: &PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    seed_task(pool, &make_task_row(TASK, USER_1))
        .await
        .expect("seed task");
}

/// Seed one open Task owned by `user_id`, tagged with each name given.
async fn seed_browsable(pool: &PgPool, task_id: &str, user_id: &str, tags: &[&str]) {
    let task = make_task_row(task_id, user_id);
    seed_task(pool, &task).await.expect("seed task");
    tag_with(pool, task_id, tags).await;
}

async fn tag_with(pool: &PgPool, task_id: &str, tags: &[&str]) {
    if tags.is_empty() {
        return;
    }
    let names: Vec<String> = tags.iter().map(|name| (*name).to_string()).collect();
    let created = crate::tag::test_support::find_or_create_tags(pool, &names)
        .await
        .expect("create tags");
    let ids: Vec<String> = created.into_iter().map(|tag| tag.tag_id).collect();
    crate::tag::test_support::add_task_tags(pool, task_id, &ids)
        .await
        .expect("tag task");
}

/// A browse reaches the Tasks of somebody the viewer is not, and each filter
/// narrows it.
///
/// The browse drops only the *follow* rule — none of these Tasks is followed or
/// owned by the viewer.
#[sqlx::test]
async fn a_browse_reaches_tasks_the_viewer_neither_follows_nor_owns(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    seed_browsable(&pool, "browse-hers-tagged", USER_1, &["backend"]).await;
    seed_browsable(&pool, "browse-hers-plain", USER_1, &[]).await;
    seed_browsable(&pool, "browse-theirs-tagged", USER_2, &["backend"]).await;

    let listing = listing_over(&pool);
    let viewer = ScopeContext::admin(ADMIN.to_string());

    let by_owner = listing
        .browsable(
            &viewer,
            &ListingFilter::new(Some(USER_1.to_string()), None, false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&by_owner),
        vec!["browse-hers-plain", "browse-hers-tagged"],
        "an owner filter reaches every Task of that person's and nobody else's"
    );

    let by_tag = listing
        .browsable(
            &viewer,
            &ListingFilter::new(None, Some("backend".to_string()), false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&by_tag),
        vec!["browse-hers-tagged", "browse-theirs-tagged"],
        "a tag filter reaches the tagged Tasks of everyone"
    );

    let by_both = listing
        .browsable(
            &viewer,
            &ListingFilter::new(Some(USER_1.to_string()), Some("backend".to_string()), false)
                .expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&by_both),
        vec!["browse-hers-tagged"],
        "the two filters narrow together rather than widening"
    );
}

/// Closed Tasks come back only when the browse asks for them.
///
/// Closing is T3's archiving; a browse folds both halves into one read, and
/// this flag picks which.
#[sqlx::test]
async fn a_browse_leaves_closed_tasks_out_until_it_is_asked_for_them(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    seed_browsable(&pool, "browse-open", USER_1, &[]).await;
    let mut closed = make_task_row("browse-closed", USER_1);
    closed.status = TaskStatus::Closed;
    seed_task(&pool, &closed).await.expect("seed task");

    let listing = listing_over(&pool);
    let viewer = ScopeContext::admin(ADMIN.to_string());
    let owner = || Some(USER_1.to_string());

    let open_only = listing
        .browsable(
            &viewer,
            &ListingFilter::new(owner(), None, false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(browsed_ids(&open_only), vec!["browse-open"]);

    let with_closed = listing
        .browsable(
            &viewer,
            &ListingFilter::new(owner(), None, true).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&with_closed),
        vec!["browse-closed", "browse-open"],
        "asked for them, the closed half arrives alongside the open one"
    );
}

/// A viewer who is not an administrator browses what they may read, which is
/// what makes the filter safe to serve to everyone: the read rule is untouched.
#[sqlx::test]
async fn a_browse_by_a_non_administrator_reaches_only_what_they_may_read(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    seed_browsable(&pool, "browse-private", USER_1, &["backend"]).await;
    let mut public = make_task_row("browse-public", USER_1);
    public.visibility = Visibility::Public;
    seed_task(&pool, &public).await.expect("seed task");
    tag_with(&pool, "browse-public", &["backend"]).await;

    let browsed = listing_over(&pool)
        .browsable(
            &ScopeContext::user(USER_2.to_string()),
            &ListingFilter::new(None, Some("backend".to_string()), false).expect("a filter"),
        )
        .await
        .unwrap();
    assert_eq!(
        browsed_ids(&browsed),
        vec!["browse-public"],
        "somebody else's private Task is not reachable by tagging it"
    );
}

fn browsed_ids(conversations: &[ListedTask]) -> Vec<String> {
    let mut ids: Vec<String> = conversations
        .iter()
        .map(|conversation| conversation.task_id.clone())
        .collect();
    ids.sort();
    ids
}

async fn record_turn(pool: &PgPool, turn: i32, status: &str, error: Option<&str>) {
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status) VALUES ($1, $2, $3)
         ON CONFLICT (task_id, turn_number) DO UPDATE SET status = EXCLUDED.status",
    )
    .bind(TASK)
    .bind(turn)
    .bind(status)
    .execute(pool)
    .await
    .expect("record turn");
    if let Some(error) = error {
        sqlx::query(
            "INSERT INTO task_events (event_id, task_id, type, sub_type, is_error, turn_number, payload, timestamp)
             VALUES ($1, $2, 'agent', 'error', true, $3, $4, '')",
        )
        .bind(format!("{TASK}-error-{turn}"))
        .bind(TASK)
        .bind(turn)
        .bind(serde_json::json!({ "error": error, "reason": "usage_limit" }))
        .execute(pool)
        .await
        .expect("record error event");
    }
}

/// A Turn's error stays the conversation's until the next Turn starts.
#[sqlx::test]
async fn the_newest_turns_error_is_read_until_another_turn_starts(pool: PgPool) {
    seed_owned_task(&pool).await;
    let listing = listing_over(&pool);
    let owner = ScopeContext::user(USER_1.to_string());
    let turn_error = |row: Option<ListedTask>| row.expect("readable").turn_error;

    record_turn(&pool, 1, "error", Some("You've hit your usage limit.")).await;
    assert_eq!(
        turn_error(listing.readable_by_id(&owner, TASK).await.unwrap()).as_deref(),
        Some("You've hit your usage limit.")
    );
    let errored = listing
        .readable_by_id(&owner, TASK)
        .await
        .unwrap()
        .expect("readable");
    assert_eq!(errored.turn_error_reason.as_deref(), Some("usage_limit"));

    record_turn(&pool, 2, "active", None).await;
    let running = listing
        .readable_by_id(&owner, TASK)
        .await
        .unwrap()
        .expect("readable");
    assert_eq!(running.turn_error_reason, None);
    assert_eq!(
        turn_error(listing.readable_by_id(&owner, TASK).await.unwrap()),
        None
    );

    record_turn(&pool, 2, "completed", None).await;
    assert_eq!(
        turn_error(listing.readable_by_id(&owner, TASK).await.unwrap()),
        None
    );
}

/// A listed row names its parent whenever the viewer may read the parent,
/// followed or not, and a parent counts the children the viewer may read.
#[sqlx::test]
async fn a_listed_row_names_a_readable_parent_and_counts_its_children(pool: PgPool) {
    let seed = |task_id: &'static str, owner: &'static str| {
        let pool = pool.clone();
        async move {
            let mut task = make_task_row(task_id, owner);
            task.workspace_id = Some("ws-lineage".to_string());
            seed_task(&pool, &task).await.expect("seed task");
        }
    };
    for task_id in [
        "parent",
        "child",
        "fork",
        "regular-fork",
        "orphan",
        "unfollowed",
        "stray",
    ] {
        seed(task_id, USER_1).await;
    }
    seed("foreign", USER_2).await;
    let edges = [
        ("child", "parent", "created_by", serde_json::json!({})),
        (
            "regular-fork",
            "parent",
            "forked_from",
            serde_json::json!({ "side_chat": false }),
        ),
        (
            "fork",
            "parent",
            "forked_from",
            serde_json::json!({ "at_turn": 2, "side_chat": true }),
        ),
        ("orphan", "foreign", "created_by", serde_json::json!({})),
        ("stray", "unfollowed", "forked_from", serde_json::json!({})),
    ];
    for (from, to, relation, context) in edges {
        sqlx::query(
            "INSERT INTO task_relations (id, from_task_id, to_task_id, relation_type, context)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(format!("{from}->{to}"))
        .bind(from)
        .bind(to)
        .bind(relation)
        .bind(context)
        .execute(&pool)
        .await
        .expect("seed relation");
    }
    sqlx::query(
        "INSERT INTO task_follows (user_id, task_id)
         SELECT $1, task_id FROM tasks ON CONFLICT DO NOTHING",
    )
    .bind(USER_1)
    .execute(&pool)
    .await
    .expect("follow everything");
    sqlx::query("DELETE FROM task_follows WHERE user_id = $1 AND task_id = 'unfollowed'")
        .bind(USER_1)
        .execute(&pool)
        .await
        .expect("unfollow");

    let viewer = ScopeContext::user(USER_1.to_string());
    let rows = listing_over(&pool).followed(&viewer).await.unwrap();
    let parent_of = |task_id: &str| {
        let row = rows
            .iter()
            .find(|row| row.task_id == task_id)
            .expect("listed");
        (
            row.parent_task_id.clone(),
            row.parent_relation.clone(),
            row.fork_at_turn,
        )
    };
    let named = |parent: &str, relation: &str, at: Option<i32>| {
        (Some(parent.to_string()), Some(relation.to_string()), at)
    };
    assert_eq!(parent_of("child"), named("parent", "created_by", None));
    assert_eq!(parent_of("fork"), named("parent", "forked_from", Some(2)));
    assert!(
        rows.iter()
            .find(|row| row.task_id == "fork")
            .unwrap()
            .side_chat
    );
    assert!(
        !rows
            .iter()
            .find(|row| row.task_id == "child")
            .unwrap()
            .side_chat
    );
    assert_eq!(parent_of("orphan"), (None, None, None), "unreadable parent");
    assert_eq!(
        parent_of("stray"),
        named("unfollowed", "forked_from", None),
        "unfollowed parent"
    );
    assert_eq!(parent_of("parent"), (None, None, None));

    let row = |task_id: &str| rows.iter().find(|row| row.task_id == task_id).unwrap();
    assert_eq!(
        row("parent").child_count,
        2,
        "the spawned child and ordinary fork, excluding the side chat"
    );
    assert_eq!(
        listing_over(&pool)
            .readable_by_id(&viewer, "parent")
            .await
            .unwrap()
            .unwrap()
            .child_count,
        2
    );
    assert_eq!(row("child").child_count, 0);
}

/// A read by id reuses the statement's generic plan rather than planning it
/// on every call. One connection, so the prepared statement is observable.
#[sqlx::test]
async fn a_read_by_id_reuses_the_generic_plan(pool: PgPool) {
    seed_owned_task(&pool).await;
    let single = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("single-connection pool");
    let listing = listing_over(&single);
    let owner = ScopeContext::user(USER_1.to_string());

    for _ in 0..3 {
        listing
            .readable_by_id(&owner, TASK)
            .await
            .unwrap()
            .expect("readable");
    }

    let (generic, custom): (i64, i64) = sqlx::query_as(
        "SELECT generic_plans, custom_plans FROM pg_prepared_statements
         WHERE statement LIKE 'WITH picked AS MATERIALIZED%'",
    )
    .fetch_one(&single)
    .await
    .expect("the listing statement is prepared");
    assert_eq!((generic, custom), (3, 0));
}

/// A client opening its shell and its archive at once gets both: the second
/// full listing queues for the replica's permit instead of being refused.
#[sqlx::test]
async fn two_full_listings_at_once_both_complete(pool: PgPool) {
    seed_owned_task(&pool).await;
    let listing = listing_over(&pool);
    let owner = ScopeContext::user(USER_1.to_string());

    let (open, closed) = tokio::join!(
        listing.followed(&owner),
        listing.followed_closed(&owner, None)
    );

    open.expect("the open listing completes");
    closed.expect("the archive listing completes");
}

/// The open listing stops at [`FOLLOWED_LIMIT`] newest rows, and a Task past
/// the cap is still read by id, which is how the shell feed brings it back
/// when it next changes.
#[sqlx::test]
async fn the_followed_listing_keeps_the_newest_and_reads_the_rest_by_id(pool: PgPool) {
    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    let ids: Vec<String> = (0..=FOLLOWED_LIMIT)
        .map(|n| format!("followed-{n:03}"))
        .collect();
    for id in &ids {
        seed_task(&pool, &make_task_row(id, USER_1))
            .await
            .expect("seed task");
        crate::task::test_support::follow_task(&pool, USER_1, id)
            .await
            .expect("follow task");
    }
    // `followed-000` oldest, one minute apart.
    sqlx::query(
        "UPDATE tasks SET updated_at = now() - make_interval(mins => $1 - \
         substring(task_id from 10)::int) WHERE task_id LIKE 'followed-%'",
    )
    .bind(i32::try_from(FOLLOWED_LIMIT).expect("small"))
    .execute(&pool)
    .await
    .expect("age the tasks");

    let listing = listing_over(&pool);
    let viewer = ScopeContext::user(USER_1.to_string());
    let followed = listing.followed(&viewer).await.expect("followed");

    assert_eq!(
        followed.len(),
        usize::try_from(FOLLOWED_LIMIT).expect("small")
    );
    assert_eq!(followed[0].task_id, ids[ids.len() - 1], "newest first");
    assert!(followed.iter().all(|row| row.task_id != "followed-000"));
    let past_the_cap = listing
        .followed_by_ids(&viewer, &["followed-000".to_string()])
        .await
        .expect("by id");
    assert_eq!(past_the_cap.len(), 1);
}

/// The archive picks its page from `tasks_listing_pick_reporter_idx` alone, so
/// its cost before the cap is the index, not a heap row per followed closed
/// Task.
#[sqlx::test]
async fn the_archive_picks_its_page_without_reading_the_task_rows(pool: PgPool) {
    use super::listed_dao::{Page, Reach, bind, statement};

    let repo = make_repository_row("repo-1", USER_1);
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    for n in 0..400 {
        let mut task = make_task_row(&format!("archived-{n:03}"), USER_1);
        task.status = TaskStatus::Closed;
        seed_task(&pool, &task).await.expect("seed task");
    }
    // About 1.5 kB of incompressible prompt keeps each row inline, so the heap
    // is a hundred pages against the index's few. Under ~2 kB TOAST moves the
    // text out, the heap shrinks to 39 pages and a `Seq Scan` costs less.
    sqlx::query(
        "UPDATE tasks SET description = (SELECT string_agg(md5(task_id || n), '')
                                         FROM generate_series(1, 46) n),
                          updated_at = now() - substring(task_id FROM 10)::int * interval '1 minute'",
    )
    .execute(&pool)
    .await
    .expect("widen the rows");
    sqlx::query("INSERT INTO task_follows (user_id, task_id) SELECT $1, task_id FROM tasks ON CONFLICT DO NOTHING")
        .bind(USER_1)
        .execute(&pool)
        .await
        .expect("follow every task");
    vacuum_until_all_visible(&pool).await;

    let viewer = ScopeContext::user(USER_1.to_string());
    let page = Page {
        cap: super::ARCHIVED_LIMIT + 1,
        before: None,
    };
    let sql = statement(
        Reach::Followed,
        &[TaskStatus::Closed],
        false,
        None,
        Some(&page),
    );
    let (plan,): (serde_json::Value,) = bind(
        sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN (FORMAT JSON) {sql}"))),
        &viewer,
        None,
        None,
        None,
    )
    .fetch_one(&pool)
    .await
    .expect("explain the archive page");

    let capped = first_cte(&plan[0]["Plan"]).expect("the statement opens with a CTE");
    let mut scans = Vec::new();
    task_scans(capped, &mut scans);
    assert!(!scans.is_empty(), "the capped CTE reads tasks: {capped:#}");
    assert!(
        scans.iter().all(|scan| scan == "Index Only Scan"),
        "the capped CTE reads the tasks heap: {scans:?}"
    );
}

/// Sets the visibility map an index-only scan needs, as autovacuum does.
///
/// `VACUUM` marks no page all-visible while a snapshot in this database
/// predates its rows, such as an autoanalyze worker's under CI load. Then
/// `relallvisible` stays 0 and the plan is a `Seq Scan`, so retry until every
/// page is marked rather than vacuum once.
async fn vacuum_until_all_visible(pool: &PgPool) {
    for _ in 0..100 {
        sqlx::query("VACUUM ANALYZE tasks, task_follows")
            .execute(pool)
            .await
            .expect("vacuum");
        let (pages, all_visible): (i32, i32) =
            sqlx::query_as("SELECT relpages, relallvisible FROM pg_class WHERE relname = 'tasks'")
                .fetch_one(pool)
                .await
                .expect("read the visibility map size");
        if all_visible >= pages {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("a snapshot in this database kept tasks from becoming all-visible for 10s");
}

fn first_cte(node: &serde_json::Value) -> Option<&serde_json::Value> {
    let children = node["Plans"].as_array()?;
    children
        .iter()
        .find(|child| {
            child["Subplan Name"]
                .as_str()
                .is_some_and(|name| name.starts_with("CTE "))
        })
        .or_else(|| children.iter().find_map(first_cte))
}

fn task_scans(node: &serde_json::Value, scans: &mut Vec<String>) {
    if node["Relation Name"] == "tasks" {
        scans.push(node["Node Type"].as_str().unwrap_or_default().to_string());
    }
    for child in node["Plans"].as_array().into_iter().flatten() {
        task_scans(child, scans);
    }
}
