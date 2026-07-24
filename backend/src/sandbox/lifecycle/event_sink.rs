//! [`LifecycleEventSink`] over the transactional outbox.
//!
//! Publishes a derived lifecycle event to Redis under its deterministic id.
//! Completion facts are already committed to `task_events` by
//! [`super::orchestrator::LifecycleStore::commit_fact_and_clear_marker`], so they publish
//! with `payload_persisted = true`; other facts are persisted by the event
//! consumer.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_events::{SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType};

use crate::events::EventPublisher;

use super::events::LifecycleEvent;
use super::orchestrator::LifecycleEventSink;
use super::outcome::Diagnostics;
use super::types::{ObservedStatus, OperationGeneration};

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

    async fn publish_diagnostics(
        &self,
        task_id: &str,
        generation: OperationGeneration,
        observed: ObservedStatus,
        diagnostics: &Diagnostics,
    ) {
        let Some(publisher) = &self.publisher else {
            return;
        };
        for (index, warning) in diagnostics.degradations.iter().enumerate() {
            let payload = SandboxEventPayload {
                message: warning.message.clone(),
                lifecycle_event: Some(warning.code.clone()),
                error: Some(warning.message.clone()),
                ..Default::default()
            };
            match serde_json::to_value(payload) {
                Ok(payload) => {
                    publisher
                        .publish_raw_event_with_id(
                            &format!("sandbox:{task_id}:{generation}:degradation:{index}"),
                            task_id,
                            "sandbox",
                            observed_status(observed).as_str(),
                            None,
                            None,
                            payload,
                            false,
                        )
                        .await;
                }
                Err(e) => {
                    tracing::warn!(
                        task_id,
                        error = %e,
                        "failed to serialize lifecycle degradation"
                    );
                }
            }
        }
        for (index, failure) in diagnostics.server_failures.iter().enumerate() {
            let payload = ServerEventPayload {
                server_name: failure.server.clone(),
                status: ServerSubType::Failed,
                message: failure.message.clone(),
            };
            match serde_json::to_value(payload) {
                Ok(payload) => {
                    publisher
                        .publish_raw_event_with_id(
                            &format!("server:{task_id}:{generation}:failed:{index}"),
                            task_id,
                            "server",
                            ServerSubType::Failed.as_str(),
                            None,
                            None,
                            payload,
                            false,
                        )
                        .await;
                }
                Err(e) => {
                    tracing::warn!(
                        task_id,
                        server = %failure.server,
                        error = %e,
                        "failed to serialize lifecycle server failure"
                    );
                }
            }
        }
    }
}

fn observed_status(observed: ObservedStatus) -> SandboxStatus {
    match observed {
        ObservedStatus::Ready => SandboxStatus::Ready,
        ObservedStatus::Starting => SandboxStatus::Initializing,
        ObservedStatus::Stopped => SandboxStatus::Stopped,
        ObservedStatus::Removed => SandboxStatus::Removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degradation_status_tracks_the_runtime_observation() {
        assert_eq!(observed_status(ObservedStatus::Ready), SandboxStatus::Ready);
        assert_eq!(
            observed_status(ObservedStatus::Starting),
            SandboxStatus::Initializing
        );
        assert_eq!(
            observed_status(ObservedStatus::Stopped),
            SandboxStatus::Stopped
        );
        assert_eq!(
            observed_status(ObservedStatus::Removed),
            SandboxStatus::Removed
        );
    }
}
