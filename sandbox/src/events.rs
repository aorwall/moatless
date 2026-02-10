//! gRPC event publisher for sandbox agent events.
//!
//! Publishes events to the backend via gRPC. The backend then writes them
//! to Redis Streams for SSE consumption.
//!
//! This replaces the previous `RedisEventPublisher` which published
//! directly to Redis from the sandbox. The public API is unchanged —
//! all publish methods are fire-and-forget.

use sandbox_proto::PublishEventRequest;
use tracing::debug;

use crate::backend_client::BackendClient;

/// Publishes sandbox events to the backend via gRPC.
///
/// All publish operations are fire-and-forget — failures are logged
/// but never propagated.
#[derive(Clone)]
pub struct GrpcEventPublisher {
    client: BackendClient,
}

impl GrpcEventPublisher {
    /// Create a new publisher backed by the given gRPC client.
    pub fn new(client: BackendClient) -> Self {
        Self { client }
    }

    /// Access the underlying backend client (for direct gRPC calls).
    pub fn client(&self) -> &BackendClient {
        &self.client
    }

    /// Publish a task lifecycle event (e.g. status changes, metadata updates).
    pub async fn publish_task_event(
        &self,
        task_id: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, "task", sub_type, agent_type, None, payload)
            .await;
    }

    /// Publish an agent lifecycle event (e.g. started, completed, error, message).
    ///
    /// Uses `event_type="agent"` so the backend consumer routes these as
    /// `"agent.completed"`, `"agent.error"`, etc.
    pub async fn publish_agent_event(
        &self,
        task_id: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, "agent", sub_type, agent_type, None, payload)
            .await;
    }

    /// Publish a task turn event (e.g. turn active, completed, failed).
    #[allow(dead_code)]
    pub async fn publish_task_turn_event(
        &self,
        task_id: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, "taskTurn", sub_type, agent_type, turn_number, payload)
            .await;
    }

    /// Internal: publish a generic event via gRPC.
    async fn publish_event(
        &self,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        let is_error = sub_type == "error" || sub_type == "server_failed";

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

        self.client
            .publish_event(PublishEventRequest {
                task_id: task_id.to_string(),
                event_type: event_type.to_string(),
                sub_type: sub_type.to_string(),
                agent_type: agent_type.map(|s| s.to_string()),
                turn_number,
                is_error,
                payload_json,
            })
            .await;
    }
}
