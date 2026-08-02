//! What an issue changing state does.
//!
//! Three separate things, and they are independent of each other: every bound
//! Task is told the issue moved; an issue entering a configured state may spawn
//! a Task that has no Linear session behind it; and an issue entering a
//! terminal state may close the Tasks it produced (that last one lives in
//! [`close`](super::close), because a notification can trigger it too).
//!
//! The identity of a state change is the issue plus Linear's own delivery
//! timestamp, not the state it moved into: an issue that leaves a state and
//! comes back produces two changes a person made separately, and naming them
//! after the state would silently drop the second as a duplicate.
//!
//! A status-entry Task is bound with `linear_issue`, not
//! `linear_agent_session`. There is no panel to write activities into, which is
//! why this workflow links the Task back with an attachment instead, and why a
//! preflight failure has to be reported as an ordinary issue comment.

use std::fmt::Write as _;

use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, SubscriptionMatch, ADAPTER_EVENT_BINDING_ORIGIN};
use crate::bindings::dao::PgTaskBindingDao;
use crate::state::AppState;
use crate::task::models::Visibility;

use super::agent_api::LinearAgentApi;
use super::close::close_active_bindings_for_issue;
use super::config::LinearRuleConfig;
use super::subject::{ADAPTER, SUBJECT_KIND_SESSION as SUBJECT_KIND};
use super::webhook_payloads::IssuePayload;

const ADAPTER_LABEL: &str = "linear";

/// Handle an `Issue.update` payload.
///
/// On any real state transition we forward a notification message to every
/// bound task so the agent learns the issue changed status. When the new
/// state is in the rule's `terminal_states` and `close_task_on_close` is set,
/// the existing close path also runs and terminates the task.
///
/// `delivered_at` is Linear's own `webhookTimestamp`. It is what tells two
/// changes to one issue apart — including an issue that leaves a state and
/// later returns to it, where nothing else about the two deliveries differs —
/// while a retry of one change repeats it and is recognised as the same
/// delivery.
pub(super) async fn handle_issue_update(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    envelope_updated_from: Option<&serde_json::Value>,
    issue: &IssuePayload,
    delivered_at: i64,
) -> Result<(), AdapterError> {
    let state_kind = issue.state_kind();
    let prev_state_id = envelope_updated_from
        .and_then(|v| v.get("stateId"))
        .and_then(serde_json::Value::as_str);
    let new_state_id = issue.state.as_ref().map(|s| s.id.as_str());

    // Only act on real state transitions.
    if prev_state_id.is_none() || new_state_id.is_none() || prev_state_id == new_state_id {
        return Ok(());
    }

    // Persist the new workflow state onto every binding for this issue — for
    // every transition, not just terminal ones. Best-effort.
    persist_issue_status(state, connection, issue).await;

    let Some(rule) = rules.first() else {
        debug!(issue = %issue.identifier, "No Linear rule found for team — ignoring update");
        return Ok(());
    };
    let rule_config = LinearRuleConfig::from_rule(rule);

    // Forward a state-change notification to every bound task (best-effort).
    notify_bindings_of_state_change(state, connection, issue, delivered_at).await;

    if rule_config.close_task_on_close
        && rule_config.terminal_states.iter().any(|s| s == state_kind)
    {
        close_active_bindings_for_issue(state, api, rule, issue).await?;
    }

    // Status-entry creation: spawn a fresh task for every rule opted into
    // `create_on_status_entry` whose `trigger_states` include the new state.
    // Independent of the notify/close paths above (which only touch already-
    // bound tasks). Best-effort — failures are logged per rule.
    create_tasks_for_status_entry(state, api, connection, rules, issue).await;

    Ok(())
}

