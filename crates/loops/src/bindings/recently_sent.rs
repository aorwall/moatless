//! Short-lived in-memory cache of `(task_id, external_event_id)` pairs we
//! have just sent outbound, so the inbound webhook for the same id can be
//! dropped before it reaches the per-row DB tag — closing the race where the
//! webhook arrives before the outbound `set_external_event_id` UPDATE
//! commits.
//!
//! Lossy by design: if the backend restarts between outbound and webhook the
//! DB tag still catches it, and the per-thread `should_filter_self_echo`
//! guard catches anything that slips past both.
//!
//! TTL is `120s` — comfortably above realistic webhook delivery latency and
//! well below realistic external-id reuse windows.
//!
//! A second, commit-scoped claim shape lives here for `git push`. A push
//! returns no event identifier, only the abbreviated sha it wrote — so the
//! claim is keyed on the commit and matched against the full sha the inbound
//! `pull_request.synchronize` webhook carries, by prefix rather than equality.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_mins(2);

/// Shortest abbreviated sha we will claim or match against. Git's default
/// abbreviation is 7 characters; anything shorter is treated as degenerate and
/// rejected, because a 1-2 character prefix would match most commits on the PR
/// rather than the one that was pushed.
const MIN_ABBREV_SHA: usize = 7;

/// Process-wide cache of recent outbound external ids.
#[derive(Debug, Default)]
pub struct RecentlySent {
    inner: Mutex<HashMap<String, Instant>>,
    /// Abbreviated commits a task claimed by pushing them, keyed by task id.
    /// A task can push several times inside one TTL window, so each task holds
    /// a list.
    commits: Mutex<HashMap<String, Vec<(String, Instant)>>>,
}

impl RecentlySent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that we just sent an outbound that produced `ext_id` for `task_id`.
    /// Subsequent inbound webhooks carrying the same id within the TTL window
    /// will be deduped via [`Self::contains`].
    pub fn note(&self, task_id: &str, ext_id: &str) {
        let key = key(task_id, ext_id);
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(key, Instant::now());
        }
    }

    /// Check whether `(task_id, ext_id)` was recorded within the TTL window.
    /// Sweeps expired entries opportunistically on each call.
    pub fn contains(&self, task_id: &str, ext_id: &str) -> bool {
        let key = key(task_id, ext_id);
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };

        let now = Instant::now();
        guard.retain(|_, ts| now.duration_since(*ts) < TTL);

        match guard.get(&key) {
            Some(ts) => now.duration_since(*ts) < TTL,
            None => false,
        }
    }

    /// Record that `task_id` just pushed `abbrev_sha`, as printed on the range
    /// line of `git push` output. Matched later by [`Self::contains_commit`].
    ///
    /// Returns `false` — recording nothing — for a sha that is empty, shorter
    /// than [`MIN_ABBREV_SHA`], or not hex: such a prefix would suppress
    /// commits the task never pushed.
    pub fn note_commit(&self, task_id: &str, abbrev_sha: &str) -> bool {
        if !is_claimable_sha(abbrev_sha) {
            return false;
        }
        let Ok(mut guard) = self.commits.lock() else {
            return false;
        };
        guard
            .entry(task_id.to_string())
            .or_default()
            .push((abbrev_sha.to_ascii_lowercase(), Instant::now()));
        true
    }

    /// Check whether `full_sha` begins with a commit `task_id` claimed within
    /// the TTL window. Sweeps expired claims opportunistically on each call.
    pub fn contains_commit(&self, task_id: &str, full_sha: &str) -> bool {
        if !is_claimable_sha(full_sha) {
            return false;
        }
        let Ok(mut guard) = self.commits.lock() else {
            return false;
        };

        let now = Instant::now();
        for claims in guard.values_mut() {
            claims.retain(|(_, ts)| now.duration_since(*ts) < TTL);
        }
        guard.retain(|_, claims| !claims.is_empty());

        let full_sha = full_sha.to_ascii_lowercase();
        guard.get(task_id).is_some_and(|claims| {
            claims
                .iter()
                .any(|(claimed, _)| full_sha.starts_with(claimed.as_str()))
        })
    }
}

