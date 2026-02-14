//! Event publisher trait and Redis implementation.
//!
//! Publishes events to Redis Streams that the frontend consumes via SSE.
//! The stream format must match exactly:
//!
//! ```text
//! XADD task-events:{taskId} MAXLEN ~ {max} *
//!   eventId {uuid}
//!   type sandbox|task|taskTurn|agent|server|resources
//!   payload {fullEventJson}
//! ```

use async_trait::async_trait;
use moatless_events::{
    AgentSubType, EventType, SandboxSubType, ServerEventPayload, ServerSubType, SubType,
    TaskSubType, TaskTurnSubType,
};
use redis::aio::ConnectionManager;
use serde::Serialize;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Trait for publishing lifecycle events.
///
/// Abstracts event publishing so handlers can be tested without Redis.
/// All methods are fire-and-forget — implementations should log errors
/// but never propagate them.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish a sandbox lifecycle event.
    async fn publish_sandbox_event(
        &self,
        task_id: &str,
        sub_type: SandboxSubType,
        error_message: Option<&str>,
    );

    /// Publish a sandbox lifecycle event with a custom JSON payload.
    async fn publish_sandbox_event_with_payload(
        &self,
        task_id: &str,
        sub_type: SandboxSubType,
        payload: serde_json::Value,
    );

    /// Publish a server container event.
    async fn publish_server_event(
        &self,
        task_id: &str,
        sub_type: ServerSubType,
        payload: &ServerEventPayload,
    );

    /// Publish a task lifecycle event (e.g. "open", "closed").
    async fn publish_task_event(
        &self,
        task_id: &str,
        sub_type: TaskSubType,
        agent_type: Option<&str>,
        payload: serde_json::Value,
    );

    /// Publish an agent lifecycle event (e.g. "completed", "error", "message", "stopped").
    async fn publish_agent_event(
        &self,
        task_id: &str,
        sub_type: AgentSubType,
        agent_type: Option<&str>,
        payload: serde_json::Value,
    );

    /// Publish a task turn event (e.g. "active", "completed", "error").
    async fn publish_task_turn_event(
        &self,
        task_id: &str,
        sub_type: TaskTurnSubType,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    );

    /// Publish a raw event with string types — used by the gRPC server
    /// to forward events from the sandbox without re-parsing sub_types.
    async fn publish_raw_event(
        &self,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    );
}

/// Redis Streams implementation of [`EventPublisher`].
///
/// Uses `ConnectionManager` for automatic reconnection. All publish
/// operations are fire-and-forget — failures are logged but never
/// propagated to callers.
#[derive(Clone)]
pub struct RedisEventPublisher {
    conn: ConnectionManager,
    max_stream_length: usize,
}

/// Generic event payload matching the TS `Event` schema exactly.
///
/// Fields use camelCase to match the JSON the TS consumer expects
/// when it does `JSON.parse(payload)`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamEvent {
    event_id: String,
    #[serde(rename = "type")]
    event_type: String,
    sub_type: String,
    task_id: String,
    timestamp: String,
    is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_number: Option<i32>,
    payload: serde_json::Value,
}

