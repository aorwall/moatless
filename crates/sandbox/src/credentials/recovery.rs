//! Per-credential failure flags.
//!
//! The Codex harness recognises an expired credential from the named
//! `codexErrorInfo` on the failed Turn and flips this flag; the agent loop
//! polls it at turn-failure boundaries and triggers a credential pull + retry.
//! Only typed, turn-scoped errors set it, never scanned background stderr.
//!
//! Adding a new credential is a one-entry change to [`KNOWN_CREDENTIALS`].

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The Codex credential blob, written to `$CODEX_HOME/auth.json` by
/// `CodexAuthJsonSink`. The only credential with a failure signal today.
pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";

/// Every credential that can be flagged. Keys the coordinator maps to a
/// human-readable provider name (`coordinator.rs`).
const KNOWN_CREDENTIALS: &[&str] = &[CODEX_AUTH_JSON];

/// Per-credential triggered flags. Cheap to clone (Arc inside).
#[derive(Clone)]
pub struct CredentialFailureDetector {
    triggered: Arc<HashMap<String, Arc<AtomicBool>>>,
}

impl Default for CredentialFailureDetector {
    fn default() -> Self {
        Self {
            triggered: Arc::new(
                KNOWN_CREDENTIALS
                    .iter()
                    .map(|key| ((*key).to_string(), Arc::new(AtomicBool::new(false))))
                    .collect(),
            ),
        }
    }
}

impl CredentialFailureDetector {
    /// Flag a credential as failed. Called by the harness when a Turn fails
    /// with the named code for an unauthorized request. Unknown keys are
    /// ignored — the flag map is fixed at construction.
    pub fn trigger(&self, key: &str) {
        if let Some(flag) = self.triggered.get(key) {
            flag.store(true, Ordering::SeqCst);
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
    fn trigger_sets_the_flag_once() {
        let d = CredentialFailureDetector::default();
        d.trigger(CODEX_AUTH_JSON);
        assert!(d.take(CODEX_AUTH_JSON));
        // Subsequent take returns false — flag was cleared.
        assert!(!d.take(CODEX_AUTH_JSON));
    }

    #[test]
    fn untriggered_credentials_stay_clear() {
        let d = CredentialFailureDetector::default();
        assert!(!d.take(CODEX_AUTH_JSON));
        assert!(d.take_all().is_empty());
    }

    #[test]
    fn an_unknown_key_is_ignored() {
        let d = CredentialFailureDetector::default();
        d.trigger("NOT_A_CREDENTIAL");
        assert!(d.take_all().is_empty());
    }

    #[test]
    fn take_all_returns_set_keys() {
        let d = CredentialFailureDetector::default();
        d.trigger(CODEX_AUTH_JSON);
        assert_eq!(d.take_all(), vec![CODEX_AUTH_JSON.to_string()]);
        // Cleared.
        assert!(d.take_all().is_empty());
    }
}
