use std::time::Duration;

use sqlx::{PgConnection, PgPool};

use super::position::{EventChange, RETAINED_CHANGES};

/// Record `event` — the serialized envelope, verbatim — and queue it for
/// dispatch, returning the allocated position.
///
/// `None` when `event_id` is already recorded: `IngressRelay` re-publishing a
/// payload it could not confirm. Takes a transaction, not a pool, so the
/// record stays invisible until the writer commits.
pub async fn record_event(
    tx: &mut PgConnection,
    event_id: &str,
    task_id: &str,
    event: &str,
) -> Result<Option<i64>, sqlx::Error> {
    // `queued` is unreferenced on purpose: a data-modifying CTE runs whether or
    // not the outer query reads it, and reading it would drop the `recorded`
    // row on a dedup.
    //
    // `existing` reads the pre-statement snapshot, so it sees a row only on a
    // genuine dedup and never the one `recorded` just inserted. Re-arming
    // through it replays nothing to clients: no new position is allocated, and
    // both readers are keyed by position.
    //
    // Clearing `claimed_at` can cut a live claim short, letting a second
    // replica take events the first is still handling. Deliberate: the
    // handlers are idempotent and `RECLAIM_AFTER` already permits it, and a
    // relay re-publish is the case where the claimer is most likely dead.
    let position: Option<(i64,)> = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO task_event_position (id, position) VALUES (TRUE, 1)
               ON CONFLICT (id) DO UPDATE
                 SET position = task_event_position.position + 1
               RETURNING position
           ),
           recorded AS (
               INSERT INTO task_event_changes (position, event_id, task_id, event)
               SELECT position, $1, $2, $3 FROM allocated
               ON CONFLICT (event_id) DO NOTHING
               RETURNING position
           ),
           existing AS (
               SELECT position FROM task_event_changes WHERE event_id = $1
           ),
           queued AS (
               INSERT INTO task_event_dispatch_queue (position)
               SELECT position FROM recorded
               UNION ALL
               SELECT position FROM existing
               ON CONFLICT (position) DO UPDATE SET claimed_at = NULL
           )
           SELECT position FROM recorded"#,
    )
    .bind(event_id)
    .bind(task_id)
    .bind(event)
    .fetch_optional(tx)
    .await?;
    Ok(position.map(|(position,)| position))
}

/// The highest position this deployment has reached; 0 before the first event.
///
/// The counter is the authority, never `MAX(task_event_changes.position)`:
/// [`prune_changes`] bounds that window, so `MAX` goes stale on a deployment
/// older than it.
pub async fn current_position(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) =
        sqlx::query_as(r#"SELECT COALESCE((SELECT position FROM task_event_position), 0)"#)
            .fetch_one(pool)
            .await?;
    Ok(position)
}

/// Every event recorded after `after`, oldest first, capped at `limit`.
///
/// Safe as a polling cursor only because commits happen in position order, so
/// nothing can appear below a position already returned.
pub async fn changes_after(
    pool: &PgPool,
    after: i64,
    limit: i64,
) -> Result<Vec<EventChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT c.position, c.event
           FROM task_event_changes c
           WHERE c.position > $1
           ORDER BY c.position
           LIMIT $2"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Take up to `limit` events whose claim is free or aged past `reclaim_after`.
///
/// Commits before the caller runs any handler, which would otherwise sit
/// `idle in transaction` across Adapter HTTP. The caller must
/// [`release_dispatch`] what it finishes, or the claim ages out and another
/// replica takes it.
pub async fn claim_dispatch(
    pool: &PgPool,
    limit: i64,
    reclaim_after: Duration,
) -> Result<Vec<EventChange>, sqlx::Error> {
    // `SKIP LOCKED` on the inner select so two replicas claiming at once take
    // disjoint rows rather than one waiting on the other.
    sqlx::query_as(
        r#"WITH claimed AS (
               UPDATE task_event_dispatch_queue
               SET claimed_at = NOW()
               WHERE position IN (
                   SELECT position FROM task_event_dispatch_queue
                   WHERE claimed_at IS NULL
                      OR claimed_at < NOW() - make_interval(secs => $2)
                   ORDER BY position
                   FOR UPDATE SKIP LOCKED
                   LIMIT $1
               )
               RETURNING position
           )
           SELECT ch.position, ch.event
           FROM claimed cl
           JOIN task_event_changes ch ON ch.position = cl.position
           ORDER BY ch.position"#,
    )
    .bind(limit)
    .bind(reclaim_after.as_secs_f64())
    .fetch_all(pool)
    .await
}

/// Drop `positions` from the dispatch queue, marking them dispatched.
pub async fn release_dispatch(pool: &PgPool, positions: &[i64]) -> Result<(), sqlx::Error> {
    sqlx::query(r#"DELETE FROM task_event_dispatch_queue WHERE position = ANY($1)"#)
        .bind(positions)
        .execute(pool)
        .await?;
    Ok(())
}

/// How many events are waiting to be dispatched.
///
/// [`prune_changes`] skips whatever is queued, so a dispatcher that has stopped
/// draining grows both feed tables without bound. This is the number that says
/// so before the volume fills.
pub async fn dispatch_depth(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (depth,): (i64,) = sqlx::query_as(r#"SELECT COUNT(*) FROM task_event_dispatch_queue"#)
        .fetch_one(pool)
        .await?;
    Ok(depth)
}

/// Delete the events past the retention window; returns how many went.
///
/// Skips anything still queued, so pruning cannot be what loses an event its
/// handlers. Safe from every replica: two racing prunes hit the same rows and
/// the second finds none.
pub async fn prune_changes(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM task_event_changes c
           WHERE c.position <= (SELECT position FROM task_event_position) - $1
             AND NOT EXISTS (
                 SELECT 1 FROM task_event_dispatch_queue q WHERE q.position = c.position
             )"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
}
