//! Durable admission for every Linear webhook event kind.
//!
//! Linear does not resend an acknowledged delivery, so all five kinds are
//! admitted into [`moatless_loops::adapters::delivery`] before the response
//! returns; only verification, parsing, and shape guards run on the response
//! path.
//!
//! `event_id` is the existing [`dedup_key`](super::ingress) string, not
//! Linear's `webhookId` (identical on every delivery). Each kind converges on
//! an identity it does not invent, so a retry after the execution timeout is a
//! no-op — except a timeout landing between task creation and the binding write
//! in `provision_session_task`.

use std::sync::Arc;

use tracing::{debug, info, warn};

use moatless_adapter_model::apps::AdapterAppReader;
use moatless_core::config::Config;

use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::delivery::{
    AdapterDeliveryFailure, AdapterDeliveryHandlers, AdapterDeliveryRequest,
};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::adapters::delivery::{ConnectionDeliveryPayload, register_connection_delivery};
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::runtime::RoutingService;
use moatless_ports::identity::AppCredentials;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::MessageAcceptor;
use moatless_ports::task::RelationRecorder;
use moatless_ports::task::TaskFinisher;
use moatless_ports::task::TaskOpener;
use moatless_ports::task::TaskReader;
use moatless_types::adapter::AdapterConnection;

use super::active_turns::ActiveLinearTurns;
use super::webhook_payloads::{
    AgentSession, AppUserNotificationData, CommentPayloadData, IssuePayload,
};

/// Adapter name on every record this module admits.
pub const ADAPTER: &str = moatless_types::adapter::AdapterKind::Linear.as_str();

/// Provision the task behind a newly created Linear agent session.
pub const SESSION_PROVISION: &str = "linear.session_provision";

/// Forward a follow-up prompt sent to an existing Linear agent session.
pub const SESSION_PROMPT: &str = "linear.session_prompt";

/// Act on an issue's workflow state change: notify, close, and status-entry
/// task creation.
pub const ISSUE_UPDATE: &str = "linear.issue_update";

/// Forward a comment on a bound issue to the tasks listening to it.
pub const COMMENT: &str = "linear.comment";

/// Act on a workspace notification from Linear's OAuth-app inbox stream.
pub const NOTIFICATION: &str = "linear.notification";

/// The Linear side of durable delivery: the five worker entry points as
/// methods, over the downward ports they read.
///
/// Constructed once at the composition root and shared behind an `Arc` by
/// [`register`]; ports live here rather than on
/// [`LinearAdapter`](super::adapter::LinearAdapter) so the adapter stays a
/// description of the provider.
pub struct LinearInboundRouter {
    adapter_connections: Arc<dyn ConnectionLookup>,
    app_tokens: Arc<dyn AppCredentials>,
    config: Arc<Config>,
    apps: Arc<dyn AdapterAppReader>,
    loops: Arc<dyn LoopReader>,
    task_bindings: Arc<Bindings>,
    tasks: Arc<dyn TaskReader>,
    task_creation: Arc<dyn TaskOpener>,
    task_relations: Arc<dyn RelationRecorder>,
    message_send: Arc<dyn MessageAcceptor>,
    sandbox_waker: Arc<dyn SandboxWaker>,
    task_closer: Arc<dyn TaskFinisher>,
    linear_active_turns: Arc<ActiveLinearTurns>,
    routing: Arc<RoutingService>,
}

