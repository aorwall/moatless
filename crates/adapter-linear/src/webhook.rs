//! One Linear envelope in, a [`Plan`] out. Nothing else.
//!
//! This module is pure: no database, no Linear call, no admission. Everything
//! it decides it decides from the request body, which is what makes the whole
//! `type` × `action` matrix — five event types, every payload-shape guard, and
//! each of the seven ignore reasons — reachable from a table-driven test with
//! no `AppState` and no HTTP request.
//!
//! What it produces is a description, not an action:
//!
//! - **Requests.** One envelope fans into one or more [`AdapterDeliveryRequest`]s,
//!   each already carrying the delivery identity [`ingress`](super::ingress)
//!   derived. Linear sends one kind per envelope today, so the vector holds one
//!   element; it is a vector because the shape of an ingress is "this envelope
//!   means this work", and Linear's `Comment.create` fan-out already proves an
//!   envelope can mean more than one thing downstream.
//! - **Effects.** The one thing that has to happen on the response path, applied
//!   by `ingress` and never here. See [`Effect`].
//! - **Requirements.** The one gate that needs I/O to settle. See
//!   [`Requirement`].
//!
//! Everything a delivery is *rejected* for is an [`ignore reason`](super::ingress::reason)
//! rather than an error, with the single exception of a delivery that carries
//! work but no identity — see [`IngressError::Unidentifiable`].

use moatless_adapter_model::delivery::AdapterDeliveryRequest;

use super::ingress::{IngressError, reason};
use super::webhook_payloads::LinearWebhookEnvelope;

/// What one envelope means.
pub(super) enum Plan {
    Admit {
        /// The admission requests, each carrying the identity `ingress` derived.
        requests: Vec<AdapterDeliveryRequest>,
        /// Applied by `ingress` before it admits, never by [`normalize`].
        effects: Vec<Effect>,
        /// Settled by `ingress` before the effects are applied. `None` for every
        /// event type but a created session.
        requires: Option<Requirement>,
    },
    /// The delivery carries no work this Adapter acts on. Acknowledged, and
    /// metered `ignored` with this reason.
    Ignored { reason: &'static str },
}

/// The one thing that must happen before the `200`, because Linear's panel goes
/// dark after ten seconds.
///
/// It is described here and applied by [`ingress::receive`](super::ingress::receive)
/// so that `normalize` stays drivable without a Linear client, and so that the
/// exception to "nothing reaches Linear before the acknowledgment" is a value
/// that can be asserted rather than a call buried in a workflow. Same shape as
/// Slack's notice.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Effect {
    SessionHeartbeat {
        session_id: String,
        text: &'static str,
    },
}

/// A precondition on the admission that only I/O can settle.
///
/// A created session is the one event whose admission is gated on state
/// `normalize` cannot see. The gate is not moved by this refactor: the
/// Subscription lookup runs before the heartbeat, exactly where it always did.
/// An unmet requirement is [`reason::NO_SUBSCRIPTION`]; rather than the
/// heartbeat, the session is closed with a terminal `error` activity so its
/// panel doesn't spin forever behind an admission that never happens.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Requirement {
    /// At least one active Subscription bound to this Linear team.
    ActiveSubscription { team_id: String },
}

/// Ephemeral thought posted to a freshly created session's panel, before
/// anything slow happens. Ephemeral so the next activity replaces it in
/// Linear's UI rather than piling up.
const SESSION_CREATED_HEARTBEAT: &str = "Spinning up sandbox…";

/// Turn one parsed envelope into the work it carries.
///
/// `event_id` is the delivery identity — see [`super::ingress::dedup_key`] for
/// why it is that string and not a new one. It becomes the `event_id` of every
/// request below, so a Linear retry of one envelope collapses onto the records
/// its first delivery admitted.
pub(super) fn normalize(
    envelope: &LinearWebhookEnvelope,
    connection_id: &str,
    event_id: &str,
) -> Result<Plan, IngressError> {
    match envelope.event_type.as_str() {
        "AgentSessionEvent" => agent_session_event(envelope, connection_id, event_id),
        "Issue" => issue_event(envelope, connection_id, event_id),
        "Comment" => comment_event(envelope, connection_id, event_id),
        "AppUserNotification" => notification_event(envelope, connection_id, event_id),
        _ => Ok(Plan::Ignored {
            reason: reason::UNSUPPORTED_EVENT,
        }),
    }
}

