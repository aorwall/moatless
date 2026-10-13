//! The subscription seam for a task's own event stream.
//!
//! An Adapter registers a [`TaskEventSubscriber`] per [`EventKey`]; the
//! consumer fans events out via [`TaskEventSubscribers::dispatch`]. Keying
//! by `EventKey` means adding an Adapter never edits a shared match arm.
//!
//! `on_task_event` runs inline and must stay cheap: `tokio::spawn` any
//! provider call, detached — a lost notification is fine, durable work
//! belongs in `adapters::delivery`. A state-keeping subscriber must emit
//! then update state itself, in order; a panic in one is caught and logged.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt as _;
use tracing::error;

use moatless_tasks::agent_event::{AgentEvent, EventKey};

/// An Adapter's reaction to one task event.
///
/// Must be cheap and must not panic; see the module docs for what belongs here
/// vs. durable delivery.
#[async_trait::async_trait]
pub trait TaskEventSubscriber: Send + Sync {
    /// A name for logs — the provider this subscriber speaks for.
    fn name(&self) -> &'static str;

    /// React to `event`. Called once per event this subscriber is registered
    /// for, after the generic handlers for that event have run.
    async fn on_task_event(&self, event: &AgentEvent);
}

/// Which subscribers react to which events.
///
/// Built once at the composition root and handed to the event-handler consumer,
/// which owns it for the process lifetime.
#[derive(Default)]
pub struct TaskEventSubscribers(HashMap<EventKey, Vec<Arc<dyn TaskEventSubscriber>>>);

impl TaskEventSubscribers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `subscriber` against every key in `keys`.
    ///
    /// Registrations accumulate: two Adapters subscribing to the same key both
    /// receive the event, in registration order.
    pub fn subscribe(
        &mut self,
        keys: impl IntoIterator<Item = EventKey>,
        subscriber: Arc<dyn TaskEventSubscriber>,
    ) {
        for key in keys {
            self.0.entry(key).or_default().push(subscriber.clone());
        }
    }

    /// Hand `event` to every subscriber registered for its key.
    ///
    /// A key nobody subscribed to is a no-op; a subscriber that panics is
    /// logged and skipped, and the rest still run.
    pub async fn dispatch(&self, event: &AgentEvent) {
        let Some(subscribers) = self.0.get(&event.event_key()) else {
            return;
        };
        for subscriber in subscribers {
            if AssertUnwindSafe(subscriber.on_task_event(event))
                .catch_unwind()
                .await
                .is_err()
            {
                error!(
                    subscriber = subscriber.name(),
                    task_id = %event.task_id,
                    event_id = %event.event_id,
                    "Task-event subscriber panicked — skipping it for this event"
                );
            }
        }
    }

    /// The keys at least one subscriber is registered for.
    #[cfg(any(test, feature = "test-support"))]
    pub fn keys(&self) -> Vec<EventKey> {
        self.0.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use moatless_types::agent::AgentSubType;
    use moatless_types::event::EventType;
    use moatless_types::sandbox::SandboxStatus;

    use super::*;

    #[derive(Default)]
    struct Counting {
        seen: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl TaskEventSubscriber for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        async fn on_task_event(&self, _event: &AgentEvent) {
            self.seen.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Panicking;

    #[async_trait::async_trait]
    impl TaskEventSubscriber for Panicking {
        fn name(&self) -> &'static str {
            "panicking"
        }
        async fn on_task_event(&self, _event: &AgentEvent) {
            panic!("subscriber blew up");
        }
    }

    fn agent_event(sub_type: &str) -> AgentEvent {
        serde_json::from_value(serde_json::json!({
            "eventId": "evt-1",
            "type": EventType::Agent,
            "subType": sub_type,
            "taskId": "task-1",
            "timestamp": "2026-01-01T00:00:00Z",
            "isError": false,
            "agentProfile": null,
            "turnNumber": null,
            "payload": {},
        }))
        .expect("build test event")
    }

    #[tokio::test]
    async fn overlapping_subscribers_both_receive_one_event() {
        let a = Arc::new(Counting::default());
        let b = Arc::new(Counting::default());
        let mut subs = TaskEventSubscribers::new();
        subs.subscribe([EventKey::Agent(AgentSubType::Started)], a.clone());
        subs.subscribe([EventKey::Agent(AgentSubType::Started)], b.clone());

        subs.dispatch(&agent_event("started")).await;

        assert_eq!(a.seen.load(Ordering::SeqCst), 1);
        assert_eq!(b.seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_non_matching_key_receives_nothing() {
        let a = Arc::new(Counting::default());
        let mut subs = TaskEventSubscribers::new();
        subs.subscribe([EventKey::Sandbox(SandboxStatus::Error)], a.clone());

        subs.dispatch(&agent_event("started")).await;

        assert_eq!(a.seen.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_panicking_subscriber_leaves_its_siblings_running() {
        let after = Arc::new(Counting::default());
        let mut subs = TaskEventSubscribers::new();
        subs.subscribe(
            [EventKey::Agent(AgentSubType::Started)],
            Arc::new(Panicking),
        );
        subs.subscribe([EventKey::Agent(AgentSubType::Started)], after.clone());

        subs.dispatch(&agent_event("started")).await;

        assert_eq!(
            after.seen.load(Ordering::SeqCst),
            1,
            "the sibling after the panicking subscriber still ran"
        );
    }
}
