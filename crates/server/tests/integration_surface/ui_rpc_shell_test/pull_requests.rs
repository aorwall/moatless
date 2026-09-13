//! Linked pull requests.

use super::harness::{
    archived_snapshot, archived_thread_ids, defaults, parse, reads, thread, thread_ids,
    thread_shell_row, viewer,
};
use super::seeding::{seed_live_project, seed_public_task, seed_task, unfollow};
use crate::common::make_task_row;
use crate::common::make_workspace_row;
use chrono::DateTime;
use chrono::SubsecRound;
use chrono::Utc;
use moatless_loops::bindings::test_support as binding_fixtures;
use moatless_t3code::Deployment;
use moatless_t3code::thread_snapshot;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_workspaces::compose;
use serde_json::Value;
use sqlx::PgPool;

// =========================================================================
// Linked pull requests
// =========================================================================

/// The connection every seeded binding hangs off, since `connection_id` is a
/// foreign key.
async fn seed_pr_connection(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO adapter_connections
               (id, adapter_kind, connection_kind, external_account_id, created_by,
                scope, owner_user_id)
           VALUES ('conn-gh', 'github', 'org', 'acme', 'user-1', 'user', 'user-1')
           ON CONFLICT (id) DO NOTHING"#,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// Bind a Task to `subject_key`, in the shape the outbound path writes: a
/// binding carries no status blob until a refresh fetches one, and `state_kind`
/// is what that refresh later fills in.
async fn seed_pr_binding(
    pool: &PgPool,
    binding_id: &str,
    task_id: &str,
    subject_key: &str,
    state_kind: Option<&str>,
) {
    binding_fixtures::insert_binding(
        pool,
        &binding_fixtures::BindingFixture {
            id: binding_id,
            task_id,
            adapter_kind: "github",
            subject_kind: "github_pr",
            subject_key,
            connection_id: "conn-gh",
            origin: "connected_thread",
            external_state_kind: state_kind,
        },
    )
    .await
    .unwrap();
}

/// Fill in the status blob a refresh writes, in the shape
/// `github_external_status` builds it. The listing reads the title and the two
/// branches back out of it.
async fn refresh_pr_binding(pool: &PgPool, binding_id: &str, title: &str, head_ref: &str) {
    binding_fixtures::set_external_status_raw(
        pool,
        binding_id,
        &serde_json::json!({"title": title, "baseRef": "main", "headRef": head_ref}),
        "2026-02-03T04:05:06Z".parse().unwrap(),
    )
    .await
    .unwrap();
}

/// Regression: a binding is written with `external_status_raw` NULL, filled in
/// only by a refresh the listing can't trigger.
///
/// A link conditioned on that blob never appeared for a pull request the agent
/// had just opened.
#[sqlx::test]
async fn a_thread_links_a_pull_request_whose_status_was_never_fetched(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-pr", "user-1", Some("ws-live")).await;
    seed_pr_connection(&pool).await;
    seed_pr_binding(&pool, "b-1", "t-pr", "acme/web#7", None).await;

    let row = thread(&pool, &viewer("user-1"), "t-pr").await;

    assert_eq!(row["linkedPullRequest"]["repository"], "acme/web");
    assert_eq!(row["linkedPullRequest"]["number"], 7);
    // Recovered from `subject_key`, with no blob to read it out of.
    assert_eq!(
        row["linkedPullRequest"]["url"],
        "https://github.com/acme/web/pull/7"
    );
    assert_eq!(row["linkedPullRequests"].as_array().unwrap().len(), 1);
}

#[sqlx::test]
async fn a_thread_links_every_bound_pull_request_live_first_then_newest(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-prs", "user-1", Some("ws-live")).await;
    seed_pr_connection(&pool).await;
    seed_pr_binding(&pool, "b-1", "t-prs", "acme/web#7", Some("open")).await;
    seed_pr_binding(&pool, "b-2", "t-prs", "acme/web#9", Some("merged")).await;
    seed_pr_binding(&pool, "b-3", "t-prs", "acme/api#8", None).await;

    let row = thread(&pool, &viewer("user-1"), "t-prs").await;
    let linked = row["linkedPullRequests"].as_array().unwrap();

    // Live before terminal, then highest number — so the merged #9 sorts last
    // and the unrefreshed #8 counts as live.
    let ordered: Vec<_> = linked
        .iter()
        .map(|pr| (pr["repository"].as_str().unwrap(), pr["number"].as_i64()))
        .collect();
    assert_eq!(
        ordered,
        vec![
            ("acme/api", Some(8)),
            ("acme/web", Some(7)),
            ("acme/web", Some(9)),
        ]
    );
    // The single reference every existing reader uses is the list's primary.
    assert_eq!(row["linkedPullRequest"], linked[0]);
}

/// Spares every pull-request surface a `pullRequests.summary` round trip.
///
/// A refreshed binding's title, branches, and state reach the client on the
/// thread row itself; an unrefreshed one still carries the bare reference.
#[sqlx::test]
async fn a_thread_carries_the_status_of_each_pull_request_that_was_refreshed(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-prs", "user-1", Some("ws-live")).await;
    seed_pr_connection(&pool).await;
    seed_pr_binding(&pool, "b-1", "t-prs", "acme/web#7", Some("draft")).await;
    refresh_pr_binding(&pool, "b-1", "Ship the composer pill", "feature/pill").await;
    seed_pr_binding(&pool, "b-2", "t-prs", "acme/api#8", None).await;

    let row = thread(&pool, &viewer("user-1"), "t-prs").await;
    let linked = row["linkedPullRequests"].as_array().unwrap();
    let refreshed = linked.iter().find(|pr| pr["number"] == 7).unwrap();

    assert_eq!(refreshed["title"], "Ship the composer pill");
    // A draft has no state of its own in the contract, so `isDraft` is what
    // tells it apart from an open pull request.
    assert_eq!(refreshed["state"], "open");
    assert_eq!(refreshed["isDraft"], true);
    assert_eq!(refreshed["baseBranch"], "main");
    assert_eq!(refreshed["headBranch"], "feature/pill");
    assert_eq!(refreshed["updatedAt"], "2026-02-03T04:05:06.000Z");

    let unrefreshed = linked.iter().find(|pr| pr["number"] == 8).unwrap();
    for absent in [
        "title",
        "state",
        "isDraft",
        "baseBranch",
        "headBranch",
        "updatedAt",
    ] {
        assert_eq!(unrefreshed.get(absent), None, "{absent} was never fetched");
    }
}

/// The guard exists so a malformed key drops its own row: the `::bigint` cast
/// runs after the `WHERE`, so a key that reached it with a non-numeric second
/// field would fail the whole listing rather than one binding.
#[sqlx::test]
async fn a_binding_whose_key_is_not_owner_repo_number_drops_without_failing_the_listing(
    pool: PgPool,
) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-odd", "user-1", Some("ws-live")).await;
    seed_pr_connection(&pool).await;
    seed_pr_binding(&pool, "b-1", "t-odd", "acme/web#7", None).await;
    seed_pr_binding(&pool, "b-2", "t-odd", "acme#web#7", None).await;
    seed_pr_binding(&pool, "b-3", "t-odd", "acme/web#none", None).await;

    let row = thread(&pool, &viewer("user-1"), "t-odd").await;
    let linked = row["linkedPullRequests"].as_array().unwrap();

    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["repository"], "acme/web");
    assert_eq!(linked[0]["number"], 7);
}

