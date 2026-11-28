//! The Linear agent-session workflows: a session created, a prompt typed into
//! one that exists, and a session the app opens on a reopened issue.
//!
//! `created` and `prompted` open a heartbeat before slow work (`created`'s in
//! [`webhook::normalize`](super::webhook::normalize)). `created` always spawns
//! a Task; `prompted` routes into its bound Task, and a missing Binding means
//! that Task closed, not an error.

use serde::Deserialize;
use tracing::{info, warn};

use moatless_core::config::Config;

use moatless_adapter_model::binding::{ADAPTER_EVENT_BINDING_ORIGIN, BindScope};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::operations::Bindings;
use moatless_loops::bindings::runtime::RoutingService;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::RelationRecorder;
use moatless_ports::task::TaskOpener;
use moatless_ports::task::TaskReader;
use moatless_ports::task::{InboundTurn, MessageAcceptor, TurnOrigin};
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_types::loops::SubscriptionMatch;

use super::active_turns::ActiveLinearTurns;
use super::agent_api::{ActivityKind, LinearAgentApi};
use super::config::{LinearRuleConfig, rules_need_issue_fetch, select_spawn_rule};
use super::delivery::IssueUpdateDelivery;
use super::issues::{insert_event_task, transition_issue_state};
use super::reference_resolution::LINEAR_PROVIDER;
use super::subject::{ADAPTER, SUBJECT_KIND_SESSION as SUBJECT_KIND};
use super::webhook_payloads::{AgentSession, AgentSessionIssue, LinearActor, LinearWorkflowState};

const ADAPTER_LABEL: &str = moatless_types::adapter::AdapterKind::Linear.as_str();

/// Body of the terminal `error` activity posted when a created session reaches
/// no task (no active Subscription for the team, or one disabled between
/// admission and execution).
///
/// Linear infers session state from the last activity, so only a terminal
/// `error` closes a panel stuck `active` from the ephemeral heartbeat alone.
pub(super) const NO_ACTIVE_RULE_DECLINE: &str = "No Moatless automation is configured for this Linear team, so I can't start a task for this issue.";

/// Body of the terminal `error` activity posted when the issue carries one of
/// the selected rule's `exclude_labels`.
pub(super) const EXCLUDED_LABEL_DECLINE: &str =
    "This issue carries a label that keeps Moatless agents off it, so I won't start a task for it.";

/// Close a Linear agent session with a terminal `error` activity when no task
/// will be provisioned behind it.
///
/// The heartbeat alone only resets Linear's 10-second stale timer, so without
/// this the panel sits `active` forever; best-effort — a post failure is
/// logged, not propagated, since the caller already decided not to do the work.
pub(super) async fn decline_session(api: &LinearAgentApi, session_id: &str, message: &str) {
    if let Err(e) = api
        .agent_activity_create(session_id, ActivityKind::Error, message, false)
        .await
    {
        warn!(session_id, error = %e, "Failed to post Linear session decline activity");
    }
}

