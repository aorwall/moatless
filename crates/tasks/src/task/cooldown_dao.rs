//! The `harness_cooldowns` rows [`super::cooldown::HarnessCooldowns`] reads
//! and writes.

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;
use moatless_types::agent::Harness;

/// Set `user_id`'s cooldown on `harness`, replacing any earlier one.
pub(super) async fn upsert(
    pool: &DbPool,
    user_id: &str,
    harness: Harness,
    until: DateTime<Utc>,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO harness_cooldowns (user_id, harness, until, reason, updated_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (user_id, harness) DO UPDATE \
             SET until = EXCLUDED.until, reason = EXCLUDED.reason, \
                 updated_at = EXCLUDED.updated_at",
    )
    .bind(user_id)
    .bind(harness.as_str())
    .bind(until)
    .bind(reason)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// `user_id`'s cooldown on `harness`, when it ends after `now`.
pub(super) async fn active_until(
    pool: &DbPool,
    user_id: &str,
    harness: Harness,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT until FROM harness_cooldowns \
              WHERE user_id = $1 AND harness = $2 AND until > $3",
    )
    .bind(user_id)
    .bind(harness.as_str())
    .bind(now)
    .fetch_optional(pool)
    .await
}
