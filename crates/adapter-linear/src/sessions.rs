//! The two Linear agent-session workflows: a session created, and a prompt
//! typed into one that already exists.
//!
//! Both open a heartbeat activity before any slow work, since a session gets
//! a visible panel — `created`'s heartbeat is in
//! [`webhook::normalize`](super::webhook::normalize)/`ingress`,
//! `prompted`'s here. `created` always spawns a Task (a mention is explicit);
//! `prompted` routes into its already-bound Task — a missing Binding means
//! that task closed already, not an error.

use tracing::{info, warn};

use moatless_core::config::Config;

use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::operations::Bindings;
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
use super::issues::{insert_event_task, transition_issue_state};
use super::subject::{ADAPTER, SUBJECT_KIND_SESSION as SUBJECT_KIND};
use super::webhook_payloads::AgentSession;

const ADAPTER_LABEL: &str = moatless_types::adapter::AdapterKind::Linear.as_str();

/// Body of the terminal `error` activity posted when a created session reaches
/// no task (no active Subscription for the team, or one disabled between
/// admission and execution).
///
/// Linear infers session state from the last activity, so only a terminal
/// `error` closes a panel stuck `active` from the ephemeral heartbeat alone.
pub(super) const NO_ACTIVE_RULE_DECLINE: &str = "No Moatless automation is configured for this Linear team, so I can't start a task for this issue.";

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
    reason = "the seven downward ports the session provisioning path reads \
              (bindings, tasks, config, task-creation, task-relations, \
              message-send, active-turns) inverted out of the single AppState; \
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
        .map(|i| (i.id.as_str(), i.identifier.as_str()))
        .unwrap_or_default();
    let Some(rule) = select_session_spawn_rule(api, rules, issue_id, issue_identifier).await else {
        warn!(
            session_id = %session.id,
            "Linear session created but no spawn rule selected — skipping task creation"
        );
        // Close the session rather than leaving Linear's panel spinning.
        decline_session(api, &session.id, NO_ACTIVE_RULE_DECLINE).await;
        return Ok(());
    };

    provision_session_task(
        task_bindings,
        tasks,
        config,
        task_creation,
        task_relations,
        message_send,
        linear_active_turns,
        api,
        connection,
        &rule,
        session,
    )
    .await
}

/// Project/label-aware spawn-rule selection for a mention/assignment session.
///
/// Skips the Linear API when no rule is project-scoped
/// ([`rules_need_issue_fetch`] is false); otherwise fetches the issue once,
/// mirroring [`create_tasks_for_status_entry`]. A fetch failure degrades to
/// `(None, &[])`, routing to the unscoped/oldest catch-all rule.
async fn select_session_spawn_rule(
    api: &LinearAgentApi,
    rules: &[SubscriptionMatch],
    issue_id: &str,
    issue_identifier: &str,
) -> Option<SubscriptionMatch> {
    if !rules_need_issue_fetch(rules) {
        return rules.first().cloned();
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
                .map(|arr| {
                    arr.iter()
                        .filter_map(|n| n.get("name").and_then(serde_json::Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
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

    select_spawn_rule(rules, project_id.as_deref(), &labels).cloned()
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

#[expect(
    clippy::too_many_arguments,
    reason = "the seven downward ports one session task's creation reads \
              (bindings, tasks, config, task-creation, task-relations, \
              message-send, active-turns) inverted out of the single AppState; \
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

    // Find an earlier task already bound to this Linear issue, if any. The
    // first row by created_at is the peer that touched this issue earlier.
    // The lookup uses destination_config.issue_id so it matches both
    // `linear_agent_session` and `linear_issue` bindings. The new task is
    // linked as `related_to` — a peer sharing the issue, not a child.
    let related = match task_bindings
        .active_by_destination_field(ADAPTER, SUBJECT_KIND, "issue_id", &issue.id)
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
    let task_id = match insert_event_task(
        task_creation,
        rule,
        &issue.identifier,
        &issue.title,
        &preview,
    )
    .await
    {
        Ok(id) => {
            info!(task_id = %id, subscription_id = %rule.id, "Created task from Linear AgentSession");
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
    if let Some(ref issue) = session.issue {
        if let Err(e) = api
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
    // status-entry path. Best-effort and idempotent: skipped if already in
    // the target state. No self-retrigger: this loop's status-entry path is
    // off, so the resulting Issue.update doesn't feed back here.
    if let Some(start_state) = LinearRuleConfig::from_rule(rule).start_state.as_deref() {
        let current_state = issue.state.as_ref().map_or("", |s| s.name.as_str());
        if !current_state.eq_ignore_ascii_case(start_state) {
            transition_issue_state(api, &team_id, &issue.id, &issue.identifier, start_state).await;
        }
    }

    Ok(())
}
