//! [`LifecycleEventSink`] over the transactional outbox.
//!
//! Publishes a derived lifecycle event to Redis under its deterministic id.
//! Completion facts are already committed to `task_events` by
//! [`super::orchestrator::LifecycleStore::commit_fact_and_clear_marker`], so they publish
//! with `payload_persisted = true`; other facts are persisted by the event
//! consumer.

use std::sync::Arc;

use async_trait::async_trait;

use crate::events::EventPublisher;

use super::events::LifecycleEvent;
use super::orchestrator::LifecycleEventSink;

/// Publishes lifecycle events through the existing Redis outbox path.
///
/// The publisher is optional: with no event transport (Redis disabled) the sink
/// no-ops, matching the rest of the system's Redis-down degradation. Completion
/// facts stay durable regardless — the store commits them to `task_events`
/// before publication.
pub struct OutboxEventSink {
    publisher: Option<Arc<dyn EventPublisher>>,
}

impl OutboxEventSink {
    pub fn new(publisher: Option<Arc<dyn EventPublisher>>) -> Self {
        Self { publisher }
    }
}

#[async_trait]
impl LifecycleEventSink for OutboxEventSink {
    async fn publish(&self, task_id: &str, event: &LifecycleEvent) {
        let Some(publisher) = &self.publisher else {
            return;
        };
        let payload = match serde_json::to_value(event.payload()) {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(task_id, error = %e, "failed to serialize lifecycle event payload");
                return;
            }
        };
        publisher
            .publish_raw_event_with_id(
                &event.event_id,
                task_id,
                "sandbox",
                event.sub_type(),
                None,
                None,
                payload,
                event.persisted_as_completion(),
            )
            .await;
    }
}
