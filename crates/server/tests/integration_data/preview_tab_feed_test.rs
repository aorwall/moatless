#![cfg(feature = "db-tests")]
#![expect(clippy::unwrap_used, reason = "tests fail by panicking")]
//! The preview-relay feed's invariants. Needs a real database: the allocation
//! is an `ON CONFLICT` interaction across two CTEs, and the prune's window is
//! measured against the counter rather than against the rows.

use moatless_core::change_feed::preview::{
    RETAINED_CHANGES, current_position, opens_after, prune_opens, record_open,
};
use sqlx::PgPool;

#[sqlx::test]
async fn a_recorded_open_takes_a_position_and_reads_back_whole(pool: PgPool) {
    let position = record_open(
        &pool,
        "replica-a",
        "task-a",
        "script-dev",
        "https://a.example",
    )
    .await
    .unwrap();
    assert_eq!(position, 1);
    assert_eq!(current_position(&pool).await.unwrap(), 1);

    let opens = opens_after(&pool, 0, 10).await.unwrap();
    assert_eq!(opens.len(), 1);
    // Whole, because there is nothing durable to read the tab back from: it
    // exists only in the origin replica's memory.
    assert_eq!(opens[0].origin, "replica-a");
    assert_eq!(opens[0].task_id, "task-a");
    assert_eq!(opens[0].tab_id, "script-dev");
    assert_eq!(opens[0].url, "https://a.example");
}

/// What a reader's cursor does between polls: never re-reads, never skips.
#[sqlx::test]
async fn a_cursor_reads_each_open_once_in_order(pool: PgPool) {
    for n in 1..=5 {
        record_open(
            &pool,
            "replica-a",
            "task-a",
            &format!("tab-{n}"),
            "https://a.example",
        )
        .await
        .unwrap();
    }

    let mut cursor = 0;
    let mut seen = Vec::new();
    loop {
        let opens = opens_after(&pool, cursor, 2).await.unwrap();
        if opens.is_empty() {
            break;
        }
        for open in opens {
            cursor = open.position;
            seen.push(open.tab_id);
        }
    }
    assert_eq!(seen, ["tab-1", "tab-2", "tab-3", "tab-4", "tab-5"]);
    assert_eq!(cursor, 5);
}

/// A replica seeding its cursor at the head replays nothing, which is what
/// keeps a boot from raising tabs for Scripts that have since stopped.
#[sqlx::test]
async fn a_reader_starting_at_the_head_sees_only_what_comes_after(pool: PgPool) {
    record_open(&pool, "replica-a", "task-a", "before", "https://a.example")
        .await
        .unwrap();
    let head = current_position(&pool).await.unwrap();
    assert!(opens_after(&pool, head, 10).await.unwrap().is_empty());

    record_open(&pool, "replica-a", "task-a", "after", "https://a.example")
        .await
        .unwrap();
    let opens = opens_after(&pool, head, 10).await.unwrap();
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0].tab_id, "after");
}

/// The prune measures its window from the counter, so it reclaims nothing on a
/// deployment that has not reached the retention.
#[sqlx::test]
async fn the_prune_keeps_the_retention_window(pool: PgPool) {
    for n in 0..3 {
        record_open(
            &pool,
            "replica-a",
            "task-a",
            &format!("tab-{n}"),
            "https://a.example",
        )
        .await
        .unwrap();
    }
    assert_eq!(prune_opens(&pool).await.unwrap(), 0);
    assert_eq!(opens_after(&pool, 0, 10).await.unwrap().len(), 3);

    // Past the window, by moving the counter rather than by writing a million
    // rows: the prune reads the counter, so this is the same condition.
    sqlx::query("UPDATE preview_tab_position SET position = position + $1")
        .bind(RETAINED_CHANGES)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(prune_opens(&pool).await.unwrap(), 3);
    assert!(opens_after(&pool, 0, 10).await.unwrap().is_empty());
}
