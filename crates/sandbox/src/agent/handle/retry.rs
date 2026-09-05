//! Retry/backoff policy for coordinator-managed agent turns.

use std::time::SystemTime;

use crate::agent::harness::{ErrorClass, HarnessError};

// ── Retry constants ──────────────────────────────────────────────
/// Maximum retry attempts. `pub(crate)` so `CoordinatorService` can read it.
pub(crate) const MAX_RETRIES: u32 = 5;
const BACKOFF_BASE_SECS: f64 = 5.0;
const BACKOFF_CAP_SECS: f64 = 60.0;
const JITTER_FACTOR: f64 = 0.2;

/// Whether a terminal harness failure is transient and worth retrying.
/// The harness reports stable error classes, so retry policy is independent of
/// Claude/Codex native SDK error shapes and relies on the `MAX_RETRIES` cap.
pub(crate) fn is_retryable_harness_error(err: &HarnessError) -> bool {
    matches!(
        err.class,
        ErrorClass::RateLimit | ErrorClass::ProcessExit | ErrorClass::Connectivity
    )
}

/// Compute exponential backoff with jitter for a given attempt (0-indexed).
/// `pub(crate)` so `CoordinatorService` can reuse the same backoff curve.
pub(crate) fn compute_backoff(attempt: u32) -> std::time::Duration {
    let base = BACKOFF_BASE_SECS * 2.0_f64.powi(attempt as i32);
    let capped = base.min(BACKOFF_CAP_SECS);

    // Deterministic-ish jitter from system time nanos
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let jitter_norm = (nanos as f64 / u32::MAX as f64) * 2.0 - 1.0;
    let jitter = capped * JITTER_FACTOR * jitter_norm;

    let secs = (capped + jitter).max(1.0);
    std::time::Duration::from_secs_f64(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_harness_errors_are_rate_limit_process_exit_and_connectivity() {
        let retryable = HarnessError::new(
            crate::agent::harness::FailureStage::Execution,
            ErrorClass::RateLimit,
            crate::agent::harness::SessionImpact::Intact,
            "rate limited",
        );
        assert!(is_retryable_harness_error(&retryable));

        let retryable = HarnessError::new(
            crate::agent::harness::FailureStage::Execution,
            ErrorClass::ProcessExit,
            crate::agent::harness::SessionImpact::Unknown,
            "process exited",
        );
        assert!(is_retryable_harness_error(&retryable));

        // Pod egress recovering (DNS, NAT, proxy) is transient, so the turn is
        // worth re-running rather than failing the user's message outright.
        let retryable = HarnessError::new(
            crate::agent::harness::FailureStage::Execution,
            ErrorClass::Connectivity,
            crate::agent::harness::SessionImpact::Intact,
            "unable to connect to API",
        );
        assert!(is_retryable_harness_error(&retryable));
    }

    #[test]
    fn non_retryable_harness_errors_are_terminal() {
        let err = HarnessError::new(
            crate::agent::harness::FailureStage::Execution,
            ErrorClass::InvalidConfiguration,
            crate::agent::harness::SessionImpact::Unknown,
            "bad config",
        );
        assert!(!is_retryable_harness_error(&err));
    }
}
