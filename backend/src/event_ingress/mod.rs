//! Durable admission of sandbox-published events (the durable-ingress decision).
//!
//! The backend acknowledges a sandbox event only after the **complete dispatch
//! payload** has reached the durability that event kind requires. Everything in
//! this module exists to make "gRPC success" mean exactly that.
//!
//! # Admission semantics
//!
//! | event kind | durable admission | why that is enough |
//! |---|---|---|
//! | state-bearing (`taskTurn.*`, `agent.completed`/`error`/`stopped`) | `apply_ingress_event` commits the event row, turn projection and task-status flip in one transaction | the state the event carries is committed before any fan-out |
//! | `agent.message` | the full payload is written to the [transactional ingress outbox](outbox) | `task_events` holds no Message payload or skeleton, so the outbox is its only durable home until the conversation commits |
//! | everything else | the full payload is committed to `task_events` | `task_events` is the event history; Redis is a disposable live tail |
//!
//! Publication to Redis happens **after** durable admission and never gates it:
//! a failed publish leaves an already-durable payload that the
//! [relay](relay::IngressRelay) (for `agent.message`) or the committed row (for
//! everything else) still accounts for. What *does* fail admission is a missing
//! or failing durability path, and an event transport that is disabled when the
//! payload has no durable substitute without it — see [`AdmissionError`].
//! Consumer *presence* is deliberately not part of that predicate; see the note
//! in [`EventAdmission::admit`] for why it cannot be checked here and where a
//! consumer-less deployment surfaces instead.
//!
//! # Terminology
//!
//! "Transactional outbox" in this codebase names exactly one thing: the durable
//! payload-and-relay path in [`outbox`] + [`relay`]. The ordering guarantee that
//! commits state before publishing is *commit-before-fan-out*, not an outbox.

pub mod outbox;
pub mod relay;

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use tonic::Status;
use tracing::warn;

use crate::event_handler::skeleton::is_agent_message;
use crate::events::EventPublisher;
use crate::task_event::dao::{InsertTaskEvent, TaskEventDao};
use crate::task_turn::dao::{TaskTurnDao, TurnStateEvent};

use outbox::{IngressOutbox, IngressPayload, RETRY_BASE_SECS};

/// Why the backend refused to admit an event — and, decisively, whether the
/// sandbox should send it again.
///
/// # Retry contract
///
/// This type is the **only** place `publish_event` builds a gRPC status, so the
/// sandbox's retry policy can be stated in one table:
///
/// | gRPC status | meaning | sandbox action |
/// |---|---|---|
/// | `OK` | the complete payload is durably admitted | drop the event; never resend |
/// | `UNAVAILABLE` | nothing was admitted | resend with the **same** `event_id` and `sequence` |
/// | `INVALID_ARGUMENT` | the request violates the wire contract | terminal; resending cannot succeed |
///
/// Every variant except [`AdmissionError::Invalid`] is retryable, so a new
/// failure mode is retryable unless it is deliberately made terminal. Callers
/// classify with [`AdmissionError::is_retryable`] rather than by matching
/// variants.
#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    /// Terminal: the request cannot be admitted as sent, no matter how often it
    /// is repeated. The message is backend-authored and safe to return.
    #[error("{0}")]
    Invalid(String),

    /// Retryable: a durability path this event kind requires is not configured.
    #[error("{component} is not configured")]
    NotConfigured { component: &'static str },

    /// Retryable: the durable write failed.
    #[error("failed to commit {what}")]
    CommitFailed {
        what: &'static str,
        #[source]
        source: sqlx::Error,
    },

    /// Retryable: this event's content only reaches its canonical table through
    /// the consumer path, and no event transport is configured — the outbox
    /// would spool a payload nothing can ever deliver.
    #[error("event transport is disabled and {what} has no durable substitute")]
    NoDeliveryPath { what: &'static str },

    /// Retryable: a concurrent request already owns admission for this event
    /// id. Answering `OK` would acknowledge an admission that may still fail;
    /// the retry converges on the winner's outcome.
    #[error("admission for this event id is already in flight")]
    AdmissionInFlight,
}

impl AdmissionError {
    /// Whether the sandbox should resend this event under the same id.
    pub fn is_retryable(&self) -> bool {
        !matches!(self, Self::Invalid(_))
    }

    /// The gRPC status for this failure, logging the causal chain exactly once.
    ///
    /// Redaction happens here, at the transport boundary: retryable failures
    /// collapse to one stable message so no SQL, connection or credential
    /// detail crosses the wire, while the full source chain is logged.
    pub fn into_status(self, task_id: &str, event_id: &str) -> Status {
        match self {
            Self::Invalid(message) => Status::invalid_argument(message),
            retryable => {
                warn!(
                    task_id,
                    event_id,
                    error = %retryable,
                    source = ?std::error::Error::source(&retryable),
                    "event not durably admitted; answering retryable"
                );
                Status::unavailable("event not durably admitted; retry with the same event_id")
            }
        }
    }
}

/// One decoded sandbox event, ready for admission.
pub struct AdmissionRequest {
    pub event_id: String,
    pub task_id: String,
    /// Dispatch string derived from the wire enum (never a hand-rolled table).
    pub event_type: &'static str,
    pub sub_type: String,
    pub turn_number: Option<i32>,
    pub payload: Value,
    /// Present when the event bears turn state, which commits transactionally
    /// before fan-out.
    pub state: Option<TurnStateEvent>,
}

/// The durability paths available to admission. Each is optional because the
/// backend runs without a database or without Redis in some deployments; a
/// missing path an event *needs* is a retryable failure, never a silent ack.
pub struct EventAdmission<'a> {
    pub task_turns: Option<&'a Arc<dyn TaskTurnDao>>,
    pub task_events: Option<&'a Arc<dyn TaskEventDao>>,
    pub ingress_outbox: Option<&'a Arc<dyn IngressOutbox>>,
    pub publisher: Option<&'a Arc<dyn EventPublisher>>,
}