/// Select the spawn rule for a created session and provision its task.
///
/// The executable half of a `linear.session_provision` record; at-least-once,
/// converging because the binding guard inside [`provision_session_task`] makes
/// a second attempt a no-op rather than a second task.
#[expect(
    clippy::too_many_arguments,
    reason = "the nine downward ports the session provisioning path reads \
              (bindings, tasks, config, task-creation, task-relations, \
              message-send, active-turns, identity, routing) inverted out of the single AppState; \
              there is no narrower grouping that is not itself that \
              AppState-shaped bag"
)]
pub(super) async fn provision_selected_session_task(
    task_bindings: &Bindings,
    tasks: &dyn TaskReader,
    config: &Config,
    task_creation: &dyn TaskOpener,
    task_relations: &dyn RelationRecorder,
    message_send: &dyn MessageAcceptor,
    linear_active_turns: &ActiveLinearTurns,
    identity: &dyn ProviderIdentityLink,
    routing: &RoutingService,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    session: &AgentSession,
) -> Result<(), AdapterError> {
    // Project/label-aware rule selection. A mention/assignment on a Vuln-Mgmt
    // issue routes to the project-scoped `vulnerability-fix` rule; anything
    // else falls back to the unscoped `soap-specs` catch-all. The `get_issue`
    // round-trip this may make is off Linear's ack path because this runs in
    // the worker, not the request.
    let (issue_id, issue_identifier) = session
        .issue
        .as_ref()
        .map_or_default(|i| (i.id.as_str(), i.identifier.as_str()));
    let Some((rule, labels)) =
        select_session_spawn_rule(api, rules, issue_id, issue_identifier).await
    else {
        warn!(
            session_id = %session.id,
            "Linear session created but no spawn rule selected — skipping task creation"
        );
        // Close the session rather than leaving Linear's panel spinning.
        decline_session(api, &session.id, NO_ACTIVE_RULE_DECLINE).await;
        return Ok(());
    };
    if LinearRuleConfig::from_rule(&rule).excluded(&labels) {
        info!(
            session_id = %session.id,
            issue = %issue_identifier,
            subscription_id = %rule.id,
            "Linear session declined: the issue carries an excluded label"
        );
        decline_session(api, &session.id, EXCLUDED_LABEL_DECLINE).await;
        return Ok(());
    }

    provision_session_task(
        task_bindings,
        tasks,
        config,
        task_creation,
        task_relations,
        message_send,
        linear_active_turns,
        identity,
        routing,
        api,
        connection,
        &rule,
        session,
    )
    .await
}

/// Project/label-aware spawn-rule selection for a mention/assignment session,
/// returned with the issue's label names.
///
/// Fetches the issue once, unless [`rules_need_issue_fetch`] is false. A fetch
/// failure degrades to `(None, &[])`: the oldest catch-all rule, unexcluded.
async fn select_session_spawn_rule(
    api: &LinearAgentApi,
    rules: &[SubscriptionMatch],
    issue_id: &str,
    issue_identifier: &str,
) -> Option<(SubscriptionMatch, Vec<String>)> {
    if !rules_need_issue_fetch(rules) {
        return rules.first().cloned().map(|rule| (rule, Vec::new()));
    }

    let (project_id, labels): (Option<String>, Vec<String>) = match api.get_issue(issue_id).await {
        Ok(detail) => {
            let project_id = detail
                .pointer("/project/id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let labels = detail
                .pointer("/labels/nodes")
                .and_then(serde_json::Value::as_array)
                .map_or_default(|arr| {
                    arr.iter()
                        .filter_map(|n| n.get("name").and_then(serde_json::Value::as_str))
                        .map(str::to_string)
                        .collect()
                });
            (project_id, labels)
        }
        Err(e) => {
            warn!(
                issue = %issue_identifier,
                error = %e,
                "Failed to fetch Linear issue for mention rule selection; using fallback rule"
            );
            (None, Vec::new())
        }
    };

    select_spawn_rule(rules, project_id.as_deref(), &labels)
        .cloned()
        .map(|rule| (rule, labels))
}

/// Route an `AgentSessionEvent.prompted` payload to the bound task.
///
/// `activity_id` (the `agentActivity` id) is what the forwarded message is
/// identified by downstream: it names *this* prompt, so a Linear retry of one
/// prompt repeats the id and is dropped as a duplicate.
#[expect(
    clippy::too_many_arguments,
    reason = "the five downward ports the prompt-forwarding path reads (bindings, \
              loops, active-turns, message-send, sandbox-waker) inverted out of \
              the single AppState; there is no narrower grouping that is not \
              itself that AppState-shaped bag"
)]
pub(super) async fn handle_session_prompted(
    task_bindings: &Bindings,
    loops: &dyn LoopReader,
    linear_active_turns: &ActiveLinearTurns,
    message_send: &dyn MessageAcceptor,
    sandbox_waker: &dyn SandboxWaker,
    api: &LinearAgentApi,
    session_id: &str,
    activity_id: &str,
    body: &str,
) -> Result<(), AdapterError> {
    let bindings: Vec<TaskBinding> = task_bindings
        .for_subject(ADAPTER, SUBJECT_KIND, session_id)
        .await?
        .into_iter()
        // Skip bindings whose task has closed — a closed task is terminal.
        .filter(|b| b.active)
        .collect();

    if bindings.is_empty() {
        warn!(
            session_id,
            "No active Linear binding for prompted session — ignoring"
        );
        return Ok(());
    }

    // Optimistically ack receipt with an ephemeral thought heartbeat so
    // Linear doesn't hang on a slow `send_message_flow`. Ephemeral so the
    // agent's next activity replaces it in the UI.
    if let Err(e) = api
        .agent_activity_create(session_id, ActivityKind::Thought, "Working on it…", true)
        .await
    {
        warn!(session_id, error = %e, "Failed to emit Linear prompted heartbeat");
    }

    let rule = super::subject::rule_for_binding(loops, &bindings[0]).await?;

    for binding in bindings {
        // Re-open the Linear turn gate for this follow-up prompt so the
        // next agent run's events flow through to Linear's UI.
        linear_active_turns.open(&binding.task_id);
        let external_event_id = super::external_id::prompt(activity_id);
        if let Err(e) = message_send
            .accept(InboundTurn {
                task_id: binding.task_id.clone(),
                text: body.to_string(),
                file_ids: Vec::new(),
                resume: true,
                origin: TurnOrigin {
                    event_id: Some(external_event_id.clone()),
                    adapter: Some(ADAPTER_LABEL.to_string()),
                    subject_key: Some(binding.subject_key.clone()),
                    source_name: rule.as_ref().map(|r| r.source_matcher.clone()),
                    // A prompt carries no actor; the session's creator is who this
                    // is attributed to, and the created path records that.
                    author: None,
                    sender_id: None,
                },
                sent_by: None,
                skills: Vec::new(),
            })
            .await
        {
            warn!(
                task_id = %binding.task_id,
                session_id,
                error = %e,
                "Failed to forward Linear prompted message"
            );
            continue;
        }

        if let Err(e) = sandbox_waker.wake(&binding.task_id).await {
            warn!(
                task_id = %binding.task_id,
                error = %e,
                "Failed to ensure sandbox for Linear prompted message"
            );
        }
    }

    Ok(())
}

