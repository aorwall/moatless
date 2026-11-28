//! An event admitted from the sandbox, and the turn state read off it.

use serde_json::Value;
use tonic::{Request, Response, Status};
use tracing::{debug, warn};

use moatless_types::agent::AgentSubType;
use sandbox_proto::{
    InterruptedTurn as ProtoInterruptedTurn, PublishEventRequest, PublishEventResponse,
    RecoverInterruptedRunRequest, RecoverInterruptedRunResponse,
};

use moatless_types::access_control::ScopeContext;

use crate::sandbox::event_dedup::Admission;
use moatless_tasks::event_ingress::{AdmissionError, AdmissionRequest};
use moatless_tasks::task_turn::models::TurnStateEvent;
use moatless_tasks::task_turn::models::classify_turn_transition;

use super::commands::record_running_commands;
use super::state::BackendSandboxServiceImpl;

/// Decode one `PublishEventRequest` into the parts admission needs, rejecting
/// wire-contract violations terminally.
///
/// All three cases (unknown event type, absent event id, non-JSON payload) are
/// unfixable by repetition, so each is [`AdmissionError::Invalid`].
fn decode_request(event: &PublishEventRequest) -> Result<(&'static str, Value), AdmissionError> {
    // `event_type` is an open integer on the wire. Validate it here, at
    // ingress, so an unknown discriminant is rejected instead of defaulting
    // to a variant.
    let event_type = match sandbox_proto::EventType::try_from(event.event_type) {
        Ok(proto_event_type) => moatless_types::event::EventType::from(proto_event_type).as_str(),
        Err(e) => {
            warn!(
                event_type = event.event_type,
                task_id = %event.task_id,
                error = %e,
                "Unknown proto EventType value, rejecting"
            );
            return Err(AdmissionError::Invalid(format!(
                "Unknown event_type: {}",
                event.event_type
            )));
        }
    };

    if event.event_id.is_empty() {
        return Err(AdmissionError::Invalid(
            "event_id is required: retries must reuse a stable, non-empty id".to_string(),
        ));
    }

    // An empty payload is a legitimate "no fields" event; anything else must
    // parse, because an unparseable payload would be admitted as `null` and
    // silently lose whatever the sandbox meant to send.
    let payload = if event.payload_json.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&event.payload_json)
            .map_err(|e| AdmissionError::Invalid(format!("payload_json is not valid JSON: {e}")))?
    };

    Ok((event_type, payload))
}

/// The count an `agent.commands_changed` event carries, or `None` for every
/// other event. See [`moatless_types::agent::AgentSubType::CommandsChanged`].
fn running_commands_reported(event_type: &str, sub_type: &str, payload: &Value) -> Option<i32> {
    if (event_type, sub_type) != ("agent", AgentSubType::CommandsChanged.as_str()) {
        return None;
    }
    // A negative or oversized count is a bug in the publisher, not a state the
    // row should take: clamp into the column's domain rather than write it.
    payload
        .get("runningCommands")
        .and_then(Value::as_i64)
        .map(|n| i32::try_from(n.max(0)).unwrap_or(i32::MAX))
}

