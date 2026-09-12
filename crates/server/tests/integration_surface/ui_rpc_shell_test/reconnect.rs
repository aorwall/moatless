//! Reconnecting.
//!
//! A client that comes back holds a position and asks for what it missed. What
//! it may be told is one of three things, and the whole of this ticket is that
//! they are three rather than two: *here is what changed*, *nothing changed*,
//! and *more changed than I kept, here is the list again*. The last two are the
//! same empty result unless they are made different, and a client that cannot
//! tell them apart renders a stale sidebar forever while reporting itself live.

use super::contract::{assert_decodable_snapshot_item, assert_decodable_upsert};
use super::feed::{ingress, one_row, other_peoples_work, reconnect, recorded_after, start_turn};
use super::harness::{reads, viewer};
use super::seeding::{seed_live_project, seed_task};
use moatless_t3code::listing::RESUME_WINDOW;
use sqlx::PgPool;

/// Acceptance criterion 1. Away briefly, things happened, and the list the
/// client already had is updated rather than replaced.
///
/// The teeth are the negative half: `assert!(!rebuilt)`. Before this ticket
/// every subscription answered with a snapshot, so an implementation that
/// ignores `afterSequence` fails here — and so does one that rebuilds whenever
/// it is unsure.
#[sqlx::test]
async fn a_client_that_missed_changes_is_replayed_rather_than_rebuilt(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // It read the listing, then the connection dropped.
    let held = reads(&pool).listing_head().await.unwrap().position;

    // While it was away, a turn ran to completion on a conversation it follows.
    let turn = start_turn(&pool, "t-live", "run-1").await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    assert!(
        recorded_after(&pool, held).await >= 2,
        "the premise failed: nothing was recorded while the client was away"
    );

    let (values, _feed) = reconnect(&pool, &viewer, Some(held)).await;

    assert!(
        !values.iter().any(|item| item["kind"] == "snapshot"),
        "the client was handed a rebuilt listing instead of what it missed: {values:#?}"
    );
    assert_eq!(
        values.len(),
        1,
        "the burst on one conversation coalesces into one row, got {values:#?}"
    );
    assert_decodable_upsert(&values[0]);
    assert_eq!(values[0]["thread"]["id"], "t-live");
    assert_eq!(
        values[0]["thread"]["latestTurn"]["state"], "completed",
        "the row carries the state the client missed, not the state it left"
    );
    assert!(values[0]["sequence"].as_i64().unwrap() > held);
}

/// Acceptance criterion 4, with the teeth the criterion does not have on its
/// own.
///
/// "Replaying from the position a client holds returns nothing" passes against
/// an implementation that always returns nothing — which is exactly the dead
/// path this ticket replaces, so on its own it is worth nothing. So the test is
/// asymmetric, and every half is a break somebody could ship:
///
/// - A change is recorded *before* the position the client holds, so there is
///   something to wrongly re-send. Replaying `WHERE position >= cursor` instead
///   of `>` fails here, which is the second clause of the criterion.
/// - A second change is recorded, and **replaying from the same held position
///   again must now carry it**. An always-empty replay fails here.
/// - The feed that same subscription was handed must deliver it live too, so
///   the empty first answer was a cursor in the right place rather than a
///   subscription that never started.
///
/// Verified by making each of those three breaks and watching this fail.
#[sqlx::test]
async fn a_caught_up_client_is_replayed_nothing_and_still_hears_what_comes_next(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // A change happens, and the client's position is taken *after* it — so this
    // is the change that produced the position it holds.
    let turn = start_turn(&pool, "t-live", "run-1").await;
    let held = reads(&pool).listing_head().await.unwrap().position;
    assert!(held > 0, "the premise failed: nothing produced a position");
    assert_eq!(
        recorded_after(&pool, held).await,
        0,
        "the premise failed: the client is not actually caught up"
    );

    let (values, mut feed) = reconnect(&pool, &viewer, Some(held)).await;
    assert!(
        values.is_empty(),
        "a caught-up client was told something; the change that produced its \
         own position was most likely re-sent: {values:#?}"
    );

    ingress(&pool, "t-live", turn, "agent", "completed").await;

    // Replaying from the *same* position now carries something. Without this,
    // the assertion above is satisfied by a replay that can never answer at
    // all — which is the path this ticket was opened to replace.
    let (now, _) = reconnect(&pool, &viewer, Some(held)).await;
    assert_eq!(
        now.len(),
        1,
        "replaying from a position with a change past it returned nothing: {now:#?}"
    );
    assert_decodable_upsert(&now[0]);

    // And the feed the first reconnection was handed is live: the same change
    // reaches it without a second subscription.
    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
    assert!(event["sequence"].as_i64().unwrap() > held);
}

