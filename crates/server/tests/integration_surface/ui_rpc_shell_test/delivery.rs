//! Delivery: a watching client sees a turn change.
//!
//! The snapshot is a listing read at a position; everything that happens after
//! it reaches the client as stream events off the change feed. These tests drive
//! the real writers — the turn-number allocator and the gRPC ingress — rather
//! than inserting change records by hand, because a record written by a test
//! helper proves only that the helper works. What is asserted is what a client
//! watching the feed is handed.

use super::feed::{
    delivered, ingress, one_item, one_row, recorded_after, start_turn, watcher, watching,
};
use super::harness::{defaults, head, reads, viewer};
use super::seeding::{seed_live_project, seed_task, unfollow};
use moatless_t3code::ShellFeed;
use moatless_t3code::shell_snapshot;
use moatless_tasks::task::models::TaskPatch;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

#[sqlx::test]
async fn starting_a_turn_moves_the_row_in_a_watching_client(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;

    // Nothing has happened yet, so the client is told nothing.
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());

    start_turn(&pool, "t-live", "run-1").await;

    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
    assert_eq!(event["thread"]["projectId"], "ws-live");
    assert_eq!(
        event["thread"]["latestTurn"]["state"], "running",
        "a turn that just started is running"
    );
}

#[sqlx::test]
async fn every_way_a_turn_ends_moves_the_row(pool: PgPool) {
    seed_live_project(&pool).await;

    // Five Moatless endings onto T3's four states. `stopped` is a person
    // cutting a turn short, which T3 calls `interrupted`; Moatless's own
    // `interrupted` is the Sandbox going away mid-turn, which is still going to
    // carry on, so T3 keeps hearing `running`.
    let endings: &[(&str, &str, &str, &str)] = &[
        ("t-completed", "taskTurn", "completed", "completed"),
        ("t-agent-completed", "agent", "completed", "completed"),
        ("t-error", "agent", "error", "error"),
        ("t-stopped", "agent", "stopped", "interrupted"),
        ("t-interrupted", "taskTurn", "turn_interrupted", "running"),
    ];

    let viewer = viewer("user-1");
    for (task_id, event_type, sub_type, expected) in endings {
        seed_task(&pool, task_id, "user-1", Some("ws-live")).await;
        let turn = start_turn(&pool, task_id, "run-1").await;

        // Watch from *after* the start, so the only change the feed can carry
        // is the ending itself.
        let mut feed = watching(&pool, &viewer).await;
        ingress(&pool, task_id, turn, event_type, sub_type).await;

        let event = one_row(&pool, &mut feed, &viewer).await;
        assert_eq!(
            event["thread"]["id"], *task_id,
            "{event_type}.{sub_type} delivered someone else's row"
        );
        assert_eq!(
            event["thread"]["latestTurn"]["state"], *expected,
            "{event_type}.{sub_type} should reach the client as `{expected}`"
        );
    }
}

#[sqlx::test]
async fn a_burst_on_one_conversation_delivers_one_row(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    start_turn(&pool, "t-live", "run-1").await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;
    let watched_from = reads(&pool).listing_head().await.unwrap().position;

    // Four state moves on one conversation between two polls: a new run's turn
    // is allocated, the pod goes away, the run resumes, the turn completes.
    let turn = start_turn(&pool, "t-live", "run-2").await;
    ingress(&pool, "t-live", turn, "taskTurn", "turn_interrupted").await;
    ingress(&pool, "t-live", turn, "taskTurn", "active").await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;

    // The premise: four changes really were recorded. Without this the
    // coalescing assertion below would also pass against a writer that recorded
    // one change in four, which is a different bug and not this one's fix.
    assert_eq!(recorded_after(&pool, watched_from).await, 4);

    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");

    // The row is re-read whole at the burst's last position, so it carries the
    // end state and not the state of whichever change was collapsed last.
    assert_eq!(event["thread"]["latestTurn"]["state"], "completed");
    assert_eq!(
        event["sequence"],
        watched_from + 4,
        "the delivered row stands in for every change up to the burst's last"
    );
}

#[sqlx::test]
async fn a_change_reaches_only_the_clients_whose_listing_holds_it(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-mine", "user-1", Some("ws-live")).await;
    // Not readable: someone else's private conversation.
    seed_task(&pool, "t-theirs", "user-2", Some("ws-live")).await;
    // Readable but not followed — the listing's other filter, and the one a
    // re-read keyed only on access would let through.
    seed_task(&pool, "t-unfollowed", "user-1", Some("ws-live")).await;
    unfollow(&pool, "user-1", "t-unfollowed").await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;
    let watched_from = reads(&pool).listing_head().await.unwrap().position;

    start_turn(&pool, "t-theirs", "run-1").await;
    start_turn(&pool, "t-unfollowed", "run-1").await;
    start_turn(&pool, "t-mine", "run-1").await;

    // All three turns recorded a change — the feed is filtering, not empty.
    assert_eq!(recorded_after(&pool, watched_from).await, 3);

    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(
        event["thread"]["id"], "t-mine",
        "only what is in this viewer's own listing is delivered"
    );

    // And the cursor moved past the change that was not theirs, rather than
    // stalling on it.
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
}

