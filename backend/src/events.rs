//! Redis event publisher for environment and task lifecycle events.
//!
//! Publishes events to Redis Streams that the frontend consumes via SSE.
//! The stream format must match exactly:
//!
//! ```text
//! XADD task-events:{taskId} MAXLEN ~ {max} *
//!   eventId {uuid}
//!   type environment|task|taskTurn
//!   payload {fullEventJson}
//! ```

use redis::aio::ConnectionManager;
use serde::Serialize;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Publishes environment lifecycle events to Redis Streams.
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

    /// Publish an environment lifecycle event.
    ///
    /// Fire-and-forget: logs warnings on failure, never returns errors.
    pub async fn publish_environment_event(
        &self,
        task_id: &str,
        sub_type: &str,
        error_message: Option<&str>,
    ) {
        let payload = match error_message {
            Some(msg) => serde_json::json!({ "error": msg }),
            None => serde_json::json!({}),
        };
        self.publish_environment_event_with_payload(task_id, sub_type, payload)
            .await;
    }

    /// Publish an environment lifecycle event with a custom JSON payload.
    ///
    /// Fire-and-forget: logs warnings on failure, never returns errors.
    pub async fn publish_environment_event_with_payload(
        &self,
        task_id: &str,
        sub_type: &str,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, "environment", sub_type, None, None, payload)
            .await;
    }

    /// Publish a task lifecycle event (e.g. "open", "closed", "running").
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

    /// Publish an agent lifecycle event (e.g. "completed", "error", "message", "stopped").
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

    /// Publish a task turn event (e.g. "active", "completed", "failed").
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

    /// Internal: publish a generic event to the Redis stream.
    async fn publish_event(
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
        let is_error = sub_type == "error" || sub_type == "server_failed";

        let event = StreamEvent {
            event_id: event_id.clone(),
            event_type: event_type.to_string(),
            sub_type: sub_type.to_string(),
            task_id: task_id.to_string(),
            timestamp,
            is_error,
            agent_type: agent_type.map(|s| s.to_string()),
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
