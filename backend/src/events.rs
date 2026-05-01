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
    AgentSubType, EventType, SandboxEventPayload, SandboxSubType, ServerEventPayload,
    ServerSubType, StreamEvent, SubType, TaskSubType, TaskTurnSubType,
};
use redis::aio::ConnectionManager;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Maximum number of retry attempts after the initial publish fails.
const MAX_RETRIES: usize = 2;

/// Fixed delays (in ms) between retry attempts.
const RETRY_DELAYS_MS: [u64; MAX_RETRIES] = [50, 200];

/// Per-attempt timeout for the underlying Redis command. Caps how long any
/// single publish can stall a calling handler when Redis is unhealthy — total
/// worst case is roughly `(MAX_RETRIES + 1) * PUBLISH_ATTEMPT_TIMEOUT_MS` plus
/// the retry sleeps. Keep it small: the publish is fire-and-forget, and any
/// hang on a request handler hot path is worse than dropping an event.
const PUBLISH_ATTEMPT_TIMEOUT_MS: u64 = 750;

/// Timeout for the health-check `PING` used by `/ready`.
const HEALTH_CHECK_TIMEOUT_MS: u64 = 500;

/// Register Prometheus metric descriptions for event publishing counters.
///
/// Call once at startup after the Prometheus recorder is installed.
pub fn describe_event_metrics() {
    metrics::describe_counter!(
        "events_published_total",
        "Total events successfully published to Redis"
    );
    metrics::describe_counter!(
        "events_publish_failures_total",
        "Total events that failed to publish after all retries"
    );
    metrics::describe_counter!(
        "events_publish_retries_total",
        "Total retry attempts that preceded a successful publish"
    );
}

/// Trait for publishing lifecycle events.
///
/// Abstracts event publishing so handlers can be tested without Redis.
/// All methods are fire-and-forget — implementations should log errors
/// but never propagate them.
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` in async-trait methods.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish a sandbox lifecycle event with a message.
    async fn publish_sandbox_event(&self, task_id: &str, sub_type: SandboxSubType, message: &str);

    /// Publish a sandbox lifecycle event with a typed payload.
    async fn publish_sandbox_event_with_payload(
        &self,
        task_id: &str,
        sub_type: SandboxSubType,
        payload: &SandboxEventPayload,
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

    /// Probe the underlying transport. Returns `Ok(())` if a round-trip
    /// completes within a short bound, `Err(message)` otherwise. Used by the
    /// `/ready` Kubernetes readiness probe.
    async fn health_check(&self) -> Result<(), String>;
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

        let parsed_event_type = match EventType::try_from(event_type) {
            Ok(et) => et,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Unknown event type, skipping publish");
                return;
            }
        };

        let event = StreamEvent {
            event_id: event_id.clone(),
            event_type: parsed_event_type,
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

        let mut last_err = None;
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    RETRY_DELAYS_MS[attempt - 1],
                ))
                .await;
            }

            let mut conn = self.conn.clone();
            let mut cmd = redis::cmd("XADD");
            cmd.arg(&stream_key)
                .arg("MAXLEN")
                .arg("~")
                .arg(max_len)
                .arg("*")
                .arg("eventId")
                .arg(&event_id)
                .arg("type")
                .arg(event_type)
                .arg("payload")
                .arg(&event_json);

            let cmd_future = cmd.query_async::<String>(&mut conn);
            let result: Result<String, redis::RedisError> = match tokio::time::timeout(
                std::time::Duration::from_millis(PUBLISH_ATTEMPT_TIMEOUT_MS),
                cmd_future,
            )
            .await
            {
                Ok(redis_result) => redis_result,
                Err(_elapsed) => Err(redis::RedisError::from((
                    redis::ErrorKind::IoError,
                    "publish timed out",
                ))),
            };

            match result {
                Ok(_) => {
                    if attempt > 0 {
                        metrics::counter!("events_publish_retries_total").increment(attempt as u64);
                        info!(
                            task_id = %task_id,
                            event_type = %event_type,
                            sub_type = %sub_type,
                            attempt = attempt + 1,
                            "Published event to Redis after retry"
                        );
                    }
                    metrics::counter!("events_published_total", "type" => event_type.to_string())
                        .increment(1);
                    debug!(
                        task_id = %task_id,
                        event_type = %event_type,
                        sub_type = %sub_type,
                        "Published event to Redis"
                    );
                    return;
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        // All attempts exhausted
        if let Some(e) = last_err {
            metrics::counter!("events_publish_failures_total", "type" => event_type.to_string())
                .increment(1);
            warn!(
                task_id = %task_id,
                event_type = %event_type,
                sub_type = %sub_type,
                attempts = MAX_RETRIES + 1,
                error = %e,
                "Failed to publish event to Redis after all retries"
            );
        }
    }
}

#[async_trait]
impl EventPublisher for RedisEventPublisher {
    async fn publish_sandbox_event(&self, task_id: &str, sub_type: SandboxSubType, message: &str) {
        let payload = SandboxEventPayload {
            message: message.to_string(),
            ..Default::default()
        };
        self.publish_sandbox_event_with_payload(task_id, sub_type, &payload)
            .await;
    }

    async fn publish_sandbox_event_with_payload(
        &self,
        task_id: &str,
        sub_type: SandboxSubType,
        payload: &SandboxEventPayload,
    ) {
        let payload_value = serde_json::to_value(payload).unwrap_or_default();
        self.publish_event_typed(
            task_id,
            EventType::Sandbox,
            sub_type.into(),
            None,
            None,
            payload_value,
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

    async fn health_check(&self) -> Result<(), String> {
        let mut conn = self.conn.clone();
        let cmd = redis::cmd("PING");
        let ping = cmd.query_async::<String>(&mut conn);
        match tokio::time::timeout(
            std::time::Duration::from_millis(HEALTH_CHECK_TIMEOUT_MS),
            ping,
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("redis ping failed: {e}")),
            Err(_) => Err(format!(
                "redis ping timed out after {HEALTH_CHECK_TIMEOUT_MS}ms"
            )),
        }
    }
}