/// A created session, or a prompt typed into one that already exists.
fn agent_session_event(
    envelope: &LinearWebhookEnvelope,
    connection_id: &str,
    event_id: &str,
) -> Result<Plan, IngressError> {
    let Some(session) = envelope.agent_session.as_ref() else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    match envelope.action.as_str() {
        // The one kind that touches Linear before answering, and it is
        // deliberate: the heartbeat keeps the session's panel alive inside
        // Linear's 10-second window. The provisioning behind it is admitted,
        // not run.
        "created" => {
            let Some(issue) = session.issue.as_ref() else {
                return Ok(Plan::Ignored {
                    reason: reason::MALFORMED_PAYLOAD,
                });
            };
            let team_id = issue
                .team_id
                .clone()
                .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
                .unwrap_or_default();
            Ok(Plan::Admit {
                requests: vec![super::delivery::session_provision_request(
                    event_id,
                    connection_id,
                    session,
                )?],
                effects: vec![Effect::SessionHeartbeat {
                    session_id: session.id.clone(),
                    text: SESSION_CREATED_HEARTBEAT,
                }],
                requires: Some(Requirement::ActiveSubscription { team_id }),
            })
        }
        "prompted" => {
            let Some(activity) = envelope
                .agent_activity
                .as_ref()
                .filter(|a| !a.body.is_empty())
            else {
                return Ok(Plan::Ignored {
                    reason: reason::EMPTY_BODY,
                });
            };
            Ok(admit(super::delivery::session_prompt_request(
                event_id,
                connection_id,
                &session.id,
                &activity.id,
                &activity.body,
            )?))
        }
        _ => Ok(Plan::Ignored {
            reason: reason::UNSUPPORTED_ACTION,
        }),
    }
}

/// An issue state change.
///
/// The guards are the ones this path always had, minus the rule lookup: which
/// rules are active is re-resolved when the record executes, so an operator
/// disabling one between the delivery and its execution stops the work.
fn issue_event(
    envelope: &LinearWebhookEnvelope,
    connection_id: &str,
    event_id: &str,
) -> Result<Plan, IngressError> {
    // Only act on state updates.
    if envelope.action != "update" {
        return Ok(Plan::Ignored {
            reason: reason::UNSUPPORTED_ACTION,
        });
    }

    let Some(data) = envelope.data.as_ref() else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    let Ok(issue) = serde_json::from_value::<super::webhook_payloads::IssuePayload>(data.clone())
    else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    if issue.resolved_team_id().is_none() {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    }

    // The issue-state notification is deduped on the semantic transition, not
    // the webhook attempt. Linear can deliver the same issue mutation twice
    // with different webhook timestamps, so `updatedAt` is the preferred clock
    // and `webhookTimestamp` is only the fallback.
    let Some(transition_id) = super::ingress::issue_state_transition_id(envelope) else {
        return Err(IngressError::Unidentifiable(
            "updatedFrom.stateId, data.state.id, and updatedAt or webhookTimestamp",
        ));
    };

    Ok(admit(super::delivery::issue_update_request(
        event_id,
        connection_id,
        &issue,
        envelope.updated_from.as_ref(),
        &transition_id,
        envelope.webhook_timestamp.unwrap_or(0),
    )?))
}

/// A comment on an issue.
fn comment_event(
    envelope: &LinearWebhookEnvelope,
    connection_id: &str,
    event_id: &str,
) -> Result<Plan, IngressError> {
    if envelope.action != "create" {
        return Ok(Plan::Ignored {
            reason: reason::UNSUPPORTED_ACTION,
        });
    }

    let Some(data) = envelope.data.as_ref() else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    let Ok(comment) =
        serde_json::from_value::<super::webhook_payloads::CommentPayloadData>(data.clone())
    else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    let Some(issue_id) = comment.resolved_issue_id().map(str::to_string) else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    Ok(admit(super::delivery::comment_request(
        event_id,
        connection_id,
        &issue_id,
        &comment,
    )?))
}

/// A workspace notification.
///
/// Every action is admitted, including the ones this adapter does not act on.
/// Which actions are handled is a decision, and a decision made before
/// admission leaves a whole class of Linear delivery with no durable trace of
/// having arrived — so the action is carried into the record and the worker
/// no-ops on it.
fn notification_event(
    envelope: &LinearWebhookEnvelope,
    connection_id: &str,
    event_id: &str,
) -> Result<Plan, IngressError> {
    let Some(raw) = envelope.notification.as_ref() else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    let Ok(notif) =
        serde_json::from_value::<super::webhook_payloads::AppUserNotificationData>(raw.clone())
    else {
        return Ok(Plan::Ignored {
            reason: reason::MALFORMED_PAYLOAD,
        });
    };

    Ok(admit(super::delivery::notification_request(
        event_id,
        connection_id,
        &envelope.action,
        &notif,
    )?))
}