impl LinearInboundRouter {
    #[expect(
        clippy::too_many_arguments,
        reason = "the fourteen ports the five linear delivery handlers read, \
                  wired in from the composition root; the rejected alternatives \
                  are threading thirteen params through five delivery closures, \
                  and extending the shared RoutingService with one provider's \
                  ports"
    )]
    pub fn new(
        adapter_connections: Arc<dyn ConnectionLookup>,
        app_tokens: Arc<dyn AppCredentials>,
        config: Arc<Config>,
        apps: Arc<dyn AdapterAppReader>,
        loops: Arc<dyn LoopReader>,
        task_bindings: Arc<Bindings>,
        tasks: Arc<dyn TaskReader>,
        task_creation: Arc<dyn TaskOpener>,
        task_relations: Arc<dyn RelationRecorder>,
        message_send: Arc<dyn MessageAcceptor>,
        sandbox_waker: Arc<dyn SandboxWaker>,
        task_closer: Arc<dyn TaskFinisher>,
        linear_active_turns: Arc<ActiveLinearTurns>,
        routing: Arc<RoutingService>,
    ) -> Self {
        Self {
            adapter_connections,
            app_tokens,
            config,
            apps,
            loops,
            task_bindings,
            tasks,
            task_creation,
            task_relations,
            message_send,
            sandbox_waker,
            task_closer,
            linear_active_turns,
            routing,
        }
    }
}

/// Register the Linear delivery handlers on the process-wide registry.
///
/// One call from the composition root; the worker then claims exactly these
/// kinds. A build without this call leaves Linear records for a replica that
/// has it, rather than failing them.
pub fn register(handlers: &mut AdapterDeliveryHandlers, router: Arc<LinearInboundRouter>) {
    register_connection_delivery::<SessionProvisionDelivery, _, _>(
        handlers,
        SESSION_PROVISION,
        router.adapter_connections.clone(),
        {
            let router = router.clone();
            move |delivery, connection, payload| {
                let router = router.clone();
                async move {
                    router
                        .session_provision(delivery, connection, payload)
                        .await
                }
            }
        },
    );
    register_connection_delivery::<SessionPromptDelivery, _, _>(
        handlers,
        SESSION_PROMPT,
        router.adapter_connections.clone(),
        {
            let router = router.clone();
            move |delivery, connection, payload| {
                let router = router.clone();
                async move { router.session_prompt(delivery, connection, payload).await }
            }
        },
    );
    register_connection_delivery::<IssueUpdateDelivery, _, _>(
        handlers,
        ISSUE_UPDATE,
        router.adapter_connections.clone(),
        {
            let router = router.clone();
            move |delivery, connection, payload| {
                let router = router.clone();
                async move { router.issue_update(delivery, connection, payload).await }
            }
        },
    );
    register_connection_delivery::<CommentDelivery, _, _>(
        handlers,
        COMMENT,
        router.adapter_connections.clone(),
        {
            let router = router.clone();
            move |delivery, connection, payload| {
                let router = router.clone();
                async move { router.comment(delivery, connection, payload).await }
            }
        },
    );
    register_connection_delivery::<NotificationDelivery, _, _>(
        handlers,
        NOTIFICATION,
        router.adapter_connections.clone(),
        {
            let router = router.clone();
            move |delivery, connection, payload| {
                let router = router.clone();
                async move { router.notification(delivery, connection, payload).await }
            }
        },
    );
}

/// Everything the provisioning handler needs to redo the work from scratch.
///
/// The connection is referenced by id, not snapshotted, so a webhook secret
/// never lands in a long-retained table; the session is carried verbatim since
/// it's the sender's own payload and can't be re-fetched later.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SessionProvisionDelivery {
    pub(super) connection_id: String,
    pub(super) session: AgentSession,
}

impl ConnectionDeliveryPayload for SessionProvisionDelivery {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
}

/// Everything the prompt handler needs to forward one follow-up prompt.
///
/// `activity_id` identifies the forwarded message downstream
/// (`linear:prompt:{activity_id}`) and must survive the restart with the body;
/// rebuilding it from a clock or fresh uuid would double-deliver the prompt to
/// the Agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SessionPromptDelivery {
    pub(super) connection_id: String,
    pub(super) session_id: String,
    pub(super) activity_id: String,
    pub(super) body: String,
}

impl ConnectionDeliveryPayload for SessionPromptDelivery {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
}

