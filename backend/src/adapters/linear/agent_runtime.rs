//! Linear Agents runtime — handles `AgentSessionEvent` and downstream
//! `Issue.update` events.
//!
//! Inbound paths:
//! - `handle_session_created` — mention or assignment. Always spawns a task;
//!   issue workflow state is not consulted.
//! - `handle_session_prompted` — follow-up prompt on an existing session.
//!   Routes to `send_message_flow` for the bound task.
//! - `handle_issue_update` — closes active tasks when the issue transitions
//!   INTO `terminal_states`. Non-terminal transitions are ignored.

use std::fmt::Write as _;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, TaskBinding, SubscriptionMatch, ADAPTER_EVENT_BINDING_ORIGIN,
};
use crate::bindings::dao::PgTaskBindingRepository;
use crate::sandbox::lifecycle::ArchiveRetention;
use crate::shared_models::AgentType;
use crate::state::AppState;
use crate::task::creation::NewTaskRequest;
use crate::task::dao::TaskPatch;
use crate::task::models::{TaskStatus, Visibility};

use super::agent_api::{default_http_client, ActivityKind, LinearAgentApi};
use super::config::{rules_need_issue_fetch, select_spawn_rule, LinearRuleConfig};
use super::webhook_payloads::{
    AgentSession, AppUserNotificationData, CommentPayloadData, IssuePayload,
};

const ADAPTER: &str = "linear";
const SUBJECT_KIND: &str = "linear_agent_session";
const ADAPTER_LABEL: &str = "linear";

/// Resolve every active task trigger bound to a given Linear team. Returns
/// every rule attached to the team — multi-rule support is preserved.
pub async fn resolve_rules_for_team(
    state: &AppState,
    connection: &AdapterConnection,
    team_id: &str,
) -> Result<Vec<SubscriptionMatch>, AdapterError> {
    let rules = state.loops
        .list_subscriptions_by_source_matcher(ADAPTER, team_id)
        .await?;
    Ok(rules
        .into_iter()
        .filter(|r| {
            if r.connection_id == connection.id && r.active {
                true
            } else {
                debug!(
                    rule_id = %r.id,
                    "Linear trigger ignored (different connection or inactive)"
                );
                false
            }
        })
        .collect())
}

