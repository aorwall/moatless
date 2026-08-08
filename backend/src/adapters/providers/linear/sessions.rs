//! The two Linear agent-session workflows: a session created, and a prompt
//! typed into one that already exists.
//!
//! A session is what Linear opens when someone mentions or assigns the agent.
//! It comes with a panel in the issue view, and the panel is the whole reason
//! these two workflows look the way they do: it is visible, so it must be
//! answered promptly, and it is where every later activity for the task lands.
//!
//! `created` always spawns a Task — a mention is an explicit instruction, so
//! the issue's workflow state is not consulted. `prompted` never does: it
//! routes into the Task the session is already bound to, and a prompt naming no
//! Binding is an ordinary outcome (the session belongs to a Task that has since
//! closed) rather than an error.
//!
//! Both open with a heartbeat activity before doing anything slow, because
//! Linear renders the panel as pending until something arrives. `created`'s is
//! the one thing this Adapter posts to Linear before acknowledging a webhook,
//! so it is described by [`webhook::normalize`](super::webhook::normalize) and
//! applied by [`ingress`](super::ingress); `prompted`'s runs here, in the
//! worker. Either way it is the first entry in the workflow's API-call budget,
//! and the reason `AgentSessionEvent.created` costs one call even when it
//! decides to do nothing.

use tracing::{info, warn};

use crate::adapters::error::{AdapterError, SharedAdapterError};
use crate::adapters::models::{
    ADAPTER_EVENT_BINDING_ORIGIN, AdapterConnection, SubscriptionMatch, TaskBinding,
};
use crate::state::AppState;

use super::agent_api::{ActivityKind, LinearAgentApi};
use super::config::{LinearRuleConfig, rules_need_issue_fetch, select_spawn_rule};
use super::issues::{insert_event_task, transition_issue_state};
use super::subject::{ADAPTER, SUBJECT_KIND_SESSION as SUBJECT_KIND};
use super::webhook_payloads::AgentSession;

const ADAPTER_LABEL: &str = "linear";

/// Body of the terminal `error` activity posted when a created session reaches
/// no task: no active Subscription for the team, or one disabled between
/// admission and execution. Linear infers session state from the last activity,
/// so a created session that only ever got the ephemeral heartbeat stays
/// `active` in the panel forever; a terminal `error` closes it and tells the
/// user why nothing started.
pub(super) const NO_ACTIVE_RULE_DECLINE: &str = "No Moatless automation is configured for this Linear team, so I can't start a task for this issue.";

/// Close a Linear agent session with a terminal `error` activity when no task
/// will be provisioned behind it.
///
/// A created session that received only the ephemeral heartbeat sits `active`
/// in Linear's UI indefinitely — the heartbeat resets the 10-second stale timer
/// but nothing moves the session to a terminal state. Emitting `error` is what
/// closes the panel. Best-effort: a failure to post is logged, not propagated,
/// because the caller has already decided not to do the work.
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
/// This is the body of what used to be a detached spawn, now the executable
/// half of a `linear.session_provision` record. It is at-least-once, and it
/// converges: the binding guard inside [`provision_session_task`] is what makes
/// a second attempt a no-op rather than a second task.
pub(super) async fn provision_selected_session_task(
    state: &AppState,
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

    provision_session_task(state, api, connection, &rule, session).await
}

/// Project/label-aware spawn-rule selection for a mention/assignment session.
///
/// Skips the Linear API entirely when no rule is project-scoped
/// ([`rules_need_issue_fetch`] is false) — all-unscoped teams keep the cheap
/// `rules.first()` path and pay nothing. Otherwise fetches the issue once to
/// learn its project + labels (mirroring the extraction in
/// [`create_tasks_for_status_entry`]); a fetch failure degrades to `(None, &[])`,
/// which the selector routes to the unscoped catch-all / oldest rule.
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
/// `activity_id` is the id of the `agentActivity` carrying the prompt. It is
/// what the forwarded message is identified by downstream: it names *this*
/// prompt, so a second prompt on the same session is a second message, and a
/// Linear retry of one prompt repeats the id and is dropped as a duplicate.
pub(super) async fn handle_session_prompted(
    state: &AppState,
    api: &LinearAgentApi,
    session_id: &str,
    activity_id: &str,
    body: &str,
) -> Result<(), AdapterError> {
    let binding_repo = state.task_bindings.clone();
    let bindings: Vec<TaskBinding> = binding_repo
        .find_by_subject(ADAPTER, SUBJECT_KIND, session_id)
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

    let rule = super::subject::rule_for_binding(state, &bindings[0]).await?;

    for binding in bindings {
        // Re-open the Linear turn gate for this follow-up prompt so the
        // next agent run's events flow through to Linear's UI.
        state.linear_active_turns.open(&binding.task_id);
        let external_event_id = super::external_id::prompt(activity_id);
        if let Err(e) = state
            .message_send
            .send_message_flow(
                &binding.task_id,
                body,
                None,
                None,
                None,
                &[],
                &[],
                true,
                crate::message::send::ExternalOrigin {
                    event_id: Some(&external_event_id),
                    adapter: Some(ADAPTER_LABEL),
                    subject_key: Some(&binding.subject_key),
                    source_name: rule.as_ref().map(|r| r.source_matcher.as_str()),
                    // A prompt carries no actor; the session's creator is who this
                    // is attributed to, and the created path records that.
                    author: None,
                    source_task_id: None,
                },
                false,
                None,
                None,
            )
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

        if let Err(e) =
            crate::sandbox::notify::ensure_sandbox_and_notify(state, &binding.task_id).await
        {
            warn!(
                task_id = %binding.task_id,
                error = %e,
                "Failed to ensure sandbox for Linear prompted message"
            );
        }
    }

    Ok(())
}

