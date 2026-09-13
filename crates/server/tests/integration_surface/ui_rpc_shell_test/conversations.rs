//! Which conversations a viewer sees.

use super::contract::{assert_decodable_item, assert_decodable_thread};
use super::harness::{admin, reads, snapshot, thread_ids, thread_shell_row, viewer};
use super::seeding::{
    follow, seed_conversation_matrix, seed_live_project, seed_public_task, seed_task, unfollow,
};
use crate::common::make_task_row;
use crate::common::make_workspace_row;
use crate::common::run_config;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::models::Visibility;
use moatless_workspaces::compose;
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

// =========================================================================
// Which conversations a viewer sees
// =========================================================================

/// The listing's query, run against a real Postgres with nothing seeded.
///
/// `sqlx::query_as` without the macro is unchecked until a connection sees it,
/// so a bad column, bind, or unplannable LATERAL surfaces here as a runtime
/// error — failing here means the statement is wrong, not that the result was
/// merely empty.
#[sqlx::test]
async fn postgres_accepts_the_conversation_query(pool: PgPool) {
    let repository = reads(&pool);
    for viewer in [viewer("user-1"), admin("root")] {
        let listed = repository
            .followed_task_ids(&viewer)
            .await
            .unwrap_or_else(|error| {
                panic!("the conversation query was rejected: {error}\n\n{error:?}")
            });
        assert!(listed.is_empty(), "nothing is seeded");
    }
}

/// The production incident's failure shape, replayed against a real Postgres.
///
/// One shell query blocks in the database while another snapshot request hits
/// the same backend replica; the second must be rejected before it acquires a
/// pool connection, and Postgres must cancel the first after five seconds.
#[sqlx::test]
async fn a_blocked_full_listing_cannot_monopolize_the_pool(pool: PgPool) {
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();

    let listing_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let repository = reads(&listing_pool);
    let running_repository = repository.clone();
    let running = tokio::spawn(async move {
        running_repository
            .followed_task_ids(&viewer("user-1"))
            .await
    });

    // The first read has a connection immediately and blocks when it reaches
    // `tasks`. Give it time to take the repository's full-listing permit.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let rejected_at = Instant::now();
    let rejected = repository
        .followed_task_ids(&viewer("user-1"))
        .await
        .expect_err("a second full listing must not enter PostgreSQL");
    assert!(matches!(rejected, sqlx::Error::PoolTimedOut));
    assert!(
        rejected_at.elapsed() < Duration::from_secs(1),
        "overload was queued until a pool timeout instead of rejected"
    );

    let timed_out = tokio::time::timeout(Duration::from_secs(7), running)
        .await
        .expect("the database statement timeout did not fire")
        .expect("listing task panicked")
        .expect_err("the blocked listing unexpectedly completed");
    match timed_out {
        sqlx::Error::Database(error) => {
            assert_eq!(error.code().as_deref(), Some("57014"), "{error}");
        }
        other => panic!("expected PostgreSQL query cancellation, got {other:?}"),
    }

    blocker.rollback().await.unwrap();

    let listed = repository
        .followed_task_ids(&viewer("user-1"))
        .await
        .expect("the timed-out connection must return to the pool");
    assert!(listed.is_empty());
}

/// Each per-conversation message lookup in the full listing has a purpose-built
/// partial index. Naming them here makes a dropped migration fail the database
/// tier instead of returning as a production query-plan regression.
#[sqlx::test]
async fn the_shell_message_lookups_have_dedicated_indexes(pool: PgPool) {
    let expected: HashSet<String> = [
        "task_messages_latest_user_created_idx",
        "task_messages_latest_assistant_idx",
        "task_messages_turn_assistant_model_idx",
        "task_messages_turn_requested_model_idx",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let names: HashSet<String> = sqlx::query_scalar(
        "SELECT indexname
         FROM pg_indexes
         WHERE schemaname = current_schema()
           AND indexname = ANY($1)",
    )
    .bind(expected.iter().cloned().collect::<Vec<_>>())
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();

    assert_eq!(names, expected);
}

#[sqlx::test]
async fn a_viewer_sees_the_conversations_they_follow_and_may_still_read(pool: PgPool) {
    seed_conversation_matrix(&pool).await;

    // Every followed combination except the one the viewer may not read:
    // someone else's private, user-scoped conversation. Public and global
    // conversations they follow are listed whoever owns them.
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec![
            "other-private-global-followed".to_string(),
            "other-public-global-followed".to_string(),
            "other-public-user-followed".to_string(),
            "own-private-global-followed".to_string(),
            "own-private-user-followed".to_string(),
            "own-public-global-followed".to_string(),
            "own-public-user-followed".to_string(),
        ]
    );
}

#[sqlx::test]
async fn an_unfollowed_conversation_is_absent_however_readable_it_is(pool: PgPool) {
    seed_conversation_matrix(&pool).await;

    let listed = thread_ids(&pool, &viewer("user-1")).await;
    for id in &listed {
        assert!(
            id.ends_with("-followed"),
            "{id} is in the listing without being followed"
        );
    }
    // Including the viewer's own, which they can both read and control.
    assert!(!listed.contains(&"own-public-user-unfollowed".to_string()));
    assert!(!listed.contains(&"own-private-user-unfollowed".to_string()));
}

