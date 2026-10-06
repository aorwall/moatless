//! `task_inbox` reads and writes.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use moatless_core::db::DbPool;

use super::models::{InboxItem, InboxRow, NewInboxItem, RunStamp};

/// Inserts `item`, or returns `None` when its id or its Adapter message id is
/// already stored: a redelivery, which must not wake the Sandbox again.
pub(crate) async fn submit(
    tx: &mut Transaction<'_, Postgres>,
    item: &NewInboxItem,
) -> Result<Option<InboxItem>, sqlx::Error> {
    let origin = &item.origin;
    let channel = origin.channel();
    let sender = origin.sender();
    let options = serde_json::to_value(&item.options).map_err(|e| sqlx::Error::Encode(e.into()))?;
    let row: Option<InboxRow> = sqlx::query_as(
        r#"INSERT INTO task_inbox
               (id, task_id, text, file_ids, delivery_options, answers_tool_use_id, origin,
                sent_by, source_task_id, external_sender_id, external_author,
                adapter, external_message_id, external_subject_key, external_source_name)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
           ON CONFLICT DO NOTHING
           RETURNING *"#,
    )
    .bind(item.id)
    .bind(&item.task_id)
    .bind(&item.text)
    .bind(&item.file_ids)
    .bind(options)
    .bind(&item.answers_tool_use_id)
    .bind(origin.as_str())
    .bind(origin.sent_by())
    .bind(origin.source_task_id())
    .bind(sender.and_then(|s| s.id.as_deref()))
    .bind(sender.and_then(|s| s.author.as_deref()))
    .bind(channel.map(|c| c.adapter.as_str()))
    .bind(channel.map(|c| c.external_message_id.as_str()))
    .bind(channel.and_then(|c| c.subject_key.as_deref()))
    .bind(channel.and_then(|c| c.source_name.as_deref()))
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    // The reconciler must see waiting work while the pod is down, so the wake
    // signal lives on the `sandboxes` row and commits with the item. Ahead of
    // the Projector's change-feed writes, per the lock order in
    // `moatless_core::change_feed`.
    sqlx::query(
        r#"UPDATE sandboxes s
              SET pending_work_at = GREATEST(s.pending_work_at, NOW()),
                  updated_at = NOW()
             FROM tasks t
            WHERE t.task_id = $1
              AND s.sandbox_id = t.sandbox_id"#,
    )
    .bind(&item.task_id)
    .execute(&mut **tx)
    .await?;

    InboxItem::try_from(row).map(Some)
}

/// Oldest first, at most `limit`, less every `queue` item when `hold_queued`.
pub(crate) async fn pending(
    pool: &DbPool,
    task_id: &str,
    limit: u32,
    hold_queued: bool,
) -> Result<Vec<InboxItem>, sqlx::Error> {
    let rows: Vec<InboxRow> = sqlx::query_as(
        r#"SELECT * FROM task_inbox
            WHERE task_id = $1 AND status = 'pending'
              AND NOT ($3 AND COALESCE(delivery_options ? 'queue', false))
            ORDER BY created_at, id
            LIMIT $2"#,
    )
    .bind(task_id)
    .bind(i64::from(limit))
    .bind(hold_queued)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(InboxItem::try_from).collect()
}