/// Spawn new tasks for rules opted into `create_on_status_entry` whose
/// `trigger_states` include the issue's new workflow state. Applies each rule's
/// project + label filters and is idempotent per `(issue, trigger)`.
///
/// Project + label filtering needs data the `Issue.update` webhook may omit, so
/// when a matching rule filters on them we fetch the full issue once via
/// `get_issue` (skipped when the webhook already carries the project id and no
/// rule filters on labels). A fetch failure logs and falls back to the
/// webhook-carried project id with empty labels.
async fn create_tasks_for_status_entry(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    issue: &IssuePayload,
) {
    let state_name = issue.state_name();
    let candidates: Vec<(&SubscriptionMatch, LinearRuleConfig)> = rules
        .iter()
        .map(|r| (r, LinearRuleConfig::from_rule(r)))
        .filter(|(_, cfg)| cfg.creates_on_status(state_name))
        .collect();
    if candidates.is_empty() {
        return;
    }

    let webhook_project = issue.resolved_project_id().map(str::to_string);
    let needs_labels = candidates
        .iter()
        .any(|(_, cfg)| !cfg.require_labels.is_empty());
    let needs_project_fetch = webhook_project.is_none()
        && candidates
            .iter()
            .any(|(_, cfg)| !cfg.project_ids.is_empty());

    let detail = if needs_labels || needs_project_fetch {
        match api.get_issue(&issue.id).await {
            Ok(v) => Some(v),
            Err(e) => {
                warn!(
                    issue = %issue.identifier,
                    error = %e,
                    "Failed to fetch Linear issue for status-entry filtering; using webhook payload"
                );
                None
            }
        }
    } else {
        None
    };

    let project_id: Option<String> = detail
        .as_ref()
        .and_then(|d| d.pointer("/project/id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(webhook_project);

    let labels: Vec<String> = detail
        .as_ref()
        .and_then(|d| d.pointer("/labels/nodes"))
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n.get("name").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    for (rule, cfg) in candidates {
        if !cfg.project_allowed(project_id.as_deref()) {
            debug!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                "Linear status-entry skipped: issue project not in allowlist"
            );
            continue;
        }
        if !cfg.labels_allowed(&labels) {
            debug!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                "Linear status-entry skipped: required labels missing"
            );
            continue;
        }
        if let Err(e) = provision_issue_task(state, api, connection, rule, &cfg, issue).await {
            warn!(
                issue = %issue.identifier,
                subscription_id = %rule.id,
                error = %e,
                "Failed to provision Linear status-entry task"
            );
        }
    }
}

/// Create + provision a task for an `Issue.update` status-entry match, bind it
/// to the issue via `linear_issue`, link it back on the Linear issue, and
/// forward the inbound "read the ticket" prompt.
///
/// Idempotent per `(issue, trigger)`: if an active `linear_issue` binding for
/// this issue already exists for this trigger+connection, returns early so a
/// re-entry into the same state doesn't double-create.
async fn provision_issue_task(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    cfg: &LinearRuleConfig,
    issue: &IssuePayload,
) -> Result<(), AdapterError> {
    let existing = PgTaskBindingDao(state.db.clone())
        .find_active_by_destination_field(ADAPTER, super::SUBJECT_KIND_ISSUE, "issue_id", &issue.id)
        .await?;
    if existing
        .iter()
        .any(|b| b.connection_id == connection.id && b.loop_id.as_deref() == Some(rule.id.as_str()))
    {
        info!(
            issue = %issue.identifier,
            subscription_id = %rule.id,
            "Linear issue already has an active task for this trigger — skipping status-entry creation"
        );
        return Ok(());
    }

    let preview = super::text::build_issue_inbound_text(issue);
    let task_id = match insert_event_task(state, rule, &issue.identifier, &issue.title, &preview)
        .await
    {
        Ok(id) => id,
        // Preflight failed: surface the reason as an issue comment (the
        // status-entry path has no agent session), then bail out — the errored
        // task gets no binding or prompt.
        Err(AdapterError::AccessDenied(msg)) => {
            if let Err(e) = api
                .create_comment(&issue.id, &format!("Couldn't start this task: {msg}"))
                .await
            {
                warn!(issue = %issue.identifier, error = %e, "Failed to post Linear preflight error");
            }
            return Err(AdapterError::AccessDenied(msg));
        }
        Err(e) => return Err(e),
    };
    // Open the Linear turn gate so sandbox/agent events on this task surface in
    // Linear's UI. Provisioning already ran inside the creation service.
    state.linear_active_turns.open(&task_id);

    let team_id = issue.resolved_team_id().unwrap_or_default().to_string();
    let destination_config = super::subject::issue_destination_config(issue, &team_id);
    let binding = crate::bindings::runtime::upsert_binding(
        state,
        connection,
        Some(rule),
        &task_id,
        ADAPTER,
        super::SUBJECT_KIND_ISSUE,
        &issue.id,
        destination_config,
        ADAPTER_EVENT_BINDING_ORIGIN,
        None,
    )
    .await?;

    // Link the Moatless task back onto the Linear issue.
    let task_url = super::text::task_url(state, &task_id);
    let attachment_title =
        super::text::task_attachment_title(state, &task_id, super::text::ATTACHMENT_TITLE_CHILD)
            .await;
    if let Err(e) = api
        .attachment_link_url(&issue.id, &task_url, &attachment_title)
        .await
    {
        warn!(
            issue = %issue.identifier,
            error = %e,
            "Failed to attach Moatless task URL to Linear issue"
        );
    }

    // Forward the inbound prompt: issue link + read-the-ticket instructions.
    let inbound_text = super::text::build_issue_inbound_text(issue);
    let external_event_id = super::external_id::issue_created(&issue.id);
    if let Err(e) = crate::message::send::send_message_flow(
        state,
        &task_id,
        &inbound_text,
        None,
        None,
        None,
        &[],
        &[],
        false,
        Some(&external_event_id),
        Some(ADAPTER_LABEL),
        Some(&binding.subject_key),
        Some(rule.source_matcher.as_str()),
        None,
        false,
        None,
        None,
    )
    .await
    {
        warn!(
            task_id = %task_id,
            issue = %issue.identifier,
            error = %e,
            "Failed to forward Linear issue prompt to task"
        );
    }

    // Move the issue into the configured "started" state (e.g. In Progress) so
    // the Linear board reflects that the agent picked it up. Best-effort, and
    // safe against self-retrigger: the resulting `Issue.update` re-enters this
    // path but the binding created above trips the idempotency guard. We only
    // transition when the issue isn't already in the target state, so a loop
    // whose `start_state` overlaps `trigger_states` doesn't thrash.
    if let Some(start_state) = cfg.start_state.as_deref() {
        if !issue.state_name().eq_ignore_ascii_case(start_state) {
            transition_issue_state(api, &team_id, &issue.id, &issue.identifier, start_state).await;
        }
    }

    info!(
        task_id = %task_id,
        issue = %issue.identifier,
        subscription_id = %rule.id,
        "Created task from Linear issue status entry"
    );
    Ok(())
}

