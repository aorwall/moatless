//! Tuning for durable Adapter delivery: the store and worker in this crate read
//! it, nothing outside them does. The contract itself is the Adapter kernel's
//! (`moatless_adapter_model::delivery`).

use std::time::Duration;

use chrono::{DateTime, Utc};

/// The admission, retry, capacity, and retention policy for Adapter delivery.
///
/// Tests override the timing fields to exercise retry and recovery without
/// waiting.
#[derive(Debug, Clone, Copy)]
pub struct AdapterDeliveryPolicy {
    /// Attempts a record gets before it goes terminal.
    pub max_attempts: i32,
    /// First retry delay; each further attempt doubles it.
    pub base_backoff: Duration,
    /// Ceiling on the doubling, so a long-failing record still retries at a
    /// predictable rate.
    pub max_backoff: Duration,
    /// How long a claim is honoured. A worker that dies mid-execution leaves a
    /// claimed record; after this lease any worker may take it back.
    pub claim_lease: Duration,
    /// Per-record execution deadline. Strictly shorter than `claim_lease` so a
    /// slow handler cannot still be running when its record is reclaimed.
    pub execution_timeout: Duration,
    /// Records claimed per poll, and therefore the worker's concurrency bound.
    pub batch_size: i64,
    /// How often the worker looks for due records — the delay a caller trades
    /// away by admitting instead of spawning. Tuned for interactive work; to go
    /// faster, wake the worker on admission instead of spinning the poll.
    pub poll_interval: Duration,
    /// How often the worker recovers stale claims, refreshes backlog gauges,
    /// and prunes finished records.
    pub maintenance_interval: Duration,
    /// Unfinished (`pending` + `claimed`) records past which admission is
    /// refused. Not an exact cutoff — concurrent admissions share one snapshot
    /// and can overshoot by the number in flight, and it never ratchets down.
    pub backlog_capacity: i64,
    /// How long succeeded records are kept. Deduplication reads these rows, so
    /// this must exceed every sender's *automatic* retry schedule — a manual
    /// redelivery after this window is admitted as new, not deduplicated.
    pub succeeded_retention: Duration,
    /// How long terminal failures are kept for operators.
    pub failed_retention: Duration,
    /// Delay after a quota refusal that named no reset time. GitHub asks for
    /// at least a minute after a secondary limit without `retry-after`.
    pub throttle_fallback: Duration,
    /// Ceiling on a provider-named reset, so a skewed or hostile header cannot
    /// park a record for days. GitHub's primary window is one hour.
    pub max_throttle_delay: Duration,
    /// Age past which a quota refusal spends an attempt like any retryable
    /// failure, so a quota that never returns still ends the record.
    pub throttle_deadline: Duration,
}

impl Default for AdapterDeliveryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            base_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_mins(5),
            claim_lease: Duration::from_mins(5),
            execution_timeout: Duration::from_mins(2),
            batch_size: 8,
            poll_interval: Duration::from_millis(250),
            maintenance_interval: Duration::from_secs(30),
            backlog_capacity: 10_000,
            succeeded_retention: Duration::from_hours(24 * 7),
            failed_retention: Duration::from_hours(24 * 30),
            throttle_fallback: Duration::from_mins(1),
            max_throttle_delay: Duration::from_mins(65),
            throttle_deadline: Duration::from_hours(24),
        }
    }
}

impl AdapterDeliveryPolicy {
    /// Delay before the next attempt after `attempts` have failed.
    ///
    /// Exponential from `base_backoff`, capped at `max_backoff`; no jitter,
    /// since claims use `SKIP LOCKED` and replicas never converge on one
    /// record.
    pub(crate) fn backoff(&self, attempts: i32) -> Duration {
        let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
        self.base_backoff
            .saturating_mul(1u32 << doublings)
            .min(self.max_backoff)
    }

    /// When a throttled record is next due: the provider's `not_before`, or
    /// `throttle_fallback` without one, held between `base_backoff` and
    /// `max_throttle_delay` from `now`.
    pub(crate) fn throttled_retry_at(
        &self,
        now: DateTime<Utc>,
        not_before: Option<DateTime<Utc>>,
    ) -> DateTime<Utc> {
        let after = |delay: Duration| {
            now + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::zero())
        };
        // `max` then `min`, not `clamp`: `clamp` panics when an overridden
        // `base_backoff` exceeds `max_throttle_delay`.
        not_before
            .unwrap_or_else(|| after(self.throttle_fallback))
            .max(after(self.base_backoff))
            .min(after(self.max_throttle_delay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_the_base_and_stops_at_the_ceiling() {
        let policy = AdapterDeliveryPolicy::default();
        assert_eq!(policy.backoff(1), Duration::from_secs(5));
        assert_eq!(policy.backoff(2), Duration::from_secs(10));
        assert_eq!(policy.backoff(3), Duration::from_secs(20));
        assert_eq!(policy.backoff(8), policy.max_backoff);
        // Far past the attempt limit the doubling must not overflow.
        assert_eq!(policy.backoff(i32::MAX), policy.max_backoff);
        // A zeroth attempt is not reachable, but must not panic either.
        assert_eq!(policy.backoff(0), policy.base_backoff);
    }

    #[test]
    fn a_throttled_retry_honours_the_reset_inside_its_bounds() {
        let policy = AdapterDeliveryPolicy::default();
        let now = DateTime::from_timestamp(1_793_000_000, 0).unwrap();
        let reset = now + chrono::Duration::minutes(40);
        assert_eq!(policy.throttled_retry_at(now, Some(reset)), reset);
        assert_eq!(
            policy.throttled_retry_at(now, None),
            now + chrono::Duration::minutes(1)
        );
        // A reset already past, or days away, is held to the bounds.
        assert_eq!(
            policy.throttled_retry_at(now, Some(now - chrono::Duration::minutes(5))),
            now + chrono::Duration::seconds(5)
        );
        assert_eq!(
            policy.throttled_retry_at(now, Some(now + chrono::Duration::days(3))),
            now + chrono::Duration::minutes(65)
        );
    }

    #[test]
    fn execution_deadline_stays_inside_the_claim_lease() {
        let policy = AdapterDeliveryPolicy::default();
        assert!(
            policy.execution_timeout < policy.claim_lease,
            "a handler must not still be running when its record is reclaimed"
        );
    }
}
