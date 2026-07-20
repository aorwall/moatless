//! Per-username login throttle.
//!
//! Keyed on the lowercased username so `Admin` and `admin` share a counter.

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;

/// Failed attempts tolerated before a username is locked out.
const MAX_FAILURES: u32 = 10;
/// How long the counter survives without a new failure.
const WINDOW_SECS: u64 = 15 * 60;

/// Tracks consecutive failed password attempts per username.
#[derive(Clone)]
pub struct LoginThrottle {
    failures: Arc<Cache<String, u32>>,
}

impl LoginThrottle {
    pub fn new() -> Self {
        Self {
            failures: Arc::new(
                Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(Duration::from_secs(WINDOW_SECS))
                    .build(),
            ),
        }
    }

    fn key(username: &str) -> String {
        username.to_lowercase()
    }

    /// Whether this username has exhausted its attempts.
    pub async fn is_locked(&self, username: &str) -> bool {
        self.failures
            .get(&Self::key(username))
            .await
            .is_some_and(|n| n >= MAX_FAILURES)
    }

    /// Record a failed attempt.
    pub async fn record_failure(&self, username: &str) {
        let key = Self::key(username);
        let next = self.failures.get(&key).await.unwrap_or(0) + 1;
        self.failures.insert(key, next).await;
    }

    /// Clear the counter after a successful login.
    pub async fn clear(&self, username: &str) {
        self.failures.invalidate(&Self::key(username)).await;
    }
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn locks_after_max_failures() {
        let throttle = LoginThrottle::new();
        for _ in 0..MAX_FAILURES - 1 {
            throttle.record_failure("alice").await;
        }
        assert!(!throttle.is_locked("alice").await);
        throttle.record_failure("alice").await;
        assert!(throttle.is_locked("alice").await);
    }

    #[tokio::test]
    async fn counter_is_case_insensitive() {
        let throttle = LoginThrottle::new();
        for _ in 0..MAX_FAILURES {
            throttle.record_failure("ALICE").await;
        }
        assert!(throttle.is_locked("alice").await);
    }

    #[tokio::test]
    async fn clear_resets_the_counter() {
        let throttle = LoginThrottle::new();
        for _ in 0..MAX_FAILURES {
            throttle.record_failure("alice").await;
        }
        throttle.clear("Alice").await;
        assert!(!throttle.is_locked("alice").await);
    }

    #[tokio::test]
    async fn usernames_are_tracked_independently() {
        let throttle = LoginThrottle::new();
        for _ in 0..MAX_FAILURES {
            throttle.record_failure("alice").await;
        }
        assert!(throttle.is_locked("alice").await);
        assert!(!throttle.is_locked("bob").await);
    }
}