#[sqlx::test]
async fn a_thread_bound_to_no_pull_request_links_none(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-bare", "user-1", Some("ws-live")).await;

    let row = thread(&pool, &viewer("user-1"), "t-bare").await;

    assert!(row["linkedPullRequest"].is_null());
    assert!(row["linkedPullRequests"].as_array().unwrap().is_empty());
}

/// Absence is one answer with one meaning, and the reader cannot tell which of
/// its causes applies — which is the point, because each of them is a different
/// fact about somebody else's work.
#[sqlx::test]
async fn a_listing_row_for_a_conversation_out_of_reach_is_absent(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-private", "user-2", Some("ws-live")).await;
    let mut errored = make_task_row("t-errored", "user-1");
    errored.workspace_id = Some("ws-live".to_string());
    errored.status = TaskStatus::Error;
    moatless_tasks::task::test_support::seed_task(&pool, &errored)
        .await
        .unwrap();

    for task_id in ["t-private", "t-errored", "t-never-existed"] {
        assert!(
            thread_shell_row(&pool, &viewer("user-1"), task_id)
                .await
                .is_none(),
            "{task_id} disclosed a listing row"
        );
    }
}

/// Open and archived partition what a person follows: archiving is closing, so
/// a conversation moves between them by changing status alone.
///
/// `archivedAt` tells the client which half a row came from; the status write
/// maintains it, stamping on close and clearing on reopen, with no separate
/// call to forget.
#[sqlx::test]
async fn a_closed_conversation_moves_to_the_archived_listing(pool: PgPool) {
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

    // `error` belongs to neither half: it is not open, and it was not put
    // away. Its conversation is unfinished work whose agent fell over.
    assert_eq!(
        archived_thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-closed".to_string()]
    );

    let before: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&pool)
        .await
        .unwrap();
    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-open",
        &TaskPatch {
            status: Some(TaskStatus::Closed),
            ..TaskPatch::default()
        },
        &viewer("user-1"),
    )
    .await
    .unwrap();

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        Vec::<String>::new(),
        "a closed conversation is off the sidebar"
    );
    let archived = archived_snapshot(&pool, &viewer("user-1")).await;
    let closed = archived["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == "t-open")
        .expect("the conversation just closed is in the archive");
    assert!(
        parse(&closed["archivedAt"]) >= before.trunc_subsecs(3),
        "archivedAt is the time it was closed, got {}",
        closed["archivedAt"]
    );

    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-open",
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
        vec!["t-open".to_string()]
    );
    assert_eq!(
        archived_thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-closed".to_string()],
        "reopening takes the conversation out of the archive"
    );
    assert_eq!(
        thread(&pool, &viewer("user-1"), "t-open").await["archivedAt"],
        Value::Null,
        "reopening clears the close time"
    );
}