/// The Moatless user who started a session: the one linked to its Linear id,
/// else the one holding its email.
///
/// `None` leaves the Task with the Loop's run-as user, so a failed lookup is
/// logged and treated as an unlinked creator rather than failing the delivery.
async fn requester(
    identity: &dyn ProviderIdentityLink,
    creator: Option<&LinearActor>,
) -> Option<String> {
    let creator = creator?;
    if !creator.id.is_empty() {
        match identity.user_at(LINEAR_PROVIDER, &creator.id).await {
            Ok(Some(user)) => return Some(user.id.to_string()),
            Ok(None) => {}
            Err(e) => {
                warn!(linear_user = %creator.id, error = %e, "Linear session creator lookup failed");
            }
        }
    }
    let email = creator.email.as_deref().filter(|email| !email.is_empty())?;
    match identity.user_by_email(email).await {
        Ok(user) => user.map(|user| user.id.to_string()),
        Err(e) => {
            warn!(linear_user = %creator.id, error = %e, "Linear session creator email lookup failed");
            None
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the nine downward ports one session task's creation reads \
              (bindings, tasks, config, task-creation, task-relations, \
              message-send, active-turns, identity, routing) inverted out of the single AppState; \
              there is no narrower grouping that is not itself that \
              AppState-shaped bag"
)]
async fn provision_session_task(
    task_bindings: &Bindings,
    tasks: &dyn TaskReader,
    config: &Config,
    task_creation: &dyn TaskOpener,
    task_relations: &dyn RelationRecorder,
    message_send: &dyn MessageAcceptor,
    linear_active_turns: &ActiveLinearTurns,
    identity: &dyn ProviderIdentityLink,
    routing: &RoutingService,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    session: &AgentSession,
) -> Result<(), AdapterError> {
    let issue = session
        .issue
        .as_ref()
        .ok_or_else(|| SharedAdapterError::BadRequest("AgentSession missing issue".into()))?;

    // Idempotency guard: an existing binding for this session id means the
    // task is already provisioned — durable admission collapses a `created`
    // redelivery, but execution is at-least-once (a restart re-claims an
    // abandoned attempt), so this guard makes execution converge. Keyed on
    // session id: distinct sessions on one issue each get their own task.
    match task_bindings
        .for_subject(ADAPTER, SUBJECT_KIND, &session.id)
        .await
    {
        Ok(existing) if !existing.is_empty() => {
            info!(
                session_id = %session.id,
                task_id = %existing[0].task_id,
                "Linear AgentSession already provisioned — skipping duplicate task creation"
            );
            return Ok(());
        }
        Ok(_) => {}
        Err(e) => {
            warn!(
                error = %e,
                session_id = %session.id,
                "Failed to check for existing Linear session binding — proceeding"
            );
        }
    }

    // The binding guard above is a read, so it cannot stop two concurrent
    // attempts: the reopen path provisioning the session it opened while
    // Linear's `created` webhook for that session runs. The insert decides.
    if !routing
        .claim_task_creation_once(
            rule,
            SESSION_TASK_SUBJECT_KIND,
            &session.id,
            ADAPTER_LABEL,
            Some(&super::external_id::session_created(&session.id)),
        )
        .await?
    {
        info!(
            session_id = %session.id,
            "Linear AgentSession task already being created — skipping duplicate task creation"
        );
        return Ok(());
    }

    // Find an earlier task already bound to this Linear issue, if any. The
    // first row by created_at is the peer that touched this issue earlier.
    // The lookup uses destination_config.issue_id so it matches both
    // `linear_agent_session` and `linear_issue` bindings. The new task is
    // linked as `related_to` — a peer sharing the issue, not a child.
    let related = match task_bindings
        .by_destination_field(ADAPTER, SUBJECT_KIND, "issue_id", &issue.id)
        .await
    {
        Ok(bindings) => bindings
            .into_iter()
            .next()
            .map(|b| moatless_types::task::RelatedTask {
                related_task_id: b.task_id.clone(),
                context: serde_json::json!({
                    "via": "linear_session_created",
                    "adapter": ADAPTER,
                    "issue_id": issue.id,
                    "issue_identifier": issue.identifier,
                    "origin_binding_id": b.id,
                }),
            }),
        Err(e) => {
            warn!(error = %e, issue_id = %issue.id, "Failed to look up peer Linear task");
            None
        }
    };

    // Create the task (short-id, preflight, insert, tags, provision, and the
    // `Open` event all run inside the service), then the peer relation and the
    // binding.
    let preview = super::text::build_inbound_text(session);
    let requested_by = requester(identity, session.creator.as_ref()).await;
    let created = insert_event_task(
        task_creation,
        rule,
        &issue.identifier,
        &issue.title,
        &preview,
        requested_by,
    )
    .await;
    // Released when no task came of it, so the redelivery retries rather than
    // finding the claim held and dropping the session.
    if let Err(e) = &created
        && !matches!(e, AdapterError::Shared(SharedAdapterError::AccessDenied(_)))
        && let Err(release) = routing
            .release_task_creation(&rule.id, SESSION_TASK_SUBJECT_KIND, &session.id)
            .await
    {
        warn!(session_id = %session.id, error = %release, "Failed to release Linear session claim");
    }
    let task_id = match created {
        Ok(id) => {
            info!(task_id = %id, subscription_id = %rule.id, "Created task from Linear AgentSession");
            if let Err(e) = routing
                .record_created_task(&rule.id, SESSION_TASK_SUBJECT_KIND, &session.id, &id)
                .await
            {
                warn!(task_id = %id, error = %e, "Failed to record the Linear session's task");
            }
            id
        }
        // Preflight failed: the task exists but is errored, and no binding/turn
        // gate exists to surface the service's error event in Linear. Post the
        // reason onto the agent session directly so the user isn't left staring
        // at silence, then bail out — no binding, no prompt for a dead task.
        Err(AdapterError::Shared(SharedAdapterError::AccessDenied(msg))) => {
            if let Err(e) = api
                .agent_activity_create(
                    &session.id,
                    ActivityKind::Error,
                    &format!("Couldn't start this task: {msg}"),
                    false,
                )
                .await
            {
                warn!(session_id = %session.id, error = %e, "Failed to post Linear preflight error");
            }
            return Err(SharedAdapterError::AccessDenied(msg).into());
        }
        Err(e) => return Err(e),
    };
    if let Some(ref r) = related {
        task_relations.record_related_to(&task_id, r).await;
    }
    // Open the Linear turn gate so subsequent sandbox lifecycle / agent events
    // on this task surface in Linear's UI.
    linear_active_turns.open(&task_id);

    let team_id = issue
        .team_id
        .clone()
        .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
        .unwrap_or_default();
    let destination_config = super::subject::session_destination_config(
        session,
        &issue.id,
        &issue.identifier,
        issue.url.as_deref(),
        &team_id,
    );
    let binding = task_bindings
        .bind(
            connection,
            Some(rule),
            &task_id,
            ADAPTER,
            SUBJECT_KIND,
            &session.id,
            destination_config,
            ADAPTER_EVENT_BINDING_ORIGIN,
            None,
            &BindScope::default(),
        )
        .await?;

    let task_url = super::text::task_url(config, &task_id);
    let attachment_title = super::task_links::task_attachment_title(
        tasks,
        &task_id,
        super::text::ATTACHMENT_TITLE_CHILD,
    )
    .await;
    if let Err(e) = api
        .agent_session_update_external_urls(&session.id, &attachment_title, &task_url)
        .await
    {
        warn!(session_id = %session.id, error = %e, "Failed to update Linear externalUrls");
    }
    if let Some(ref issue) = session.issue
        && let Err(e) = api
            .attachment_link_url(&issue.id, &task_url, &attachment_title)
            .await
    {
        warn!(
            session_id = %session.id,
            issue_id = %issue.id,
            error = %e,
            "Failed to attach Moatless task URL to Linear issue"
        );
    }

    // Forward the inbound prompt: issue link + instructions to read the
    // ticket and its comments via the `moat` CLI.
    let inbound_text = super::text::build_inbound_text(session);
    let external_event_id = super::external_id::session_created(&session.id);
    let creator_handle = super::webhook_payloads::actor_handle(session.creator.as_ref());
    if let Err(e) = message_send
        .accept(InboundTurn {
            task_id: task_id.clone(),
            text: inbound_text.clone(),
            file_ids: Vec::new(),
            resume: false,
            origin: TurnOrigin {
                event_id: Some(external_event_id.clone()),
                adapter: Some(ADAPTER_LABEL.to_string()),
                subject_key: Some(binding.subject_key.clone()),
                source_name: Some(rule.source_matcher.clone()),
                author: creator_handle.clone(),
                sender_id: None,
            },
            sent_by: session.creator.as_ref().map(|c| c.id.clone()),
            skills: Vec::new(),
        })
        .await
    {
        warn!(
            task_id = %task_id,
            session_id = %session.id,
            error = %e,
            "Failed to forward Linear prompt to task"
        );
    }

    // Move the issue into the rule's "started" state on mention/assignment
    // pickup, so the board reflects it — same team-scoped transition as the
    // status-entry path. Best-effort. No self-retrigger: this loop's
    // status-entry path is off, so the resulting Issue.update doesn't feed
    // back here.
    let start_state = LinearRuleConfig::from_rule(rule).start_state;
    let already_there = |start: &str| {
        issue
            .state
            .as_ref()
            .is_some_and(|s| s.name.eq_ignore_ascii_case(start))
    };
    if let Some(start_state) = start_state.as_deref().filter(|s| !already_there(s)) {
        let current = current_issue_state(api, issue).await;
        if moves_to_start_state(current.as_ref(), start_state) {
            transition_issue_state(api, &team_id, &issue.id, &issue.identifier, start_state).await;
        }
    }

    Ok(())
}

/// Subject kind of the creation claim on one agent session, keyed by its id.
/// No Binding carries it.
const SESSION_TASK_SUBJECT_KIND: &str = "linear_session_task";

/// Subject kind of the creation claim on one reopen of an issue, keyed by the
/// issue and Linear's clock. No Binding carries it.
const REOPEN_SUBJECT_KIND: &str = "linear_issue_reopen";

/// Open a new session and Task on an issue moved back to a non-terminal state
/// while delegated to the app, once all its sessions have closed.
///
/// Linear opens a session only on a first delegation or a mention. An open
/// session Task is skipped: `notify_bindings_of_state_change` tells it. A
/// re-delegation that moves no state never reaches here (`issue_event`).
#[expect(
    clippy::too_many_arguments,
    reason = "the session provisioning ports of `provision_selected_session_task`; \
              the same AppState-shaped bag"
)]
pub(super) async fn restart_session_on_reopen(
    task_bindings: &Bindings,
    tasks: &dyn TaskReader,
    config: &Config,
    task_creation: &dyn TaskOpener,
    task_relations: &dyn RelationRecorder,
    message_send: &dyn MessageAcceptor,
    linear_active_turns: &ActiveLinearTurns,
    identity: &dyn ProviderIdentityLink,
    routing: &RoutingService,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    update: &IssueUpdateDelivery,
) -> Result<(), AdapterError> {
    let issue = &update.issue;
    let Some(rule) = rules.first() else {
        return Ok(());
    };
    if !reopens(update, rules) {
        return Ok(());
    }
    let sessions =
        super::subject::bindings_on_issue(task_bindings, connection, SUBJECT_KIND, &issue.id)
            .await?;
    if !all_sessions_closed(&sessions) {
        return Ok(());
    }
    let Some(app_user) = connection
        .transport_config
        .get("agent_viewer_id")
        .and_then(serde_json::Value::as_str)
    else {
        warn!(connection_id = %connection.id, "Linear connection names no agent_viewer_id — cannot tell whether a reopened issue is delegated");
        return Ok(());
    };
    let detail = api.get_issue(&issue.id).await?;
    if detail
        .pointer("/delegate/id")
        .and_then(serde_json::Value::as_str)
        != Some(app_user)
    {
        return Ok(());
    }

    let transition = update
        .transition_id
        .clone()
        .unwrap_or_else(|| update.delivered_at.to_string());
    let event_id = super::external_id::issue_state(&issue.id, &transition);
    let claim_key = format!("{}:{transition}", issue.id);
    if !routing
        .claim_task_creation_once(
            rule,
            REOPEN_SUBJECT_KIND,
            &claim_key,
            ADAPTER_LABEL,
            Some(&event_id),
        )
        .await?
    {
        return Ok(());
    }

    // Released on failure so the redelivery retries rather than finding the
    // claim held and dropping the reopen.
    let session_id = match api.agent_session_create_on_issue(&issue.id).await {
        Ok(id) => id,
        Err(e) => {
            if let Err(release) = routing
                .release_task_creation(&rule.id, REOPEN_SUBJECT_KIND, &claim_key)
                .await
            {
                warn!(issue = %issue.identifier, error = %release, "Failed to release Linear reopen claim");
            }
            return Err(e);
        }
    };
    info!(
        issue = %issue.identifier,
        session_id,
        "Opened a Linear agent session on a reopened issue"
    );
    let session = AgentSession {
        id: session_id,
        issue: Some(AgentSessionIssue {
            id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
            description: None,
            url: issue.url.clone(),
            team_id: issue.resolved_team_id().map(str::to_string),
            team: None,
            state: issue.state.clone(),
        }),
        comment: None,
        creator: update.actor.clone(),
        prompt_context: None,
    };
    // Linear's `created` webhook for this session can run concurrently; the
    // session claim in `provision_session_task` lets only one create the task.
    provision_selected_session_task(
        task_bindings,
        tasks,
        config,
        task_creation,
        task_relations,
        message_send,
        linear_active_turns,
        identity,
        routing,
        api,
        connection,
        rules,
        &session,
    )
    .await
}

