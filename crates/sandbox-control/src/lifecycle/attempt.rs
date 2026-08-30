//! The reconciler's leader-local attempt cap and backoff.
//!
//! Deliberately **not** a durable budget: it resets on leader failover
//! (accepted looseness). It uses exponential backoff, retries only transient
//! error classes (runtime/infra), and never permanent ones (bad plan, Closed).
//! It resets when a new caller-requested operation generation is accepted or a
//! newly observed, causally eligible pending message requests a wake;
//! reconciler retries that **reuse the same generation** do not reset it.

use std::time::Duration;

use moatless_sandbox_runtime::types::OperationGeneration;

/// A single Task's leader-local retry budget.
#[derive(Debug, Clone)]
pub struct AttemptBudget {
    cap: u32,
    generation: Option<OperationGeneration>,
    attempts: u32,
}

impl AttemptBudget {
    pub fn new(cap: u32) -> Self {
        Self {
            cap,
            generation: None,
            attempts: 0,
        }
    }

    /// Begin (or continue) driving an operation for `generation`.
    ///
    /// A **new** generation resets the budget (a new caller-requested
    /// operation). The **same** generation continues it, so reconciler retries
    /// that reuse the generation keep consuming the same cap.
    pub fn begin(&mut self, generation: OperationGeneration) {
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.attempts = 0;
        }
    }

    /// Record a failed attempt.
    pub fn record_failure(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    /// A causally eligible pending message resets the budget so a wake is never
    /// starved by prior exhausted retries.
    pub fn reset_for_wake(&mut self) {
        self.attempts = 0;
    }

    /// Exhaust the budget outright — a permanent failure (bad plan, Closed) is
    /// never retried for this generation. A new generation (`begin`) or a wake
    /// (`reset_for_wake`) reopens it.
    pub fn block(&mut self) {
        self.attempts = self.cap;
    }

    /// Attempts recorded for the current generation. Test introspection only —
    /// the driver gates on [`exhausted`](Self::exhausted).
    #[cfg(test)]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Whether the cap is exhausted.
    pub fn exhausted(&self) -> bool {
        self.attempts >= self.cap
    }

    /// The backoff delay before the next attempt.
    pub fn backoff(&self, base: Duration, max: Duration) -> Duration {
        backoff(self.attempts, base, max)
    }
}

/// Exponential backoff: `base * 2^attempt`, capped at `max`, overflow-safe.
pub fn backoff(attempt: u32, base: Duration, max: Duration) -> Duration {
    // Cap the shift so `2^shift` stays within `u32` and cannot wrap; 31 already
    // saturates any realistic base against `max`. A larger factor that would
    // overflow `Duration` saturates to `max` via `checked_mul`.
    let shift = attempt.min(31);
    let factor = 1u32 << shift;
    let scaled = base.checked_mul(factor).unwrap_or(max);
    scaled.min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(n: i64) -> OperationGeneration {
        OperationGeneration(n)
    }

    #[test]
    fn retries_of_one_generation_consume_the_same_cap() {
        let mut b = AttemptBudget::new(3);
        b.begin(gen(1));
        assert_eq!(b.attempts(), 0);
        b.record_failure();
        b.begin(gen(1)); // same generation — does NOT reset
        b.record_failure();
        assert_eq!(b.attempts(), 2);
        assert!(!b.exhausted());
        b.record_failure();
        assert!(b.exhausted());
    }

    #[test]
    fn new_caller_generation_resets_the_cap() {
        let mut b = AttemptBudget::new(2);
        b.begin(gen(1));
        b.record_failure();
        b.record_failure();
        assert!(b.exhausted());
        // A new caller-requested generation resets.
        b.begin(gen(2));
        assert_eq!(b.attempts(), 0);
        assert!(!b.exhausted());
    }

    #[test]
    fn eligible_wake_resets_the_cap() {
        let mut b = AttemptBudget::new(2);
        b.begin(gen(1));
        b.record_failure();
        b.record_failure();
        assert!(b.exhausted());
        b.reset_for_wake();
        assert!(!b.exhausted());
        assert_eq!(b.attempts(), 0);
    }

    #[test]
    fn block_exhausts_until_a_new_generation_or_wake() {
        let mut b = AttemptBudget::new(5);
        b.begin(gen(1));
        b.block();
        assert!(b.exhausted());
        // A wake reopens it.
        b.reset_for_wake();
        assert!(!b.exhausted());
        // Re-block, then a new generation reopens it.
        b.block();
        assert!(b.exhausted());
        b.begin(gen(2));
        assert!(!b.exhausted());
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(30);
        assert_eq!(backoff(0, base, max), Duration::from_secs(1));
        assert_eq!(backoff(1, base, max), Duration::from_secs(2));
        assert_eq!(backoff(2, base, max), Duration::from_secs(4));
        assert_eq!(backoff(3, base, max), Duration::from_secs(8));
        assert_eq!(backoff(4, base, max), Duration::from_secs(16));
        // Capped.
        assert_eq!(backoff(5, base, max), Duration::from_secs(30));
        assert_eq!(backoff(100, base, max), Duration::from_secs(30));
    }

    #[test]
    fn budget_backoff_uses_attempt_count() {
        let mut b = AttemptBudget::new(5);
        b.begin(gen(1));
        b.record_failure();
        assert_eq!(
            b.backoff(Duration::from_secs(1), Duration::from_mins(1)),
            Duration::from_secs(2)
        );
    }
}
