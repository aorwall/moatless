//! PostgreSQL tests for the harness cooldown rows.

use chrono::{Duration, Utc};
use moatless_types::agent::Harness;
use sqlx::PgPool;

use super::{COOLDOWN_WITHOUT_RESET, HarnessCooldowns};

#[sqlx::test]
async fn a_refusal_cools_its_harness_down_for_its_user_only(pool: PgPool) {
    let cooldowns = HarnessCooldowns::new(pool);
    let now = Utc::now();
    let reset = now + Duration::hours(1);

    cooldowns
        .trip("user-1", Harness::Acp, "usage_limit", Some(reset), now)
        .await
        .expect("trip");

    let until = cooldowns
        .active_until("user-1", Harness::Acp, now)
        .await
        .expect("read");
    assert_eq!(until.map(|u| u.timestamp()), Some(reset.timestamp()));
    for (user, harness) in [("user-2", Harness::Acp), ("user-1", Harness::ClaudeCode)] {
        let other = cooldowns
            .active_until(user, harness, now)
            .await
            .expect("read");
        assert_eq!(other, None, "{user} {harness}");
    }
    let after_reset = cooldowns
        .active_until("user-1", Harness::Acp, reset + Duration::seconds(1))
        .await
        .expect("read");
    assert_eq!(after_reset, None);
}

#[sqlx::test]
async fn the_latest_refusal_replaces_the_cooldown(pool: PgPool) {
    let cooldowns = HarnessCooldowns::new(pool);
    let now = Utc::now();
    cooldowns
        .trip(
            "user-1",
            Harness::Codex,
            "usage_limit",
            Some(now + Duration::hours(5)),
            now,
        )
        .await
        .expect("trip");

    let until = cooldowns
        .trip("user-1", Harness::Codex, "rate_limit", None, now)
        .await
        .expect("trip again");

    assert_eq!(until, now + COOLDOWN_WITHOUT_RESET);
    let read = cooldowns
        .active_until("user-1", Harness::Codex, now)
        .await
        .expect("read");
    assert_eq!(read.map(|u| u.timestamp()), Some(until.timestamp()));
}