/// An administrator's sidebar is what they follow, like everyone else's. This
/// is deliberately the opposite of the project half, where an administrator
/// does see every workspace in the deployment.
#[sqlx::test]
async fn an_administrator_sees_only_the_conversations_they_follow(pool: PgPool) {
    seed_conversation_matrix(&pool).await;

    let root = admin("root");
    assert!(
        thread_ids(&pool, &root).await.is_empty(),
        "an administrator who follows nothing has an empty sidebar"
    );

    // A conversation the administrator neither owns nor could reach through
    // scope — following it is the whole reason it appears.
    follow(&pool, "root", "other-private-user-unfollowed").await;
    assert_eq!(
        thread_ids(&pool, &root).await,
        vec!["other-private-user-unfollowed".to_string()]
    );
}

/// Following is durable; access is not. The follow row outlives the access that
/// produced it, and the listing has to notice.
#[sqlx::test]
async fn a_followed_conversation_the_viewer_may_no_longer_read_drops_out(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-handed-over", "user-1", Some("ws-a")).await;

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-handed-over".to_string()]
    );

    // Ownership moves; the follow stays.
    sqlx::query("UPDATE tasks SET owner_user_id = $1, created_by = $1 WHERE task_id = $2")
        .bind("user-2")
        .bind("t-handed-over")
        .execute(&pool)
        .await
        .unwrap();
    let still_following: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM task_follows WHERE user_id = $1)")
            .bind("user-1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(still_following, "the follow is what outlives the access");

    assert!(thread_ids(&pool, &viewer("user-1")).await.is_empty());
}

/// Writing to a conversation follows it — the same DAO call `send_message`
/// makes — which is what puts someone else's conversation in your sidebar.
#[sqlx::test]
async fn writing_to_a_conversation_puts_it_in_the_sidebar(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    let mut task = make_task_row("t-shared", "user-2");
    task.workspace_id = Some("ws-a".to_string());
    task.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &task)
        .await
        .unwrap();

    // Readable — it is public — but not yet the viewer's concern.
    assert!(thread_ids(&pool, &viewer("user-1")).await.is_empty());

    follow(&pool, "user-1", "t-shared").await;
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-shared".to_string()]
    );
}

/// The listing's widening half, end to end: the conversation is readable, its
/// workspace is not, and the row still needs a project to hang under or the
/// client silently drops it.
#[sqlx::test]
async fn a_followed_conversation_in_an_unreadable_project_still_lists_under_it(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-theirs", "user-2"))
        .await
        .unwrap();
    let mut task = make_task_row("t-guest", "user-2");
    task.workspace_id = Some("ws-theirs".to_string());
    task.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &task)
        .await
        .unwrap();
    follow(&pool, "user-1", "t-guest").await;

    let viewer = viewer("user-1");
    let snapshot = snapshot(&pool, &viewer).await;

    let projects = snapshot["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1, "the referenced workspace is pulled in");
    assert_eq!(projects[0]["id"], "ws-theirs");
    assert_eq!(projects[0]["title"], "ws-ws-theirs");

    let threads = snapshot["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0]["projectId"], "ws-theirs");
}

/// The widening that admits an unreadable Workspace is justified by names only.
///
/// A server's start command carries paths and config, not a name, so it stays
/// behind the read rule; referenced projects disclose Scripts only when the
/// viewer can read the Workspace.
#[sqlx::test]
async fn a_project_the_viewer_may_not_read_discloses_no_script_commands(pool: PgPool) {
    let repo = compose::workspaces(pool.clone());
    let servers = |command: &str| {
        json!([{
            "name": "dev",
            "label": "Dev server",
            "port": 3000,
            "startCommands": [command],
            "default": true
        }])
    };

    let mut mine = make_workspace_row("ws-own", "user-1");
    mine.servers = Some(run_config(servers("bun run dev")));
    mine.scripts = Some(run_config(
        json!([{ "name": "dev", "command": "bun run dev" }]),
    ));
    repo.insert_workspace(&mine).await.unwrap();

    let mut theirs = make_workspace_row("ws-theirs", "user-2");
    theirs.servers = Some(run_config(servers(
        "bun run secret-thing --token /etc/theirs",
    )));
    theirs.scripts = Some(run_config(json!([{
        "name": "dev",
        "command": "bun run secret-thing --token /etc/theirs"
    }])));
    repo.insert_workspace(&theirs).await.unwrap();

    // The one thing that pulls their private workspace into this viewer's
    // listing: a conversation they follow that runs in it.
    let mut task = make_task_row("t-guest", "user-2");
    task.workspace_id = Some("ws-theirs".to_string());
    task.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &task)
        .await
        .unwrap();
    follow(&pool, "user-1", "t-guest").await;

    let viewer = viewer("user-1");
    let snapshot = snapshot(&pool, &viewer).await;
    let project = |id: &str| {
        snapshot["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|project| project["id"] == id)
            .unwrap_or_else(|| panic!("{id} is absent from the listing"))
            .clone()
    };

    // Their own workspace is untouched by this: the narrowing is one half of
    // the union, not the listing.
    let ours = project("ws-own");
    assert_eq!(ours["scripts"][0]["command"], "bun run dev");

    let theirs = project("ws-theirs");
    assert_eq!(
        theirs["scripts"],
        json!([]),
        "a workspace this viewer may not read disclosed its script commands"
    );
    // What the widening does justify, still there — the group renders, and the
    // conversation hanging under it is not silently dropped.
    assert_eq!(theirs["title"], "ws-ws-theirs");
    assert!(
        theirs["workspaceRoot"]
            .as_str()
            .unwrap()
            .contains("ws-theirs")
    );
    assert_eq!(
        snapshot["threads"][0]["projectId"], "ws-theirs",
        "the narrowing must not cost the row its group"
    );
    assert_decodable_item(&json!({ "kind": "snapshot", "snapshot": snapshot }));
}

/// T3 drops a conversation whose project is not in the same snapshot — no
/// error, nothing logged (`Sidebar.logic.ts:779`). Leaving it out here is what
/// makes the omission visible on this side of the wire.
#[sqlx::test]
async fn a_conversation_with_no_project_is_left_out_of_the_listing(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-placed", "user-1", Some("ws-a")).await;
    seed_task(&pool, "t-orphan", "user-1", Some("ws-a")).await;
    // Creation always resolves a workspace, so the unresolvable row has to be
    // made after the fact — as it exists in deployments predating that rule.
    sqlx::query("UPDATE tasks SET workspace_id = NULL WHERE task_id = $1")
        .bind("t-orphan")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-placed".to_string()]
    );
}

