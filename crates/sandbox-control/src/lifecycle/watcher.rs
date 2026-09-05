//! Shared watcher classification and deterministic convergence publishing.

use std::sync::Arc;

use tracing::warn;

use moatless_sandbox_runtime::classify::{Observation, ObservedTransition};
use moatless_sandbox_runtime::observation_events::PublishDecision;
use moatless_sandbox_runtime::operation::OperationGeneration;

use super::Lifecycle;

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
