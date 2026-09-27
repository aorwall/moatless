//! Publishes task lifecycle events onto the durable change feed that the SSE
//! fan-out and the event handler both read.
//!
//! One row per event, carrying the serialized [`StreamEvent`] whole at a
//! position allocated in the same transaction — see
//! [`crate::change_feed::events`] for why the position and not the row id is
//! what a reader may poll.

use crate::change_feed::events as feed;
use async_trait::async_trait;
use moatless_types::agent::AgentSubType;
use moatless_types::event::{EventType, StreamEvent, SubType};
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use moatless_types::task::{TaskSubType, TaskTurnSubType};
use sqlx::PgPool;
use tracing::{debug, warn};
use uuid::Uuid;

/// `RecordingPublisher` and friends: [`EventPublisher`] test doubles.
///
/// Gated to this crate's own tests and, via `test-support`, downstream test
/// builds only.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// Register Prometheus metric descriptions for event publishing counters.
///
/// Call once at startup after the Prometheus recorder is installed.
pub fn describe_event_metrics() {
    metrics::describe_counter!(
        "events_published_total",
        "Total events successfully recorded on the event change feed"
    );
    metrics::describe_counter!(
        "events_publish_failures_total",
        "Total events that failed to reach the event change feed"
    );
    metrics::describe_counter!(
        "events_publish_deduped_total",
        "Total publishes dropped because the event id was already recorded"
    );
}

/// Why an event did not reach the live transport.
///
/// Publication is a delivery attempt, not an admission decision — the caller
/// owns durability.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// The write failed. Another attempt with the same event may succeed.
    #[error("event transport failed")]
    Transport {
        #[source]
        source: sqlx::Error,
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
/// All methods except [`EventPublisher::publish_raw_event_with_id`] are
/// fire-and-forget: errors are logged, never propagated.
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

    /// Publish a task lifecycle event (e.g. "open", "closed").
    async fn publish_task_event(
        &self,
        task_id: &str,
        sub_type: TaskSubType,
        agent_profile: Option<&str>,
        payload: serde_json::Value,
    );

    /// Publish an agent lifecycle event (e.g. "completed", "error", "started",
    /// "stopped").
    async fn publish_agent_event(
        &self,
        task_id: &str,
        sub_type: AgentSubType,
        agent_profile: Option<&str>,
        payload: serde_json::Value,
    );

    /// Publish a task turn event (e.g. "active", "completed", "error").
    async fn publish_task_turn_event(
        &self,
        task_id: &str,
        sub_type: TaskTurnSubType,
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    );

    /// Publish a raw event with string types — used by the gRPC server to
    /// forward events from the sandbox without re-parsing `sub_types`.
    async fn publish_raw_event(
        &self,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    );

    /// Publish a raw event under `event_id`, reporting delivery success.
    ///
    /// `event_id` must match the durable record's: the feed dedups on it, which
    /// is what keeps a relay re-publish from reaching clients twice.
    // reason: mirrors the event envelope's own field set (id + task + type +
    // sub_type + agent + turn + payload); splitting into a struct would only
    // shuffle the same fields behind a one-use type.
    #[allow(clippy::too_many_arguments)]
    async fn publish_raw_event_with_id(
        &self,
        event_id: &str,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) -> Result<(), PublishError>;
}

/// Build the envelope a publish records, or say why this event has none.
///
/// `is_error` and `agent_working` are derived here rather than passed, so every
/// producer agrees on them.
fn build_envelope(
    event_id: String,
    task_id: &str,
    event_type: &str,
    sub_type: &str,
    agent_profile: Option<&str>,
    turn_number: Option<i32>,
    payload: serde_json::Value,
) -> Result<StreamEvent, PublishError> {
    let parsed_event_type = EventType::try_from(event_type).map_err(|e| {
        warn!(task_id = %task_id, error = %e, "Unknown event type, skipping publish");
        PublishError::UnknownEventType(event_type.to_string())
    })?;
    Ok(StreamEvent {
        event_id,
        event_type: parsed_event_type,
        sub_type: sub_type.to_string(),
        task_id: task_id.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        is_error: sub_type == "error" || sub_type == "failed",
        agent_profile: agent_profile.map(std::string::ToString::to_string),
        turn_number,
        agent_working: moatless_types::event::agent_working(event_type, sub_type),
        payload,
    })
}

/// Change-feed implementation of [`EventPublisher`].
#[derive(Clone)]
pub struct FeedEventPublisher {
    pool: PgPool,
}

