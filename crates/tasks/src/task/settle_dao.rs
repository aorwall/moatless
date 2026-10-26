//! Settling, by a person and by the reaper, on `task_follows` and the task's
//! own `auto_settled_at`. Each write records a listing change only when a row
//! moved.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

/// See [`super::dao::TaskDao::settle_task`].
pub(crate) async fn settle(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        // Settling is "I'm done with this", so it also drops the pin and
        // both arranged slots, as T3's own server does.
        r#"UPDATE task_follows
              SET settled_at = CASE WHEN settled_override = 'settled'
                                    THEN settled_at ELSE $3 END,
                  settled_override = 'settled',
                  pinned_at = NULL,
                  pin_order_key = NULL,
                  active_order_key = NULL
            WHERE user_id = $1 AND task_id = $2
              AND (settled_override IS DISTINCT FROM 'settled'
                   OR pinned_at IS NOT NULL
                   OR pin_order_key IS NOT NULL
                   OR active_order_key IS NOT NULL)"#,
    )
    .bind(user_id)
    .bind(task_id)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() > 0 {
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// See [`super::dao::TaskDao::unsettle_task`].
pub(crate) async fn unsettle(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // Drop the reaper's stamp too: the task is no longer settled for everyone.
    // An idle task gets it back next cycle, but stays unsettled while this
    // follower is `active`.
    sqlx::query(
        r#"UPDATE tasks SET auto_settled_at = NULL
            WHERE task_id = $1 AND auto_settled_at IS NOT NULL"#,
    )
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    let res = sqlx::query(
        r#"UPDATE task_follows
              SET settled_override = 'active',
                  settled_at = NULL
            WHERE user_id = $1 AND task_id = $2
              AND (settled_override IS DISTINCT FROM 'active' OR settled_at IS NOT NULL)"#,
    )
    .bind(user_id)
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() > 0 {
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// See [`super::dao::TaskDao::set_auto_settle`].
pub(crate) async fn set_auto_settle(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    enabled: bool,
    at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // A repeated opt-out keeps the first stamp, as T3's own server does.
    let res = sqlx::query(
        r#"UPDATE task_follows
              SET auto_settle_disabled_at = CASE WHEN $3 THEN NULL
                                                 ELSE COALESCE(auto_settle_disabled_at, $4) END
            WHERE user_id = $1 AND task_id = $2
              AND (auto_settle_disabled_at IS NULL) <> $3"#,
    )
    .bind(user_id)
    .bind(task_id)
    .bind(enabled)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() > 0 {
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// See [`super::dao::TaskDao::auto_settle_task`].
pub(crate) async fn auto_settle(
    pool: &DbPool,
    task_id: &str,
    at: DateTime<Utc>,
    idle_since: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // No `settled_override = 'active'` row here: that person unsettled it by
    // hand, and only new activity, which resets it to NULL, overrules them.
    // A snooze that woke after `idle_since`, on its timer or by hand, counts as
    // activity, as in T3's own server; skipping only a sleeping one would
    // settle it the cycle it wakes.
    let followers = sqlx::query(
        r#"UPDATE task_follows
              SET settled_override = 'settled',
                  settled_at = $2,
                  active_order_key = NULL
            WHERE task_id = $1
              AND settled_override IS NULL
              AND pinned_at IS NULL
              AND auto_settle_disabled_at IS NULL
              AND (snoozed_until IS NULL OR snoozed_until <= $3)
              AND (snooze_woke_at IS NULL OR snooze_woke_at <= $3)"#,
    )
    .bind(task_id)
    .bind(at)
    .bind(idle_since)
    .execute(&mut *tx)
    .await?;
    let task = sqlx::query(
        r#"UPDATE tasks SET auto_settled_at = $2
            WHERE task_id = $1 AND auto_settled_at IS NULL"#,
    )
    .bind(task_id)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    let moved = followers.rows_affected() > 0 || task.rows_affected() > 0;
    if moved {
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    }
    tx.commit().await?;
    Ok(moved)
}

/// See [`super::dao::TaskDao::get_settled_since_map`].
pub(crate) async fn settled_since(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // `GREATEST` skips NULLs, so a task nobody follows answers with its own
    // `auto_settled_at`, and one the reaper never settled with its followers'.
    let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT t.task_id, GREATEST(t.auto_settled_at, MAX(f.settled_at))
             FROM tasks t
             LEFT JOIN task_follows f ON f.task_id = t.task_id
            WHERE t.task_id = ANY($1)
            GROUP BY t.task_id, t.auto_settled_at
           HAVING COUNT(f.user_id)
                    = COUNT(f.user_id) FILTER (WHERE f.settled_override = 'settled')
              AND GREATEST(t.auto_settled_at, MAX(f.settled_at)) IS NOT NULL"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().collect())
}

/// Clear `settled_override`/`settled_at` for every follower of `task_id`, and
/// the task's own `auto_settled_at`.
pub(crate) async fn clear_settled_for_all_followers(
    tx: &mut sqlx::PgConnection,
    task_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE tasks SET auto_settled_at = NULL
            WHERE task_id = $1 AND auto_settled_at IS NOT NULL"#,
    )
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE task_follows
              SET settled_override = NULL,
                  settled_at = NULL
            WHERE task_id = $1
              AND (settled_override IS NOT NULL OR settled_at IS NOT NULL)"#,
    )
    .bind(task_id)
    .execute(tx)
    .await?;
    Ok(())
}
