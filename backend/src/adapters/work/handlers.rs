//! The executor side of the admission seam.
//!
//! A webhook handler admits work under a `kind`; the process that executes it
//! registers a handler for the same `kind` at startup. Keeping the two ends
//! joined by a constant rather than by a shared enum means a provider migration
//! adds a handler module and one registration line, and never edits a match
//! arm that another migration is also editing.

use std::collections::HashMap;
use std::sync::Arc;

use super::models::{AdapterWork, AdapterWorkFailure};

/// Executes one kind of admitted adapter work.
///
/// Implementations **must be idempotent**. Execution is at-least-once: a
/// process can die after the work completed but before the record was marked
/// succeeded, and the record then runs again.
#[async_trait::async_trait]
pub trait AdapterWorkHandler: Send + Sync {
    /// Perform the work described by `work.payload`.
    ///
    /// Return [`AdapterWorkFailure::Retryable`] for anything a later attempt
    /// could survive, and [`AdapterWorkFailure::Permanent`] when no retry can
    /// help — a malformed payload, a deleted task, a revoked connection.
    async fn execute(&self, work: &AdapterWork) -> Result<(), AdapterWorkFailure>;
}

/// The handlers this process can execute, by work kind.
///
/// The worker claims only these kinds, so a replica running an older build
/// leaves newer work for one that understands it instead of failing it.
#[derive(Default)]
pub struct AdapterWorkHandlers(HashMap<&'static str, Arc<dyn AdapterWorkHandler>>);

impl AdapterWorkHandlers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the handler for `kind`, replacing any previous registration.
    pub fn register(&mut self, kind: &'static str, handler: Arc<dyn AdapterWorkHandler>) {
        self.0.insert(kind, handler);
    }

    pub(crate) fn get(&self, kind: &str) -> Option<&Arc<dyn AdapterWorkHandler>> {
        self.0.get(kind)
    }

    pub(crate) fn kinds(&self) -> Vec<&'static str> {
        self.0.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noop;

    #[async_trait::async_trait]
    impl AdapterWorkHandler for Noop {
        async fn execute(&self, _work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
            Ok(())
        }
    }

    #[test]
    fn an_empty_registry_claims_nothing() {
        let handlers = AdapterWorkHandlers::new();
        assert!(handlers.kinds().is_empty());
        assert!(handlers.get("slack.inbound_message").is_none());
    }

    #[test]
    fn registered_kinds_are_the_kinds_the_worker_claims() {
        let mut handlers = AdapterWorkHandlers::new();
        handlers.register("slack.inbound_message", Arc::new(Noop));
        handlers.register("telegram.inbound_message", Arc::new(Noop));

        let mut kinds = handlers.kinds();
        kinds.sort_unstable();
        assert_eq!(kinds, ["slack.inbound_message", "telegram.inbound_message"]);
        assert!(handlers.get("slack.inbound_message").is_some());
        assert!(handlers.get("github.pr_message").is_none());
    }
}