/// Best-effort: resolve `start_state` to a team-scoped workflow-state id and
/// move the issue into it via `issueUpdate`. Primitive-arg form shared by the
/// status-entry path (`IssuePayload`) and the mention/assignment pickup path
/// (`AgentSessionIssue`) — the two carry different issue payload types, so the
/// shared move logic takes plain ids. Failures (missing team, unresolved state
/// name, API error) are logged and swallowed — the task is already created, so
/// a missed board transition must not fail the pickup. Callers guard against
/// no-op transitions (issue already in the target state) before invoking.
pub(super) async fn transition_issue_state(
    api: &LinearAgentApi,
    team_id: &str,
    issue_id: &str,
    issue_identifier: &str,
    start_state: &str,
) {
    if team_id.is_empty() {
        warn!(
            issue = %issue_identifier,
            "Cannot move Linear issue to start state: missing team id"
        );
        return;
    }
    match api.find_team_state_id(team_id, start_state).await {
        Ok(Some(state_id)) => {
            match api
                .update_issue(issue_id, serde_json::json!({ "stateId": state_id }))
                .await
            {
                Ok(_) => info!(
                    issue = %issue_identifier,
                    start_state,
                    "Moved Linear issue to start state on pickup"
                ),
                Err(e) => warn!(
                    issue = %issue_identifier,
                    start_state,
                    error = %e,
                    "Failed to move Linear issue to start state"
                ),
            }
        }
        Ok(None) => warn!(
            issue = %issue_identifier,
            start_state,
            team_id,
            "Start state not found in the issue's team — leaving issue state unchanged"
        ),
        Err(e) => warn!(
            issue = %issue_identifier,
            start_state,
            error = %e,
            "Failed to resolve Linear start state"
        ),
    }
}

/// Persist the issue's workflow state onto every binding tied to it (both
/// `linear_agent_session` and `linear_issue` subject kinds). Writes the
/// dedicated `external_status*` columns. Best-effort: failures are logged.
async fn persist_issue_status(
    state: &AppState,
    connection: &AdapterConnection,
    issue: &IssuePayload,
) {
    let repo = PgTaskBindingDao(state.db.clone());
    let status = super::status::linear_external_status(
        issue.state_name(),
        issue.state_kind(),
        Some(issue.title.as_str()),
        issue.url.as_deref(),
    );

    for subject_kind in [SUBJECT_KIND, super::SUBJECT_KIND_ISSUE] {
        let bindings = match super::subject::active_bindings_on_issue(
            state,
            connection,
            subject_kind,
            &issue.id,
        )
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!(
                    issue = %issue.identifier,
                    subject_kind,
                    error = %e,
                    "Failed to look up bindings for Linear status persistence"
                );
                continue;
            }
        };
        for binding in bindings {
            if let Err(e) = repo
                .update_external_status(
                    &binding.id,
                    status.status.as_deref(),
                    status.state_kind.as_deref(),
                    &status.raw,
                )
                .await
            {
                warn!(
                    binding_id = %binding.id,
                    error = %e,
                    "Failed to persist Linear status onto binding"
                );
            }
        }
    }
}