/// Whether `update` moved the issue into a state no rule closes on or starts a
/// status-entry Task for, the latter owning that move.
fn reopens(update: &IssueUpdateDelivery, rules: &[SubscriptionMatch]) -> bool {
    let issue = &update.issue;
    let previous = update
        .updated_from
        .as_ref()
        .and_then(|v| v.get("stateId"))
        .and_then(serde_json::Value::as_str);
    let Some(current) = issue.state.as_ref() else {
        return false;
    };
    if previous.is_none_or(|p| p == current.id) {
        return false;
    }
    rules.iter().map(LinearRuleConfig::from_rule).all(|cfg| {
        !cfg.is_terminal(issue.state_kind(), issue.state_name())
            && !cfg.creates_on_status(issue.state_name())
    })
}

/// Whether the issue had an agent session and every one has closed.
fn all_sessions_closed(sessions: &[TaskBinding]) -> bool {
    !sessions.is_empty() && sessions.iter().all(|b| !b.active)
}

/// An issue's workflow state at session pickup, with its team's default state.
#[derive(Debug, Deserialize)]
struct PickupState {
    state: LinearWorkflowState,
    #[serde(default)]
    team: Option<PickupTeam>,
}

#[derive(Debug, Deserialize)]
struct PickupTeam {
    #[serde(rename = "defaultIssueState")]
    default_issue_state: Option<LinearWorkflowState>,
}