impl BackendSandboxServiceImpl {
    /// The turn an inbound event belongs to.
    ///
    /// A run-id resolves to its turn (allocating on first signal), so a
    /// terminal event delayed past the next run's start still closes its own
    /// turn, not whichever is open on arrival.
    pub(super) async fn resolve_turn_for_event(&self, task_id: &str, run_id: &str) -> Option<i32> {
        let turns = &self.ingress.as_ref()?.task_turns;
        if run_id.is_empty() {
            return None;
        }
        match turns.resolve_turn_for_run(task_id, run_id).await {
            Ok(turn_number) => Some(turn_number),
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    run_id = %run_id,
                    error = %e,
                    "Failed to resolve turn for run"
                );
                None
            }
        }
    }

    /// The task's latest turn — the fallback for a state-bearing event that
    /// reached the ingress without a run id.
    async fn latest_turn(&self, task_id: &str) -> Option<i32> {
        self.ingress
            .as_ref()?
            .task_turns
            .latest_turn_number(task_id)
            .await
            .unwrap_or_else(|e| {
                warn!(task_id = %task_id, error = %e, "Failed to read latest turn number");
                None
            })
    }

    /// The task's configured agent profile, for stamping onto turn rows.
    async fn resolve_agent_profile(&self, task_id: &str) -> Option<String> {
        #[expect(
            clippy::disallowed_methods,
            reason = "event ingress attributes turns across all owners"
        )]
        let ctx = ScopeContext::system();
        self.handlers
            .as_ref()?
            .tasks
            .find_by_id(task_id, &ctx)
            .await
            .ok()
            .flatten()
            .map(|task| task.agent_profile.to_string())
    }

    /// Resolve the turn an inbound event belongs to and, for a state-bearing
    /// event, the turn-state write that must commit before fan-out.
    async fn build_admission(
        &self,
        event: &PublishEventRequest,
        event_type: &'static str,
        payload: Value,
    ) -> AdmissionRequest {
        // Resolve the turn for every event published from inside an agent run,
        // not just the state-bearing ones: the dispatcher then receives an
        // explicit turn number on each and never has to guess.
        // Events from outside a run (sandbox/init/heartbeat) bear no turn.
        let mut turn_number = self
            .resolve_turn_for_event(&event.task_id, &event.run_id)
            .await;

        // State-bearing events (taskTurn.*, agent.completed/error/stopped) commit
        // their turn / task-status / event rows in ONE transaction BEFORE the
        // fan-out. Everything else has no state to commit.
        let transition = classify_turn_transition(event_type, &event.sub_type);
        if transition.is_some() && turn_number.is_none() {
            // A state-bearing event that arrived without a run id — a publisher
            // that doesn't front the actor. Attribute it to the open turn.
            turn_number = self.latest_turn(&event.task_id).await;
        }
        let state = if let (Some(transition), Some(turn_number)) = (transition, turn_number) {
            let agent_profile = self.resolve_agent_profile(&event.task_id).await;
            Some(TurnStateEvent {
                event_id: event.event_id.clone(),
                task_id: event.task_id.clone(),
                event_type: event_type.to_string(),
                sub_type: event.sub_type.clone(),
                is_error: moatless_tasks::event_ingress::is_error_sub_type(&event.sub_type),
                agent_profile,
                turn_number,
                payload: payload.clone(),
                timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                transition,
            })
        } else {
            None
        };

        AdmissionRequest {
            event_id: event.event_id.clone(),
            task_id: event.task_id.clone(),
            event_type,
            sub_type: event.sub_type.clone(),
            turn_number,
            payload,
            state,
        }
    }
}

/// Admit an event from the sandbox and fan it out on the task-event feed.
///
/// Anything short of durable admission is a retryable `UNAVAILABLE` resent
/// under the same id (contract on [`AdmissionError`]); uses `authorize`, not
/// `authorize_bootstrap`, so a pre-token-swap event is rejected but not lost —
/// the outbox redelivers everything except `INVALID_ARGUMENT`.
#[tracing::instrument(
    skip_all,
    fields(request_id = %uuid::Uuid::new_v4(), task_id = tracing::field::Empty)
)]
pub(super) async fn publish_event(
    service: &BackendSandboxServiceImpl,
    request: Request<PublishEventRequest>,
) -> Result<Response<PublishEventResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let event = request.into_inner();
    tracing::Span::current().record("task_id", event.task_id.as_str());

    let (event_type_str, payload) =
        decode_request(&event).map_err(|e| e.into_status(&event.task_id, &event.event_id))?;
    // Read before `build_admission` consumes the payload.
    let running_commands = running_commands_reported(event_type_str, &event.sub_type, &payload);

    // Log sequence gaps/resets for observability (never gates admission).
    service
        .event_dedup
        .observe_sequence(&event.task_id, event.sequence)
        .await;

    // Claim admission for this id. A duplicate of an already-admitted event
    // is acknowledged without re-publishing (that is what makes a lost
    // response safe); a duplicate racing an in-flight admission is answered
    // retryable, because the winner's admission may still fail.
    let claim = match service
        .event_dedup
        .claim(&event.task_id, &event.event_id, event.sequence)
        .await
    {
        Admission::Claimed(claim) => claim,
        Admission::AlreadyAdmitted => {
            debug!(
                task_id = %event.task_id,
                event_id = %event.event_id,
                sub_type = %event.sub_type,
                "Duplicate of an admitted event acknowledged at ingress"
            );
            return Ok(Response::new(PublishEventResponse {
                event_id: event.event_id,
            }));
        }
        Admission::InFlight => {
            return Err(
                AdmissionError::AdmissionInFlight.into_status(&event.task_id, &event.event_id)
            );
        }
    };

    let request = service
        .build_admission(&event, event_type_str, payload)
        .await;
    let admission = service.ingress.as_ref().map(|i| i.admission.clone());
    let sandboxes = service.handlers.as_ref().map(|h| h.sandboxes.clone());
    let (task_id, event_id) = (event.task_id.clone(), event.event_id.clone());
    // On `admissions` so the sandbox's 10 s call deadline cannot cancel an
    // admission midway: its retry then finds the event admitted, not still in
    // flight. Everything admission owes happens in here for the same reason.
    let admitted = service.admissions.spawn(async move {
        // Databaseless there is no admission to run, and the refusal takes the
        // same path as a failed one.
        let admitted = match admission {
            Some(admission) => admission.admit(request).await,
            None => Err(AdmissionError::NotConfigured {
                component: "durable admission",
            }),
        };
        match &admitted {
            Ok(()) => {
                // Settled first: the count update can wait on the same
                // contended database, and retries answered `InFlight` meanwhile
                // count toward the sandbox dropping a committed event.
                claim.admitted().await;
                if let (Some(sandboxes), Some(running)) = (sandboxes, running_commands) {
                    record_running_commands(&sandboxes, &task_id, running).await;
                }
            }
            Err(error) => {
                warn!(%task_id, %event_id, %error, "event admission failed; the sandbox retries it");
                // Release the claim so the sandbox's retry is a real second
                // attempt rather than a duplicate of a failed admission.
                claim.failed().await;
            }
        }
        admitted
    });
    let admitted = admitted
        .await
        .map_err(|error| Status::internal(format!("event admission task failed: {error}")))?;

    match admitted {
        Ok(()) => Ok(Response::new(PublishEventResponse {
            event_id: event.event_id,
        })),
        Err(e) => Err(e.into_status(&event.task_id, &event.event_id)),
    }
}