/// Acceptance criteria 2 and 3 together, because they are the same assertion
/// from two sides: the answer to "too much happened" is a listing, and it is
/// not the answer to "nothing happened".
///
/// Both clients below are told something the other is not. Collapse the two
/// into one answer — return `[]` for the gap, or a snapshot for both — and one
/// of these halves fails.
#[sqlx::test]
async fn a_gap_too_large_to_replay_is_a_fresh_listing_and_silence_is_not(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // Someone who has been away a long time: their position, then more changes
    // than the resume window keeps.
    let stale = reads(&pool).listing_head().await.unwrap().position;
    other_peoples_work(&pool, RESUME_WINDOW + 1).await;
    let head = reads(&pool).listing_head().await.unwrap().position;
    assert_eq!(head - stale, RESUME_WINDOW + 1, "the premise failed");

    let (gap, _feed) = reconnect(&pool, &viewer, Some(stale)).await;
    assert_eq!(
        gap.len(),
        1,
        "a client past the resume window should be handed one listing: {gap:#?}"
    );
    assert_decodable_snapshot_item(&gap[0]);
    assert_eq!(
        gap[0]["snapshot"]["snapshotSequence"].as_i64().unwrap(),
        head,
        "the rebuilt listing resumes from where the deployment actually is"
    );
    // Nothing is quietly absent from it: the conversation the client follows is
    // in the list it was handed, not merely implied by an event it never got.
    let threads = gap[0]["snapshot"]["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "the rebuilt listing is missing rows");
    assert_eq!(threads[0]["id"], "t-live");

    // And the other side of the distinction, on the same deployment: a client
    // that is up to date is told nothing at all, which is a different frame and
    // not a smaller one.
    let (silence, _feed) = reconnect(&pool, &viewer, Some(head)).await;
    assert!(
        silence.is_empty(),
        "\"nothing happened\" and \"too much happened\" are the same answer: {silence:#?}"
    );
}

/// A cursor one past the window is rebuilt and a cursor exactly on it is
/// replayed, so the boundary is where the constant says it is rather than
/// somewhere near it.
#[sqlx::test]
async fn the_resume_window_is_exclusive_at_exactly_its_own_width(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    other_peoples_work(&pool, RESUME_WINDOW + 1).await;
    let head = reads(&pool).listing_head().await.unwrap().position;

    let (edge, _feed) = reconnect(&pool, &viewer, Some(head - RESUME_WINDOW)).await;
    assert!(
        !edge.iter().any(|item| item["kind"] == "snapshot"),
        "a cursor exactly at the window's width was rebuilt: {edge:#?}"
    );

    let (past, _feed) = reconnect(&pool, &viewer, Some(head - RESUME_WINDOW - 1)).await;
    assert_eq!(past.len(), 1);
    assert_eq!(past[0]["kind"], "snapshot");
}

/// The spec's Decision 7, first guard: a position this deployment has never
/// reached is impossible, and is answered with a listing rather than an empty
/// reply.
///
/// This is the failure nothing else watches. T3 guards against the client
/// falling behind the server; neither side guards against the server falling
/// behind the client. A client asking for everything after a position that will
/// never be allocated receives nothing, forever, while reporting itself live.
///
/// (The other guard — an identity derived from the database, which catches a
/// *reused* position after a restore — is ticket 07's.)
#[sqlx::test]
async fn a_position_the_deployment_has_never_reached_is_rebuilt_not_answered_with_silence(
    pool: PgPool,
) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let head = reads(&pool).listing_head().await.unwrap().position;

    for impossible in [head + 1, head + 1_000_000] {
        let (values, _feed) = reconnect(&pool, &viewer, Some(impossible)).await;
        assert_eq!(
            values.len(),
            1,
            "a client resuming from {impossible}, which this deployment has \
             never reached, was not given a listing: {values:#?}"
        );
        assert_decodable_snapshot_item(&values[0]);
    }
}

/// A client holding nothing gets the listing, exactly as before this ticket.
///
/// Kept because the resume path is the same function: a change to how
/// `afterSequence` is read must not turn the first connection of a fresh client
/// into an empty sidebar.
#[sqlx::test]
async fn a_client_holding_no_position_is_sent_the_listing(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let (values, _feed) = reconnect(&pool, &viewer, None).await;
    assert_eq!(values.len(), 1);
    assert_decodable_snapshot_item(&values[0]);
    assert_eq!(
        values[0]["snapshot"]["threads"].as_array().unwrap().len(),
        1
    );
}

/// A replay is filtered by the same two rules the snapshot is, because it is
/// the same query — someone else's change advances the cursor and tells this
/// viewer nothing.
///
/// Worth its own test because replay is the one path that could plausibly be
/// written from the change records themselves, which carry no viewer. That
/// version leaks the id of every conversation in the deployment to everyone.
#[sqlx::test]
async fn a_replay_does_not_carry_changes_the_viewer_may_not_see(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    // Someone else's conversation, in the same project, which `user-1` does not
    // follow. `insert_task` follows for its creator, so only `user-2` has it.
    seed_task(&pool, "t-theirs", "user-2", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let held = reads(&pool).listing_head().await.unwrap().position;
    start_turn(&pool, "t-theirs", "run-theirs").await;
    assert_eq!(
        recorded_after(&pool, held).await,
        1,
        "the premise failed: the change the viewer must not see was not recorded"
    );

    let (values, mut feed) = reconnect(&pool, &viewer, Some(held)).await;
    assert!(
        values.is_empty(),
        "a replay carried someone else's conversation: {values:#?}"
    );

    // And the cursor moved over it regardless, so this viewer's feed is not
    // stalled behind work that will never be theirs.
    start_turn(&pool, "t-live", "run-mine").await;
    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
}
