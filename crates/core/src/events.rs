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
use moatless_types::agent::AgentSubType;
use moatless_types::event::{EventType, StreamEvent, SubType};
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType};
use moatless_types::task::{TaskSubType, TaskTurnSubType};
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// `RecordingPublisher` and friends: an [`EventPublisher`] test double shared
/// across the workspace's test tiers. Gated so it compiles for this crate's own
/// tests and, off `test-support`, for downstream crates' test builds; never for
/// a shipping build.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

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

/// TTL (seconds) refreshed on each `task-events:{task_id}` stream key after a
/// successful publish. Caps the number of *live* stream keys without touching
/// per-stream `MAXLEN`. Any active task re-publishes well within this window,
/// so only streams for long-dead tasks expire and get reclaimed.
///
/// Sized for the stream's actual job — the live SSE tail. History is served
/// from `task_events` in Postgres (see `sse::routes`), so this only has to
/// outlive a consumer restart, not a task.
const STREAM_KEY_TTL_SECS: u64 = 6 * 60 * 60;

/// Hard cap on the serialized event bytes handed to `XADD`.
///
/// `MAXLEN` bounds a stream's entry *count*, never its bytes, so a handful of
/// fat entries can pin hundreds of MB: a single `agent` message carrying a
/// large tool result has been observed at ~920KB, and the frontend cannot
/// usefully render one anyway. Events over this cap are published with a
/// [`truncated_payload`] stub in place of their payload.
///
/// Safe only where the caller says so: the gRPC ingress durably admits an event
/// *before* publishing it (see the backend's `event_ingress`) and passes
/// `payload_recoverable` when the consumer can recover the full payload from
/// Postgres without this message, so the stub only ever degrades the live tail.
const MAX_REDIS_PAYLOAD_BYTES: usize = 256 * 1024;

/// Stub swapped in for a payload that exceeds [`MAX_REDIS_PAYLOAD_BYTES`].
///
/// Carries enough for a client to recognise the gap and refetch the full event
/// from `GET /api/v1/tasks/{task_id}/events`.
fn truncated_payload(original_bytes: usize) -> serde_json::Value {
    serde_json::json!({
        "truncated": true,
        "originalBytes": original_bytes,
        "reason": "payload exceeds the Redis live-tail cap; fetch the full event from task history",
    })
}

/// Serialize an event for `XADD`, swapping in a [`truncated_payload`] stub when
/// it exceeds [`MAX_REDIS_PAYLOAD_BYTES`] *and* the consumer can recover the
/// full payload without it.
///
/// Returns the JSON plus, when the payload was replaced, the original byte
/// length. Truncating without `pre_persisted` would make the stub the only copy
/// the consumer ever sees, so an oversized non-recoverable event is published
/// whole — the stream is bounded by `MAXLEN`/TTL, never at the cost of content.
fn encode_for_stream(
    event: StreamEvent,
    pre_persisted: bool,
) -> Result<(String, Option<usize>), serde_json::Error> {
    let event_json = serde_json::to_string(&event)?;
    if !pre_persisted || event_json.len() <= MAX_REDIS_PAYLOAD_BYTES {
        return Ok((event_json, None));
    }
    let original_bytes = event_json.len();
    let stub = StreamEvent {
        payload: truncated_payload(original_bytes),
        ..event
    };
    Ok((serde_json::to_string(&stub)?, Some(original_bytes)))
}

/// Capped exponential backoff with full jitter for Redis-consumer reconnects.
///
/// `failures` is the count of consecutive failed cycles (1 on the first
/// failure). The base delay doubles per failure — `500ms → 1s → 2s …` — capped
/// at 30s, then a full-jitter window `[delay/2, delay]` is applied so reconnects
/// across replicas decorrelate instead of hammering a struggling Redis in
/// lockstep.
pub fn reconnect_backoff_delay(failures: u32) -> std::time::Duration {
    use rand::RngExt;
    const BASE_MS: u64 = 500;
    const CAP_MS: u64 = 30_000;
    let exp = failures.saturating_sub(1).min(20);
    let base = BASE_MS.saturating_mul(1u64 << exp).min(CAP_MS);
    let half = base / 2;
    let jittered = half + rand::rng().random_range(0..=half);
    std::time::Duration::from_millis(jittered)
}