/// Authorize sandbox recovery and project the recorded task-turn decision.
pub(super) async fn recover_interrupted_run(
    service: &BackendSandboxServiceImpl,
    request: Request<RecoverInterruptedRunRequest>,
) -> Result<Response<RecoverInterruptedRunResponse>, Status> {
    // Cross-tenant guard: recovery replays another turn's messages and burns
    // a recovery attempt on the task, so gate it on task access.
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;

    let req = request.into_inner();

    if req.probe_only {
        let turn = moatless_tasks::task_turn::recovery::probe_interrupted_turn(
            service.handlers.as_ref().map(|h| h.tasks.as_ref()),
            service.handlers.as_ref().map(|h| h.messages.as_ref()),
            &req.task_id,
        )
        .await
        .map_err(|error| Status::internal(error.to_string()))?;
        return Ok(Response::new(RecoverInterruptedRunResponse {
            turn: turn.map(|interrupted| proto_interrupted_turn(interrupted, None)),
        }));
    }

    let turn = moatless_tasks::task_turn::recovery::recover_interrupted_turn(
        service.handlers.as_ref().map(|h| h.tasks.as_ref()),
        service.handlers.as_ref().map(|h| h.messages.as_ref()),
        service.ingress.as_ref().map(|i| i.task_turns.as_ref()),
        service.events.as_deref(),
        &req.task_id,
        &service.backend_config.sandbox.default_codex_model,
    )
    .await
    .map_err(|error| Status::internal(error.to_string()))?;
    Ok(Response::new(RecoverInterruptedRunResponse {
        turn: turn.map(|recovered| {
            proto_interrupted_turn(recovered.interrupted, recovered.agent_profile)
        }),
    }))
}