/// The ordinary plan: admit one request, with nothing to apply and nothing to
/// settle first.
fn admit(request: AdapterDeliveryRequest) -> Plan {
    Plan::Admit {
        requests: vec![request],
        effects: Vec::new(),
        requires: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONNECTION: &str = "conn-1";
    const EVENT_ID: &str = "Comment:create:c-1";

    fn envelope(raw: serde_json::Value) -> LinearWebhookEnvelope {
        serde_json::from_value(raw).expect("parse envelope")
    }

    /// `normalize` needs no `AppState`, so every row below is one call.
    fn plan(raw: serde_json::Value) -> Plan {
        normalize(&envelope(raw), CONNECTION, EVENT_ID).expect("normalize")
    }

    fn ignored(raw: serde_json::Value) -> &'static str {
        match plan(raw) {
            Plan::Ignored { reason } => reason,
            Plan::Admit { requests, .. } => {
                panic!("expected an ignore, got {:?}", kinds(&requests))
            }
        }
    }

    fn kinds(requests: &[AdapterDeliveryRequest]) -> Vec<&'static str> {
        requests.iter().map(|r| r.kind).collect()
    }

    fn admitted(raw: serde_json::Value) -> (Vec<&'static str>, Vec<Effect>, Option<Requirement>) {
        match plan(raw) {
            Plan::Admit {
                requests,
                effects,
                requires,
            } => (kinds(&requests), effects, requires),
            Plan::Ignored { reason } => panic!("expected an admission, got ignored/{reason}"),
        }
    }

    fn session(action: &str) -> serde_json::Value {
        serde_json::json!({
            "action": action,
            "type": "AgentSessionEvent",
            "agentSession": {
                "id": "session-1",
                "issue": {"id": "issue-1", "identifier": "ENG-7", "title": "Fix it", "teamId": "team-1"},
            },
        })
    }

    fn comment(action: &str) -> serde_json::Value {
        serde_json::json!({
            "action": action,
            "type": "Comment",
            "data": {"id": "c-1", "body": "please look", "issueId": "issue-1"},
        })
    }

    fn issue(action: &str) -> serde_json::Value {
        serde_json::json!({
            "action": action,
            "type": "Issue",
            "webhookTimestamp": 1_700_000_000_000i64,
            "updatedFrom": {"stateId": "s0"},
            "data": {
                "id": "issue-1",
                "identifier": "ENG-7",
                "title": "Fix it",
                "teamId": "team-1",
                "updatedAt": "2026-08-10T03:20:08.000Z",
                "state": {"id": "s1", "name": "Done", "type": "completed"},
            },
        })
    }

    // ---- the kind each event type fans into ----

    #[test]
    fn a_created_session_plans_provisioning_behind_a_heartbeat_and_a_subscription() {
        let (kinds, effects, requires) = admitted(session("created"));

        assert_eq!(kinds, vec![super::super::delivery::SESSION_PROVISION]);
        assert_eq!(
            effects,
            vec![Effect::SessionHeartbeat {
                session_id: "session-1".to_string(),
                text: "Spinning up sandbox…",
            }],
            "the panel is answered inside Linear's 10-second window"
        );
        assert_eq!(
            requires,
            Some(Requirement::ActiveSubscription {
                team_id: "team-1".to_string()
            })
        );
    }

    #[test]
    fn a_prompt_plans_one_forward_with_nothing_on_the_response_path() {
        let mut raw = session("prompted");
        raw["agentActivity"] = serde_json::json!({"id": "act-1", "body": "do the thing"});
        let (kinds, effects, requires) = admitted(raw);

        assert_eq!(kinds, vec![super::super::delivery::SESSION_PROMPT]);
        assert!(effects.is_empty(), "the prompted heartbeat is the worker's");
        assert!(requires.is_none());
    }

    #[test]
    fn a_comment_an_issue_update_and_a_notification_each_plan_their_own_kind() {
        assert_eq!(
            admitted(comment("create")).0,
            vec![super::super::delivery::COMMENT]
        );
        assert_eq!(
            admitted(issue("update")).0,
            vec![super::super::delivery::ISSUE_UPDATE]
        );
        assert_eq!(
            admitted(serde_json::json!({
                "action": "issueCommentMention",
                "type": "AppUserNotification",
                "notification": {"id": "n-1", "type": "issueCommentMention"},
            }))
            .0,
            vec![super::super::delivery::NOTIFICATION]
        );
    }

    #[test]
    fn every_admitted_request_carries_the_identity_ingress_derived() {
        for raw in [comment("create"), issue("update")] {
            let Plan::Admit { requests, .. } = plan(raw) else {
                panic!("expected an admission");
            };
            for request in requests {
                assert_eq!(request.event_id, EVENT_ID);
                assert_eq!(request.adapter, super::super::delivery::ADAPTER);
            }
        }
    }

    // ---- the reasons, one real path each ----

    #[test]
    fn an_event_type_this_adapter_does_not_handle_is_unsupported_event() {
        assert_eq!(
            ignored(
                serde_json::json!({"action": "create", "type": "Project", "data": {"id": "p-1"}})
            ),
            reason::UNSUPPORTED_EVENT
        );
    }

    #[test]
    fn an_action_this_adapter_does_not_act_on_is_unsupported_action() {
        assert_eq!(ignored(comment("update")), reason::UNSUPPORTED_ACTION);
        assert_eq!(ignored(issue("create")), reason::UNSUPPORTED_ACTION);
        assert_eq!(ignored(session("archived")), reason::UNSUPPORTED_ACTION);
    }

    #[test]
    fn a_payload_that_is_absent_unparseable_or_missing_a_key_field_is_malformed_payload() {
        // Absent.
        assert_eq!(
            ignored(serde_json::json!({"action": "create", "type": "Comment"})),
            reason::MALFORMED_PAYLOAD
        );
        assert_eq!(
            ignored(serde_json::json!({"action": "created", "type": "AgentSessionEvent"})),
            reason::MALFORMED_PAYLOAD
        );
        assert_eq!(
            ignored(serde_json::json!({"action": "issueMention", "type": "AppUserNotification"})),
            reason::MALFORMED_PAYLOAD
        );
        // Present, but not the shape the handler expects.
        assert_eq!(
            ignored(serde_json::json!({"action": "create", "type": "Comment", "data": {"id": 17}})),
            reason::MALFORMED_PAYLOAD
        );
        // Parsed, but missing the field the workflow keys on.
        let mut no_issue_id = comment("create");
        no_issue_id["data"] = serde_json::json!({"id": "c-1", "body": "hi"});
        assert_eq!(ignored(no_issue_id), reason::MALFORMED_PAYLOAD);

        let mut no_team = issue("update");
        no_team["data"] = serde_json::json!({"id": "issue-1", "identifier": "ENG-7"});
        assert_eq!(ignored(no_team), reason::MALFORMED_PAYLOAD);

        // A created session with no issue: nothing to provision against.
        let mut no_issue = session("created");
        no_issue["agentSession"] = serde_json::json!({"id": "session-1"});
        assert_eq!(ignored(no_issue), reason::MALFORMED_PAYLOAD);
    }

    #[test]
    fn a_prompt_with_no_body_is_empty_body() {
        let mut empty = session("prompted");
        empty["agentActivity"] = serde_json::json!({"id": "act-1", "body": ""});
        assert_eq!(ignored(empty), reason::EMPTY_BODY);

        // The same reason when the activity is missing outright — either way
        // there is no prompt to forward.
        assert_eq!(ignored(session("prompted")), reason::EMPTY_BODY);
    }

    // ---- the one refusal that is not an ignore ----

    #[test]
    fn an_issue_update_with_no_event_clock_is_refused_rather_than_given_a_colliding_identity() {
        let mut unclocked = issue("update");
        unclocked
            .as_object_mut()
            .expect("object")
            .remove("webhookTimestamp");
        unclocked["data"]
            .as_object_mut()
            .expect("data object")
            .remove("updatedAt");

        let error = normalize(&envelope(unclocked), CONNECTION, EVENT_ID)
            .err()
            .expect("refused");
        assert!(
            matches!(
                error,
                IngressError::Unidentifiable(
                    "updatedFrom.stateId, data.state.id, and updatedAt or webhookTimestamp"
                )
            ),
            "got {error:?}"
        );
    }

    /// A team on the issue's `team` object rather than `teamId` still names the
    /// Subscription the created session is gated on.
    #[test]
    fn the_subscription_requirement_reads_either_spelling_of_the_team() {
        let mut nested = session("created");
        nested["agentSession"]["issue"] = serde_json::json!({
            "id": "issue-1",
            "identifier": "ENG-7",
            "title": "Fix it",
            "team": {"id": "team-9"},
        });
        assert_eq!(
            admitted(nested).2,
            Some(Requirement::ActiveSubscription {
                team_id: "team-9".to_string()
            })
        );
    }
}
