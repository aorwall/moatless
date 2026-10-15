use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use tracing::warn;

use crate::task_event::dao::TaskEventDao;
use crate::task_event::models::InsertTaskEvent;
use crate::task_turn::dao::TaskTurnDao;
use crate::task_turn::models::TurnStateEvent;
use moatless_core::events::EventPublisher;

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

/// The durability paths available to admission.
///
/// Each is optional (a process may run without a database); a missing path an
/// event needs is a retryable failure, never a silent ack.
pub struct EventAdmission {
    task_turns: Option<Arc<dyn TaskTurnDao>>,
    task_events: Option<Arc<dyn TaskEventDao>>,
    publisher: Option<Arc<dyn EventPublisher>>,
}

impl EventAdmission {
    pub(crate) fn new(
        task_turns: Option<Arc<dyn TaskTurnDao>>,
        task_events: Option<Arc<dyn TaskEventDao>>,
        publisher: Option<Arc<dyn EventPublisher>>,
    ) -> Self {
        Self {
            task_turns,
            task_events,
            publisher,
        }
    }

    /// Durably admit `request`, then fan it out.
    ///
    /// Returns `Ok` only once the payload is recoverable without the live
    /// transport; fan-out failures after that are logged, not propagated, since
    /// re-delivering an admitted event would duplicate it.
    pub async fn admit(&self, request: AdmissionRequest) -> Result<(), AdmissionError> {
        self.commit(&request).await?;

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
    async fn commit(&self, request: &AdmissionRequest) -> Result<(), AdmissionError> {
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
            return Ok(());
        }

        self.commit_event_row(request).await?;
        metrics::counter!("event_ingress_admitted_total", "kind" => "event_row").increment(1);
        Ok(())
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
            agent_profile: None,
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

/// The sub-types stored as a failure: `error` and `failed` on any event kind,
/// and `recording_stopped`, after which the thread shows nothing more.
pub fn is_error_sub_type(sub_type: &str) -> bool {
    matches!(sub_type, "error" | "failed" | "recording_stopped")
}

fn timestamp_now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