/// A sha we are willing to treat as an identity: hex, and long enough that its
/// prefix names one commit rather than a family of them.
fn is_claimable_sha(sha: &str) -> bool {
    sha.len() >= MIN_ABBREV_SHA && sha.len() <= 40 && sha.chars().all(|c| c.is_ascii_hexdigit())
}

fn key(task_id: &str, ext_id: &str) -> String {
    format!("{task_id}::{ext_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_then_contains_within_ttl() {
        let cache = RecentlySent::new();
        cache.note("task-1", "github:ic:42");
        assert!(cache.contains("task-1", "github:ic:42"));
    }

    #[test]
    fn contains_misses_for_other_task() {
        let cache = RecentlySent::new();
        cache.note("task-1", "github:ic:42");
        assert!(!cache.contains("task-2", "github:ic:42"));
        assert!(!cache.contains("task-1", "github:ic:43"));
    }

    #[test]
    fn manual_sweep_drops_expired() {
        // We can't reliably advance time in unit tests without injecting a
        // clock. Instead, smoke-test the sweep path by inserting then forcing
        // a stale timestamp.
        let cache = RecentlySent::new();
        cache.note("task-1", "github:ic:42");
        {
            let mut guard = cache.inner.lock().unwrap();
            let key = key("task-1", "github:ic:42");
            let stale = Instant::now()
                .checked_sub(TTL + Duration::from_secs(1))
                .expect("the process has been up longer than the TTL");
            guard.insert(key, stale);
        }
        assert!(!cache.contains("task-1", "github:ic:42"));
    }

    const FULL_SHA: &str = "7fc0612a1b2c3d4e5f60718293a4b5c6d7e8f901";
    const OTHER_SHA: &str = "deadbee1a1b2c3d4e5f60718293a4b5c6d7e8f90";

    #[test]
    fn commit_claim_matches_full_sha_by_prefix() {
        let cache = RecentlySent::new();
        assert!(cache.note_commit("task-1", "7fc0612"));
        assert!(cache.contains_commit("task-1", FULL_SHA));
    }

    #[test]
    fn commit_claim_is_case_insensitive() {
        let cache = RecentlySent::new();
        assert!(cache.note_commit("task-1", "7FC0612"));
        assert!(cache.contains_commit("task-1", &FULL_SHA.to_ascii_uppercase()));
    }

    #[test]
    fn commit_claim_is_task_scoped_and_commit_scoped() {
        let cache = RecentlySent::new();
        cache.note_commit("task-1", "7fc0612");
        assert!(!cache.contains_commit("task-2", FULL_SHA));
        assert!(!cache.contains_commit("task-1", OTHER_SHA));
    }

    #[test]
    fn commit_claim_keeps_every_push_in_the_window() {
        let cache = RecentlySent::new();
        cache.note_commit("task-1", "7fc0612");
        cache.note_commit("task-1", "deadbee");
        assert!(cache.contains_commit("task-1", FULL_SHA));
        assert!(cache.contains_commit("task-1", OTHER_SHA));
    }

    #[test]
    fn degenerate_commit_claim_is_rejected_and_never_matches() {
        let cache = RecentlySent::new();
        // Empty, too short to name one commit, and non-hex are all refused at
        // the claim side...
        assert!(!cache.note_commit("task-1", ""));
        assert!(!cache.note_commit("task-1", "7fc061"));
        assert!(!cache.note_commit("task-1", "not-a-sha"));
        assert!(!cache.contains_commit("task-1", FULL_SHA));
        // ...and a degenerate lookup matches nothing either.
        cache.note_commit("task-1", "7fc0612");
        assert!(!cache.contains_commit("task-1", ""));
        assert!(!cache.contains_commit("task-1", "7fc061"));
    }

    #[test]
    fn commit_claim_sweep_drops_expired() {
        let cache = RecentlySent::new();
        cache.note_commit("task-1", "7fc0612");
        {
            let mut guard = cache.commits.lock().unwrap();
            let stale = Instant::now()
                .checked_sub(TTL + Duration::from_secs(1))
                .expect("the process has been up longer than the TTL");
            guard.insert("task-1".to_string(), vec![("7fc0612".to_string(), stale)]);
        }
        assert!(!cache.contains_commit("task-1", FULL_SHA));
    }
}