impl EventAdmission<'_> {
    /// Durably admit `request`, then fan it out.
    ///
    /// Returns `Ok` only once the complete payload is recoverable without the
    /// live transport. Fan-out failures after that point are logged, not
    /// propagated: re-delivering an admitted event would duplicate it, and the
    /// relay already owns re-publication of the one payload kind that needs it.
    pub async fn admit(&self, request: AdmissionRequest) -> Result<(), AdmissionError> {
        let needs_consumer = is_agent_message(request.event_type, &request.sub_type);

        // Refuse before writing anything: spooling a payload whose only
        // delivery path is switched off would acknowledge permanent loss.
        //
        // The predicate is the *transport*, not the consumer, because the
        // transport is the only leg of the path this process owns. Whether a
        // consumer exists is a deployment property: the consumer group is the
        // shared constant `event-handler`, so `EVENT_HANDLER_ENABLED=false`
        // means "not in this process" — it is the shadow-backend flag, and a
        // shadow shares the primary's Redis and Postgres precisely so the
        // primary consumes and commits on its behalf. Rejecting here would
        // refuse admission in the one configuration the flag exists to support.
        //
        // A deployment where *nothing* consumes is a misconfiguration this
        // process cannot detect at admission time. It surfaces instead as
        // records the relay keeps re-publishing: the
        // `event_ingress_outbox_stalled` gauge and its warning in
        // `relay::IngressRelay::report_pending`.
        if needs_consumer && self.publisher.is_none() {
            return Err(AdmissionError::NoDeliveryPath {
                what: "full agent.message content",
            });
        }

        let payload_recoverable = self.commit(&request, needs_consumer).await?;

        if let Some(publisher) = self.publisher {
            if let Err(e) = publisher
                .publish_raw_event_with_id(
                    &request.event_id,
                    &request.task_id,
                    request.event_type,
                    &request.sub_type,
                    None,
                    request.turn_number,
                    request.payload,
                    payload_recoverable,
                )
                .await
            {
                // Admitted but not live: the durable copy is the recovery path.
                warn!(
                    task_id = %request.task_id,
                    event_id = %request.event_id,
                    error = %e,
                    "admitted event failed to reach the live transport",
                );
            }
        }

        Ok(())
    }

    /// Perform the durable write this event kind requires.
    ///
    /// Returns whether the *published* payload is recoverable from durable
    /// storage by the consumer — the signal that lets the publisher swap an
    /// oversized payload for a truncated stub. `agent.message` is durable in the
    /// outbox but the consumer rebuilds `task_messages` from the published
    /// payload, so its wire copy must stay whole.
    async fn commit(
        &self,
        request: &AdmissionRequest,
        needs_consumer: bool,
    ) -> Result<bool, AdmissionError> {
        if needs_consumer {
            // An `agent.message` has one durable home on the way in: the
            // outbox. After the consumer commits, the conversation is the
            // canonical store. The outbox record alone is what gRPC success
            // speaks for.
            let outbox = self.ingress_outbox.ok_or(AdmissionError::NotConfigured {
                component: "ingress outbox",
            })?;
            let payload = IngressPayload {
                event_id: request.event_id.clone(),
                task_id: request.task_id.clone(),
                event_type: request.event_type.to_string(),
                sub_type: request.sub_type.clone(),
                turn_number: request.turn_number,
                payload: request.payload.clone(),
                timestamp: timestamp_now(),
            };
            let first_attempt_at = Utc::now() + chrono::Duration::seconds(RETRY_BASE_SECS.into());
            outbox
                .admit(&payload, first_attempt_at)
                .await
                .map_err(|source| AdmissionError::CommitFailed {
                    what: "the ingress outbox record",
                    source,
                })?;
            metrics::counter!("event_ingress_admitted_total", "kind" => "outbox").increment(1);
            return Ok(false);
        }

        if let Some(state) = request.state.as_ref() {
            let turns = self.task_turns.ok_or(AdmissionError::NotConfigured {
                component: "task turn repository",
            })?;
            turns.apply_ingress_event(state).await.map_err(|source| {
                AdmissionError::CommitFailed {
                    what: "turn state",
                    source,
                }
            })?;
            metrics::counter!("event_ingress_admitted_total", "kind" => "state").increment(1);
            return Ok(true);
        }

        self.commit_event_row(request).await?;
        metrics::counter!("event_ingress_admitted_total", "kind" => "event_row").increment(1);
        Ok(true)
    }

    /// Commit this event's `task_events` row, carrying its payload whole.
    async fn commit_event_row(&self, request: &AdmissionRequest) -> Result<(), AdmissionError> {
        let events = self.task_events.ok_or(AdmissionError::NotConfigured {
            component: "task event repository",
        })?;
        let insert = InsertTaskEvent {
            event_id: request.event_id.clone(),
            task_id: request.task_id.clone(),
            event_type: request.event_type.to_string(),
            sub_type: request.sub_type.clone(),
            is_error: is_error_sub_type(&request.sub_type),
            agent_type: None,
            turn_number: request.turn_number,
            payload: request.payload.clone(),
            timestamp: timestamp_now(),
        };
        events
            .insert_event(&insert)
            .await
            .map_err(|source| AdmissionError::CommitFailed {
                what: "the event row",
                source,
            })
    }
}

/// The two sub-types every event kind treats as a failure.
pub(crate) fn is_error_sub_type(sub_type: &str) -> bool {
    sub_type == "error" || sub_type == "failed"
}

fn timestamp_now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Register Prometheus metric descriptions for event admission and the relay.
pub fn describe_ingress_metrics() {
    metrics::describe_counter!(
        "event_ingress_admitted_total",
        "Sandbox events durably admitted, by durability kind"
    );
    metrics::describe_counter!(
        "event_ingress_relay_republished_total",
        "Ingress outbox records republished to the consumer path by the relay"
    );
    metrics::describe_counter!(
        "event_ingress_relay_failures_total",
        "Ingress outbox records the relay failed to republish"
    );
    metrics::describe_counter!(
        "event_ingress_delivered_total",
        "Ingress outbox records deleted after their consumer committed them"
    );
    metrics::describe_gauge!(
        "event_ingress_outbox_pending",
        "Ingress outbox records awaiting confirmed delivery"
    );
    metrics::describe_gauge!(
        "event_ingress_outbox_stalled",
        "Ingress outbox records undelivered for far longer than one retry"
    );
}

#[cfg(test)]
mod tests;
