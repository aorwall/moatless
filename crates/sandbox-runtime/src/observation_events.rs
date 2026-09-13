//! Generation-scoped identities and publication for runtime observations.

use crate::operation::OperationGeneration;
use moatless_core::events::EventPublisher;
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use tracing::warn;

/// The deterministic fact an event records for an operation generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFact {
    /// Watcher convergence: the generation's replacement became Ready.
    Ready,
    /// Acceptance: intent moved to `Removed`.
    Removing,
    /// Completion: the operation newly established a stopped sandbox.
    Stopped,
    /// Completion: the operation newly established a removed sandbox.
    Removed,
    /// Infrastructure: start/restart/redeploy actually did work.
    Initializing,
    /// Failure fact for an attempt.
    Error,
    /// Terminal fact: the reconciler's attempt cap for this generation is
    /// spent; it will not retry until a new caller operation or a wake.
    ///
    /// Distinct from [`Error`](Self::Error) with its own deterministic id —
    /// sharing `Error`'s id would dedup it away against the failure that caused
    /// it.
    RecoveryExhausted,
}

impl EventFact {
    /// Stable slug used as the last segment of the deterministic event id.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Removing => "removing",
            Self::Stopped => "stopped",
            Self::Removed => "removed",
            Self::Initializing => "initializing",
            Self::Error => "error",
            Self::RecoveryExhausted => "recovery_exhausted",
        }
    }

    /// The `SandboxStatus` sub-type the event publishes under.
    pub fn sandbox_status(self) -> SandboxStatus {
        match self {
            Self::Ready => SandboxStatus::Ready,
            Self::Removing => SandboxStatus::Removing,
            Self::Stopped => SandboxStatus::Stopped,
            Self::Removed => SandboxStatus::Removed,
            Self::Initializing => SandboxStatus::Initializing,
            Self::Error | Self::RecoveryExhausted => SandboxStatus::Error,
        }
    }
}

/// The deterministic event id for a `(task, generation, fact)` triple.
pub fn event_id(task_id: &str, generation: OperationGeneration, fact: EventFact) -> String {
    format!("sandbox:{task_id}:{generation}:{}", fact.slug())
}

pub struct PublishDecision {
    pub publishes: bool,
    pub event_generation: Option<OperationGeneration>,
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
                            &event_id(task_id, generation, EventFact::Ready),
                            task_id,
                            "sandbox",
                            SandboxStatus::Ready.as_str(),
                            None,
                            None,
                            payload,
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