fn proto_interrupted_turn(
    interrupted: moatless_tasks::message::models::InterruptedTurn,
    agent_profile: Option<String>,
) -> ProtoInterruptedTurn {
    ProtoInterruptedTurn {
        session_id: interrupted.session_id,
        turn_number: i64::from(interrupted.turn_number),
        prompt_text: interrupted.prompt_text,
        agent_profile,
        agent_mode: interrupted.agent_mode,
        effort: interrupted.effort,
        run_id: interrupted.run_id,
        model: interrupted.model,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the wire integer → dispatch string contract:
    /// `classify_turn_transition` and the dispatcher key off this exact
    /// string, so a rename in the shared crate cannot silently re-route
    /// ingress.
    #[test]
    fn every_wire_event_type_keeps_its_dispatch_string() {
        for (wire, expected) in [
            (sandbox_proto::EventType::Task, "task"),
            (sandbox_proto::EventType::TaskTurn, "taskTurn"),
            (sandbox_proto::EventType::Sandbox, "sandbox"),
            (sandbox_proto::EventType::Agent, "agent"),
            (sandbox_proto::EventType::Resources, "resources"),
        ] {
            let decoded = sandbox_proto::EventType::try_from(wire as i32)
                .expect("known discriminant decodes");
            assert_eq!(
                moatless_types::event::EventType::from(decoded).as_str(),
                expected,
                "dispatch string changed for {wire:?}"
            );
        }
    }

    /// Ingress rejects unknown discriminants instead of defaulting to a
    /// variant; the handler turns this failure into `InvalidArgument`.
    #[test]
    fn unknown_wire_event_type_integers_do_not_decode() {
        for unknown in [-1, 6, 99] {
            assert!(sandbox_proto::EventType::try_from(unknown).is_err());
        }
    }

    // publish_event request decoding — the terminal half of the retry contract

    use moatless_tasks::event_ingress::AdmissionError;
    use moatless_tasks::task_turn::models::TurnStatus;

    fn request(event_type: i32, event_id: &str, payload_json: &str) -> PublishEventRequest {
        PublishEventRequest {
            task_id: "task-1".to_string(),
            event_type,
            sub_type: "message".to_string(),
            payload_json: payload_json.to_string(),
            event_id: event_id.to_string(),
            sequence: 1,
            run_id: String::new(),
        }
    }

    #[test]
    fn only_a_commands_changed_event_reports_a_running_count() {
        let payload = serde_json::json!({ "runningCommands": 3 });

        assert_eq!(
            running_commands_reported("agent", "commands_changed", &payload),
            Some(3)
        );
        // Same payload shape under any other key says nothing: the count is
        // written straight onto the Sandbox row, so a stray reader would let an
        // unrelated event overwrite it.
        assert_eq!(
            running_commands_reported("agent", "message", &payload),
            None
        );
        assert_eq!(
            running_commands_reported("sandbox", "commands_changed", &payload),
            None
        );
    }

    #[test]
    fn a_commands_changed_event_without_a_usable_count_writes_nothing() {
        for payload in [
            serde_json::json!({}),
            serde_json::json!({ "runningCommands": "two" }),
            serde_json::json!({ "runningCommands": null }),
        ] {
            assert_eq!(
                running_commands_reported("agent", "commands_changed", &payload),
                None,
                "payload {payload} must leave the recorded count alone"
            );
        }
    }

    #[test]
    fn a_count_outside_the_columns_domain_is_clamped_rather_than_written() {
        assert_eq!(
            running_commands_reported(
                "agent",
                "commands_changed",
                &serde_json::json!({
                    "runningCommands": -1
                })
            ),
            Some(0)
        );
        assert_eq!(
            running_commands_reported(
                "agent",
                "commands_changed",
                &serde_json::json!({
                    "runningCommands": i64::MAX
                })
            ),
            Some(i32::MAX)
        );
    }

    #[test]
    fn decodes_a_well_formed_request() {
        let (event_type, payload) = decode_request(&request(
            sandbox_proto::EventType::Agent as i32,
            "evt-1",
            r#"{"uuid":"msg-1"}"#,
        ))
        .expect("decodes");
        assert_eq!(event_type, "agent");
        assert_eq!(payload["uuid"], "msg-1");
    }

    #[test]
    fn an_empty_payload_is_a_legitimate_no_field_event() {
        let (_, payload) = decode_request(&request(
            sandbox_proto::EventType::Sandbox as i32,
            "evt-1",
            "",
        ))
        .expect("decodes");
        assert_eq!(payload, Value::Null);
    }

    #[test]
    fn wire_contract_violations_are_terminal() {
        // An unknown discriminant, a missing event id, and an unparseable
        // payload can never be admitted — repeating them cannot help, so each
        // must be terminal rather than a retry loop.
        for (case, request) in [
            ("unknown event type", request(42, "evt-1", "{}")),
            (
                "empty event id",
                request(sandbox_proto::EventType::Agent as i32, "", "{}"),
            ),
            (
                "malformed payload",
                request(sandbox_proto::EventType::Agent as i32, "evt-1", "not json"),
            ),
        ] {
            let error = decode_request(&request).expect_err(case);
            assert!(!error.is_retryable(), "{case} must be terminal");
            assert!(matches!(error, AdmissionError::Invalid(_)), "{case}");
            assert_eq!(
                error.into_status("task-1", "evt-1").code(),
                tonic::Code::InvalidArgument,
                "{case}"
            );
        }
    }

    #[test]
    fn terminal_status_str_roundtrips() {
        // Guards the DB status strings the projection writes.
        assert_eq!(TurnStatus::Completed.as_str(), "completed");
        assert_eq!(TurnStatus::Error.as_str(), "error");
        assert_eq!(TurnStatus::Stopped.as_str(), "stopped");
    }
}
