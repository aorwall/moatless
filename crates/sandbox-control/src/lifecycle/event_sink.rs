//! [`LifecycleEventSink`] over the commit-before-publication path.
//!
//! Publishes a derived lifecycle event to Redis under its deterministic id.
//! Completion facts are already committed to `task_events` by
//! [`super::dao::LifecycleDao::commit_fact_and_clear_marker`], so they publish
//! with `payload_persisted = true`; other facts are persisted by the event
//! consumer.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_types::sandbox::{SandboxEventPayload, ServerEventPayload, ServerSubType};

use moatless_core::events::EventPublisher;

use super::events::LifecycleEvent;
use super::orchestrator::LifecycleEventSink;
use moatless_sandbox_runtime::operation::{ObservedStatus, OperationGeneration};
use moatless_sandbox_runtime::outcome::Diagnostics;

/// Publishes lifecycle events through the existing Redis publication path.
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
        // The fact this event reports is already committed (completion facts by
        // the store, the rest by the consumer), so a failed publish degrades the
        // live tail only — it is logged, not propagated.
        if let Err(e) = publisher
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
            .await
        {
            tracing::warn!(task_id, event_id = %event.event_id, error = %e, "lifecycle event did not reach the live transport");
        }
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
                    if let Err(e) = publisher
                        .publish_raw_event_with_id(
                            &format!("sandbox:{task_id}:{generation}:degradation:{index}"),
                            task_id,
                            "sandbox",
                            observed.sandbox_status().as_str(),
                            None,
                            None,
                            payload,
                            false,
                        )
                        .await
                    {
                        tracing::warn!(task_id, error = %e, "lifecycle degradation did not reach the live transport");
                    }
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
                    if let Err(e) = publisher
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
                        .await
                    {
                        tracing::warn!(task_id, server = %failure.server, error = %e, "server failure did not reach the live transport");
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_types::sandbox::SandboxStatus;

    #[test]
    fn degradation_status_tracks_the_runtime_observation() {
        assert_eq!(ObservedStatus::Ready.sandbox_status(), SandboxStatus::Ready);
        assert_eq!(
            ObservedStatus::Starting.sandbox_status(),
            SandboxStatus::Initializing
        );
        assert_eq!(
            ObservedStatus::Stopped.sandbox_status(),
            SandboxStatus::Stopped
        );
        assert_eq!(
            ObservedStatus::Removed.sandbox_status(),
            SandboxStatus::Removed
        );
    }
}
