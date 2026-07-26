//! gRPC event publisher for sandbox agent events.
//!
//! Publishes events to the backend via gRPC. The backend then writes them
//! to Redis Streams for SSE consumption.
//!
//! Publishing is enqueue-only: every method hands the event to the
//! [`crate::outbox::EventOutbox`], which owns the at-least-once delivery
//! (ordered, retried with backoff). The public API stays fire-and-forget — an
//! `enqueue` never blocks and never fails the caller.

use moatless_events::{AgentSubType, EventType, TaskSubType, TaskTurnSubType};
use sandbox_proto::PublishEventRequest;
use tracing::debug;

use crate::outbox::EventOutbox;

/// Publishes sandbox events to the backend via the at-least-once outbox.
///
/// All publish operations are non-blocking — they enqueue and return. Delivery,
/// retry, ordering and overflow handling live in [`crate::outbox`].
#[derive(Clone)]
pub struct GrpcEventPublisher {
    outbox: EventOutbox,
    /// Set on run-scoped clones (see [`GrpcEventPublisher::for_run`]); every
    /// event published through one carries it so the backend can attribute the
    /// event to that run's turn regardless of delivery order.
    run_id: Option<String>,
}

impl GrpcEventPublisher {
    /// Create a new publisher backed by the given outbox.
    pub fn new(outbox: EventOutbox) -> Self {
        Self {
            outbox,
            run_id: None,
        }
    }

    /// A clone of this publisher that stamps `run_id` on everything it emits.
    /// Used by the agent actor for the lifetime of one agent run.
    pub fn for_run(&self, run_id: &str) -> Self {
        Self {
            outbox: self.outbox.clone(),
            run_id: Some(run_id.to_string()),
        }
    }

    /// Access the underlying outbox (for shutdown flush and metrics).
    pub fn outbox(&self) -> &EventOutbox {
        &self.outbox
    }

    /// Enqueue a pre-built request. Assigns a stable `event_id` when the caller
    /// left it empty. Used for events assembled outside the typed helpers below
    /// (e.g. init's sandbox status event with its custom payload shape).
    pub fn enqueue(&self, mut req: PublishEventRequest) {
        if req.event_id.is_empty() {
            req.event_id = uuid::Uuid::new_v4().to_string();
        }
        self.outbox.enqueue(req);
    }

    /// Publish a task lifecycle event (e.g. status changes, metadata updates).
    pub async fn publish_task_event(
        &self,
        task_id: &str,
        sub_type: TaskSubType,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, EventType::Task, sub_type.as_str(), payload)
            .await;
    }

    /// Publish an agent lifecycle event (e.g. started, completed, error, message).
    ///
    /// Uses `event_type="agent"` so the backend consumer routes these as
    /// `"agent.completed"`, `"agent.error"`, etc.
    pub async fn publish_agent_event(
        &self,
        task_id: &str,
        sub_type: AgentSubType,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, EventType::Agent, sub_type.as_str(), payload)
            .await;
    }

    /// Publish a sandbox lifecycle event (e.g. ready, error, fatal).
    pub async fn publish_sandbox_event(
        &self,
        task_id: &str,
        sub_type: moatless_events::SandboxStatus,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, EventType::Sandbox, sub_type.as_str(), payload)
            .await;
    }

    /// Publish a task turn event (e.g. turn active, completed, failed).
    pub async fn publish_task_turn_event(
        &self,
        task_id: &str,
        sub_type: TaskTurnSubType,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, EventType::TaskTurn, sub_type.as_str(), payload)
            .await;
    }

    /// Internal: build a generic event and enqueue it for delivery.
    async fn publish_event(
        &self,
        task_id: &str,
        event_type: EventType,
        sub_type: &str,
        payload: serde_json::Value,
    ) {
        let payload_json = match serde_json::to_string(&payload) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to serialize event payload");
                return;
            }
        };

        // Narrowing onto the wire enum is partial: the domain enum carries
        // backend-derived types the protocol cannot express. Every caller above
        // passes a sandbox-originated type, so a rejection here is a bug in this
        // module, not a runtime condition — drop the event rather than publish it
        // under a different type. The drop goes through the outbox's accounting
        // seam so it is measured and announced, not only logged.
        let proto_event_type = match sandbox_proto::EventType::try_from(event_type) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(
                    task_id = %task_id,
                    sub_type = %sub_type,
                    error = %e,
                    "Refusing to publish event over the sandbox protocol"
                );
                self.outbox.record_unrepresentable();
                return;
            }
        };

        debug!(
            task_id = %task_id,
            event_type = %event_type,
            sub_type = %sub_type,
            "Enqueueing event for delivery"
        );

        self.outbox.enqueue(PublishEventRequest {
            task_id: task_id.to_string(),
            event_type: proto_event_type as i32,
            sub_type: sub_type.to_string(),
            payload_json,
            event_id: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
            run_id: self.run_id.clone().unwrap_or_default(),
        });
    }
}