/// Shared [`ConnectionManagerConfig`] for every Redis `ConnectionManager` in the
/// backend (publisher + both stream consumers).
///
/// Bounds each Redis round-trip so a vanished/moved server triggers
/// `ConnectionManager` reconnect — and our app-level backoff — within seconds
/// instead of hanging on the OS TCP timeout (~22s during the 2026-06-23
/// Karpenter eviction incident). The response timeout **must exceed** the
/// consumers' `XREADGROUP ... BLOCK 2000` (2s) idle window, or it would fire on
/// every idle block; 5s leaves a 3s margin.
///
/// `pub` (not `pub(crate)`) because the binary crate's event-handler bootstrap in
/// `main.rs` builds its own `ConnectionManager` and needs this same config.
pub fn redis_manager_config() -> ConnectionManagerConfig {
    ConnectionManagerConfig::new()
        .set_connection_timeout(Some(std::time::Duration::from_secs(5)))
        .set_response_timeout(Some(std::time::Duration::from_secs(5)))
}

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
    metrics::describe_counter!(
        "events_payload_truncated_total",
        "Total events published to Redis with a truncated payload stub"
    );
}

/// Why an event did not reach the live transport.
///
/// Publication is a delivery attempt, not an admission decision: the caller
/// owns durability and decides what a failure means. The ingress
/// (the backend's `event_ingress`) answers gRPC success on the strength of its
/// durable write and treats every variant here as "not live yet"; the relay
/// treats one as "try this record again".
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// The transport rejected or timed out every attempt.
    #[error("event transport failed after {attempts} attempts")]
    Transport {
        attempts: usize,
        #[source]
        source: redis::RedisError,
    },

    /// The event could not be encoded for the transport. Retrying the same
    /// event cannot help.
    #[error("failed to serialize event for the transport")]
    Encode(#[source] serde_json::Error),

    /// The event type is not one the transport understands. Retrying the same
    /// event cannot help.
    #[error("unknown event type: {0}")]
    UnknownEventType(String),
}

impl PublishError {
    /// Whether re-publishing the same event could succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transport { .. })
    }
}

