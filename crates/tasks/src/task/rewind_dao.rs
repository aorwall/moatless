//! Cutting a Task's own conversation back to a turn.
//!
//! The mirror of `message::fork_dao`, which copies rows at or below a cut onto
//! another Task; this deletes the rows above one, in place. A row with no
//! `turn_number` is kept — a queued user Message not yet picked up is pending
//! work, not history.
//!
//! The mirror stops at activities that happened on no Message: the copy takes
//! those by `position` (a fork's Sandbox never saw them); a rewind leaves them.

use moatless_core::change_feed::{conversation, listing};
use moatless_core::db::DbPool;

use super::fork_dao;
use super::models::TaskFork;

/// What a rewind removed, for the log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RewindCounts {
    pub messages: u64,
    pub activities: u64,
    pub turns: u64,
}

/// Delete everything `task_id` recorded above turn `keep_through`, and set
/// `anchor` as the `task_forks` row to resume from (or clear it), atomically —
/// else the pod could resume a deleted history.
pub(crate) async fn truncate_conversation(
    pool: &DbPool,
    task_id: &str,
    keep_through: i32,
    anchor: Option<&TaskFork>,
) -> Result<RewindCounts, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Read the uuids first: `task_activities` and `task_message_usage` are
    // keyed by them and neither has a foreign key that would cascade.
    // `task_message_contexts` does, so the Message delete below takes it.
    let doomed: Vec<String> = sqlx::query_scalar(
        "SELECT uuid FROM task_messages \
         WHERE task_id = $1 AND turn_number IS NOT NULL AND turn_number > $2",
    )
    .bind(task_id)
    .bind(keep_through)
    .fetch_all(&mut *tx)
    .await?;

    // Only what happened on a deleted Message. A `sandbox_status`,
    // `sandbox_warning` or `sandbox_error` row carries no `message_uuid`
    // (`task_activities/dao.rs::record_sandbox`) and stays: it records what
    // happened to the Sandbox, and the Sandbox is not rewound — its working
    // copy is still what those rows describe. `fork_dao::copy_conversation`
    // takes them by `position` because a fork's Sandbox never saw them.
    let activities =
        sqlx::query("DELETE FROM task_activities WHERE task_id = $1 AND message_uuid = ANY($2)")
            .bind(task_id)
            .bind(&doomed)
            .execute(&mut *tx)
            .await?
            .rows_affected();

    sqlx::query("DELETE FROM task_message_usage WHERE task_id = $1 AND message_uuid = ANY($2)")
        .bind(task_id)
        .bind(&doomed)
        .execute(&mut *tx)
        .await?;

    let messages = sqlx::query("DELETE FROM task_messages WHERE task_id = $1 AND uuid = ANY($2)")
        .bind(task_id)
        .bind(&doomed)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    // What those turns were sent goes with them; a pending item has no turn.
    sqlx::query("DELETE FROM task_inbox WHERE task_id = $1 AND turn_number > $2")
        .bind(task_id)
        .bind(keep_through)
        .execute(&mut *tx)
        .await?;

    let turns = sqlx::query("DELETE FROM task_turns WHERE task_id = $1 AND turn_number > $2")
        .bind(task_id)
        .bind(keep_through)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    match anchor {
        Some(row) => fork_dao::record_rewind_anchor(&mut tx, row).await?,
        None => fork_dao::clear_anchors(&mut tx, task_id).await?,
    }

    // The instant of the cut, which `thread.reverted` carries: the projection
    // timestamps that event from `tasks.updated_at`, and no delete above moves
    // it. It also re-sorts the thread list, which orders by the same column.
    sqlx::query("UPDATE tasks SET updated_at = NOW() WHERE task_id = $1")
        .bind(task_id)
        .execute(&mut *tx)
        .await?;

    // Above the listing change and above every change the deleted rows were
    // recorded at, so a live reader applies the cut after them.
    conversation::record_revert_change(&mut tx, task_id, keep_through).await?;

    // `latest_user_message_at`, `pending_question`, `pending_plan` and
    // `assistant_model` are all read off rows this deleted, so the listing row
    // is stale until a reader re-reads it. Last, per `record_change`'s "call
    // this last".
    listing::record_change(&mut tx, task_id).await?;

    tx.commit().await?;

    Ok(RewindCounts {
        messages,
        activities,
        turns,
    })
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "rewind_dao_db_tests.rs"]
mod rewind_dao_db_tests;