async fn provision_session_task(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    session: &AgentSession,
) -> Result<(), AdapterError> {
    let issue = session
        .issue
        .as_ref()
        .ok_or_else(|| SharedAdapterError::BadRequest("AgentSession missing issue".into()))?;

    // Idempotency guard: if a binding already exists for this session id, this
    // is a second execution of work that already provisioned the task. Durable
    // admission collapses a redelivery of one `created` event, but the record
    // behind it is executed at-least-once — an attempt abandoned at its
    // deadline or interrupted by a restart is claimed again — so the guard is
    // what makes the *execution* converge. Return early rather than creating a
    // second task. Keyed on session id, so multiple *distinct* sessions on the
    // same issue still each get their own task.
    match state
        .task_bindings
        .find_by_subject(ADAPTER, SUBJECT_KIND, &session.id)
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
    let related = match state
        .task_bindings
        .find_active_by_destination_field(ADAPTER, SUBJECT_KIND, "issue_id", &issue.id)
        .await
    {
        Ok(bindings) => bindings
            .into_iter()
            .next()
            .map(|b| crate::task::relations::RelatedTask {
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
    let task_id = match insert_event_task(state, rule, &issue.identifier, &issue.title, &preview)
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
        crate::task::relations::record_related_to(state.task_relations.as_ref(), &task_id, r).await;
    }
    // Open the Linear turn gate so subsequent sandbox lifecycle / agent events
    // on this task surface in Linear's UI.
    state.linear_active_turns.open(&task_id);

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
    let binding = crate::bindings::runtime::upsert_binding(
        state,
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

    // Update the session with an externalUrl pointing at the moatless task.
    let task_url = super::text::task_url(state, &task_id);
    let attachment_title =
        super::text::task_attachment_title(state, &task_id, super::text::ATTACHMENT_TITLE_CHILD)
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
    let creator_handle = super::actor_handle(session.creator.as_ref());
    if let Err(e) = state
        .message_send
        .send_message_flow(
            &task_id,
            &inbound_text,
            None,
            None,
            None,
            &[],
            &[],
            false,
            crate::message::send::ExternalOrigin {
                event_id: Some(&external_event_id),
                adapter: Some(ADAPTER_LABEL),
                subject_key: Some(&binding.subject_key),
                source_name: Some(rule.source_matcher.as_str()),
                author: creator_handle.as_deref(),
                source_task_id: None,
            },
            false,
            session.creator.as_ref().map(|c| c.id.as_str()),
            None,
        )
        .await
    {
        warn!(
            task_id = %task_id,
            session_id = %session.id,
            error = %e,
            "Failed to forward Linear prompt to task"
        );
    }

    // Move the issue into the rule's configured "started" state (e.g. In
    // Progress) on mention/assignment pickup, so the Linear board reflects that
    // the agent picked it up. Reuses the same team-scoped transition as the
    // status-entry path. Best-effort and idempotent: skipped when the issue is
    // already in the target state. No self-retrigger risk — this loop's
    // status-entry path is off, and the resulting `→ In Progress` Issue.update
    // doesn't feed the mention path.
    if let Some(start_state) = LinearRuleConfig::from_rule(rule).start_state.as_deref() {
        let current_state = issue.state.as_ref().map_or("", |s| s.name.as_str());
        if !current_state.eq_ignore_ascii_case(start_state) {
            transition_issue_state(api, &team_id, &issue.id, &issue.identifier, start_state).await;
        }
    }

    Ok(())
}
