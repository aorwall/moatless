//! The viewer's snooze, on their `task_follows` row. `snooze` and `unsnooze`
//! record a listing change only when the row moved.

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

/// See [`super::dao::TaskDao::snooze_task`].
pub(crate) async fn snooze(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    until: DateTime<Utc>,
    at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        r#"UPDATE task_follows
              SET snoozed_until = $3,
                  snoozed_at = $4
            WHERE user_id = $1 AND task_id = $2
              AND snoozed_until IS DISTINCT FROM $3"#,
    )
    .bind(user_id)
    .bind(task_id)
    .bind(until)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() > 0 {
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// See [`super::dao::TaskDao::unsnooze_task`].
pub(crate) async fn unsnooze(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // Clearing `snoozed_until` erases a timer wake, so it moves into
    // `snooze_woke_at`: `auto_settle` reads a wake from one or the other.
    let res = sqlx::query(
        r#"UPDATE task_follows
              SET snooze_woke_at = CASE WHEN snoozed_until IS NULL THEN snooze_woke_at
                                        ELSE LEAST(snoozed_until, $3) END,
                  snoozed_until = NULL,
                  snoozed_at = NULL
            WHERE user_id = $1 AND task_id = $2
              AND (snoozed_until IS NOT NULL OR snoozed_at IS NOT NULL)"#,
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

/// See [`super::dao::TaskDao::clear_snoozed_for_follower`].
pub(crate) async fn clear(pool: &DbPool, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE task_follows
              SET snoozed_until = NULL,
                  snoozed_at = NULL
            WHERE user_id = $1 AND task_id = $2
              AND (snoozed_until IS NOT NULL OR snoozed_at IS NOT NULL)"#,
    )
    .bind(user_id)
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}