/// Everything the issue handler needs to act on one workflow state change.
///
/// `transition_id` is the semantic state transition derived at ingress and half
/// of both the delivery identity and the forwarded message's
/// `linear:issue-state:{issue}:{transition_id}` id; older records only carried
/// `delivered_at`, kept here as a fallback.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct IssueUpdateDelivery {
    pub(super) connection_id: String,
    pub(super) issue: IssuePayload,
    /// Linear's `updatedFrom` diff, which names the state the issue left. A
    /// state change cannot be recognised without it.
    #[serde(default)]
    pub(super) updated_from: Option<serde_json::Value>,
    #[serde(default)]
    pub(super) transition_id: Option<String>,
    pub(super) delivered_at: i64,
}

impl ConnectionDeliveryPayload for IssueUpdateDelivery {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
}

/// Everything the comment handler needs to forward one comment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CommentDelivery {
    pub(super) connection_id: String,
    pub(super) issue_id: String,
    pub(super) comment: CommentPayloadData,
}

impl ConnectionDeliveryPayload for CommentDelivery {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
}

/// Everything the notification handler needs to act on one workspace
/// notification.
///
/// `action` is carried rather than filtered on at the route: whether this
/// adapter acts on it is a decision that belongs behind admission, so every
/// delivery still gets a durable record of having arrived.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct NotificationDelivery {
    pub(super) connection_id: String,
    pub(super) action: String,
    pub(super) notification: AppUserNotificationData,
}

impl ConnectionDeliveryPayload for NotificationDelivery {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
}

/// Build the admission request for one Linear delivery.
///
/// `event_id` must be the dispatcher's dedup key, so one delivery has one
/// identity no matter which kind it fans into. Fails only on a non-string map
/// key, impossible for a webhook-parsed value but not worth an `unwrap`.
fn request(
    kind: &'static str,
    event_id: &str,
    payload: &impl serde::Serialize,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    let payload = serde_json::to_value(payload).map_err(|e| {
        SharedAdapterError::Internal(format!("failed to serialize {kind} work payload: {e}"))
    })?;
    Ok(AdapterDeliveryRequest {
        adapter: ADAPTER,
        kind,
        event_id: event_id.to_string(),
        payload,
    })
}

/// Build the admission request for one session provisioning.
pub(super) fn session_provision_request(
    event_id: &str,
    connection_id: &str,
    session: &AgentSession,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        SESSION_PROVISION,
        event_id,
        &SessionProvisionDelivery {
            connection_id: connection_id.to_string(),
            session: session.clone(),
        },
    )
}

/// Build the admission request for one follow-up prompt.
pub(super) fn session_prompt_request(
    event_id: &str,
    connection_id: &str,
    session_id: &str,
    activity_id: &str,
    body: &str,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        SESSION_PROMPT,
        event_id,
        &SessionPromptDelivery {
            connection_id: connection_id.to_string(),
            session_id: session_id.to_string(),
            activity_id: activity_id.to_string(),
            body: body.to_string(),
        },
    )
}

/// Build the admission request for one issue state change.
pub(super) fn issue_update_request(
    event_id: &str,
    connection_id: &str,
    issue: &IssuePayload,
    updated_from: Option<&serde_json::Value>,
    transition_id: &str,
    delivered_at: i64,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        ISSUE_UPDATE,
        event_id,
        &IssueUpdateDelivery {
            connection_id: connection_id.to_string(),
            issue: issue.clone(),
            updated_from: updated_from.cloned(),
            transition_id: Some(transition_id.to_string()),
            delivered_at,
        },
    )
}

/// Build the admission request for one comment.
pub(super) fn comment_request(
    event_id: &str,
    connection_id: &str,
    issue_id: &str,
    comment: &CommentPayloadData,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        COMMENT,
        event_id,
        &CommentDelivery {
            connection_id: connection_id.to_string(),
            issue_id: issue_id.to_string(),
            comment: comment.clone(),
        },
    )
}

