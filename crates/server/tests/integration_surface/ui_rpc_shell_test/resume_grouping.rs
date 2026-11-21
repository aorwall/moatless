//! What survives the merge of 05 and 06.
//!
//! The opening snapshot answers both which Tasks are held and which projects
//! can group them. A resumed feed reconstructs only the grouping half.

use super::contract::{
    THREAD_REMOVED_REQUIRED, assert_decodable_snapshot_item, assert_decodable_upsert, assert_keys,
};
use super::feed::{
    Held, delivered, ingress, one_item, one_row, other_peoples_work, reconnect, reconnect_v2,
    recorded_after, start_turn, watching,
};
use super::harness::{head, reads, viewer};
use super::restore::back_up_and_restore;
use super::seeding::{follow, seed_live_project, seed_task, seed_workspaces, unfollow};
use crate::common::make_task_row;
use crate::common::make_workspace_row;
use moatless_core::change_feed::listing::record_change;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::models::Visibility;
use moatless_workspaces::compose;
use moatless_workspaces::workspace::UpdateOutcome;
use moatless_workspaces::workspace::models::WorkspacePatch;
use serde_json::Value;
use sqlx::PgPool;

// =========================================================================
// What survives the merge of 05 and 06
// =========================================================================
//
// The opening snapshot answers both which Tasks are held and which projects can
// group them. A resumed feed reconstructs only the grouping half.

/// A client that reconnects, catches up on nothing, and then watches one
/// conversation change as a row, not a listing.
///
/// Catches `ShellFeed::resuming_from` leaving the `grouped` set empty, which
/// makes `poll` treat an unrecognized project as grounds to send the whole
/// listing.
#[sqlx::test]
async fn a_resumed_client_watches_the_next_change_as_a_row_not_a_listing(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // It read the listing, the socket dropped, and nothing happened meanwhile.
    let held = head(&pool).await.position;
    let (values, mut feed) = reconnect(&pool, &viewer, Some(held)).await;
    assert!(
        values.is_empty(),
        "the premise failed: this client was not caught up, so the first live \
         poll is not what is being measured: {values:#?}"
    );

    // One turn, on a conversation in a project it already had.
    let turn = start_turn(&pool, "t-live", "run-1").await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;

    let items = delivered(&pool, &mut feed, &viewer).await;
    assert!(
        !items.iter().any(|item| item["kind"] == "snapshot"),
        "a resumed client was sent its whole listing for one conversation \
         changing: {items:#?}"
    );
    assert_eq!(
        items.len(),
        1,
        "the burst on one conversation coalesces into one row, got {items:#?}"
    );
    assert_decodable_upsert(&items[0]);
    assert_eq!(items[0]["thread"]["id"], "t-live");
    assert_eq!(items[0]["thread"]["latestTurn"]["state"], "completed");
}

/// Archiving reaches a resumed client, which is every client — the T3 web app
/// always resumes from its snapshot's sequence.
///
/// A resumed feed can't prove which ids it sent, so it may not name one in a
/// removal; archiving instead arrives as the row itself, carrying `archivedAt`.
#[sqlx::test]
async fn archiving_reaches_a_resumed_client_as_the_row_it_archived(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let held = head(&pool).await.position;
    let (_, mut feed) = reconnect(&pool, &viewer, Some(held)).await;

    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-live",
        &TaskPatch {
            status: Some(TaskStatus::Closed),
            ..TaskPatch::default()
        },
        &viewer,
    )
    .await
    .unwrap();

    let items = delivered(&pool, &mut feed, &viewer).await;
    assert_eq!(
        items.len(),
        1,
        "archiving is one event on one conversation, got {items:#?}"
    );
    assert_decodable_upsert(&items[0]);
    assert_eq!(items[0]["thread"]["id"], "t-live");
    assert!(
        !items[0]["thread"]["archivedAt"].is_null(),
        "the archived row must carry the time it was archived, or the client \
         has no reason to take it off the sidebar: {items:#?}"
    );

    // Reopening is the same shape at the other status, so the row comes back
    // with nothing to filter it out by.
    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-live",
        &TaskPatch {
            status: Some(TaskStatus::Open),
            ..TaskPatch::default()
        },
        &viewer,
    )
    .await
    .unwrap();

    let items = delivered(&pool, &mut feed, &viewer).await;
    assert_eq!(items.len(), 1, "got {items:#?}");
    assert_decodable_upsert(&items[0]);
    assert_eq!(items[0]["thread"]["archivedAt"], Value::Null);
}