/// Opening by id is filtered by access, the sidebar by Follow, so a readable
/// Task's link resolves whether or not it's on the sidebar.
///
/// Both are asserted together: the widening is only correct if the conversation
/// opens while the sidebar stays exactly as narrow as before.
#[sqlx::test]
async fn a_readable_conversation_opens_by_id_without_being_followed(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_public_task(&pool, "t-theirs", "user-2", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let snapshot = thread_snapshot(
        &reads(&pool),
        &Deployment {
            model_defaults: &defaults(),
            frontend_url: "http://localhost:3000",
        },
        &viewer,
        "t-theirs",
    )
    .await
    .unwrap();

    assert!(
        snapshot.is_some(),
        "a public conversation this viewer may read opens from its id"
    );
    assert_eq!(
        thread_ids(&pool, &viewer).await,
        Vec::<String>::new(),
        "and opening it does not put it on their sidebar; writing to it is what would"
    );
}

/// Access is the whole of the widening: an unreadable Task answers exactly as
/// it did when the follow rule refused it.
///
/// One answer for every way a conversation can be absent, so which one it was
/// stays undisclosed.
#[sqlx::test]
async fn an_unreadable_conversation_still_does_not_open_by_id(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-private", "user-2", Some("ws-live")).await;

    let snapshot = thread_snapshot(
        &reads(&pool),
        &Deployment {
            model_defaults: &defaults(),
            frontend_url: "http://localhost:3000",
        },
        &viewer("user-1"),
        "t-private",
    )
    .await
    .unwrap();

    assert!(
        snapshot.is_none(),
        "a private conversation belonging to somebody else stays absent"
    );
}

/// The archive is filtered by the same follow rule as the sidebar. A
/// conversation somebody else closed is not in this viewer's archive unless
/// they were involved in it.
#[sqlx::test]
async fn an_unfollowed_closed_conversation_is_absent_from_the_archive(pool: PgPool) {
    seed_live_project(&pool).await;
    let mut task = make_task_row("t-theirs", "user-1");
    task.workspace_id = Some("ws-live".to_string());
    task.status = TaskStatus::Closed;
    moatless_tasks::task::test_support::seed_task(&pool, &task)
        .await
        .unwrap();
    unfollow(&pool, "user-1", "t-theirs").await;

    assert!(
        archived_thread_ids(&pool, &viewer("user-1"))
            .await
            .is_empty()
    );
}

/// A workspace that was deleted is not listable, so a conversation still
/// pointing at it has no project either.
#[sqlx::test]
async fn a_conversation_whose_project_was_deleted_is_left_out(pool: PgPool) {
    let mut deleted = make_workspace_row("ws-gone", "user-1");
    deleted.deleted = true;
    compose::workspaces(pool.clone())
        .insert_workspace(&deleted)
        .await
        .unwrap();
    seed_task(&pool, "t-stranded", "user-1", Some("ws-gone")).await;

    assert!(thread_ids(&pool, &viewer("user-1")).await.is_empty());
}