/// The items `turn_number` delivered, oldest first.
pub(crate) async fn delivered_in_turn(
    pool: &DbPool,
    task_id: &str,
    turn_number: i32,
) -> Result<Vec<InboxItem>, sqlx::Error> {
    let rows: Vec<InboxRow> = sqlx::query_as(
        r#"SELECT * FROM task_inbox
            WHERE task_id = $1 AND turn_number = $2 AND status = 'delivered'
            ORDER BY created_at, id"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(InboxItem::try_from).collect()
}

pub(crate) async fn find(pool: &DbPool, id: Uuid) -> Result<Option<InboxItem>, sqlx::Error> {
    let row: Option<InboxRow> = sqlx::query_as("SELECT * FROM task_inbox WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(InboxItem::try_from).transpose()
}

/// Turn a pending `queue` item into one that steers. `false` when the item is
/// not a pending `queue` item in `task_id`: delivered already, sent to steer, or
/// never sent there.
pub(crate) async fn steer(pool: &DbPool, task_id: &str, id: Uuid) -> Result<bool, sqlx::Error> {
    let steered: Option<Uuid> = sqlx::query_scalar(
        r#"UPDATE task_inbox
              SET delivery_options = delivery_options - 'queue'
            WHERE task_id = $1 AND id = $2 AND status = 'pending'
              AND delivery_options ? 'queue'
        RETURNING id"#,
    )
    .bind(task_id)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(steered.is_some())
}

/// Cancel every pending item addressed to one of `task_ids`, and returns how
/// many it cancelled. An item a task sent to an inbox outside `task_ids` stays:
/// the caller is authorized for these tasks' inboxes only.
pub(crate) async fn dispose_pending(
    tx: &mut Transaction<'_, Postgres>,
    task_ids: &[String],
) -> Result<u64, sqlx::Error> {
    let inboxes: Vec<String> = sqlx::query_scalar(
        r#"UPDATE task_inbox
              SET status = 'cancelled'
            WHERE status = 'pending'
              AND task_id = ANY($1)
        RETURNING task_id"#,
    )
    .bind(task_ids)
    .fetch_all(&mut **tx)
    .await?;
    let mut settled = inboxes.clone();
    settled.sort();
    settled.dedup();
    for task_id in &settled {
        clear_settled_pending_work(tx, task_id).await?;
    }
    Ok(inboxes.len() as u64)
}

/// Marks the pending items among `ids`; delivered ones are left as they are.
/// Decides the turn's trigger when this is its first delivery.
pub(crate) async fn mark_delivered(
    tx: &mut Transaction<'_, Postgres>,
    task_id: &str,
    ids: &[Uuid],
    run: &RunStamp,
) -> Result<u64, sqlx::Error> {
    let marked: Vec<Uuid> = sqlx::query_scalar(
        r#"UPDATE task_inbox
              SET status = 'delivered', delivered_at = NOW(),
                  run_id = $3, session_id = $4, turn_number = $5
            WHERE task_id = $1 AND id = ANY($2) AND status = 'pending'
        RETURNING id"#,
    )
    .bind(task_id)
    .bind(ids)
    .bind(&run.run_id)
    .bind(&run.session_id)
    .bind(run.turn_number)
    .fetch_all(&mut **tx)
    .await?;
    // Only what this call moved: a redelivered id must not decide a turn.
    decide_trigger(tx, task_id, &marked, run.turn_number).await?;
    clear_settled_pending_work(tx, task_id).await?;
    Ok(marked.len() as u64)
}

/// Record on `turn_number` the Task whose item opened it: the earliest
/// non-`sandbox` item of its first delivery, `NULL` for a person or the
/// platform. Later deliveries leave it, so neither side takes over the other's
/// turn. A turn only `sandbox` items opened takes the previous turn's trigger,
/// since the turn that armed the wake owes the answer.
async fn decide_trigger(
    tx: &mut Transaction<'_, Postgres>,
    task_id: &str,
    ids: &[Uuid],
    turn_number: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"WITH delivered AS (
               SELECT source_task_id, origin, created_at, id
                 FROM task_inbox
                WHERE task_id = $1 AND id = ANY($2)
           ),
           opener AS (
               SELECT source_task_id FROM delivered
                WHERE origin <> 'sandbox'
                ORDER BY created_at, id LIMIT 1
           )
           UPDATE task_turns
              SET trigger_decided = true,
                  triggered_by_task_id = CASE
                      WHEN EXISTS (SELECT 1 FROM opener) THEN (SELECT source_task_id FROM opener)
                      ELSE (SELECT prev.triggered_by_task_id FROM task_turns prev
                             WHERE prev.task_id = $1 AND prev.turn_number < $3
                             ORDER BY prev.turn_number DESC LIMIT 1)
                  END
            WHERE task_id = $1 AND turn_number = $3 AND NOT trigger_decided
              AND EXISTS (SELECT 1 FROM delivered)"#,
    )
    .bind(task_id)
    .bind(ids)
    .bind(turn_number)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Drop the Sandbox's wake signal once nothing is waiting on it.
///
/// Runs after the rows it settles change, in their transaction; the condition
/// is exact, or it either drops a peer's pending work or wakes the Sandbox
/// forever.
pub(crate) async fn clear_settled_pending_work(
    tx: &mut Transaction<'_, Postgres>,
    task_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE sandboxes s
              SET pending_work_at = NULL, updated_at = NOW()
             FROM tasks t
            WHERE t.task_id = $1
              AND s.sandbox_id = t.sandbox_id
              AND s.pending_work_at IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1
                    FROM task_inbox i
                    JOIN tasks peer ON peer.task_id = i.task_id
                   WHERE peer.sandbox_id = s.sandbox_id
                     AND i.status = 'pending'
              )"#,
    )
    .bind(task_id)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

/// Every open Task sharing `task_id`'s Sandbox with a pending item — matches
/// `clear_settled_pending_work`, so a wake that survives always names someone.
pub(crate) async fn members_awaiting_dispatch(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT peer.task_id
             FROM tasks t
             JOIN tasks peer ON peer.sandbox_id = t.sandbox_id
            WHERE t.task_id = $1
              AND peer.status IN ('open', 'error')
              AND EXISTS (
                  SELECT 1 FROM task_inbox i
                   WHERE i.task_id = peer.task_id AND i.status = 'pending'
              )
            ORDER BY peer.created_at_db NULLS LAST, peer.task_id"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// Marks a seeded item delivered as history: stamped, with no trigger decided.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn seed_delivered(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    delivered_at: chrono::DateTime<chrono::Utc>,
    session_id: Option<&str>,
    turn_number: Option<i32>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE task_inbox
              SET status = 'delivered', delivered_at = $2, session_id = $3, turn_number = $4
            WHERE id = $1"#,
    )
    .bind(id)
    .bind(delivered_at)
    .bind(session_id)
    .bind(turn_number)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
