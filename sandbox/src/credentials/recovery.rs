//! Stderr-pattern detector for credential failures.
//!
//! Codex CLI surfaces a refresh-token race as `refresh_token_reused` on
//! stderr. The detector scans every stderr line and flips a per-credential
//! atomic flag when a known marker appears. The agent loop polls the flag at
//! turn-failure boundaries and triggers a credential pull + retry.
//!
//! Designed to be generic so adding a new credential is a one-entry change to
//! [`CredentialFailureDetector::default_patterns`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Per-credential marker substrings. Lower-case match — both sides are
/// lower-cased before comparison.
type Patterns = HashMap<String, Vec<&'static str>>;

/// Detector + per-credential triggered flags. Cheap to clone (Arc inside).
#[derive(Clone)]
pub struct CredentialFailureDetector {
    patterns: Arc<Patterns>,
    triggered: Arc<HashMap<String, Arc<AtomicBool>>>,
}

impl Default for CredentialFailureDetector {
    fn default() -> Self {
        Self::new(Self::default_patterns())
    }
}

impl CredentialFailureDetector {
    pub fn new(patterns: Patterns) -> Self {
        let triggered = patterns
            .keys()
            .map(|k| (k.clone(), Arc::new(AtomicBool::new(false))))
            .collect();
        Self {
            patterns: Arc::new(patterns),
            triggered: Arc::new(triggered),
        }
    }

    /// Default patterns. Add new credentials here as detection markers are
    /// identified upstream.
    pub fn default_patterns() -> Patterns {
        let mut p: Patterns = HashMap::new();
        p.insert(
            "CODEX_AUTH_JSON".to_string(),
            vec![
                "refresh_token_reused",
                "refresh token reused",
                "failed to refresh token",
                "401 unauthorized",
            ],
        );
        p
    }

    /// Scan a single stderr line. Flips any matching credential's flag.
    pub fn scan(&self, line: &str) {
        let lower = line.to_ascii_lowercase();
        for (key, markers) in self.patterns.iter() {
            if let Some(flag) = self.triggered.get(key) {
                if markers.iter().any(|m| lower.contains(m)) {
                    flag.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    /// Test-and-clear the flag for a credential. Returns `true` if it was set.
    pub fn take(&self, key: &str) -> bool {
        match self.triggered.get(key) {
            Some(flag) => flag.swap(false, Ordering::SeqCst),
            None => false,
        }
    }

    /// Return the keys whose flags are currently set, clearing them in the
    /// process.
    pub fn take_all(&self) -> Vec<String> {
        self.triggered
            .iter()
            .filter(|(_, flag)| flag.swap(false, Ordering::SeqCst))
            .map(|(k, _)| k.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flips_on_known_marker() {
        let d = CredentialFailureDetector::default();
        d.scan("ERROR: refresh_token_reused; cannot continue");
        assert!(d.take("CODEX_AUTH_JSON"));
        // Subsequent take returns false — flag was cleared.
        assert!(!d.take("CODEX_AUTH_JSON"));
    }

    #[test]
    fn ignores_unrelated_lines() {
        let d = CredentialFailureDetector::default();
        d.scan("INFO: turn started");
        assert!(!d.take("CODEX_AUTH_JSON"));
    }

    #[test]
    fn case_insensitive() {
        let d = CredentialFailureDetector::default();
        d.scan("Refresh_Token_Reused detected");
        assert!(d.take("CODEX_AUTH_JSON"));
    }

    #[test]
    fn take_all_returns_set_keys() {
        let d = CredentialFailureDetector::default();
        d.scan("Failed to refresh token");
        let triggered = d.take_all();
        assert_eq!(triggered, vec!["CODEX_AUTH_JSON".to_string()]);
        // Cleared.
        assert!(d.take_all().is_empty());
    }
}