/// The listing filters to unfinished work; without it a person's every
/// conversation would stay forever, since a follow is durable.
///
/// `error` is excluded along with `closed`, the narrow "open" the sidebar
/// applies by default; nothing still running is hidden, since a Task returns to
/// `open` the moment its agent produces anything.
#[sqlx::test]
async fn only_open_conversations_are_listed(pool: PgPool) {
    seed_live_project(&pool).await;
    for (task_id, status) in [
        ("t-open", TaskStatus::Open),
        ("t-closed", TaskStatus::Closed),
        ("t-errored", TaskStatus::Error),
    ] {
        let mut task = make_task_row(task_id, "user-1");
        task.workspace_id = Some("ws-live".to_string());
        task.status = status;
        moatless_tasks::task::test_support::seed_task(&pool, &task)
            .await
            .unwrap();
    }

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-open".to_string()]
    );
    // Following is untouched by it — the row is filtered out of the listing,
    // not unfollowed — so reopening one puts it straight back.
    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-closed",
        &TaskPatch {
            status: Some(TaskStatus::Open),
            ..TaskPatch::default()
        },
        &viewer("user-1"),
    )
    .await
    .unwrap();
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-closed".to_string(), "t-open".to_string()]
    );
}

/// A row by id answers about the conversation, not this viewer's involvement.
///
/// It reaches closed work and work someone else follows — enough to name a
/// conversation opened from a link or task tree — but never wider than access:
/// the same read a link resolves through.
#[sqlx::test]
async fn a_listing_row_by_id_reaches_what_neither_listing_carries(pool: PgPool) {
    seed_live_project(&pool).await;
    for (task_id, status) in [
        ("t-open", TaskStatus::Open),
        ("t-closed", TaskStatus::Closed),
    ] {
        let mut task = make_task_row(task_id, "user-1");
        task.workspace_id = Some("ws-live".to_string());
        moatless_tasks::task::test_support::seed_task(&pool, &task)
            .await
            .unwrap();
        moatless_tasks::task::test_support::patch_task(
            &pool,
            task_id,
            &TaskPatch {
                status: Some(status),
                ..TaskPatch::default()
            },
            &viewer("user-1"),
        )
        .await
        .unwrap();
    }
    seed_public_task(&pool, "t-unfollowed", "user-2", Some("ws-live")).await;
    unfollow(&pool, "user-1", "t-unfollowed").await;

    // What the sidebar shows, and what it leaves out.
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-open".to_string()]
    );

    for task_id in ["t-open", "t-closed", "t-unfollowed"] {
        let row = thread_shell_row(&pool, &viewer("user-1"), task_id)
            .await
            .unwrap_or_else(|| panic!("{task_id} has no listing row"));
        assert_eq!(row["id"], task_id);
        // The listing's row, not a second shape that happens to carry an id.
        assert_decodable_thread(&row);
    }
    assert!(
        thread_shell_row(&pool, &viewer("user-1"), "t-closed")
            .await
            .unwrap()["archivedAt"]
            .is_string(),
        "a closed conversation's row says it is archived"
    );
}