/// The delta must not be bought by weakening the case it was carved out of.
///
/// A conversation entering a project the client has no group for still arrives
/// as a listing, resumed or fresh; this fails if `grouped` admits every project
/// rather than just the ones the viewer can read.
#[sqlx::test]
async fn a_resumed_client_gaining_an_unreadable_project_still_gets_the_listing_whole(pool: PgPool) {
    seed_workspaces(&pool).await;
    // `ws-other` belongs to `user-2`, and `user-1` cannot read it.
    let mut theirs = make_task_row("t-theirs", "user-2");
    theirs.workspace_id = Some("ws-other".to_string());
    theirs.visibility = Visibility::Public;
    theirs.owner_user_id = None;
    moatless_tasks::task::test_support::seed_task(&pool, &theirs)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-1");
    let mut held = Held::opening(&pool, &watcher_viewer).await;
    assert!(
        !held.projects.contains(&"ws-other".to_string()),
        "the premise failed: the private project is in their listing already"
    );

    let (values, mut feed) = reconnect(&pool, &watcher_viewer, Some(held.sequence)).await;
    assert!(values.is_empty());

    follow(&pool, "user-1", "t-theirs").await;

    let item = one_item(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(
        item["kind"], "snapshot",
        "a resumed client was sent a row under a project it has no group for, \
         and drops it in silence: {item:#?}"
    );
    held.apply(&item);
    assert_eq!(held.threads, ["t-theirs"]);
    assert!(held.projects.contains(&"ws-other".to_string()));
}

/// Criterion 4: a removal after a resume names only ids that client was sent,
/// exactly as on a fresh subscription.
///
/// A resumed feed's removable set starts empty and grows only by delivery, so
/// ids never sent stay unmentioned no matter how they leave, while a delivered
/// one comes back named alone.
#[sqlx::test]
async fn a_removal_after_a_resume_names_only_ids_that_client_was_sent(pool: PgPool) {
    // A deployment-wide project, so this viewer can read it in their own right
    // and their resumed feed can deliver into it as a row. A project they held
    // only because a conversation referenced it would cost a listing here, and
    // that is a different test.
    seed_workspaces(&pool).await;
    // `user-2` may read neither of `user-1`'s, and follows one of its own.
    seed_task(&pool, "t-theirs", "user-1", Some("ws-global")).await;
    seed_task(&pool, "t-deleted", "user-1", Some("ws-global")).await;
    let mut mine = make_task_row("t-mine", "user-2");
    mine.workspace_id = Some("ws-global".to_string());
    mine.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(&pool, &mine)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-2");
    let held = head(&pool).await.position;
    let (values, mut feed) = reconnect(&pool, &watcher_viewer, Some(held)).await;
    assert!(values.is_empty());

    // Two ways a row leaves someone else's listing. Neither is this viewer's
    // business, and a resumed feed that had reconstructed a held set from what
    // this viewer may read would still have to stay silent here.
    unfollow(&pool, "user-1", "t-theirs").await;
    assert!(
        moatless_tasks::task::test_support::remove_task(&pool, "t-deleted", &viewer("user-1"))
            .await
            .unwrap()
    );
    assert_eq!(recorded_after(&pool, held).await, 2, "the premise failed");
    assert_eq!(
        delivered(&pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "a resumed client was told the id of a conversation it was never sent"
    );

    // Its own conversation moves, so now the feed has sent it — and only now
    // may it be taken back.
    let turn = start_turn(&pool, "t-mine", "run-1").await;
    let upsert = one_row(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(upsert["thread"]["id"], "t-mine");

    ingress(&pool, "t-mine", turn, "agent", "completed").await;
    assert!(
        moatless_tasks::task::test_support::remove_task(&pool, "t-mine", &watcher_viewer)
            .await
            .unwrap()
    );
    let items = delivered(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one removal, got {items:#?}");
    assert_keys(&items[0], "thread-removed", THREAD_REMOVED_REQUIRED, &[]);
    assert_eq!(items[0]["kind"], "thread-removed");
    assert_eq!(items[0]["threadId"], "t-mine");
}

/// Criterion 6: the held set is what the client was *shown*, not what the
/// viewer may *read*.
///
/// The two differ when a followed, readable conversation names a workspace
/// with no row: it's projected out but still in `list_followed`, so a held set
/// built from the latter would send a removal for a row never shown.
#[sqlx::test]
async fn a_removal_never_names_a_row_the_listing_did_not_show(pool: PgPool) {
    seed_workspaces(&pool).await;
    seed_task(&pool, "t-shown", "user-1", Some("ws-own")).await;
    // Followed, readable, and not listable: `tasks.workspace_id` has no foreign
    // key, so it can name a row that is gone. Seeded first against `ws-own`
    // because the Sandbox row creation writes does have one. A soft-deleted
    // workspace would not do: a readable Task naming it keeps it listed.
    seed_task(&pool, "t-orphan", "user-1", Some("ws-own")).await;
    sqlx::query("UPDATE tasks SET workspace_id = 'ws-gone' WHERE task_id = 't-orphan'")
        .execute(&pool)
        .await
        .unwrap();
    let watcher_viewer = viewer("user-1");

    // The premise, stated against the query a reconstruction would use.
    let followed = reads(&pool)
        .followed_task_ids(&watcher_viewer)
        .await
        .unwrap();
    assert!(
        followed.iter().any(|task_id| task_id == "t-orphan"),
        "the premise failed: the orphan is not among what this viewer may read"
    );
    // Opened through `shell_subscription`, not through this file's helpers, so
    // the seeding under test is the one the server actually does.
    let (opening, mut fresh) = reconnect(&pool, &watcher_viewer, None).await;
    let mut held = Held::default();
    assert_eq!(opening.len(), 1);
    held.apply(&opening[0]);
    assert_eq!(
        held.threads,
        ["t-shown"],
        "the premise failed: the orphan was shown after all"
    );

    let resumed_from = head(&pool).await.position;
    let (values, mut resumed) = reconnect(&pool, &watcher_viewer, Some(resumed_from)).await;
    assert!(values.is_empty());

    // It leaves. Neither feed may name it, because neither client ever saw it.
    assert!(
        moatless_tasks::task::test_support::remove_task(&pool, "t-orphan", &watcher_viewer)
            .await
            .unwrap()
    );
    assert_eq!(
        recorded_after(&pool, resumed_from).await,
        1,
        "the premise failed: the deletion recorded nothing to deliver"
    );
    for (what, feed) in [("opening", &mut fresh), ("resumed", &mut resumed)] {
        assert_eq!(
            delivered(&pool, feed, &watcher_viewer).await,
            Vec::<Value>::new(),
            "the {what} subscription took its held set from what this viewer \
             may read rather than from what its client was shown"
        );
    }
}

/// Criterion 5: `rebuild` must replace what the feed believes — identity,
/// cursor, and the `threads`/grouping sets together — not just the
/// client's copy.
///
/// A row from the replaced listing must never come back as a removal, and
/// a row the rebuild introduced must come back as one when it leaves.
#[sqlx::test]
async fn a_rebuilt_feed_describes_the_listing_it_sent_not_the_one_it_replaced(pool: PgPool) {
    let watcher_viewer = viewer("user-1");
    seed_live_project(&pool).await;
    seed_task(&pool, "t-in-both", "user-1", Some("ws-live")).await;

    let restored = back_up_and_restore(pool).await;

    // The deployment carries on, and its clients hold rows the copy never got.
    seed_task(&restored.origin, "t-origin-only", "user-1", Some("ws-live")).await;
    other_peoples_work(&restored.origin, 100).await;
    let mut feed = watching(&restored.origin, &watcher_viewer).await;
    assert!(
        delivered(&restored.origin, &mut feed, &watcher_viewer)
            .await
            .is_empty()
    );

    // The copy grows a project and a conversation of its own, which this client
    // has never been sent anything about.
    compose::workspaces(restored.pool.clone())
        .insert_workspace(&make_workspace_row("ws-restore-only", "user-1"))
        .await
        .unwrap();
    seed_task(
        &restored.pool,
        "t-restore-only",
        "user-1",
        Some("ws-restore-only"),
    )
    .await;

    // Repointed. One poll, one rebuilt listing.
    let items = delivered(&restored.pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one rebuild, got {items:#?}");
    assert_decodable_snapshot_item(&items[0]);
    let mut held = Held::default();
    held.apply(&items[0]);
    held.threads.sort();
    assert_eq!(held.threads, ["t-in-both", "t-restore-only"]);
    assert!(held.projects.contains(&"ws-restore-only".to_string()));
    assert!(
        !held.threads.contains(&"t-origin-only".to_string()),
        "the premise failed: the copy has the row the client is holding, so a \
         stale set would not be observable"
    );

    // A row from the listing the rebuild replaced. It changes in the database
    // that is actually there now, where it does not exist.
    let mut tx = restored.pool.begin().await.unwrap();
    record_change(&mut tx, "t-origin-only").await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        delivered(&restored.pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "the rebuilt feed took a row back from the listing it had already \
         replaced — the client has no such row, and the id names nothing"
    );

    // A row only the rebuild snapshot introduced, leaving.
    assert!(
        moatless_tasks::task::test_support::remove_task(
            &restored.pool,
            "t-restore-only",
            &watcher_viewer
        )
        .await
        .unwrap()
    );
    let items = delivered(&restored.pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one removal, got {items:#?}");
    assert_eq!(items[0]["kind"], "thread-removed");
    assert_eq!(items[0]["threadId"], "t-restore-only");

    // A project only the rebuild introduced, deleted: the rebuilt feed holds
    // it, so it sends a listing without it.
    assert!(
        compose::workspaces(restored.pool.clone())
            .soft_delete("ws-restore-only", &watcher_viewer)
            .await
            .unwrap()
    );
    let items = delivered(&restored.pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one rebuild, got {items:#?}");
    held.apply(&items[0]);
    assert!(!held.projects.contains(&"ws-restore-only".to_string()));

    restored.discard().await;
}

/// Regression: a client resuming from a listing older than a Workspace was
/// sent threads in it and never the project, so it could not send to them.
/// The replay carries the project first, then the thread, and no snapshot.
#[sqlx::test]
async fn a_resumed_v2_client_is_replayed_a_project_created_while_away(pool: PgPool) {
    seed_live_project(&pool).await;
    let viewer = viewer("user-1");
    let held = head(&pool).await.position;

    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-new", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-new", "user-1", Some("ws-new")).await;

    let (values, _feed) = reconnect_v2(&pool, &viewer, Some(held)).await;
    let kinds: Vec<&str> = values
        .iter()
        .map(|item| item["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["project.updated", "thread.updated"], "{values:#?}");
    assert_eq!(values[0]["project"]["id"], "ws-new");
    assert_eq!(values[1]["thread"]["projectId"], "ws-new");
    assert!(values[0]["sequence"].as_i64() < values[1]["sequence"].as_i64());
}

/// A rename reaches a watching V2 client as one `project.updated`; the same
/// patch applied again, as every git re-sync does, records and sends nothing.
#[sqlx::test]
async fn a_renamed_project_reaches_a_v2_client_once(pool: PgPool) {
    seed_live_project(&pool).await;
    let viewer = viewer("user-1");
    let (_, mut feed) = reconnect_v2(&pool, &viewer, Some(head(&pool).await.position)).await;
    let rename = WorkspacePatch {
        name: Some("Renamed".to_string()),
        ..WorkspacePatch::default()
    };

    for _ in 0..2 {
        assert_eq!(
            compose::workspaces(pool.clone())
                .update("ws-live", &rename, &viewer)
                .await
                .unwrap(),
            UpdateOutcome::Applied
        );
    }

    let items = delivered(&pool, &mut feed, &viewer).await;
    assert_eq!(items.len(), 1, "{items:#?}");
    assert_eq!(items[0]["kind"], "project.updated");
    assert_eq!(items[0]["project"]["title"], "Renamed");
}
