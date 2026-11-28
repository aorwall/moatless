#![cfg(feature = "db-tests")]
#![expect(clippy::unwrap_used, reason = "tests fail by panicking")]
//! The task-event feed's three invariants, a group of cases each.
//!
//! Each needs a real database: the dedup is an `ON CONFLICT` interaction across
//! four CTEs, the claim is `FOR UPDATE SKIP LOCKED` between two connections,
//! and the prune's exclusion is a correlated `NOT EXISTS`.

use std::time::Duration;

use moatless_core::change_feed::events::{
    RETAINED_CHANGES, changes_after, claim_dispatch, current_position, dispatch_depth,
    prune_changes, record_event,
};
use sqlx::PgPool;

/// Claims are never meant to age out mid-test.
const NO_RECLAIM: Duration = Duration::from_hours(1);

async fn record(pool: &PgPool, event_id: &str, event: &str) -> Option<i64> {
    let mut tx = pool.begin().await.unwrap();
    let position = record_event(&mut tx, event_id, "task-a", event)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    position
}

#[sqlx::test]
async fn a_recorded_event_takes_a_position_and_queues_for_dispatch(pool: PgPool) {
    let position = record(&pool, "event-1", r#"{"n":1}"#).await;
    assert_eq!(position, Some(1));
    assert_eq!(current_position(&pool).await.unwrap(), 1);
    assert_eq!(dispatch_depth(&pool).await.unwrap(), 1);

    let claimed = claim_dispatch(&pool, 10, NO_RECLAIM).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].position, 1);
    assert_eq!(claimed[0].event, r#"{"n":1}"#);
}

/// A producer re-publishes an event whose delivery it could not confirm, which
/// is exactly the case where the handlers may not have run. If the dedup drops
/// the re-publish without re-queueing, those handlers never run.
#[sqlx::test]
async fn a_republish_allocates_no_position_but_re_arms_dispatch(pool: PgPool) {
    let first = record(&pool, "event-1", r#"{"n":1}"#).await;
    assert_eq!(first, Some(1));

    // The dispatcher takes it and clears it, but its handler failed, so the
    // producer publishes it again.
    let claimed = claim_dispatch(&pool, 10, NO_RECLAIM).await.unwrap();
    moatless_core::change_feed::events::release_dispatch(&pool, &[claimed[0].position])
        .await
        .unwrap();
    assert_eq!(dispatch_depth(&pool).await.unwrap(), 0);

    let again = record(&pool, "event-1", r#"{"n":1}"#).await;
    assert_eq!(again, None, "a re-publish must not allocate a position");

    // No second copy for clients: one record, one position.
    let visible = changes_after(&pool, 0, 100).await.unwrap();
    assert_eq!(visible.len(), 1);

    assert_eq!(
        dispatch_depth(&pool).await.unwrap(),
        1,
        "the re-publish must re-arm dispatch, or the handler never runs again"
    );
    let reclaimed = claim_dispatch(&pool, 10, NO_RECLAIM).await.unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].position, 1);
}

/// A re-publish of an event still awaiting dispatch must not double-queue it.
#[sqlx::test]
async fn a_republish_of_an_unclaimed_event_leaves_one_queue_row(pool: PgPool) {
    record(&pool, "event-1", r#"{"n":1}"#).await;
    record(&pool, "event-1", r#"{"n":1}"#).await;
    assert_eq!(dispatch_depth(&pool).await.unwrap(), 1);
}

#[sqlx::test]
async fn a_claim_hides_its_events_from_the_next_claim(pool: PgPool) {
    record(&pool, "event-1", r#"{"n":1}"#).await;
    record(&pool, "event-2", r#"{"n":2}"#).await;

    let first = claim_dispatch(&pool, 10, NO_RECLAIM).await.unwrap();
    assert_eq!(first.len(), 2);

    // The claim committed, so the rows are still queued — but claimed, and a
    // second replica polling now must take none of them.
    assert_eq!(dispatch_depth(&pool).await.unwrap(), 2);
    let second = claim_dispatch(&pool, 10, NO_RECLAIM).await.unwrap();
    assert!(
        second.is_empty(),
        "a claimed event was handed to a second claimer before its claim aged out"
    );
}

/// What recovers a replica that died mid-batch: the claim ages out and another
/// replica takes the same events.
#[sqlx::test]
async fn a_claim_that_ages_out_is_taken_again(pool: PgPool) {
    record(&pool, "event-1", r#"{"n":1}"#).await;
    claim_dispatch(&pool, 10, NO_RECLAIM).await.unwrap();

    let retaken = claim_dispatch(&pool, 10, Duration::ZERO).await.unwrap();
    assert_eq!(retaken.len(), 1);
    assert_eq!(retaken[0].position, 1);
}

#[sqlx::test]
async fn releasing_a_claim_drains_it_from_the_queue(pool: PgPool) {
    record(&pool, "event-1", r#"{"n":1}"#).await;
    let claimed = claim_dispatch(&pool, 10, NO_RECLAIM).await.unwrap();
    let positions: Vec<i64> = claimed.iter().map(|change| change.position).collect();

    moatless_core::change_feed::events::release_dispatch(&pool, &positions)
        .await
        .unwrap();
    assert_eq!(dispatch_depth(&pool).await.unwrap(), 0);
    assert!(
        claim_dispatch(&pool, 10, Duration::ZERO)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Pruning must never be what loses an event its handlers, however far behind
/// the dispatcher has fallen.
#[sqlx::test]
async fn pruning_skips_whatever_is_still_queued(pool: PgPool) {
    // Position 1 stays queued; position 2 is dispatched and released.
    record(&pool, "event-stuck", r#"{"n":0}"#).await;
    record(&pool, "event-done", r#"{"n":1}"#).await;
    moatless_core::change_feed::events::release_dispatch(&pool, &[2])
        .await
        .unwrap();

    // Drive the head past the window rather than writing 50,000 rows to reach
    // it; the prune reads the counter, never `MAX(position)`.
    sqlx::query("UPDATE task_event_position SET position = $1 WHERE id = TRUE")
        .bind(RETAINED_CHANGES + 10)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(prune_changes(&pool).await.unwrap(), 1);

    let survivors: Vec<i64> = changes_after(&pool, 0, 10)
        .await
        .unwrap()
        .iter()
        .map(|change| change.position)
        .collect();
    assert_eq!(
        survivors,
        vec![1],
        "the queued event was pruned, so its handlers can never run"
    );
    assert_eq!(dispatch_depth(&pool).await.unwrap(), 1);
}