impl PickupState {
    fn is_team_default(&self) -> bool {
        self.team
            .as_ref()
            .and_then(|t| t.default_issue_state.as_ref())
            .is_some_and(|d| d.id == self.state.id)
    }
}

/// The issue's state at pickup, read from Linear unless the session payload
/// carries a typed state that is not `unstarted`, the one kind that needs the
/// team's default. `None` when the read fails.
async fn current_issue_state(
    api: &LinearAgentApi,
    issue: &AgentSessionIssue,
) -> Option<PickupState> {
    if let Some(state) = issue
        .state
        .as_ref()
        .filter(|s| !s.kind.is_empty() && s.kind != "unstarted")
    {
        return Some(PickupState {
            state: state.clone(),
            team: None,
        });
    }
    match api
        .fetch_issue(
            &issue.id,
            "state { id name type } team { defaultIssueState { id name type } }",
        )
        .await
    {
        Ok(detail) => serde_json::from_value(detail).ok(),
        Err(e) => {
            warn!(issue = %issue.identifier, error = %e, "Failed to read Linear issue state");
            None
        }
    }
}

/// Whether a session pickup moves an issue in `current` to `start_state`.
///
/// Only backlog, triage and the team's default `unstarted` state move. A later
/// state would be dragged back on every mention; another `unstarted` one, such
/// as `Ready`, was chosen by whoever set it, often a sub-issue's creator
/// delegating it. An unknown state or type is left alone.
fn moves_to_start_state(current: Option<&PickupState>, start_state: &str) -> bool {
    current.is_some_and(|current| {
        !current.state.name.eq_ignore_ascii_case(start_state)
            && match current.state.kind.as_str() {
                "backlog" | "triage" => true,
                "unstarted" => current.is_team_default(),
                _ => false,
            }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::rule_with;

    fn moved_to(name: &str, kind: &str) -> IssueUpdateDelivery {
        serde_json::from_value(serde_json::json!({
            "connection_id": "pc_test",
            "issue": {
                "id": "issue-1",
                "identifier": "SP-1",
                "teamId": "team-uuid",
                "state": { "id": "s-new", "name": name, "type": kind },
            },
            "updated_from": { "stateId": "s-done" },
            "delivered_at": 1,
        }))
        .expect("delivery parses")
    }

    fn session_binding(active: bool) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "ts_1".to_string(),
            task_id: "task-1".to_string(),
            adapter_kind: moatless_types::adapter::AdapterKind::Linear,
            subject_kind: SUBJECT_KIND.to_string(),
            subject_key: "session-1".to_string(),
            connection_id: "pc_test".to_string(),
            loop_id: None,
            destination_config: serde_json::json!({ "issue_id": "issue-1" }),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "ingress".to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active,
            deactivated_at: None,
            watching: true,
        }
    }

    #[test]
    fn moving_an_issue_back_to_an_open_state_reopens_it() {
        let rules = [rule_with(serde_json::json!({}), true)];
        assert!(reopens(&moved_to("Ready", "unstarted"), &rules));
        assert!(reopens(&moved_to("In Progress", "started"), &rules));
    }

    #[test]
    fn a_move_into_a_terminal_state_or_no_move_is_not_a_reopen() {
        let rules = [rule_with(serde_json::json!({}), true)];
        assert!(!reopens(&moved_to("Done", "completed"), &rules));
        assert!(!reopens(&moved_to("Canceled", "canceled"), &rules));
        let mut unmoved = moved_to("Ready", "unstarted");
        unmoved.updated_from = Some(serde_json::json!({ "stateId": "s-new" }));
        assert!(!reopens(&unmoved, &rules));
        unmoved.updated_from = None;
        assert!(!reopens(&unmoved, &rules));
    }

    /// A status-entry rule starts its own Task on that move; a session on top
    /// would be a second one.
    #[test]
    fn a_move_a_status_entry_rule_acts_on_is_left_to_it() {
        let rules = [rule_with(
            serde_json::json!({ "create_on_status_entry": true, "trigger_states": ["Ready"] }),
            true,
        )];
        assert!(!reopens(&moved_to("Ready", "unstarted"), &rules));
    }

    #[test]
    fn only_an_issue_whose_every_session_closed_gets_a_new_one() {
        assert!(all_sessions_closed(&[session_binding(false)]));
        assert!(
            !all_sessions_closed(&[]),
            "never delegated: Linear opens the first session"
        );
        assert!(
            !all_sessions_closed(&[session_binding(false), session_binding(true)]),
            "an open session Task hears the move itself"
        );
    }

    fn state(name: &str, kind: &str) -> PickupState {
        PickupState {
            state: LinearWorkflowState {
                id: format!("s-{name}"),
                name: name.to_string(),
                kind: kind.to_string(),
            },
            team: None,
        }
    }

    fn team_default(mut current: PickupState) -> PickupState {
        current.team = Some(PickupTeam {
            default_issue_state: Some(current.state.clone()),
        });
        current
    }

    #[test]
    fn a_backlog_or_triage_ticket_moves_to_the_start_state() {
        for kind in ["backlog", "triage"] {
            assert!(
                moves_to_start_state(Some(&state("Backlog", kind)), "In Progress"),
                "{kind}"
            );
        }
    }

    #[test]
    fn a_ticket_in_the_teams_default_unstarted_state_moves() {
        assert!(moves_to_start_state(
            Some(&team_default(state("Todo", "unstarted"))),
            "In Progress"
        ));
    }

    /// A sub-issue created with `--state Ready` and delegated to the app opens
    /// a session; that pickup must leave its state alone.
    #[test]
    fn a_ticket_in_a_chosen_unstarted_state_keeps_it() {
        let ready: PickupState = serde_json::from_value(serde_json::json!({
            "state": { "id": "s-ready", "name": "Ready", "type": "unstarted" },
            "team": { "defaultIssueState":
                { "id": "s-refine", "name": "Needs Refinement", "type": "backlog" } },
        }))
        .expect("pickup state parses");
        assert!(!moves_to_start_state(Some(&ready), "In Progress"));
        assert!(
            !moves_to_start_state(Some(&state("Ready", "unstarted")), "In Progress"),
            "an unknown team default is left alone"
        );
    }

    #[test]
    fn a_ticket_in_an_unknown_state_keeps_it() {
        assert!(!moves_to_start_state(None, "In Progress"));
        assert!(!moves_to_start_state(
            Some(&state("User Test in Staging", "")),
            "In Progress"
        ));
    }

    #[test]
    fn a_started_or_closed_ticket_keeps_its_state() {
        for (name, kind) in [
            ("In Progress", "started"),
            ("User Test in Staging", "started"),
            ("Done", "completed"),
            ("Canceled", "canceled"),
        ] {
            assert!(
                !moves_to_start_state(Some(&state(name, kind)), "In Progress"),
                "{name} must not move back"
            );
        }
    }

    #[test]
    fn a_ticket_already_in_the_start_state_does_not_move() {
        assert!(!moves_to_start_state(
            Some(&state("in progress", "backlog")),
            "In Progress"
        ));
    }
}
