//! The admission, retry, capacity, and retention policy for durable Adapter
//! delivery.
//!
//! The contract itself — the request, the record, the outcome, and the failure
//! classification — is the Adapter kernel's
//! (`moatless_adapter_model::delivery`). What is here is the tuning the store
//! and the worker in this crate read, and nothing outside them does.

use std::time::Duration;

/// The admission, retry, capacity, and retention policy for Adapter delivery.
///
/// Every value here is a decision the seam owes its callers, so they are named
/// and defaulted in one place rather than scattered as literals. Tests override
/// the timing fields to exercise retry and recovery without waiting.
#[derive(Debug, Clone, Copy)]
pub struct AdapterDeliveryPolicy {
    /// Attempts a record gets before it goes terminal. "Retry forever" is not a
    /// policy: eight attempts on the backoff below spans roughly ten minutes,
    /// which covers a sender or dependency blip without hiding a real bug.
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
    /// caller trades away by admitting instead of spawning.
    ///
    /// Tuned for interactive work: the first callers of this seam route inbound
    /// chat messages, where a person is waiting on the reply, so the scheduling
    /// delay is kept well below human-noticeable rather than amortised for
    /// throughput. The cost is one indexed claim query per replica per interval,
    /// which returns no rows and touches no heap pages when the backlog is
    /// empty. If a caller ever measures even this as too slow, the fix is to
    /// wake the worker on admission — not to spin the poll faster.
    pub poll_interval: Duration,
    /// How often the worker recovers stale claims, refreshes backlog gauges,
    /// and prunes finished records.
    pub maintenance_interval: Duration,
    /// Unfinished (`pending` + `claimed`) records past which admission is
    /// refused rather than accepted and forgotten.
    ///
    /// This is a ceiling, not an exact cutoff, and the difference is worth
    /// stating precisely rather than implying a hard cutoff the check does not
    /// deliver. The capacity predicate rides inside the admission `INSERT` and
    /// is evaluated against that statement's own READ COMMITTED snapshot, so
    /// admissions running concurrently can each see the same below-capacity
    /// backlog and all insert. The committed backlog can therefore reach
    /// `backlog_capacity + C - 1`, where `C` is the number of admission
    /// statements in flight at once — itself bounded by the connection pools
    /// doing the admitting (`DATABASE_MAX_CONNECTIONS`, default 5, times the
    /// replica count). At the default capacity that is a fraction of a percent.
    ///
    /// Crucially the overshoot does not ratchet: once the *committed* count
    /// reaches the capacity, every statement whose snapshot is taken afterwards
    /// refuses. The ceiling is stable, so ingress stays bounded — which is the
    /// property the bound exists for.
    ///
    /// Making the cutoff exact would mean serialising the decision (a
    /// transaction-scoped advisory lock, or an admission-token row), because
    /// Postgres cannot serialise a count over concurrent inserters otherwise.
    /// That was rejected: it puts a single global lock, and two extra round
    /// trips, on the hot path of *all* webhook ingress — on the sender
    /// response deadline this seam exists to protect — in exchange for a
    /// handful of records at the boundary. A load-shedding guard should not be
    /// the ingress path's bottleneck.
    pub backlog_capacity: i64,
    /// How long succeeded records are kept. Deduplication reads these rows, so
    /// this must comfortably exceed every sender's *automatic* retry
    /// schedule, which it does.
    ///
    /// It deliberately does not cover manual redelivery: GitHub keeps a
    /// delivery redeliverable for 30 days, so one replayed by hand after this
    /// window is admitted as a new delivery rather than recognised as a duplicate.
    /// That is the useful behaviour — an operator replaying a delivery by hand
    /// is asking for it to run again — but it is a narrower guarantee than
    /// "never runs twice", and worth knowing before relying on the latter.
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
    /// Exponential from `base_backoff`, capped at `max_backoff`. Deterministic
    /// on purpose: claims are handed out with `SKIP LOCKED`, so replicas do not
    /// converge on the same record and jitter would only make the retry
    /// schedule harder to reason about in tests and in incident timelines.
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