#[sqlx::test]
async fn a_caught_up_watcher_is_told_nothing(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;
    let turn = start_turn(&pool, "t-live", "run-1").await;
    one_row(&pool, &mut feed, &viewer).await;

    // Re-delivering the same event changes no row, so nothing is recorded and
    // nothing is sent: the Redis consumer re-applies every event the ingress
    // already committed, and a client that saw one row per re-application would
    // see the sidebar move for work that had already finished.
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    one_row(&pool, &mut feed, &viewer).await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;

    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
}

/// A second pool onto the same database, closed. Every query through it fails,
/// which is the workspace half of the same injection with no stub to write.
async fn a_pool_that_has_gone_away(pool: &PgPool) -> PgPool {
    let closed = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    closed.close().await;
    closed
}

/// A poll that fails after reading its change records still delivers them.
///
/// The cursor is the client's only record of what it has been told about, and
/// `watch_shell` deliberately keeps a subscription alive across a transient
/// failure rather than presenting a database blip as a dead sidebar. That
/// decision is sound and its premise is that the failed poll left the cursor
/// alone. Advance the cursor before the reads that build the batch and the two
/// together lose the batch permanently: the next poll starts above it, the
/// client is never told, and it goes on reporting itself live — the silent
/// staleness this whole feed exists to prevent, arriving through its own error
/// handling.
///
/// Both reads that happen after the change records are covered, and both
/// separately, because a cursor moved to sit between them fails only one: a fix
/// that advanced after the conversation re-read but before the workspace read
/// would pass the first half of this test and lose every batch whose workspace
/// read failed.
#[sqlx::test]
async fn a_poll_that_fails_after_reading_its_changes_delivers_them_on_the_next_one(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    let turn = start_turn(&pool, "t-live", "run-1").await;
    one_row(&pool, &mut feed, &viewer).await;

    // (1) The conversation re-read fails.
    let watched_from = head(&pool).await.position;
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    assert_eq!(
        recorded_after(&pool, watched_from).await,
        1,
        "the premise failed: nothing was recorded for the failed poll to lose"
    );
    let refused = feed
        .poll(&reads(&pool).refusing_the_row_read(), &defaults(), &viewer)
        .await;
    assert!(
        refused.is_err(),
        "the premise failed: the read did not fail"
    );

    let row = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(row["thread"]["id"], "t-live");
    assert_eq!(
        row["thread"]["latestTurn"]["state"], "completed",
        "the turn ending was recorded, read, and then dropped by the poll that \
         failed after moving past it"
    );
    held.apply(&row);

    // (2) The workspace read fails, one statement further on.
    let watched_from = head(&pool).await.position;
    moatless_tasks::task::test_support::patch_task(
        &pool,
        "t-live",
        &TaskPatch {
            name: Some("Renamed".to_string()),
            ..TaskPatch::default()
        },
        &viewer,
    )
    .await
    .unwrap();
    assert_eq!(recorded_after(&pool, watched_from).await, 1);
    let gone = a_pool_that_has_gone_away(&pool).await;
    let refused = feed
        .poll(
            &reads(&pool).with_workspaces_from(&reads(&gone)),
            &defaults(),
            &viewer,
        )
        .await;
    assert!(
        refused.is_err(),
        "the premise failed: the read did not fail"
    );

    let item = one_item(&pool, &mut feed, &viewer).await;
    assert_eq!(item["kind"], "thread-upserted");
    assert_eq!(item["thread"]["title"], "Renamed");
    held.apply(&item);
}

#[sqlx::test]
async fn each_delivery_carries_a_position_past_the_one_before_it(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let snapshot = shell_snapshot(&reads(&pool), &defaults(), &viewer)
        .await
        .unwrap();
    let mut feed = ShellFeed::resuming_from(snapshot.identity.clone(), snapshot.sequence)
        .holding(snapshot.thread_ids(), snapshot.project_ids());

    let turn = start_turn(&pool, "t-live", "run-1").await;
    let started = one_row(&pool, &mut feed, &viewer).await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    let finished = one_row(&pool, &mut feed, &viewer).await;

    // The client discards anything at or below the position it holds, so a
    // delivery that did not advance past the snapshot's would be dropped in
    // silence and the row would never move.
    let started = started["sequence"].as_i64().unwrap();
    let finished = finished["sequence"].as_i64().unwrap();
    assert!(
        started > snapshot.sequence,
        "the start was delivered at {started}, at or below the snapshot's {}",
        snapshot.sequence
    );
    assert!(finished > started);
    assert_eq!(
        finished,
        reads(&pool).listing_head().await.unwrap().position,
        "the last delivery sits at the position the deployment has reached"
    );
}
