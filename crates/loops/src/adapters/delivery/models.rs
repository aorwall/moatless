//! Tuning for durable Adapter delivery: the store and worker in this crate read
//! it, nothing outside them does. The contract itself is the Adapter kernel's
//! (`moatless_adapter_model::delivery`).

use std::time::Duration;

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
    /// How often the worker looks for due records, and therefore the delay a
    /// caller trades away by admitting instead of spawning. Tuned for
    /// interactive work; to go faster, wake the worker on admission rather than
    /// spinning the poll.
    pub poll_interval: Duration,
    /// How often the worker recovers stale claims, refreshes backlog gauges,
    /// and prunes finished records.
    pub maintenance_interval: Duration,
    /// Unfinished (`pending` + `claimed`) records past which admission is
    /// refused.
    ///
    /// A stable ceiling, not an exact cutoff: the predicate rides inside the
    /// admission `INSERT` and reads that statement's own snapshot, so concurrent
    /// admissions can overshoot by the number in flight. It does not ratchet —
    /// once the committed count reaches capacity, every later snapshot refuses.
    pub backlog_capacity: i64,
    /// How long succeeded records are kept. Deduplication reads these rows, so
    /// this must exceed every sender's *automatic* retry schedule. It does not
    /// cover manual redelivery: a delivery replayed by hand after this window is
    /// admitted as new, not recognised as a duplicate.
    pub succeeded_retention: Duration,
    /// How long terminal failures are kept for operators.
    pub failed_retention: Duration,
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
        }
    }
}

impl AdapterDeliveryPolicy {
    /// Delay before the next attempt after `attempts` have failed.
    ///
    /// Exponential from `base_backoff`, capped at `max_backoff`. Deterministic:
    /// claims use `SKIP LOCKED`, so replicas do not converge on one record and
    /// jitter buys nothing.
    pub(crate) fn backoff(&self, attempts: i32) -> Duration {
        let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
        self.base_backoff
            .saturating_mul(1u32 << doublings)
            .min(self.max_backoff)
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
    fn execution_deadline_stays_inside_the_claim_lease() {
        let policy = AdapterDeliveryPolicy::default();
        assert!(
            policy.execution_timeout < policy.claim_lease,
            "a handler must not still be running when its record is reclaimed"
        );
    }
}