/// Trait for publishing lifecycle events.
///
/// Abstracts event publishing so handlers can be tested without Redis.
/// The typed-outcome method is [`EventPublisher::publish_raw_event_with_id`];
/// the rest are fire-and-forget — implementations log errors but never
/// propagate them.
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` in async-trait methods.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish a sandbox lifecycle event with a message.
    async fn publish_sandbox_event(&self, task_id: &str, sub_type: SandboxStatus, message: &str);

    /// Publish a sandbox lifecycle event with a typed payload.
    async fn publish_sandbox_event_with_payload(
        &self,
        task_id: &str,
        sub_type: SandboxStatus,
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

    /// Publish a raw event under a caller-supplied `event_id`, reporting
    /// whether it reached the transport.
    ///
    /// This is the one publication path with a typed outcome, because it is the
    /// one whose caller has already made a durability decision and needs to know
    /// whether the live tail agreed. Used by the gRPC ingress after its durable
    /// admission: the same `event_id` is stamped on the durable record and on
    /// the published event, so the consumer's `ON CONFLICT (event_id) DO
    /// NOTHING` dedups the two into one row (commit-before-fan-out ordering).
    ///
    /// `payload_recoverable` asserts the consumer can recover the full payload
    /// from durable storage without this message. Only then may this path
    /// replace an oversized payload with a truncated stub — otherwise the stub
    /// would be the only copy the consumer ever sees. Pass `false` whenever the
    /// consumer rebuilds state from the published payload itself.
    // reason: mirrors the raw Redis event field set (id + task + type + sub_type
    // + agent + turn + payload); splitting into a struct would only shuffle the
    // same fields behind a one-use type.
    #[allow(clippy::too_many_arguments)]
    async fn publish_raw_event_with_id(
        &self,
        event_id: &str,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
        payload_recoverable: bool,
    ) -> Result<(), PublishError>;

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

        match ConnectionManager::new_with_config(client, redis_manager_config()).await {
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
        // Fire-and-forget: these callers have no durability decision to make,
        // so a failure is logged inside `publish_event_internal` and dropped.
        let _ = self
            .publish_event_internal(
                None,
                task_id,
                event_type.as_str(),
                sub_type.as_str(),
                agent_type,
                turn_number,
                payload,
                false,
            )
            .await;
    }

    /// Internal: publish a generic event to the Redis stream.
    ///
    /// `event_id` lets the caller pin the id (the durable-admission path); when
    /// `None` a fresh UUIDv4 is generated as before.
    ///
    /// `pre_persisted` marks callers whose consumer can recover the full payload
    /// from durable storage. Only those may have an oversized payload replaced
    /// by a stub — for every other path Redis is still the only route to the
    /// consumer, so truncating would silently drop content.
    // reason: one field per Redis event column; a params struct would add a
    // one-use type without reducing the fan-out surface.
    #[allow(clippy::too_many_arguments)]
    async fn publish_event_internal(
        &self,
        event_id: Option<String>,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
        pre_persisted: bool,
    ) -> Result<(), PublishError> {
        let event_id = event_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let is_error = sub_type == "error" || sub_type == "failed";

        let parsed_event_type = match EventType::try_from(event_type) {
            Ok(et) => et,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Unknown event type, skipping publish");
                return Err(PublishError::UnknownEventType(event_type.to_string()));
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

        let (event_json, truncated_from) = match encode_for_stream(event, pre_persisted) {
            Ok(encoded) => encoded,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to serialize event");
                return Err(PublishError::Encode(e));
            }
        };

        if let Some(original_bytes) = truncated_from {
            metrics::counter!(
                "events_payload_truncated_total",
                "type" => event_type.to_string()
            )
            .increment(1);
            warn!(
                task_id = %task_id,
                event_type = %event_type,
                sub_type = %sub_type,
                original_bytes,
                cap_bytes = MAX_REDIS_PAYLOAD_BYTES,
                "Event payload exceeds live-tail cap, publishing truncated stub"
            );
        }

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
            // XADD + EXPIRE travel in one pipelined round-trip. EXPIRE refreshes
            // the stream key's TTL on every publish (sliding window), so a key
            // only expires once its task has been silent for the full TTL —
            // bounding live key count with no extra hot-path round-trip.
            // `.ignore()` drops the EXPIRE reply so the result decodes as the
            // XADD id alone.
            let mut pipe = redis::pipe();
            pipe.cmd("XADD")
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
                .cmd("EXPIRE")
                .arg(&stream_key)
                .arg(STREAM_KEY_TTL_SECS)
                .ignore();

            let cmd_future = pipe.query_async::<(String,)>(&mut conn);
            let result: Result<String, redis::RedisError> = match tokio::time::timeout(
                std::time::Duration::from_millis(PUBLISH_ATTEMPT_TIMEOUT_MS),
                cmd_future,
            )
            .await
            {
                Ok(redis_result) => redis_result.map(|(id,)| id),
                Err(_elapsed) => Err(redis::RedisError::from((
                    redis::ErrorKind::Io,
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
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        // All attempts exhausted. The error is logged once here and its source
        // returned, so a caller that owns durability can classify the failure
        // without re-deriving it from a message.
        let attempts = MAX_RETRIES + 1;
        match last_err {
            Some(e) => {
                metrics::counter!("events_publish_failures_total", "type" => event_type.to_string())
                    .increment(1);
                warn!(
                    task_id = %task_id,
                    event_type = %event_type,
                    sub_type = %sub_type,
                    attempts,
                    error = %e,
                    "Failed to publish event to Redis after all retries"
                );
                Err(PublishError::Transport {
                    attempts,
                    source: e,
                })
            }
            // Unreachable: the loop either returns `Ok` or records an error.
            None => Ok(()),
        }
    }
}

#[async_trait]
impl EventPublisher for RedisEventPublisher {
    async fn publish_sandbox_event(&self, task_id: &str, sub_type: SandboxStatus, message: &str) {
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
        sub_type: SandboxStatus,
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
        let _ = self
            .publish_event_internal(
                None,
                task_id,
                event_type,
                sub_type,
                agent_type,
                turn_number,
                payload,
                false,
            )
            .await;
    }

    async fn publish_raw_event_with_id(
        &self,
        event_id: &str,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_type: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
        payload_recoverable: bool,
    ) -> Result<(), PublishError> {
        self.publish_event_internal(
            Some(event_id.to_string()),
            task_id,
            event_type,
            sub_type,
            agent_type,
            turn_number,
            payload,
            payload_recoverable,
        )
        .await
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

#[cfg(test)]
mod tests {
    use super::{encode_for_stream, reconnect_backoff_delay, MAX_REDIS_PAYLOAD_BYTES};
    use moatless_types::event::{EventType, StreamEvent};

    fn event_with_payload_bytes(n: usize) -> StreamEvent {
        StreamEvent {
            event_id: "evt-1".to_string(),
            event_type: EventType::Agent,
            sub_type: "message".to_string(),
            task_id: "task-1".to_string(),
            timestamp: "2026-07-17T00:00:00.000Z".to_string(),
            is_error: false,
            agent_type: Some("claude-code".to_string()),
            turn_number: Some(1),
            payload: serde_json::json!({ "text": "x".repeat(n) }),
        }
    }

    #[test]
    fn small_payloads_pass_through_untouched() {
        let event = event_with_payload_bytes(128);
        let (json, truncated) = encode_for_stream(event, true).expect("serializes");
        assert!(truncated.is_none());
        assert!(json.contains(&"x".repeat(128)));
    }

    #[test]
    fn oversized_payload_is_truncated_when_already_persisted() {
        let event = event_with_payload_bytes(MAX_REDIS_PAYLOAD_BYTES + 1);
        let (json, truncated) = encode_for_stream(event, true).expect("serializes");

        assert!(truncated.is_some(), "oversized payload should be stubbed");
        assert!(json.len() < MAX_REDIS_PAYLOAD_BYTES, "stub must be small");
        assert!(json.contains("\"truncated\":true"));
        // The envelope survives so clients can dedupe by id and refetch.
        assert!(json.contains("evt-1"));
    }

    #[test]
    fn oversized_payload_survives_intact_when_not_persisted() {
        // Redis is the only route to Postgres here — truncating would silently
        // destroy the event, so the full payload must go out regardless of size.
        let event = event_with_payload_bytes(MAX_REDIS_PAYLOAD_BYTES + 1);
        let (json, truncated) = encode_for_stream(event, false).expect("serializes");

        assert!(truncated.is_none(), "must not stub a non-durable event");
        assert!(json.len() > MAX_REDIS_PAYLOAD_BYTES);
        assert!(!json.contains("\"truncated\":true"));
    }

    #[test]
    fn backoff_first_failure_within_base_jitter_window() {
        // failures=1 → base 500ms, full-jitter window [250ms, 500ms].
        for _ in 0..1000 {
            let ms = reconnect_backoff_delay(1).as_millis() as u64;
            assert!((250..=500).contains(&ms), "got {ms}ms");
        }
    }

    #[test]
    fn backoff_grows_then_caps() {
        // failures=3 → base 2000ms → window [1000, 2000].
        for _ in 0..1000 {
            let ms = reconnect_backoff_delay(3).as_millis() as u64;
            assert!((1000..=2000).contains(&ms), "got {ms}ms");
        }
        // Large failure count caps the base at 30s → window [15s, 30s].
        for _ in 0..1000 {
            let ms = reconnect_backoff_delay(1000).as_millis() as u64;
            assert!((15_000..=30_000).contains(&ms), "got {ms}ms");
        }
    }
}
