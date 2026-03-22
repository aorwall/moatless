//! gRPC event publisher for sandbox agent events.
//!
//! Publishes events to the backend via gRPC. The backend then writes them
//! to Redis Streams for SSE consumption.
//!
//! This replaces the previous `RedisEventPublisher` which published
//! directly to Redis from the sandbox. The public API is unchanged —
//! all publish methods are fire-and-forget.

use std::sync::Arc;

use moatless_events::{AgentSubType, EventType, TaskSubType, TaskTurnSubType};
use sandbox_proto::PublishEventRequest;
use tracing::debug;

use crate::backend_client::BackendClient;

/// Publishes sandbox events to the backend via gRPC.
///
/// All publish operations are fire-and-forget — failures are logged
/// but never propagated.
#[derive(Clone)]
pub struct GrpcEventPublisher {
    client: Arc<dyn BackendClient>,
}

impl GrpcEventPublisher {
    /// Create a new publisher backed by the given backend client.
    pub fn new(client: Arc<dyn BackendClient>) -> Self {
        Self { client }
    }

    /// Access the underlying backend client (for direct gRPC calls).
    pub fn client(&self) -> &dyn BackendClient {
        &*self.client
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
        sub_type: moatless_events::SandboxSubType,
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

    /// Internal: publish a generic event via gRPC.
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

        debug!(
            task_id = %task_id,
            event_type = %event_type,
            sub_type = %sub_type,
            "Publishing event via gRPC"
        );

        let proto_event_type: sandbox_proto::EventType = event_type.into();

        self.client
            .publish_event(PublishEventRequest {
                task_id: task_id.to_string(),
                event_type: proto_event_type as i32,
                sub_type: sub_type.to_string(),
                payload_json,
            })
            .await;
    }
}
