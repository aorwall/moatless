//! Shared watcher classification and deterministic convergence publishing.

use std::sync::Arc;

use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use tracing::warn;

use moatless_core::events::EventPublisher;

use moatless_sandbox_runtime::classify::{Observation, ObservedTransition};
use super::events::{self, EventFact};
use moatless_sandbox_runtime::types::OperationGeneration;

use super::Lifecycle;

pub struct PublishDecision {
    pub publishes: bool,
    pub event_generation: Option<OperationGeneration>,
}

pub async fn classify_observation(
    lifecycle: Option<&Arc<Lifecycle>>,
    task_id: &str,
    transition: ObservedTransition,
    generation: Option<OperationGeneration>,
) -> PublishDecision {
    let Some(lifecycle) = lifecycle else {
        return PublishDecision {
            publishes: true,
            event_generation: generation,
        };
    };
    match lifecycle
        .classify_observation(task_id, Observation::new(transition, generation))
        .await
    {
        Ok(result) => PublishDecision {
            publishes: result.classification.publishes(),
            event_generation: result.event_generation,
        },
        Err(e) => {
            warn!(
                task_id,
                error = %e,
                "Failed to classify runtime observation; publishing fail-open"
            );
            PublishDecision {
                publishes: true,
                event_generation: generation,
            }
        }
    }
}

pub async fn publish_sandbox_observation(
    event_publisher: &dyn EventPublisher,
    task_id: &str,
    status: SandboxStatus,
    payload: &SandboxEventPayload,
    generation: Option<OperationGeneration>,
) {
    if status == SandboxStatus::Ready {
        if let Some(generation) = generation {
            match serde_json::to_value(payload) {
                Ok(payload) => {
                    if let Err(e) = event_publisher
                        .publish_raw_event_with_id(
                            &events::event_id(task_id, generation, EventFact::Ready),
                            task_id,
                            "sandbox",
                            SandboxStatus::Ready.as_str(),
                            None,
                            None,
                            payload,
                            false,
                        )
                        .await
                    {
                        warn!(task_id, error = %e, "ready event did not reach the live transport");
                    }
                    return;
                }
                Err(e) => {
                    warn!(
                        task_id,
                        error = %e,
                        "Failed to serialize deterministic Ready observation; using typed publish"
                    );
                }
            }
        }
    }
    event_publisher
        .publish_sandbox_event_with_payload(task_id, status, payload)
        .await;
}