impl FeedEventPublisher {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Internal: typed event publish.
    async fn publish_event_typed(
        &self,
        task_id: &str,
        event_type: EventType,
        sub_type: SubType,
        agent_profile: Option<&str>,
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
                agent_profile,
                turn_number,
                payload,
            )
            .await;
    }

    /// Internal: record a generic event on the change feed.
    ///
    /// `event_id` pins the id; `None` generates a UUIDv4, which can never
    /// collide and so can never dedup.
    // reason: one argument per envelope field; a params struct would add a
    // one-use type without reducing the fan-out surface.
    #[allow(clippy::too_many_arguments)]
    async fn publish_event_internal(
        &self,
        event_id: Option<String>,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) -> Result<(), PublishError> {
        let event = build_envelope(
            event_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            task_id,
            event_type,
            sub_type,
            agent_profile,
            turn_number,
            payload,
        )?;
        let event_json = serde_json::to_string(&event).map_err(|e| {
            warn!(task_id = %task_id, error = %e, "Failed to serialize event");
            PublishError::Encode(e)
        })?;

        match self.record(&event.event_id, task_id, &event_json).await {
            Ok(Some(position)) => {
                metrics::counter!("events_published_total", "type" => event_type.to_string())
                    .increment(1);
                debug!(
                    task_id = %task_id,
                    event_type = %event_type,
                    sub_type = %sub_type,
                    position,
                    "Recorded event on the change feed"
                );
                Ok(())
            }
            // Already recorded, so it already reached both readers. The relay
            // re-publishing an unconfirmed payload is the expected source.
            Ok(None) => {
                metrics::counter!("events_publish_deduped_total", "type" => event_type.to_string())
                    .increment(1);
                debug!(
                    task_id = %task_id,
                    event_id = %event.event_id,
                    "Event id already recorded, skipping duplicate publish"
                );
                Ok(())
            }
            Err(source) => {
                metrics::counter!("events_publish_failures_total", "type" => event_type.to_string())
                    .increment(1);
                warn!(
                    task_id = %task_id,
                    event_type = %event_type,
                    sub_type = %sub_type,
                    error = %source,
                    "Failed to record event on the change feed"
                );
                Err(PublishError::Transport { source })
            }
        }
    }

    /// Allocate a position and write the record under it, in one transaction.
    async fn record(
        &self,
        event_id: &str,
        task_id: &str,
        event_json: &str,
    ) -> Result<Option<i64>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let position = feed::record_event(&mut tx, event_id, task_id, event_json).await?;
        tx.commit().await?;
        Ok(position)
    }
}

#[async_trait]
impl EventPublisher for FeedEventPublisher {
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

    async fn publish_task_event(
        &self,
        task_id: &str,
        sub_type: TaskSubType,
        agent_profile: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.publish_event_typed(
            task_id,
            EventType::Task,
            sub_type.into(),
            agent_profile,
            None,
            payload,
        )
        .await;
    }

    async fn publish_agent_event(
        &self,
        task_id: &str,
        sub_type: AgentSubType,
        agent_profile: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.publish_event_typed(
            task_id,
            EventType::Agent,
            sub_type.into(),
            agent_profile,
            None,
            payload,
        )
        .await;
    }

    async fn publish_task_turn_event(
        &self,
        task_id: &str,
        sub_type: TaskTurnSubType,
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        self.publish_event_typed(
            task_id,
            EventType::TaskTurn,
            sub_type.into(),
            agent_profile,
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
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) {
        let _ = self
            .publish_event_internal(
                None,
                task_id,
                event_type,
                sub_type,
                agent_profile,
                turn_number,
                payload,
            )
            .await;
    }

    async fn publish_raw_event_with_id(
        &self,
        event_id: &str,
        task_id: &str,
        event_type: &str,
        sub_type: &str,
        agent_profile: Option<&str>,
        turn_number: Option<i32>,
        payload: serde_json::Value,
    ) -> Result<(), PublishError> {
        self.publish_event_internal(
            Some(event_id.to_string()),
            task_id,
            event_type,
            sub_type,
            agent_profile,
            turn_number,
            payload,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{PublishError, build_envelope};
    use moatless_types::event::EventType;

    fn envelope(
        event_type: &str,
        sub_type: &str,
    ) -> Result<moatless_types::event::StreamEvent, PublishError> {
        build_envelope(
            "evt-1".to_string(),
            "task-1",
            event_type,
            sub_type,
            Some("claude-code"),
            Some(1),
            serde_json::json!({ "text": "hi" }),
        )
    }

    #[test]
    fn an_envelope_carries_the_producers_fields_through_untouched() {
        let event = envelope("agent", "started").expect("known event type");
        assert_eq!(event.event_id, "evt-1");
        assert_eq!(event.task_id, "task-1");
        assert_eq!(event.event_type, EventType::Agent);
        assert_eq!(event.sub_type, "started");
        assert_eq!(event.agent_profile.as_deref(), Some("claude-code"));
        assert_eq!(event.turn_number, Some(1));
        assert_eq!(event.payload["text"], "hi");
    }

    /// Derived rather than passed, so no producer can disagree about them.
    #[test]
    fn the_error_flag_follows_the_sub_type_and_nothing_else() {
        assert!(!envelope("agent", "started").expect("known").is_error);
        assert!(envelope("agent", "error").expect("known").is_error);
        assert!(envelope("sandbox", "failed").expect("known").is_error);
    }

    #[test]
    fn an_unknown_event_type_is_refused_rather_than_recorded() {
        let error = envelope("nonsense", "message").expect_err("unknown event type");
        assert!(matches!(error, PublishError::UnknownEventType(ref t) if t == "nonsense"));
        assert!(!error.is_retryable(), "re-publishing cannot fix the type");
    }
}
