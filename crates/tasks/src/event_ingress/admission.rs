use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use tracing::warn;

use crate::agent_event::is_agent_message;
use crate::task_event::dao::TaskEventDao;
use crate::task_event::models::InsertTaskEvent;
use crate::task_turn::dao::TaskTurnDao;
use crate::task_turn::models::TurnStateEvent;
use moatless_core::events::EventPublisher;

use super::outbox_dao::{IngressOutbox, IngressPayload, RETRY_BASE_SECS};
use super::relay::IngressRelay;

use super::AdmissionError;

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
pub struct EventAdmission {
    task_turns: Option<Arc<dyn TaskTurnDao>>,
    task_events: Option<Arc<dyn TaskEventDao>>,
    ingress_outbox: Option<Arc<dyn IngressOutbox>>,
    publisher: Option<Arc<dyn EventPublisher>>,
}

impl EventAdmission {
    pub(crate) fn new(
        task_turns: Option<Arc<dyn TaskTurnDao>>,
        task_events: Option<Arc<dyn TaskEventDao>>,
        ingress_outbox: Option<Arc<dyn IngressOutbox>>,
        publisher: Option<Arc<dyn EventPublisher>>,
    ) -> Self {
        Self {
            task_turns,
            task_events,
            ingress_outbox,
            publisher,
        }
    }

    /// Drop the outbox record for `event_id` now that its content is committed
    /// to `task_messages` — see [`IngressOutbox::delete_delivered`], the only
    /// delete in the path. `Ok(false)` when no record was present, and when this
    /// process runs without an outbox.
    pub async fn delivered(&self, event_id: &str) -> Result<bool, sqlx::Error> {
        match self.ingress_outbox.as_ref() {
            Some(outbox) => outbox.delete_delivered(event_id).await,
            None => Ok(false),
        }
    }

    /// The relay that re-publishes admitted payloads whose delivery is
    /// unconfirmed. `None` without an outbox or without a transport: with
    /// neither is there anything to recover.
    pub fn relay(&self) -> Option<IngressRelay> {
        Some(IngressRelay::new(
            self.ingress_outbox.clone()?,
            self.publisher.clone()?,
        ))
    }

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

        if let Some(publisher) = self.publisher.as_ref() {
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
            let outbox = self
                .ingress_outbox
                .as_ref()
                .ok_or(AdmissionError::NotConfigured {
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
            let turns = self
                .task_turns
                .as_ref()
                .ok_or(AdmissionError::NotConfigured {
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
        let events = self
            .task_events
            .as_ref()
            .ok_or(AdmissionError::NotConfigured {
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
pub fn is_error_sub_type(sub_type: &str) -> bool {
    sub_type == "error" || sub_type == "failed"
}

fn timestamp_now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
