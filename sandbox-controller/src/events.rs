//! Redis event publisher for environment lifecycle events.
//!
//! Publishes events to Redis Streams that the TS backend consumes via
//! `RedisEventTransport`. The stream format must match exactly:
//!
//! ```text
//! XADD task-events:{taskId} MAXLEN ~ {max} *
//!   eventId {uuid}
//!   type environment
//!   payload {fullEventJson}
//! ```

use redis::aio::ConnectionManager;
use serde::Serialize;
use tracing::{info, warn};
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

/// Environment event payload matching the TS `Event` schema exactly.
///
/// Fields use camelCase to match the JSON the TS consumer expects
/// when it does `JSON.parse(payload)`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentEvent {
    event_id: String,
    #[serde(rename = "type")]
    event_type: &'static str,
    sub_type: String,
    task_id: String,
    timestamp: String,
    is_error: bool,
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
        let event_id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let is_error = sub_type == "error";

        let payload = match error_message {
            Some(msg) => serde_json::json!({ "error": msg }),
            None => serde_json::json!({}),
        };

        let event = EnvironmentEvent {
            event_id: event_id.clone(),
            event_type: "environment",
            sub_type: sub_type.to_string(),
            task_id: task_id.to_string(),
            timestamp,
            is_error,
            payload,
        };

        let event_json = match serde_json::to_string(&event) {
            Ok(json) => json,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to serialize environment event");
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
            .arg("environment")
            .arg("payload")
            .arg(&event_json)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(_) => {
                info!(
                    task_id = %task_id,
                    sub_type = %sub_type,
                    "Published environment event to Redis"
                );
            }
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    sub_type = %sub_type,
                    error = %e,
                    "Failed to publish environment event to Redis"
                );
            }
        }
    }
}
