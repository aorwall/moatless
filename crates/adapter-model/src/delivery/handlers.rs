//! The executor side of the admission seam.
//!
//! A webhook handler admits a delivery under a `kind`; the process that
//! executes it registers a handler for the same `kind` at startup. Keeping the
//! two ends joined by a constant rather than by a shared enum means adding an
//! Adapter adds a handler module and one registration line, and never edits a
//! match arm another Adapter is also editing.

use std::collections::HashMap;
use std::sync::Arc;

use super::{AdapterDelivery, AdapterDeliveryFailure};

/// Executes one kind of admitted Adapter delivery.
///
/// Implementations **must be idempotent**. Execution is at-least-once: a
/// process can die after the work completed but before the record was marked
/// succeeded, and the record then runs again.
#[async_trait::async_trait]
pub trait AdapterDeliveryHandler: Send + Sync {
    /// Perform the work this delivery describes, from `delivery.payload`.
    ///
    /// Return [`AdapterDeliveryFailure::Retryable`] for anything a later attempt
    /// could survive, and [`AdapterDeliveryFailure::Permanent`] when no retry can
    /// help — a malformed payload, a deleted task, a revoked connection.
    async fn execute(&self, delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure>;
}

/// The handlers this process can execute, by delivery kind.
///
/// The worker claims only these kinds, so a replica running an older build
/// leaves newer deliveries for one that understands them instead of failing
/// them.
#[derive(Default)]
pub struct AdapterDeliveryHandlers(HashMap<&'static str, Arc<dyn AdapterDeliveryHandler>>);

impl AdapterDeliveryHandlers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the handler for `kind`, replacing any previous registration.
    pub fn register(&mut self, kind: &'static str, handler: Arc<dyn AdapterDeliveryHandler>) {
        self.0.insert(kind, handler);
    }

    /// Public because the worker that dispatches on a claimed record's kind
    /// sits in the loops band, above this crate.
    pub fn get(&self, kind: &str) -> Option<&Arc<dyn AdapterDeliveryHandler>> {
        self.0.get(kind)
    }

    /// The kinds the worker claims, and the observable claim set an
    /// installation contract test asserts.
    pub fn kinds(&self) -> Vec<&'static str> {
        self.0.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noop;

    #[async_trait::async_trait]
    impl AdapterDeliveryHandler for Noop {
        async fn execute(&self, _delivery: &AdapterDelivery) -> Result<(), AdapterDeliveryFailure> {
            Ok(())
        }
    }

    #[test]
    fn an_empty_registry_claims_nothing() {
        let handlers = AdapterDeliveryHandlers::new();
        assert!(handlers.kinds().is_empty());
        assert!(handlers.get("slack.inbound_message").is_none());
    }

    #[test]
    fn registered_kinds_are_the_kinds_the_worker_claims() {
        let mut handlers = AdapterDeliveryHandlers::new();
        handlers.register("slack.inbound_message", Arc::new(Noop));
        handlers.register("telegram.inbound_message", Arc::new(Noop));

        let mut kinds = handlers.kinds();
        kinds.sort_unstable();
        assert_eq!(kinds, ["slack.inbound_message", "telegram.inbound_message"]);
        assert!(handlers.get("slack.inbound_message").is_some());
        assert!(handlers.get("github.pr_message").is_none());
    }
}