impl RedisEventPublisher {
    /// Connect to Redis. Returns `None` on failure (non-fatal).
    pub async fn connect(redis_url: &str, max_stream_length: usize) -> Option<Self> {
        let client = match redis::Client::open(redis_url) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "Failed to create Redis client — event publishing disabled");
                return None;
            }
        };

        match ConnectionManager::new(client).await {
            Ok(conn) => {
                info!("Redis event publisher connected");
                Some(Self {
                    conn,
                    max_stream_length,
                })
            }
            Err(e) => {
                warn!(error = %e, "Failed to connect to Redis — event publishing disabled");
                None
            }
        }
    }

    // =========================================================================
    // Internal helpers (Redis-specific)
    // =========================================================================

    /// Internal: typed event publish.
    async fn publish_event_typed(
        &self,
        task_id: &str,
        event_type: EventType,
        sub_type: SubType,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        self.publish_event_internal(
            task_id,
            event_type.as_str(),
            sub_type.as_str(),
            agent_type,
            turn_number,
            payload,
        )
        .await;
    }

    /// Internal: publish a generic event to the Redis stream.
    async fn publish_event_internal(
        &self,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        let event_id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let is_error = sub_type == "error" || sub_type == "failed";

        let event = StreamEvent {
            event_id: event_id.clone(),
            event_type: event_type.to_string(),
            sub_type: sub_type.to_string(),
            task_id: task_id.to_string(),
            timestamp,
            is_error,
            agent_type: agent_type.map(std::string::ToString::to_string),
            turn_number,
            payload,
        };

        let event_json = match serde_json::to_string(&event) {
            Ok(json) => json,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to serialize event");
                return;
            }
        };

        let stream_key = format!("task-events:{task_id}");
        let max_len = self.max_stream_length;

        let mut conn = self.conn.clone();
        let result: Result<String, redis::RedisError> = redis::cmd("XADD")
            .arg(&stream_key)
            .arg("MAXLEN")
            .arg("~")
            .arg(max_len)
            .arg("*")
            .arg("eventId")
            .arg(&event_id)
            .arg("type")
            .arg(event_type)
            .arg("payload")
            .arg(&event_json)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(_) => {
                debug!(
                    task_id = %task_id,
                    event_type = %event_type,
                    sub_type = %sub_type,
                    "Published event to Redis"
                );
            }
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    event_type = %event_type,
                    sub_type = %sub_type,
                    error = %e,
                    "Failed to publish event to Redis"
                );
            }
        }
    }
}

#[async_trait]
impl EventPublisher for RedisEventPublisher {
    async fn publish_sandbox_event(
        &self,
        task_id: &str,
        sub_type: SandboxSubType,
        error_message: Option<&str>,
    ) {
        let payload = match error_message {
            Some(msg) => serde_json::json!({ "error": msg }),
            None => serde_json::json!({}),
        };
        self.publish_sandbox_event_with_payload(task_id, sub_type, payload)
            .await;
    }

    async fn publish_sandbox_event_with_payload(
        &self,
        task_id: &str,
        sub_type: SandboxSubType,
        payload: serde_json::Value,
    ) {
        self.publish_event_typed(
            task_id,
            EventType::Sandbox,
            sub_type.into(),
            None,
            None,
            payload,
        )
        .await;
    }

    async fn publish_server_event(
        &self,
        task_id: &str,
        sub_type: ServerSubType,
        payload: &ServerEventPayload,
    ) {
        let payload_value = serde_json::to_value(payload).unwrap_or_default();
        self.publish_event_typed(
            task_id,
            EventType::Server,
            sub_type.into(),
            None,
            None,
            payload_value,
        )
        .await;
    }

    async fn publish_task_event(
        &self,
        task_id: &str,
        sub_type: TaskSubType,
        agent_type: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.publish_event_typed(
            task_id,
            EventType::Task,
            sub_type.into(),
            agent_type,
            None,
            payload,
        )
        .await;
    }

    async fn publish_agent_event(
        &self,
        task_id: &str,
        sub_type: AgentSubType,
        agent_type: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.publish_event_typed(
            task_id,
            EventType::Agent,
            sub_type.into(),
            agent_type,
            None,
            payload,
        )
        .await;
    }

    async fn publish_task_turn_event(
        &self,
        task_id: &str,
        sub_type: TaskTurnSubType,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        self.publish_event_typed(
            task_id,
            EventType::TaskTurn,
            sub_type.into(),
            agent_type,
            turn_number,
            payload,
        )
        .await;
    }

    async fn publish_raw_event(
        &self,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        self.publish_event_internal(
            task_id,
            event_type,
            sub_type,
            agent_type,
            turn_number,
            payload,
        )
        .await;
    }
}