/// Build the admission request for one workspace notification.
pub(super) fn notification_request(
    event_id: &str,
    connection_id: &str,
    action: &str,
    notification: &AppUserNotificationData,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    request(
        NOTIFICATION,
        event_id,
        &NotificationDelivery {
            connection_id: connection_id.to_string(),
            action: action.to_string(),
            notification: notification.clone(),
        },
    )
}

impl LinearInboundRouter {
    async fn session_provision(
        &self,
        _delivery: moatless_adapter_model::delivery::AdapterDelivery,
        connection: AdapterConnection,
        payload: SessionProvisionDelivery,
    ) -> Result<(), AdapterDeliveryFailure> {
        // Rules are re-resolved rather than snapshotted: a trigger disabled
        // between admission and execution should stop the provisioning, which
        // is what an operator disabling it means.
        let team_id = payload
            .session
            .issue
            .as_ref()
            .and_then(|issue| {
                issue
                    .team_id
                    .clone()
                    .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
            })
            .unwrap_or_default();
        let rules = super::subscription_rules::resolve_rules_for_team(
            self.loops.as_ref(),
            &connection,
            &team_id,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)?;

        if rules.is_empty() {
            info!(
                session_id = %payload.session.id,
                team_id = %team_id,
                "Linear session provisioning skipped: no active rule for the team"
            );
            // The rule that admitted this session was disabled before the worker
            // ran. Close the session so its Linear panel doesn't spin forever.
            //
            // Resolved here rather than above the guard, and best-effort: there is
            // no work left to redo on this record, so a credential failure must not
            // turn a settled no-op into a retry.
            match super::auth::api_for_connection(
                self.config.http.linear,
                self.app_tokens.clone(),
                self.apps.as_ref(),
                &connection,
            )
            .await
            {
                Ok(api) => {
                    super::sessions::decline_session(
                        &api,
                        &payload.session.id,
                        super::sessions::NO_ACTIVE_RULE_DECLINE,
                    )
                    .await;
                }
                Err(e) => warn!(
                    session_id = %payload.session.id,
                    error = %e,
                    "Failed to resolve Linear app config to decline session"
                ),
            }
            return Ok(());
        }

        let api = super::auth::api_for_connection(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.apps.as_ref(),
            &connection,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)?;
        super::sessions::provision_selected_session_task(
            self.task_bindings.as_ref(),
            self.tasks.as_ref(),
            &self.config,
            self.task_creation.as_ref(),
            self.task_relations.as_ref(),
            self.message_send.as_ref(),
            &self.linear_active_turns,
            &api,
            &connection,
            &rules,
            &payload.session,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }

    async fn session_prompt(
        &self,
        _delivery: moatless_adapter_model::delivery::AdapterDelivery,
        connection: AdapterConnection,
        payload: SessionPromptDelivery,
    ) -> Result<(), AdapterDeliveryFailure> {
        let api = super::auth::api_for_connection(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.apps.as_ref(),
            &connection,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)?;
        super::sessions::handle_session_prompted(
            self.task_bindings.as_ref(),
            self.loops.as_ref(),
            &self.linear_active_turns,
            self.message_send.as_ref(),
            self.sandbox_waker.as_ref(),
            &api,
            &payload.session_id,
            &payload.activity_id,
            &payload.body,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }

    async fn issue_update(
        &self,
        _delivery: moatless_adapter_model::delivery::AdapterDelivery,
        connection: AdapterConnection,
        payload: IssueUpdateDelivery,
    ) -> Result<(), AdapterDeliveryFailure> {
        // Re-resolved rather than snapshotted, for the same reason the provisioning
        // handler re-resolves: a rule disabled between admission and execution
        // should stop the work, which is what disabling it means.
        let team_id = payload.issue.resolved_team_id().unwrap_or_default();
        let rules = super::subscription_rules::resolve_rules_for_team(
            self.loops.as_ref(),
            &connection,
            team_id,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)?;
        if rules.is_empty() {
            debug!(
                issue = %payload.issue.identifier,
                team_id,
                "Linear issue update skipped: no active rule for the team"
            );
            return Ok(());
        }

        let api = super::auth::api_for_connection(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.apps.as_ref(),
            &connection,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)?;
        super::issues::handle_issue_update(
            self.task_bindings.as_ref(),
            self.tasks.as_ref(),
            &self.config,
            self.task_closer.as_ref(),
            self.sandbox_waker.as_ref(),
            &self.linear_active_turns,
            self.task_creation.as_ref(),
            self.message_send.as_ref(),
            &api,
            &connection,
            &rules,
            payload.updated_from.as_ref(),
            &payload.issue,
            payload.transition_id.as_deref(),
            payload.delivered_at,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }

    async fn comment(
        &self,
        _delivery: moatless_adapter_model::delivery::AdapterDelivery,
        connection: AdapterConnection,
        payload: CommentDelivery,
    ) -> Result<(), AdapterDeliveryFailure> {
        let api = super::auth::api_for_connection(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.apps.as_ref(),
            &connection,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)?;
        super::comments::handle_comment_created(
            &self.config,
            &self.routing,
            self.task_bindings.as_ref(),
            self.loops.as_ref(),
            self.message_send.as_ref(),
            self.sandbox_waker.as_ref(),
            &api,
            &connection,
            &payload.issue_id,
            &payload.comment,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }

    async fn notification(
        &self,
        _delivery: moatless_adapter_model::delivery::AdapterDelivery,
        connection: AdapterConnection,
        payload: NotificationDelivery,
    ) -> Result<(), AdapterDeliveryFailure> {
        // An action this adapter does not act on is a *successful* no-op, not a
        // failure: the delivery arrived, it was recorded, and there was nothing to
        // do with it. Ending the record any other way would put a delivery Linear
        // sends routinely into an operator's failed-record queue.
        if payload.action != "issueStatusChanged" {
            warn!(
                action = %payload.action,
                org_id = connection.scope_value.as_str(),
                notification_id = payload.notification.id.as_deref().unwrap_or(""),
                notification_kind = payload.notification.kind.as_str(),
                "Linear AppUserNotification: action not handled, nothing to do"
            );
            return Ok(());
        }

        let api = super::auth::api_for_connection(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.apps.as_ref(),
            &connection,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)?;
        super::notifications::handle_issue_status_notification(
            self.task_bindings.as_ref(),
            self.tasks.as_ref(),
            &self.config,
            self.task_closer.as_ref(),
            self.loops.as_ref(),
            &api,
            &connection,
            &payload.notification,
        )
        .await
        .map_err(AdapterDeliveryFailure::from)
    }
}

/// Log line shared by every admission site, so an operator can follow one
/// delivery from ingress into the queue.
pub(super) fn log_admission(
    kind: &'static str,
    event_id: &str,
    admission: moatless_adapter_model::delivery::Admission,
) {
    moatless_adapter_model::ingress::log_admission(ADAPTER, kind, event_id, admission);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> AgentSession {
        serde_json::from_value(serde_json::json!({
            "id": "session-A",
            "issue": {
                "id": "issue-1",
                "identifier": "ENG-7",
                "title": "Fix the thing",
                "teamId": "team-1",
                "state": {"id": "s1", "name": "Todo", "type": "unstarted"}
            },
            "comment": {"id": "c-1", "body": "@agent please"},
            "promptContext": "<context/>"
        }))
        .expect("parse session")
    }

    /// The payload is the whole of what the handler gets after a restart, and
    /// Linear's field names are renames — so the round trip has to preserve
    /// both the values and the wire spelling.
    #[test]
    fn a_session_payload_survives_the_round_trip() {
        let request =
            session_provision_request("AgentSessionEvent:created:session-A", "conn-1", &session())
                .expect("serialize");
        let back: SessionProvisionDelivery =
            serde_json::from_value(request.payload).expect("deserialize");

        assert_eq!(back.connection_id, "conn-1");
        assert_eq!(back.session.id, "session-A");
        let issue = back.session.issue.expect("issue survives");
        assert_eq!(issue.identifier, "ENG-7");
        assert_eq!(issue.team_id.as_deref(), Some("team-1"));
        assert_eq!(
            issue.state.expect("workflow state survives").kind,
            "unstarted"
        );
        assert_eq!(
            back.session.prompt_context.as_deref(),
            Some("<context/>"),
            "the prompt context is what the agent is given; losing it changes the task"
        );
        assert_eq!(
            back.session.comment.expect("comment survives").body,
            "@agent please"
        );
    }

    #[test]
    fn a_request_carries_the_kind_and_the_senders_key() {
        let request = session_provision_request("evt-1", "conn-1", &session()).expect("serialize");
        assert_eq!(request.adapter, ADAPTER);
        assert_eq!(request.kind, "linear.session_provision");
        assert_eq!(request.event_id, "evt-1");
    }

    /// Every kind is its own record, so a permanent failure in one cannot spend
    /// the retry budget of another derived from the same delivery.
    #[test]
    fn the_kinds_are_distinct_and_namespaced() {
        let kinds = [
            SESSION_PROVISION,
            SESSION_PROMPT,
            ISSUE_UPDATE,
            COMMENT,
            NOTIFICATION,
        ];
        let mut unique = kinds.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len(), "two kinds share a name");
        assert!(kinds.iter().all(|k| k.starts_with("linear.")));
    }

    /// One delivery has one identity; two kinds admitted under it are two
    /// records because the *kind* differs, not the key — asserting that here
    /// guards a future "make the key unique per kind" change from silently
    /// breaking the dedup the identity provides.
    #[test]
    fn every_kind_admits_under_the_dispatchers_delivery_identity() {
        let event_id = "AgentSessionEvent:prompted:activity-1";
        let prompt = session_prompt_request(event_id, "conn-1", "session-A", "activity-1", "hi")
            .expect("serialize");
        let provision =
            session_provision_request(event_id, "conn-1", &session()).expect("serialize");

        assert_eq!(prompt.event_id, event_id);
        assert_eq!(provision.event_id, event_id);
        assert_ne!(prompt.kind, provision.kind);
    }

    /// The activity id is what the forwarded prompt is identified by
    /// downstream, so losing it in the round trip would double-deliver the
    /// prompt on the first retry.
    #[test]
    fn a_prompt_payload_survives_the_round_trip() {
        let request = session_prompt_request(
            "AgentSessionEvent:prompted:activity-9",
            "conn-1",
            "session-A",
            "activity-9",
            "please look again",
        )
        .expect("serialize");
        assert_eq!(request.kind, "linear.session_prompt");

        let back: SessionPromptDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.connection_id, "conn-1");
        assert_eq!(back.session_id, "session-A");
        assert_eq!(back.activity_id, "activity-9");
        assert_eq!(back.body, "please look again");
    }

    /// The transition id is half of the forwarded message's identity, so it has
    /// to come back off the record exactly — an attempt that lost it would look
    /// like a different state change to everything downstream.
    #[test]
    fn an_issue_payload_survives_the_round_trip_with_its_transition_id() {
        let issue: IssuePayload = serde_json::from_value(serde_json::json!({
            "id": "issue-1",
            "identifier": "ENG-7",
            "title": "Fix the thing",
            "teamId": "team-1",
            "updatedAt": "2026-08-10T03:20:08.000Z",
            "state": {"id": "s2", "name": "In Progress", "type": "started"}
        }))
        .expect("parse issue");
        let updated_from = serde_json::json!({"stateId": "s1"});

        let request = issue_update_request(
            "Issue:update:issue-1:1700000000000",
            "conn-1",
            &issue,
            Some(&updated_from),
            "s1:s2:2026-08-10T03:20:08.000Z",
            1_700_000_000_000,
        )
        .expect("serialize");
        assert_eq!(request.kind, "linear.issue_update");

        let back: IssueUpdateDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.connection_id, "conn-1");
        assert_eq!(back.issue.id, "issue-1");
        assert_eq!(back.issue.resolved_team_id(), Some("team-1"));
        assert_eq!(back.issue.state_kind(), "started");
        assert_eq!(
            back.updated_from
                .as_ref()
                .and_then(|v| v.get("stateId"))
                .and_then(serde_json::Value::as_str),
            Some("s1"),
            "without the diff the handler cannot tell a state change happened"
        );
        assert_eq!(
            back.transition_id.as_deref(),
            Some("s1:s2:2026-08-10T03:20:08.000Z")
        );
        assert_eq!(back.delivered_at, 1_700_000_000_000);
    }

    #[test]
    fn a_comment_payload_survives_the_round_trip() {
        let comment: CommentPayloadData = serde_json::from_value(serde_json::json!({
            "id": "comment-1",
            "body": "have a look",
            "issueId": "issue-1",
            "parentId": "comment-0",
            "user": {"id": "user-1", "name": "Alice"}
        }))
        .expect("parse comment");

        let request = comment_request("Comment:create:comment-1", "conn-1", "issue-1", &comment)
            .expect("serialize");
        assert_eq!(request.kind, "linear.comment");

        let back: CommentDelivery = serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.issue_id, "issue-1");
        assert_eq!(back.comment.id, "comment-1");
        assert_eq!(back.comment.body, "have a look");
        assert_eq!(
            back.comment.parent_id.as_deref(),
            Some("comment-0"),
            "the parent decides which task a threaded reply belongs to"
        );
        assert_eq!(
            back.comment.user.as_ref().map(|u| u.id.as_str()),
            Some("user-1"),
            "the author is what the echo filter reads"
        );
    }

