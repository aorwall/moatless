//! Cutting a Task's own conversation back to a turn, in place: every row of a
//! later turn goes, frozen or not, and its `task_turns` row moves to
//! `task_rewound_turns`. A row with no `turn_number` is kept — a
//! queued user Message not yet picked up is pending work, not history — and
//! so is a Sandbox activity, since the Sandbox is not rewound.

use moatless_core::change_feed::{conversation, listing};
use moatless_core::db::DbPool;

use super::fork_dao;
use super::models::TaskFork;

/// What a rewind removed, for the log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RewindCounts {
    pub messages: u64,
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

    let messages =
        crate::conversation::dao::delete_projected(&mut tx, task_id, Some(keep_through)).await?;

    // What those turns were sent goes with them; a pending item has no turn.
    // One a fork inherited is kept `rewound`: its Message reads it, and a
    // rebuild of the fork replays it (`replay_dao::inbox_items`).
    sqlx::query(
        r#"UPDATE task_inbox i SET status = 'rewound', rewound_at = now()
            WHERE i.task_id = $1 AND i.status = 'delivered' AND i.turn_number > $2
              AND EXISTS (SELECT 1 FROM conversation_messages m WHERE m.inbox_id = i.id)"#,
    )
    .bind(task_id)
    .bind(keep_through)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM task_inbox WHERE task_id = $1 AND turn_number > $2 AND status = 'delivered'",
    )
    .bind(task_id)
    .bind(keep_through)
    .execute(&mut *tx)
    .await?;

    // Kept for readers that show what a rewind cut, and read by the next
    // turn's number, which must not reuse theirs. A late terminal event can
    // re-create a turn already recorded here, hence `DO NOTHING`.
    sqlx::query(
        r#"INSERT INTO task_rewound_turns (task_id, turn_number, run_id, status, started_at, ended_at)
           SELECT task_id, turn_number, run_id, status, started_at, ended_at
             FROM task_turns WHERE task_id = $1 AND turn_number > $2
           ON CONFLICT (task_id, turn_number) DO NOTHING"#,
    )
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

    Ok(RewindCounts { messages, turns })
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "rewind_dao_db_tests.rs"]
mod rewind_dao_db_tests;
