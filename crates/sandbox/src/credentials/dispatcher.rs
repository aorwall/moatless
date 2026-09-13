use super::CredentialSink;
#[cfg(test)]
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Dispatcher mapping credential keys to their sinks.
///
/// Construct at sandbox init, register sinks with [`Dispatcher::register`],
/// hand the dispatcher to the gRPC server. The gRPC handler calls
/// [`Dispatcher::dispatch`] on every `RefreshCredential` push.
pub struct Dispatcher {
    sinks: HashMap<String, Arc<dyn CredentialSink>>,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            sinks: HashMap::new(),
        }
    }

    pub fn register(&mut self, key: impl Into<String>, sink: Arc<dyn CredentialSink>) {
        self.sinks.insert(key.into(), sink);
    }

    /// Apply a credential. Returns `Ok(true)` if a sink handled it,
    /// `Ok(false)` if no sink is registered for the key.
    pub async fn dispatch(&self, key: &str, value: &str) -> Result<bool, String> {
        let Some(sink) = self.sinks.get(key) else {
            info!(key, "No sink registered for credential — skipping dispatch");
            return Ok(false);
        };
        match sink.apply(value).await {
            Ok(()) => {
                info!(key, "Credential applied");
                Ok(true)
            }
            Err(e) => {
                warn!(error = %e, key, "Credential sink failed");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSink {
        count: Arc<AtomicUsize>,
        last_value: Arc<tokio::sync::Mutex<String>>,
    }

    #[async_trait]
    impl CredentialSink for CountingSink {
        async fn apply(&self, value: &str) -> Result<(), String> {
            self.count.fetch_add(1, Ordering::SeqCst);
            *self.last_value.lock().await = value.to_string();
            Ok(())
        }
    }

    fn counting_sink() -> (
        Arc<AtomicUsize>,
        Arc<tokio::sync::Mutex<String>>,
        Arc<CountingSink>,
    ) {
        let count = Arc::new(AtomicUsize::new(0));
        let last_value = Arc::new(tokio::sync::Mutex::new(String::new()));
        let sink = Arc::new(CountingSink {
            count: Arc::clone(&count),
            last_value: Arc::clone(&last_value),
        });
        (count, last_value, sink)
    }

    #[tokio::test]
    async fn dispatch_routes_to_registered_sink() {
        let (count, last_value, sink) = counting_sink();
        let mut d = Dispatcher::new();
        d.register("KEY_A", sink);

        let handled = d.dispatch("KEY_A", "secret").await.unwrap();
        assert!(handled);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(*last_value.lock().await, "secret");
    }

    #[tokio::test]
    async fn dispatch_returns_false_for_unregistered_key() {
        let d = Dispatcher::new();
        let handled = d.dispatch("UNKNOWN", "v").await.unwrap();
        assert!(!handled);
    }

    #[tokio::test]
    async fn dispatch_only_invokes_matching_sink() {
        let (count_a, _, sink_a) = counting_sink();
        let (count_b, _, sink_b) = counting_sink();
        let mut d = Dispatcher::new();
        d.register("KEY_A", sink_a);
        d.register("KEY_B", sink_b);

        d.dispatch("KEY_A", "v1").await.unwrap();

        assert_eq!(count_a.load(Ordering::SeqCst), 1);
        assert_eq!(count_b.load(Ordering::SeqCst), 0);
    }
}