/// Process an `AgentSessionEvent.created` payload. Always proceeds — mention
/// and assignment both spawn a task regardless of the issue's workflow state.
///
/// Performs synchronous Linear API calls (`agentActivityCreate`) before
/// spawning the long-running task creation so the webhook response stays
/// inside Linear's 10-second window.
pub async fn handle_session_created(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    session: &AgentSession,
) -> Result<(), AdapterError> {
    let issue = session
        .issue
        .as_ref()
        .ok_or_else(|| AdapterError::BadRequest("AgentSession missing issue payload".into()))?;

    // Emit an ephemeral heartbeat thought synchronously so Linear sees
    // activity within the 10s window. Marked ephemeral so the next
    // activity (e.g. the `Provisioned sandbox` action) replaces it in
    // the Linear UI rather than piling up.
    if let Err(e) = api
        .agent_activity_create(
            &session.id,
            ActivityKind::Thought,
            "Spinning up sandbox…",
            true,
        )
        .await
    {
        warn!(session_id = %session.id, error = %e, "Failed to emit Linear thought activity");
    }

    // Cheap guard only. The actual rule SELECTION — which may need a `get_issue`
    // round-trip to learn the issue's project/labels — happens inside the spawn
    // below so it never delays Linear's webhook ack.
    if rules.is_empty() {
        let team_id = issue
            .team_id
            .clone()
            .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
            .unwrap_or_default();
        warn!(
            team_id = %team_id,
            "Linear session created but no active rule found for team — emitting heartbeat only"
        );
        return Ok(());
    }

    let state_clone = state.clone();
    let connection_clone = connection.clone();
    let rules_clone = rules.to_vec();
    let session_clone = session.clone();
    let api_clone = api.clone();
    tokio::spawn(async move {
        // Project/label-aware rule selection. A mention/assignment on a
        // Vuln-Mgmt issue routes to the project-scoped `vulnerability-fix` rule;
        // anything else falls back to the unscoped `soap-specs` catch-all. The
        // `get_issue` round-trip this may make is kept off Linear's ack path by
        // running here, inside the spawn.
        let (issue_id, issue_identifier) = session_clone
            .issue
            .as_ref()
            .map(|i| (i.id.as_str(), i.identifier.as_str()))
            .unwrap_or_default();
        let Some(rule) =
            select_session_spawn_rule(&api_clone, &rules_clone, issue_id, issue_identifier).await
        else {
            warn!(
                session_id = %session_clone.id,
                "Linear session created but no spawn rule selected — skipping task creation"
            );
            return;
        };

        if let Err(e) = provision_session_task(
            &state_clone,
            &api_clone,
            &connection_clone,
            &rule,
            &session_clone,
        )
        .await
        {
            warn!(
                session_id = %session_clone.id,
                error = %e,
                "Failed to provision Linear session task"
            );
        }
    });

    Ok(())
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
pub async fn handle_session_prompted(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    session_id: &str,
    body: &str,
) -> Result<(), AdapterError> {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
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

    let rule = rule_for_binding(state, connection, &bindings[0]).await?;

    for binding in bindings {
        // Re-open the Linear turn gate for this follow-up prompt so the
        // next agent run's events flow through to Linear's UI.
        state.linear_active_turns.open(&binding.task_id);
        let external_event_id = format!(
            "linear:prompt:{session_id}:{}",
            Utc::now().timestamp_millis()
        );
        if let Err(e) = crate::message::send::send_message_flow(
            state,
            &binding.task_id,
            body,
            None,
            None,
            &[],
            &[],
            true,
            Some(&external_event_id),
            Some(ADAPTER_LABEL),
            Some(&binding.subject_key),
            rule.as_ref().map(|r| r.source_matcher.as_str()),
            None,
            false,
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

/// Route a `Comment.create` payload to tasks listening to the comment's issue.
/// This includes both `linear_agent_session` bindings created when Linear
/// started an agent session and `linear_issue` bindings created by direct issue
/// sends/status-entry tasks. No binding → no-op.
///
/// The agent's own outbound replies are filtered downstream: every reply
/// posted via `LinearAdapter::send_reply_for_binding` is recorded in
/// `AppState::recently_sent` and persisted to `task_messages.external_event_id`
/// under the key `linear:comment:<uuid>`. The inbound forward below builds the
/// same key, so `send_message_flow` recognises and drops the echo by comment
/// id rather than by author.
pub async fn handle_comment_created(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    issue_id: &str,
    comment: &CommentPayloadData,
) -> Result<(), AdapterError> {
    if comment.body.trim().is_empty() {
        debug!(comment_id = %comment.id, "Empty Comment.create body — ignoring");
        return Ok(());
    }

    // Echo filter: our own outbound comments end with a `Moatless task:` footer
    // link to the authoring task (see `LinearAdapter::send_reply_for_binding`).
    // Recover that task id so the per-binding loop below can drop the comment
    // for the task that authored it while still delivering it to any other tasks
    // bound to the same issue. The footer is visible content — nothing to strip,
    // so the body is forwarded unchanged.
    let source_task_id = crate::adapters::footer::parse_task_id_from_body(
        &state.config.auth.frontend_url,
        &comment.body,
    );
    let forwarded_body = comment.body.as_str();

    let parent_task_id = resolve_parent_comment_task(state, comment).await?;
    let bindings =
        comment_delivery_bindings(state, connection, issue_id, parent_task_id.as_deref()).await?;

    if bindings.is_empty() {
        info!(
            issue_id,
            comment_id = %comment.id,
            parent_id = comment.parent_id.as_deref().unwrap_or(""),
            parent_task_id = parent_task_id.as_deref().unwrap_or(""),
            "Comment.create with no active Linear listener binding for this issue — ignoring"
        );
        return Ok(());
    }

    let rule = rule_for_binding(state, connection, &bindings[0]).await?;

    for binding in bindings {
        // Skip the task that authored this comment — its own webhook echo must
        // not be fed back to it (other tasks on the same issue still get it).
        if source_task_id.as_deref() == Some(binding.task_id.as_str()) {
            debug!(
                task_id = %binding.task_id,
                comment_id = %comment.id,
                "Skipping Linear comment for originating task (self-echo)"
            );
            continue;
        }

        // Best-effort heartbeat into the bound agent session so Linear's
        // panel reflects that we're processing the comment. Ephemeral so
        // the next real activity replaces it in the UI.
        if let Some(session_id) = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
        {
            if let Err(e) = api
                .agent_activity_create(
                    session_id,
                    ActivityKind::Thought,
                    "Reading new comment…",
                    true,
                )
                .await
            {
                warn!(
                    session_id,
                    error = %e,
                    "Failed to emit Linear comment heartbeat"
                );
            }
        }

        let external_event_id = format!("linear:comment:{}", comment.id);
        if let Err(e) = crate::message::send::send_message_flow(
            state,
            &binding.task_id,
            forwarded_body,
            None,
            None,
            &[],
            &[],
            true,
            Some(&external_event_id),
            Some(ADAPTER_LABEL),
            Some(&binding.subject_key),
            rule.as_ref().map(|r| r.source_matcher.as_str()),
            None,
            false,
            comment.user.as_ref().map(|u| u.id.as_str()),
        )
        .await
        {
            warn!(
                task_id = %binding.task_id,
                comment_id = %comment.id,
                error = %e,
                "Failed to forward Linear comment to task"
            );
            continue;
        }

        if let Err(e) =
            crate::sandbox::notify::ensure_sandbox_and_notify(state, &binding.task_id).await
        {
            warn!(
                task_id = %binding.task_id,
                error = %e,
                "Failed to ensure sandbox for Linear comment"
            );
        }
    }

    Ok(())
}

async fn resolve_parent_comment_task(
    state: &AppState,
    comment: &CommentPayloadData,
) -> Result<Option<String>, AdapterError> {
    let Some(parent_id) = comment.parent_id.as_deref() else {
        return Ok(None);
    };
    let parent_external_id = linear_comment_external_id(parent_id);
    state
        .messages
        .find_task_id_by_any_external_event_id(&[parent_external_id])
        .await
        .map_err(AdapterError::Db)
}

async fn comment_delivery_bindings(
    state: &AppState,
    connection: &AdapterConnection,
    issue_id: &str,
    parent_task_id: Option<&str>,
) -> Result<Vec<TaskBinding>, AdapterError> {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
    let mut rows = Vec::new();
    for subject_kind in [SUBJECT_KIND, super::SUBJECT_KIND_ISSUE] {
        rows.extend(
            binding_repo
                .find_active_by_destination_field(ADAPTER, subject_kind, "issue_id", issue_id)
                .await?,
        );
    }

    // A reply inside a Linear comment thread belongs to the task that authored
    // the parent bot comment. For ordinary issue-level comments without
    // parentId, fan out to every active listener for the issue.
    if let Some(task_id) = parent_task_id {
        rows.retain(|b| b.task_id == task_id);
    }

    let mut seen_tasks = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        // Skip bindings whose task has closed — a closed task is terminal and
        // must not be resurrected by a new comment.
        .filter(|b| b.connection_id == connection.id && b.active)
        // A task can have both a session binding and a direct issue binding for
        // the same issue. Deliver once per task; session bindings are queried
        // first so they win and can emit the Linear heartbeat.
        .filter(|b| seen_tasks.insert(b.task_id.clone()))
        .collect())
}

fn linear_comment_external_id(comment_id: &str) -> String {
    format!("linear:comment:{comment_id}")
}

/// Handle an `Issue.update` payload.
///
/// On any real state transition we forward a notification message to every
/// bound task so the agent learns the issue changed status. When the new
/// state is in the rule's `terminal_states` and `close_task_on_close` is set,
/// the existing close path also runs and terminates the task.
pub async fn handle_issue_update(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[SubscriptionMatch],
    envelope_updated_from: Option<&serde_json::Value>,
    issue: &IssuePayload,
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
    notify_bindings_of_state_change(state, connection, issue, new_state_id.unwrap_or("")).await;

    if rule_config.close_task_on_close
        && rule_config.terminal_states.iter().any(|s| s == state_kind)
    {
        close_active_bindings_for_issue(state, api, connection, rule, issue).await?;
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
                rule_id = %rule.id,
                "Linear status-entry skipped: issue project not in allowlist"
            );
            continue;
        }
        if !cfg.labels_allowed(&labels) {
            debug!(
                issue = %issue.identifier,
                rule_id = %rule.id,
                "Linear status-entry skipped: required labels missing"
            );
            continue;
        }
        if let Err(e) = provision_issue_task(state, api, connection, rule, &cfg, issue).await {
            warn!(
                issue = %issue.identifier,
                rule_id = %rule.id,
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
    let existing = PgTaskBindingRepository(state.db.clone())
        .find_active_by_destination_field(ADAPTER, super::SUBJECT_KIND_ISSUE, "issue_id", &issue.id)
        .await?;
    if existing.iter().any(|b| {
        b.connection_id == connection.id && b.trigger_id.as_deref() == Some(rule.id.as_str())
    }) {
        info!(
            issue = %issue.identifier,
            rule_id = %rule.id,
            "Linear issue already has an active task for this trigger — skipping status-entry creation"
        );
        return Ok(());
    }

    let task_id = match create_issue_event_task(state, rule, issue).await {
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
    let destination_config = serde_json::json!({
        "issue_id": issue.id,
        "issue_identifier": issue.identifier,
        "identifier": issue.identifier,
        "team_id": team_id,
        "project_id": issue.resolved_project_id(),
        "issue_url": issue.url,
    });
    let now = Utc::now();
    let binding = TaskBinding {
        id: format!("ts_{}", uuid::Uuid::new_v4()),
        task_id: task_id.clone(),
        provider: ADAPTER.to_string(),
        subject_kind: super::SUBJECT_KIND_ISSUE.to_string(),
        subject_key: issue.id.clone(),
        connection_id: connection.id.clone(),
        trigger_id: Some(rule.id.clone()),
        destination_config,
        metadata: serde_json::json!({}),
        last_external_event_id: None,
        origin: ADAPTER_EVENT_BINDING_ORIGIN.to_string(),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    };
    PgTaskBindingRepository(state.db.clone())
        .upsert(&binding)
        .await?;

    // Link the Moatless task back onto the Linear issue.
    let task_url = task_url(state, &task_id);
    let attachment_title = task_attachment_title(state, &task_id, ATTACHMENT_TITLE_CHILD).await;
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
    let inbound_text = build_issue_inbound_text(issue);
    let external_event_id = format!("linear:issue:created:{}", issue.id);
    if let Err(e) = crate::message::send::send_message_flow(
        state,
        &task_id,
        &inbound_text,
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
        rule_id = %rule.id,
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
async fn transition_issue_state(
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

/// Close active bindings when an `AppUserNotification.issueStatusChanged`
/// arrives for a terminal state. Inbox-stream analog of `handle_issue_update`'s
/// close path — used when the workspace only subscribes to AppUserNotification
/// (not Issue) webhooks. Acts on the current state alone (the notification
/// carries no diff). `close_active_bindings_for_issue` is idempotent, so
/// double delivery from the Issue path is safe.
pub async fn handle_issue_status_notification(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    notif: &AppUserNotificationData,
) -> Result<(), AdapterError> {
    let Some(issue) = notif.resolved_issue() else {
        debug!(
            notif_id = notif.id.as_deref().unwrap_or(""),
            "AppUserNotification.issueStatusChanged missing issue payload — ignoring"
        );
        return Ok(());
    };

    let Some(team_id) = issue.resolved_team_id().map(str::to_string) else {
        debug!(
            issue = %issue.identifier,
            "AppUserNotification missing team_id — ignoring"
        );
        return Ok(());
    };

    let rules = resolve_rules_for_team(state, connection, &team_id).await?;
    let Some(rule) = rules.first() else {
        debug!(
            team_id = %team_id,
            "No active Linear rule for team — ignoring AppUserNotification"
        );
        return Ok(());
    };

    let rule_config = LinearRuleConfig::from_rule(rule);
    let state_kind = issue.state_kind();
    if rule_config.close_task_on_close
        && rule_config.terminal_states.iter().any(|s| s == state_kind)
    {
        close_active_bindings_for_issue(state, api, connection, rule, issue).await?;
    }

    Ok(())
}

/// Persist the issue's workflow state onto every binding tied to it (both
/// `linear_agent_session` and `linear_issue` subject kinds). Writes the
/// dedicated `external_status*` columns. Best-effort: failures are logged.
async fn persist_issue_status(
    state: &AppState,
    connection: &AdapterConnection,
    issue: &IssuePayload,
) {
    let repo = PgTaskBindingRepository(state.db.clone());
    let status = crate::bindings::status_service::linear_external_status(
        issue.state_name(),
        issue.state_kind(),
        Some(issue.title.as_str()),
        issue.url.as_deref(),
    );

    for subject_kind in [SUBJECT_KIND, super::SUBJECT_KIND_ISSUE] {
        let bindings = match repo
            .find_active_by_destination_field(ADAPTER, subject_kind, "issue_id", &issue.id)
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
        for binding in bindings
            .into_iter()
            .filter(|b| b.connection_id == connection.id)
        {
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
    new_state_id: &str,
) {
    let bindings = match PgTaskBindingRepository(state.db.clone())
        .find_active_by_destination_field(ADAPTER, SUBJECT_KIND, "issue_id", &issue.id)
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

    for binding in bindings
        .into_iter()
        .filter(|b| b.connection_id == connection.id)
    {
        // Dedup on (issue, new state id) so Linear re-deliveries collapse.
        let external_event_id = format!("linear:issue-state:{}:{new_state_id}", issue.id);
        if let Err(e) = crate::message::send::send_message_flow(
            state,
            &binding.task_id,
            &body,
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

async fn close_active_bindings_for_issue(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    issue: &IssuePayload,
) -> Result<(), AdapterError> {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
    // Look up every active binding tied to this issue — both `@mention`/
    // assignment sessions (`linear_agent_session`) and status-entry tasks
    // (`linear_issue`), so either kind auto-closes when the issue goes terminal.
    let subject_kinds = vec![
        SUBJECT_KIND.to_string(),
        super::SUBJECT_KIND_ISSUE.to_string(),
    ];
    let rows = sqlx::query_as::<_, TaskBinding>(
        "SELECT id, task_id, adapter_kind AS provider, subject_kind, subject_key, connection_id, \
                trigger_id, destination_config, metadata, last_external_event_id, \
                origin, created_at, last_event_at \
         FROM task_bindings \
         WHERE adapter_kind = $1 \
           AND subject_kind = ANY($2) \
           AND destination_config->>'issue_id' = $3 \
           AND COALESCE(destination_config->>'pending', 'false') <> 'true'",
    )
    .bind(ADAPTER)
    .bind(&subject_kinds)
    .bind(&issue.id)
    .fetch_all(&state.db)
    .await
    .map_err(AdapterError::Db)?;

    for binding in rows {
        // Sibling guard: only close if this is the sole binding for the task.
        let siblings_count = match binding_repo.find_by_task_id(&binding.task_id).await {
            Ok(rows) => rows.len(),
            Err(e) => {
                warn!(
                    task_id = %binding.task_id,
                    issue = %issue.identifier,
                    error = %e,
                    "Failed to look up sibling bindings — skipping auto-close"
                );
                continue;
            }
        };
        if siblings_count > 1 {
            info!(
                task_id = %binding.task_id,
                issue = %issue.identifier,
                other_binding_count = siblings_count - 1,
                "Linear issue closed but task has other connected bindings — leaving task open"
            );
            continue;
        }

        // Only auto-close tasks created by THIS trigger.
        if binding.origin != ADAPTER_EVENT_BINDING_ORIGIN
            || binding.trigger_id.as_deref() != Some(rule.id.as_str())
        {
            debug!(
                task_id = %binding.task_id,
                issue = %issue.identifier,
                "Binding not eligible for auto-close (different origin or rule)"
            );
            continue;
        }

        match state.tasks.find_by_id_internal(&binding.task_id).await {
            Ok(Some(task)) if task.status == TaskStatus::Closed => {
                debug!(task_id = %binding.task_id, issue = %issue.identifier, "Task already closed, skipping");
                continue;
            }
            Ok(None) => {
                warn!(task_id = %binding.task_id, issue = %issue.identifier, "Task not found for close");
                continue;
            }
            Err(e) => {
                warn!(task_id = %binding.task_id, error = %e, "Failed to fetch task for close");
                continue;
            }
            _ => {}
        }

        let session_id = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        let mut text = format!(
            "Linear issue {} transitioned to {}; closing this task.",
            issue.identifier,
            issue.state_name()
        );
        if let Some(ref url) = issue.url {
            let _ = write!(text, "\n\n{url}");
        }
        // Append the `Moatless task:` footer so this comment is recognised and
        // dropped by the inbound echo filter (`handle_comment_created`) when
        // Linear webhooks it back, rather than being re-forwarded into the task.
        if let Some(footer) =
            crate::adapters::footer::task_comment_footer(state, &binding.task_id).await
        {
            text.push_str(&footer);
        }

        if !session_id.is_empty() {
            if let Err(e) = api
                .agent_activity_create(session_id, ActivityKind::Response, &text, false)
                .await
            {
                warn!(session_id, error = %e, "Failed to emit close response activity");
            }
        }

        if let Err(e) = state
            .tasks
            .update_task(
                &binding.task_id,
                &TaskPatch {
                    status: Some(TaskStatus::Closed),
                    ..Default::default()
                },
            )
            .await
        {
            warn!(task_id = %binding.task_id, error = %e, "Failed to close task");
            continue;
        }

        // Remove the sandbox (best-effort, Closed already committed). Lifecycle
        // issues the graceful agent stop and retains S3 archives per config.
        let retention = ArchiveRetention::on_close(state.config.sandbox.retain_storage_on_close);
        if let Err(e) = state.sandbox.remove(&binding.task_id, retention).await {
            warn!(task_id = %binding.task_id, error = %e, "Failed to remove sandbox on Linear close");
        }

        // A closed task is terminal — deactivate its bindings so later comments
        // on the issue don't resurrect it. Best-effort.
        if let Err(e) = binding_repo.deactivate_by_task(&binding.task_id).await {
            warn!(task_id = %binding.task_id, error = %e, "Failed to deactivate bindings on Linear close");
        }

        if let Some(ref events) = state.events {
            events
                .publish_task_event(
                    &binding.task_id,
                    moatless_events::TaskSubType::Closed,
                    Some("claude-code"),
                    serde_json::json!({
                        "reason": "linear_state_terminal",
                        "issue_identifier": issue.identifier,
                        "state_name": issue.state_name(),
                    }),
                )
                .await;
        }

        info!(
            task_id = %binding.task_id,
            issue = %issue.identifier,
            "Task closed due to Linear issue terminal state"
        );
    }

    let _ = connection;
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
        .ok_or_else(|| AdapterError::BadRequest("AgentSession missing issue".into()))?;

    // Idempotency guard: if a binding already exists for this session id, a
    // retry is re-delivering `created` (e.g. cross-pod, where the per-pod
    // in-memory dedup cache doesn't apply). Return early rather than creating
    // a second task. Keyed on session id, so multiple *distinct* sessions on
    // the same issue still each get their own task.
    match PgTaskBindingRepository(state.db.clone())
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
    let related = match PgTaskBindingRepository(state.db.clone())
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
    let task_id = match create_provider_event_task(state, rule, session).await {
        Ok(id) => id,
        // Preflight failed: the task exists but is errored, and no binding/turn
        // gate exists to surface the service's error event in Linear. Post the
        // reason onto the agent session directly so the user isn't left staring
        // at silence, then bail out — no binding, no prompt for a dead task.
        Err(AdapterError::AccessDenied(msg)) => {
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
            return Err(AdapterError::AccessDenied(msg));
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
    let destination_config = serde_json::json!({
        "session_id": session.id,
        "issue_id": issue.id,
        "identifier": issue.identifier,
        "team_id": team_id,
        "issue_url": issue.url,
    });
    let now = Utc::now();
    let binding = TaskBinding {
        id: format!("ts_{}", uuid::Uuid::new_v4()),
        task_id: task_id.clone(),
        provider: ADAPTER.to_string(),
        subject_kind: SUBJECT_KIND.to_string(),
        subject_key: session.id.clone(),
        connection_id: connection.id.clone(),
        trigger_id: Some(rule.id.clone()),
        destination_config,
        metadata: serde_json::json!({}),
        last_external_event_id: None,
        origin: ADAPTER_EVENT_BINDING_ORIGIN.to_string(),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    };

    PgTaskBindingRepository(state.db.clone())
        .upsert(&binding)
        .await?;

    // Update the session with an externalUrl pointing at the moatless task.
    let task_url = task_url(state, &task_id);
    let attachment_title = task_attachment_title(state, &task_id, ATTACHMENT_TITLE_CHILD).await;
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
    let inbound_text = build_inbound_text(session);
    let external_event_id = format!("linear:session:created:{}", session.id);
    if let Err(e) = crate::message::send::send_message_flow(
        state,
        &task_id,
        &inbound_text,
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
        session.creator.as_ref().map(|c| c.id.as_str()),
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

fn build_inbound_text(session: &AgentSession) -> String {
    // Intentionally minimal: the issue description, comments, and Linear's
    // pre-built promptContext are NOT inlined — they go stale as the ticket
    // evolves. The agent reads the live ticket via the `moat` CLI instead.
    match session.issue.as_ref() {
        Some(issue) => issue_read_block(&issue.identifier, &issue.title, issue.url.as_deref()),
        None => String::new(),
    }
}

/// Inbound prompt for a task spawned from an `Issue.update` status entry.
/// Mirrors [`build_inbound_text`] — the agent reads the live ticket rather than
/// receiving an inlined (and quickly-stale) snapshot.
fn build_issue_inbound_text(issue: &IssuePayload) -> String {
    issue_read_block(&issue.identifier, &issue.title, issue.url.as_deref())
}

/// Shared issue-link + "read the live ticket via `moat`" instruction block used
/// by both the session- and issue-initiated creation paths. Keeping it in one
/// place prevents the two prompts from drifting apart.
fn issue_read_block(identifier: &str, title: &str, url: Option<&str>) -> String {
    let mut text = String::new();
    let _ = write!(text, "## Linear issue {identifier} — {title}");
    if let Some(url) = url.filter(|s| !s.is_empty()) {
        let _ = write!(text, "\n\n{url}");
    }
    let id = identifier;
    let _ = write!(
        text,
        "\n\nBefore working on this task, read the Linear issue, all of its \
         comments, and its linked PRs and agent tasks using the `moat` CLI:\n\n\
         - Issue details and description: `moat linear issue get {id} --json`\n\
         - Comments, linked PRs, and related agent tasks: \
         `moat linear graphql 'query {{ issue(id: \"{id}\") {{ \
         attachments {{ nodes {{ title url sourceType }} }} \
         comments {{ nodes {{ body createdAt user {{ displayName }} }} }} }} }}'`"
    );
    text
}

async fn create_provider_event_task(
    state: &AppState,
    rule: &SubscriptionMatch,
    session: &AgentSession,
) -> Result<String, AdapterError> {
    let issue = session
        .issue
        .as_ref()
        .ok_or_else(|| AdapterError::BadRequest("AgentSession missing issue".into()))?;
    let preview = build_inbound_text(session);
    let task_id = insert_event_task(state, rule, &issue.identifier, &issue.title, &preview).await?;
    info!(task_id = %task_id, rule_id = %rule.id, "Created task from Linear AgentSession");
    Ok(task_id)
}

/// Create a task from an `Issue.update` status-entry match.
async fn create_issue_event_task(
    state: &AppState,
    rule: &SubscriptionMatch,
    issue: &IssuePayload,
) -> Result<String, AdapterError> {
    let preview = build_issue_inbound_text(issue);
    insert_event_task(state, rule, &issue.identifier, &issue.title, &preview).await
}

/// Build + insert the Task row shared by the session- and issue-initiated
/// Linear creation paths, assign the trigger's tags, and publish the `Open`
/// event. Returns the new task id. Callers own subsequent provisioning,
/// binding, and inbound-message delivery.
async fn insert_event_task(
    state: &AppState,
    rule: &SubscriptionMatch,
    identifier: &str,
    title: &str,
    preview: &str,
) -> Result<String, AdapterError> {
    let append_system_prompt = {
        let mut parts = Vec::new();
        if let Some(ref tpl) = rule.prompt_template {
            parts.push(tpl.to_string());
        }
        if let Some(providers) = state.adapters.as_ref() {
            if let Some(provider) = providers.get_by_name(ADAPTER_LABEL) {
                if let Some(extra) = provider.reply_instructions() {
                    parts.push(extra);
                }
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    };

    let request = NewTaskRequest {
        repository_id: rule.repository_id.clone(),
        agent_type: AgentType(rule.agent_type.clone()),
        name: rule
            .task_name
            .clone()
            .or_else(|| Some(format!("{identifier} — {title}"))),
        description: Some(crate::bindings::outbound::truncate_text(preview, 200)),
        branch: None,
        created_by: Some(rule.effective_user_id().to_string()),
        headless: true,
        // Tasks auto-created from a Linear webhook default to public so the
        // people collaborating on the Linear issue can open the task.
        visibility: Visibility::Public,
        scope: rule.scope,
        team_id: rule.team_id.clone(),
        owner_user_id: (rule.scope == moatless_backend_core::scope::Scope::User)
            .then(|| rule.effective_user_id().to_string()),
        trigger_id: Some(rule.id.clone()),
        workspace_id: None,
        template_id: None,
        vibe: None,
        append_system_prompt,
        ttl_minutes: crate::task::models::DEFAULT_TTL_MINUTES,
        inactive_ttl_days: rule
            .inactive_ttl_days
            .unwrap_or(crate::task::models::DEFAULT_INACTIVE_TTL_DAYS),
        tag_ids: rule.tag_ids.clone(),
        parent: None,
        related_to: None,
        // Adapters deliver the inbound prompt themselves through the binding
        // (`send_message_flow`), so the service sends no first message.
        first_message: None,
        event_subtype: moatless_events::TaskSubType::Open,
        repository: None,
    };

    // Preflight, insert, tags, provisioning, and the `Open` event all run
    // inside the creation service. On preflight failure it records the task as
    // errored (with a `SandboxStatus::Error` event) and returns an error, which
    // propagates here so the caller skips the binding/message steps for the
    // dead task. Preflight is surfaced as `AccessDenied` (a repo-token access
    // failure) so callers can post the reason back to Linear — the service's
    // error event alone stays invisible there because no turn gate is open for
    // a task that never got a binding.
    let source_matcher = rule.source_matcher.clone();
    let outcome = state
        .task_creation
        .create(
            request,
            move |_task| async move {
                serde_json::json!({
                    "adapter": ADAPTER_LABEL,
                    "source_matcher": source_matcher,
                })
            },
            |_task_id, _first_message| async move {},
        )
        .await
        .map_err(|e| match e {
            crate::task::creation::TaskCreationError::Preflight(msg) => {
                AdapterError::AccessDenied(msg)
            }
            other => AdapterError::Internal(format!("Failed to create task: {other}")),
        })?;

    Ok(outcome.task.task_id)
}

pub(crate) fn task_url(state: &AppState, task_id: &str) -> String {
    let base = state.config.auth.frontend_url.trim_end_matches('/');
    format!("{base}/tasks/{task_id}")
}

/// Label prefix for the Linear ↔ Moatless link when the Linear side is the
/// parent (Moatless task spawned from a `@mention` / assignment).
pub(crate) const ATTACHMENT_TITLE_CHILD: &str = "Moatless task";

/// Label prefix for the Linear ↔ Moatless link when the Moatless side is the
/// parent (Linear issue created by the CLI or adapter_send on behalf of a task).
pub(crate) const ATTACHMENT_TITLE_PARENT: &str = "Moatless parent task";

fn format_attachment_title(prefix: &str, name: Option<&str>) -> String {
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => format!("{prefix}: {n}"),
        None => prefix.to_string(),
    }
}

pub(crate) async fn task_attachment_title(state: &AppState, task_id: &str, prefix: &str) -> String {
    match state.tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) => format_attachment_title(prefix, task.name.as_deref()),
        Ok(None) => prefix.to_string(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                task_id,
                "Failed to load task name for Linear attachment title"
            );
            prefix.to_string()
        }
    }
}

async fn rule_for_binding(
    state: &AppState,
    _connection: &AdapterConnection,
    binding: &TaskBinding,
) -> Result<Option<SubscriptionMatch>, AdapterError> {
    if let Some(ref trigger_id) = binding.trigger_id {
        Ok(state.loops
            .find_subscription_by_id(trigger_id)
            .await?)
    } else {
        Ok(None)
    }
}

/// Resolve every active Linear `(api, session_id)` pair bound to `task_id`.
/// Internal helper used by every fire-and-forget activity emitter — keeps the
/// task-binding + workspace-OAuth lookup in one place.
///
/// Returns an empty vec on any DB error (logged) so callers can skip emission
/// silently without bubbling errors into the event-handler hot path.
async fn resolve_linear_sessions_for_task(
    state: &AppState,
    task_id: &str,
) -> Vec<(LinearAgentApi, String)> {
    let rows = match sqlx::query_as::<_, TaskBinding>(
        "SELECT id, task_id, adapter_kind AS provider, subject_kind, subject_key, connection_id, \
                trigger_id, destination_config, metadata, last_external_event_id, \
                origin, created_at, last_event_at \
         FROM task_bindings \
         WHERE adapter_kind = $1 \
           AND subject_kind = $2 \
           AND task_id = $3 \
           AND COALESCE(destination_config->>'pending', 'false') <> 'true'",
    )
    .bind(ADAPTER)
    .bind(SUBJECT_KIND)
    .bind(task_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id, error = %e, "linear sessions: binding lookup failed");
            return Vec::new();
        }
    };

    let mut sessions = Vec::with_capacity(rows.len());
    for binding in rows {
        let Some(session_id) = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        // Column is `external_account_id` on disk; the rest of the codebase
        // reads it via `external_account_id AS scope_value`. Use the DAO so
        // we stay in sync with future schema changes.
        let connection = match PgAdapterConnectionRepository(state.db.clone())
            .find_by_id(&binding.connection_id)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    task_id,
                    connection_id = %binding.connection_id,
                    error = %e,
                    "linear sessions: connection lookup failed"
                );
                continue;
            }
        };
        let Some(scope_value) = connection.map(|c| c.scope_value) else {
            continue;
        };

        let api = LinearAgentApi::new(
            default_http_client(),
            state.db.clone(),
            scope_value,
            state.config.linear.oauth_client_id.clone(),
            state.config.linear.oauth_client_secret.clone(),
        );
        sessions.push((api, session_id));
    }
    sessions
}

/// Best-effort emit a Linear activity on every active Linear session bound to
/// `task_id`. Fire-and-forget: any failure (no binding, missing connection,
/// network error, GraphQL rejection) is logged and swallowed — callers in the
/// event-consumer hot path must not be affected.
pub async fn notify_linear_status(
    state: AppState,
    task_id: String,
    kind: ActivityKind,
    body: String,
    ephemeral: bool,
) {
    for (api, session_id) in resolve_linear_sessions_for_task(&state, &task_id).await {
        if let Err(e) = api
            .agent_activity_create(&session_id, kind, &body, ephemeral)
            .await
        {
            warn!(
                task_id,
                session_id,
                error = %e,
                "notify_linear_status: agent_activity_create failed"
            );
        }
    }
}

/// Sibling of [`notify_linear_status`] that emits an `action` activity (with
/// optional `result`) on every active Linear session bound to `task_id`.
/// Same best-effort semantics: failures are logged and swallowed so callers
/// in the event-consumer hot path are unaffected.
pub async fn notify_linear_action(
    state: AppState,
    task_id: String,
    action: String,
    parameter: String,
    result: Option<String>,
    ephemeral: bool,
) {
    for (api, session_id) in resolve_linear_sessions_for_task(&state, &task_id).await {
        if let Err(e) = api
            .agent_activity_create_action(
                &session_id,
                &action,
                &parameter,
                result.as_deref(),
                ephemeral,
            )
            .await
        {
            warn!(
                task_id,
                session_id,
                error = %e,
                "notify_linear_action: agent_activity_create_action failed"
            );
        }
    }
}

/// Best-effort: walk an assistant message's `content` block array and emit one
/// Linear activity per block — `text` blocks become ephemeral `thought`s, and
/// `tool_use` blocks become ephemeral `action`s with a one-line parameter
/// summary derived from `tool_input`.
///
/// Caller is responsible for guarding on `payload.type == "assistant"`; this
/// function will silently no-op on non-assistant content shapes.
///
/// Skips:
/// - `tool_result` blocks (system/user role messages).
/// - `text` blocks shorter than 4 chars after trim (Claude connector tokens).
pub async fn notify_linear_message_activities(
    state: AppState,
    task_id: String,
    message: serde_json::Value,
) {
    let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) else {
        return;
    };
    if blocks.is_empty() {
        return;
    }

    let sessions = resolve_linear_sessions_for_task(&state, &task_id).await;
    if sessions.is_empty() {
        return;
    }

    for block in blocks {
        let block_type = block
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        match block_type {
            "text" => {
                let text = block
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim();
                if text.chars().count() < 4 {
                    continue;
                }
                for (api, session_id) in &sessions {
                    if let Err(e) = api
                        .agent_activity_create(session_id, ActivityKind::Thought, text, false)
                        .await
                    {
                        warn!(
                            task_id,
                            session_id,
                            error = %e,
                            "notify_linear_message_activities: thought failed"
                        );
                    }
                }
            }
            "tool_use" => {
                let Some(tool_name) = block.get("name").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let empty = serde_json::Value::Null;
                let input = block.get("input").unwrap_or(&empty);
                let parameter = summarize_tool_input(tool_name, input);
                for (api, session_id) in &sessions {
                    if let Err(e) = api
                        .agent_activity_create_action(session_id, tool_name, &parameter, None, true)
                        .await
                    {
                        warn!(
                            task_id,
                            session_id,
                            tool_name,
                            error = %e,
                            "notify_linear_message_activities: action failed"
                        );
                    }
                }
            }
            _ => {
                // tool_result blocks are handled by notify_linear_tool_results
                // on the user-role agent.message sibling; any other block kinds
                // are intentionally ignored.
            }
        }
    }
}

/// Walk a user-role agent message and, for every `tool_result` block, emit a
/// non-ephemeral `action` activity carrying the tool's result. Pairs with the
/// ephemeral pending `action` emitted at `tool_use` time so a Linear viewer
/// sees both the call and its outcome.
///
/// Fire-and-forget: any DB or GraphQL failure is logged and swallowed.
pub async fn notify_linear_tool_results(
    state: AppState,
    task_id: String,
    message: serde_json::Value,
) {
    let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) else {
        return;
    };
    if blocks.is_empty() {
        return;
    }

    // Collect tool_use_ids up-front so we can skip the sessions lookup when the
    // user message carries no tool_result blocks (e.g. plain text follow-ups).
    let tool_use_ids: Vec<String> = blocks
        .iter()
        .filter_map(|b| {
            let kind = b.get("type").and_then(serde_json::Value::as_str)?;
            if kind != "tool_result" {
                return None;
            }
            b.get("tool_use_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect();
    if tool_use_ids.is_empty() {
        return;
    }

    let sessions = resolve_linear_sessions_for_task(&state, &task_id).await;
    if sessions.is_empty() {
        return;
    }

    for tool_use_id in tool_use_ids {
        let row = match state.messages.find_tool_call(&task_id, &tool_use_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                debug!(
                    task_id,
                    tool_use_id, "notify_linear_tool_results: tool_use lookup returned no row"
                );
                continue;
            }
            Err(e) => {
                debug!(
                    task_id,
                    tool_use_id,
                    error = %e,
                    "notify_linear_tool_results: tool_use lookup failed"
                );
                continue;
            }
        };

        // Race guard: pending ephemeral stays in place until the result row
        // exists. Without truncated result content there is nothing to surface.
        let Some(result_text) = row.result_content.as_deref() else {
            continue;
        };

        let parameter = summarize_tool_input(&row.tool_name, &row.tool_input);
        let result = truncate_result_for_linear(result_text, row.is_error.unwrap_or(false));

        for (api, session_id) in &sessions {
            if let Err(e) = api
                .agent_activity_create_action(
                    session_id,
                    &row.tool_name,
                    &parameter,
                    Some(&result),
                    false,
                )
                .await
            {
                warn!(
                    task_id,
                    session_id,
                    tool_name = %row.tool_name,
                    error = %e,
                    "notify_linear_tool_results: action failed"
                );
            }
        }
    }
}

/// Maximum length of the `parameter` field in a Linear `action` activity.
/// Linear's UI renders activities in a narrow row, so longer strings get
/// truncated visually anyway.
const PARAMETER_MAX_LEN: usize = 200;

/// Maximum length of the `result` field in a Linear `action` activity.
/// Tool results (file contents, command output) are often multi-KB; cap to
/// keep Linear's UI legible.
const RESULT_MAX_LEN: usize = 500;

/// One-line summary of a Claude tool-use input suitable for Linear's
/// `action.parameter` slot. Falls through to a JSON dump for unknown tools.
fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    fn s(v: &serde_json::Value, key: &str) -> Option<String> {
        v.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }

    let raw = match tool_name {
        "Bash" => s(input, "command").unwrap_or_default(),
        "Edit" | "Write" | "Read" | "NotebookEdit" => s(input, "file_path").unwrap_or_default(),
        "Grep" => {
            let pattern = s(input, "pattern").unwrap_or_default();
            match s(input, "path") {
                Some(p) if !p.is_empty() => format!("{pattern} in {p}"),
                _ => pattern,
            }
        }
        "Glob" => s(input, "pattern").unwrap_or_default(),
        "WebFetch" | "WebSearch" => s(input, "url")
            .or_else(|| s(input, "query"))
            .unwrap_or_default(),
        "Task" | "Agent" => s(input, "description").unwrap_or_default(),
        name if name.starts_with("mcp__") => {
            let suffix = name.rsplit("__").next().unwrap_or(name);
            let detail = s(input, "query").or_else(|| s(input, "name"));
            match detail {
                Some(d) if !d.is_empty() => format!("{suffix}: {d}"),
                _ => suffix.to_string(),
            }
        }
        _ => serde_json::to_string(input).unwrap_or_default(),
    };

    truncate_chars(&raw, PARAMETER_MAX_LEN)
}

/// Truncate `s` to at most `max_chars` Unicode scalar values, appending an
/// ellipsis (`…`) when truncation occurs.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Build the `result` field for a completed Linear `action` activity from a
/// raw tool result string. Truncates to [`RESULT_MAX_LEN`] Unicode scalars
/// and prefixes `"❌ "` when the tool reported an error so the failure is
/// obvious at a glance in Linear's UI.
fn truncate_result_for_linear(text: &str, is_error: bool) -> String {
    let trimmed = text.trim();
    let body = truncate_chars(trimmed, RESULT_MAX_LEN);
    if is_error {
        format!("❌ {body}")
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::linear::webhook_payloads::{
        AgentSessionComment, AgentSessionIssue, LinearActor,
    };

    fn issue() -> AgentSessionIssue {
        AgentSessionIssue {
            id: "issue-1".into(),
            identifier: "SP-1".into(),
            title: "Bug".into(),
            description: Some("Long description here.".into()),
            url: Some("https://linear.app/acme/issue/SP-1".into()),
            team_id: Some("team-1".into()),
            team: None,
            state: None,
        }
    }

    fn session_with(
        comment: Option<AgentSessionComment>,
        prompt_context: Option<String>,
    ) -> AgentSession {
        AgentSession {
            id: "session-1".into(),
            issue: Some(issue()),
            comment,
            creator: None,
            prompt_context,
        }
    }

    fn user_comment(id: &str, body: &str, user_name: &str) -> AgentSessionComment {
        AgentSessionComment {
            id: id.into(),
            body: body.into(),
            user: Some(LinearActor {
                id: "user-1".into(),
                name: user_name.into(),
                email: None,
            }),
        }
    }

    #[test]
    fn build_inbound_text_links_issue_with_read_instructions() {
        let session = session_with(None, None);
        let text = build_inbound_text(&session);
        assert!(text.contains("## Linear issue SP-1 — Bug"));
        assert!(text.contains("https://linear.app/acme/issue/SP-1"));
        assert!(text.contains(
            "read the Linear issue, all of its comments, and its linked PRs and agent tasks"
        ));
        assert!(text.contains("moat linear issue get SP-1 --json"));
        assert!(text.contains(
            "moat linear graphql 'query { issue(id: \"SP-1\") { \
             attachments { nodes { title url sourceType } } \
             comments { nodes { body createdAt user { displayName } } } } }'"
        ));
        // The issue description is not inlined — the agent reads the live ticket.
        assert!(!text.contains("Long description here."));
    }

    #[test]
    fn build_issue_inbound_text_matches_session_variant() {
        use crate::adapters::linear::webhook_payloads::IssuePayload;
        let issue = IssuePayload {
            id: "issue-1".into(),
            identifier: "SP-1".into(),
            title: "Bug".into(),
            url: Some("https://linear.app/acme/issue/SP-1".into()),
            team_id: Some("team-1".into()),
            team: None,
            project_id: Some("proj-1".into()),
            project: None,
            state: None,
        };
        let from_issue = build_issue_inbound_text(&issue);
        // Identical to the session-initiated prompt for the same issue identity.
        let from_session = build_inbound_text(&session_with(None, None));
        assert_eq!(from_issue, from_session);
        assert!(from_issue.contains("## Linear issue SP-1 — Bug"));
        assert!(from_issue.contains("moat linear issue get SP-1 --json"));
    }

    #[test]
    fn build_inbound_text_ignores_prompt_context() {
        let session = session_with(None, Some("<context>stale snapshot</context>".into()));
        let text = build_inbound_text(&session);
        assert!(!text.contains("stale snapshot"));
        assert!(text.contains("moat linear issue get SP-1 --json"));
    }

    #[test]
    fn build_inbound_text_never_inlines_comment_bodies() {
        for comment in [
            AgentSessionComment {
                id: "sys-1".into(),
                body: "This thread is for an agent session with moatless.".into(),
                user: None,
            },
            user_comment("c-1", "please look at this", "Alice"),
        ] {
            let session = session_with(Some(comment.clone()), None);
            let text = build_inbound_text(&session);
            assert!(text.contains("## Linear issue SP-1 — Bug"));
            assert!(!text.contains(&comment.body));
            assert!(!text.contains("just mentioned the agent"));
        }
    }

    #[test]
    fn summarize_tool_input_bash_command() {
        let input = serde_json::json!({ "command": "cargo check" });
        assert_eq!(summarize_tool_input("Bash", &input), "cargo check");
    }

    #[test]
    fn summarize_tool_input_edit_path() {
        let input = serde_json::json!({
            "file_path": "/repo/src/main.rs",
            "old_string": "x",
            "new_string": "y"
        });
        assert_eq!(summarize_tool_input("Edit", &input), "/repo/src/main.rs");
        assert_eq!(summarize_tool_input("Read", &input), "/repo/src/main.rs");
        assert_eq!(summarize_tool_input("Write", &input), "/repo/src/main.rs");
    }

    #[test]
    fn summarize_tool_input_grep_with_path() {
        let input = serde_json::json!({ "pattern": "TODO", "path": "backend/src" });
        assert_eq!(summarize_tool_input("Grep", &input), "TODO in backend/src");
    }

    #[test]
    fn summarize_tool_input_grep_without_path() {
        let input = serde_json::json!({ "pattern": "TODO" });
        assert_eq!(summarize_tool_input("Grep", &input), "TODO");
    }

    #[test]
    fn summarize_tool_input_mcp_tool() {
        let input = serde_json::json!({ "query": "rust async" });
        assert_eq!(
            summarize_tool_input("mcp__context7__search", &input),
            "search: rust async"
        );
    }

    #[test]
    fn summarize_tool_input_unknown_falls_back_to_json() {
        let input = serde_json::json!({ "foo": "bar" });
        let out = summarize_tool_input("Mystery", &input);
        assert!(out.contains("\"foo\""));
        assert!(out.contains("\"bar\""));
    }

    #[test]
    fn summarize_tool_input_truncates_oversized() {
        let long = "x".repeat(500);
        let input = serde_json::json!({ "command": long });
        let out = summarize_tool_input("Bash", &input);
        assert_eq!(out.chars().count(), PARAMETER_MAX_LEN);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_result_for_linear_short_ok() {
        let out = truncate_result_for_linear("hello world", false);
        assert_eq!(out, "hello world");
    }

    #[test]
    fn truncate_result_for_linear_trims_whitespace() {
        let out = truncate_result_for_linear("  hello  \n", false);
        assert_eq!(out, "hello");
    }

    #[test]
    fn truncate_result_for_linear_prefixes_error() {
        let out = truncate_result_for_linear("file not found", true);
        assert_eq!(out, "❌ file not found");
    }

    #[test]
    fn truncate_result_for_linear_caps_length() {
        let long = "x".repeat(RESULT_MAX_LEN + 50);
        let out = truncate_result_for_linear(&long, false);
        assert_eq!(out.chars().count(), RESULT_MAX_LEN);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_result_for_linear_caps_then_prefixes_error() {
        let long = "x".repeat(RESULT_MAX_LEN + 50);
        let out = truncate_result_for_linear(&long, true);
        // "❌ " prefix added after truncation, so the visible body stays at the cap.
        assert!(out.starts_with("❌ "));
        assert!(out.ends_with('…'));
        // 2 chars ("❌ ") + RESULT_MAX_LEN body — but "❌" is a single char, plus space = 2 chars total prefix.
        assert_eq!(out.chars().count(), RESULT_MAX_LEN + 2);
    }

    #[test]
    fn build_inbound_text_handles_missing_issue() {
        let session = AgentSession {
            id: "session-1".into(),
            issue: None,
            comment: Some(user_comment("c-1", "hey", "Alice")),
            creator: None,
            prompt_context: None,
        };
        let text = build_inbound_text(&session);
        assert!(text.is_empty());
    }

    #[test]
    fn format_attachment_title_with_name() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, Some("Fix login bug")),
            "Moatless task: Fix login bug"
        );
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_PARENT, Some("Fix login bug")),
            "Moatless parent task: Fix login bug"
        );
    }

    #[test]
    fn format_attachment_title_falls_back_when_name_missing() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, None),
            "Moatless task"
        );
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_PARENT, None),
            "Moatless parent task"
        );
    }

    #[test]
    fn format_attachment_title_treats_whitespace_only_as_empty() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, Some("   ")),
            "Moatless task"
        );
    }

    #[test]
    fn format_attachment_title_trims_surrounding_whitespace() {
        assert_eq!(
            format_attachment_title(ATTACHMENT_TITLE_CHILD, Some("  Hi  ")),
            "Moatless task: Hi"
        );
    }
}
