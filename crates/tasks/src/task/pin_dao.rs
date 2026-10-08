//! The viewer's pin and manual sidebar order, on their `task_follows` row.
//! Each write records a listing change only when the row moved.

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

/// Which of the viewer's two arranged blocks an order key places a Task in.
#[derive(Debug, Clone, Copy)]
pub(crate) enum OrderSlot {
    Pinned,
    Active,
}

/// See [`super::dao::TaskDao::pin_task`].
pub(crate) async fn pin(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    order_key: Option<String>,
    at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        r#"UPDATE task_follows
              SET pin_order_key = CASE WHEN pinned_at IS NULL
                                       THEN COALESCE($3, pin_order_key)
                                       ELSE pin_order_key END,
                  pinned_at = COALESCE(pinned_at, $4),
                  settled_override = CASE WHEN settled_override = 'settled'
                                          THEN 'active' ELSE settled_override END,
                  settled_at = CASE WHEN settled_override = 'settled'
                                    THEN NULL ELSE settled_at END,
                  snoozed_until = NULL,
                  snoozed_at = NULL
            WHERE user_id = $1 AND task_id = $2
              AND (pinned_at IS NULL
                   OR settled_override = 'settled'
                   OR snoozed_until IS NOT NULL
                   OR snoozed_at IS NOT NULL)"#,
    )
    .bind(user_id)
    .bind(task_id)
    .bind(order_key)
    .bind(at)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() > 0 {
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// See [`super::dao::TaskDao::unpin_task`].
pub(crate) async fn unpin(pool: &DbPool, user_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        r#"UPDATE task_follows
              SET pinned_at = NULL,
                  pin_order_key = NULL
            WHERE user_id = $1 AND task_id = $2
              AND (pinned_at IS NOT NULL OR pin_order_key IS NOT NULL)"#,
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

/// Write the key one block sorts by; a key equal to the stored one writes
/// nothing.
pub(crate) async fn set_order_key(
    pool: &DbPool,
    user_id: &str,
    task_id: &str,
    slot: OrderSlot,
    order_key: &str,
) -> Result<(), sqlx::Error> {
    let sql = match slot {
        OrderSlot::Pinned => {
            r#"UPDATE task_follows
                  SET pin_order_key = $3
                WHERE user_id = $1 AND task_id = $2
                  AND pin_order_key IS DISTINCT FROM $3"#
        }
        OrderSlot::Active => {
            r#"UPDATE task_follows
                  SET active_order_key = $3
                WHERE user_id = $1 AND task_id = $2
                  AND active_order_key IS DISTINCT FROM $3"#
        }
    };
    let mut tx = pool.begin().await?;
    let res = sqlx::query(sql)
        .bind(user_id)
        .bind(task_id)
        .bind(order_key)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() > 0 {
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
    }
    tx.commit().await?;
    Ok(())
}