/// Post a state-change message to every active agent-session binding for the
/// issue. Best-effort: failures are logged so the close path still runs.
async fn notify_bindings_of_state_change(
    state: &AppState,
    connection: &AdapterConnection,
    issue: &IssuePayload,
    delivered_at: i64,
) {
    let bindings =
        match super::subject::active_bindings_on_issue(state, connection, SUBJECT_KIND, &issue.id)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!(
                    issue = %issue.identifier,
                    error = %e,
                    "Failed to look up bindings for Linear state-change notification"
                );
                return;
            }
        };

    let new_state = issue.state_name();
    let mut body = format!(
        "Linear issue {} moved to \"{}\".",
        issue.identifier, new_state
    );
    if let Some(ref url) = issue.url {
        let _ = write!(body, "\n\n{url}");
    }

    for binding in bindings {
        let external_event_id = super::external_id::issue_state(&issue.id, delivered_at);
        if let Err(e) = crate::message::send::send_message_flow(
            state,
            &binding.task_id,
            &body,
            None,
            None,
            None,
            &[],
            &[],
            true,
            Some(&external_event_id),
            Some(ADAPTER_LABEL),
            Some(&binding.subject_key),
            None,
            None,
            false,
            None,
            None,
        )
        .await
        {
            warn!(
                task_id = %binding.task_id,
                issue = %issue.identifier,
                error = %e,
                "Failed to forward Linear state change to task"
            );
        }
    }
}

/// Build + insert the Task row shared by the session- and issue-initiated
/// Linear creation paths, assign the trigger's tags, and publish the `Open`
/// event. Returns the new task id. Callers own subsequent provisioning,
/// binding, and inbound-message delivery.
pub(super) async fn insert_event_task(
    state: &AppState,
    rule: &SubscriptionMatch,
    identifier: &str,
    title: &str,
    preview: &str,
) -> Result<String, AdapterError> {
    // Compose the shared spine's single task-creation entry point. Linear adds
    // no origin prelude; its only per-path inputs are the `{identifier} — {title}`
    // name fallback, public visibility (so issue collaborators can open the
    // task), and the `source_matcher` event metadata. The inbound prompt is
    // delivered by the caller through the binding, so no first message is passed.
    //
    // Preflight, insert, tags, provisioning, and the `Open` event all run inside
    // the creation service. On preflight failure it records the task as errored
    // (with a `SandboxStatus::Error` event) and returns an error; here we surface
    // it as `AccessDenied` so callers can post the reason back to Linear — the
    // service's error event alone stays invisible there because no turn gate is
    // open for a task that never got a binding.
    let config = crate::bindings::runtime::subscription_context(rule, ADAPTER_LABEL);
    let outcome = crate::loops::firing::inbound::create_task_from_loop(
        state,
        &config,
        crate::loops::firing::inbound::CreateTaskParams {
            name: rule
                .task_name
                .clone()
                .or_else(|| Some(format!("{identifier} — {title}"))),
            description: crate::bindings::outbound::truncate_text(preview, 200),
            visibility: Visibility::Public,
            origin_prelude: None,
            related_to: None,
            event_metadata: serde_json::json!({
                "adapter": ADAPTER_LABEL,
                "source_matcher": rule.source_matcher,
            }),
        },
    )
    .await
    .map_err(|e| match e {
        crate::task::creation::TaskCreationError::Preflight(msg) => AdapterError::AccessDenied(msg),
        other => AdapterError::TaskCreation(other),
    })?;

    Ok(outcome.task.task_id)
}

/// Attach the moatless task URL to a Linear issue as a "source" pill, titled
/// for the case where the Moatless side is the parent. Idempotent on
/// (issue_id, url). Failures are logged and swallowed — the caller has already
/// created the issue / posted the comment this links back from.
pub(super) async fn attach_parent_task_link(
    state: &AppState,
    api: &LinearAgentApi,
    issue_uuid: &str,
    task_id: &str,
) {
    let url = super::text::task_url(state, task_id);
    let title =
        super::text::task_attachment_title(state, task_id, super::text::ATTACHMENT_TITLE_PARENT)
            .await;
    if let Err(e) = api.attachment_link_url(issue_uuid, &url, &title).await {
        warn!(
            error = %e,
            task_id = %task_id,
            issue = %issue_uuid,
            "Failed to attach Moatless task URL to Linear issue"
        );
    }
}
