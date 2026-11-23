//! Until when a user's harness refuses turns: the circuit a Loop's Task is
//! routed around. See `crates/core/migrations/0250_harness_cooldowns.sql`.

use chrono::{DateTime, Duration, Utc};
use moatless_core::db::DbPool;
use moatless_types::agent::Harness;

use super::cooldown_dao;

/// How long a refusal that names no reset time keeps its harness cooling
/// down: one `rate_limit` or `provider_unavailable` already outlasted the
/// sandbox's retries.
pub const COOLDOWN_WITHOUT_RESET: Duration = Duration::minutes(15);

/// The cooldown rows, one per user and harness.
pub struct HarnessCooldowns {
    db: DbPool,
}

impl HarnessCooldowns {
    pub(crate) fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// Record that `harness` refused `user_id`'s turn for `reason`, cooling it
    /// down until `resets_at`, else for [`COOLDOWN_WITHOUT_RESET`] from `now`.
    /// The latest refusal wins, so a shorter reset replaces a longer one.
    pub async fn trip(
        &self,
        user_id: &str,
        harness: Harness,
        reason: &str,
        resets_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, sqlx::Error> {
        let until = cooldown_until(resets_at, now);
        cooldown_dao::upsert(&self.db, user_id, harness, until, reason, now).await?;
        Ok(until)
    }

    /// Until when `harness` cools down for `user_id`; `None` once it is past.
    pub async fn active_until(
        &self,
        user_id: &str,
        harness: Harness,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        cooldown_dao::active_until(&self.db, user_id, harness, now).await
    }
}

/// A reset already past counts as none: the provider refused just now.
fn cooldown_until(resets_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> DateTime<Utc> {
    resets_at
        .filter(|at| *at > now)
        .unwrap_or(now + COOLDOWN_WITHOUT_RESET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_cools_down_until_its_reset_else_for_the_default() {
        let now = Utc::now();
        let reset = now + Duration::hours(2);
        assert_eq!(cooldown_until(Some(reset), now), reset);
        assert_eq!(cooldown_until(None, now), now + COOLDOWN_WITHOUT_RESET);
        assert_eq!(
            cooldown_until(Some(now - Duration::seconds(1)), now),
            now + COOLDOWN_WITHOUT_RESET
        );
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "cooldown_db_tests.rs"]
mod cooldown_db_tests;