    #[test]
    fn a_notification_payload_survives_the_round_trip() {
        let notification: AppUserNotificationData = serde_json::from_value(serde_json::json!({
            "id": "notif-1",
            "type": "issueStatusChanged",
            "issueId": "issue-1",
            "issue": {
                "id": "issue-1",
                "identifier": "ENG-7",
                "title": "Fix the thing",
                "teamId": "team-1",
                "state": {"id": "s3", "name": "Done", "type": "completed"}
            }
        }))
        .expect("parse notification");

        let request = notification_request(
            "AppUserNotification:issueStatusChanged:notif-1",
            "conn-1",
            "issueStatusChanged",
            &notification,
        )
        .expect("serialize");
        assert_eq!(request.kind, "linear.notification");

        let back: NotificationDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(
            back.action, "issueStatusChanged",
            "the worker decides whether an action is handled, so it has to arrive"
        );
        assert_eq!(back.notification.kind, "issueStatusChanged");
        let issue = back.notification.resolved_issue().expect("issue survives");
        assert_eq!(issue.state_kind(), "completed");
        assert_eq!(issue.resolved_team_id(), Some("team-1"));
    }

    /// An action this adapter does not act on is still admitted, and the record
    /// still names it. Filtering at the route would leave the delivery with no
    /// durable trace at all.
    #[test]
    fn an_unhandled_notification_action_is_still_carried_into_the_record() {
        let notification: AppUserNotificationData = serde_json::from_value(serde_json::json!({
            "id": "notif-2",
            "type": "issueMention",
            "issueId": "issue-1",
        }))
        .expect("parse notification");

        let request = notification_request(
            "AppUserNotification:issueMention:notif-2",
            "conn-1",
            "issueMention",
            &notification,
        )
        .expect("serialize");

        let back: NotificationDelivery =
            serde_json::from_value(request.payload).expect("deserialize");
        assert_eq!(back.action, "issueMention");
        assert_eq!(back.notification.id.as_deref(), Some("notif-2"));
    }
}
