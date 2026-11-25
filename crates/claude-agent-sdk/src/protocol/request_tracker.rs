use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::oneshot;

/// Information about a control error response.
#[derive(Debug, Clone)]
pub(crate) struct ControlErrorInfo {
    pub error: String,
}

/// Tracks pending outgoing control requests by ID, providing oneshot channels
/// for response correlation.
pub(crate) struct RequestTracker {
    counter: u64,
    pending: HashMap<String, oneshot::Sender<Result<Option<Value>, ControlErrorInfo>>>,
}

impl RequestTracker {
    pub(crate) fn new() -> Self {
        Self {
            counter: 0,
            pending: HashMap::new(),
        }
    }

    /// Register a new pending request. Returns the request ID and a receiver
    /// that will resolve with the response.
    pub(crate) fn register(
        &mut self,
    ) -> (
        String,
        oneshot::Receiver<Result<Option<Value>, ControlErrorInfo>>,
    ) {
        self.counter += 1;
        let request_id = format!("sdk-{}", self.counter);
        let (tx, rx) = oneshot::channel();
        self.pending.insert(request_id.clone(), tx);
        (request_id, rx)
    }

    /// Resolve a pending request with a success response.
    pub(crate) fn resolve_success(&mut self, request_id: &str, response: Option<Value>) -> bool {
        if let Some(tx) = self.pending.remove(request_id) {
            let _ = tx.send(Ok(response));
            true
        } else {
            false
        }
    }

    /// Resolve a pending request with an error response.
    pub(crate) fn resolve_error(&mut self, request_id: &str, error: ControlErrorInfo) -> bool {
        if let Some(tx) = self.pending.remove(request_id) {
            let _ = tx.send(Err(error));
            true
        } else {
            false
        }
    }

    /// Cancel all pending requests (used during shutdown).
    pub(crate) fn cancel_all(&mut self) {
        for (_, tx) in self.pending.drain() {
            let _ = tx.send(Err(ControlErrorInfo {
                error: "Transport closed".to_string(),
            }));
        }
    }

    /// Check if a request ID is pending.
    pub(crate) fn is_pending(&self, request_id: &str) -> bool {
        self.pending.contains_key(request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_resolve_success() {
        let mut tracker = RequestTracker::new();
        let (id, mut rx) = tracker.register();

        assert!(tracker.is_pending(&id));
        assert!(tracker.resolve_success(&id, Some(serde_json::json!({"ok": true}))));
        assert!(!tracker.is_pending(&id));

        let result = rx.try_recv().unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn register_and_resolve_error() {
        let mut tracker = RequestTracker::new();
        let (id, mut rx) = tracker.register();

        assert!(tracker.resolve_error(
            &id,
            ControlErrorInfo {
                error: "test error".to_string(),
            }
        ));

        let result = rx.try_recv().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn resolve_unknown_returns_false() {
        let mut tracker = RequestTracker::new();
        assert!(!tracker.resolve_success("nonexistent", None));
    }

    #[test]
    fn cancel_all() {
        let mut tracker = RequestTracker::new();
        let (_id1, mut rx1) = tracker.register();
        let (_id2, mut rx2) = tracker.register();

        tracker.cancel_all();

        assert!(rx1.try_recv().unwrap().is_err());
        assert!(rx2.try_recv().unwrap().is_err());
    }

    #[test]
    fn request_ids_are_sequential() {
        let mut tracker = RequestTracker::new();
        let (id1, _) = tracker.register();
        let (id2, _) = tracker.register();

        assert_eq!(id1, "sdk-1");
        assert_eq!(id2, "sdk-2");
    }
}
