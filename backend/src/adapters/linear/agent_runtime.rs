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
use moatless_api_schemas::linear::LinearIssueComment;
use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, TaskBinding, TaskTriggerRoute, ADAPTER_EVENT_BINDING_ORIGIN,
};
use crate::bindings::dao::PgTaskBindingRepository;
use crate::shared_models::AgentType;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::models::{Task, TaskStatus, Visibility};
use crate::triggers::rule_dao::PgTaskTriggerRouteRepository;

use super::agent_api::{default_http_client, ActivityKind, LinearAgentApi};
use super::config::LinearRuleConfig;
use super::webhook_payloads::{AgentSession, CommentPayloadData, IssuePayload};

const ADAPTER: &str = "linear";
const SUBJECT_KIND: &str = "linear_agent_session";
const ADAPTER_LABEL: &str = "linear";

/// Resolve every active task trigger bound to a given Linear team. Returns
/// every rule attached to the team — multi-rule support is preserved.
pub async fn resolve_rules_for_team(
    state: &AppState,
    connection: &AdapterConnection,
    team_id: &str,
) -> Result<Vec<TaskTriggerRoute>, AdapterError> {
    let rules = PgTaskTriggerRouteRepository(state.db.clone())
        .list_by_source_matcher(ADAPTER, team_id)
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
    rules: &[TaskTriggerRoute],
    session: &AgentSession,
) -> Result<(), AdapterError> {
    let issue = session.issue.as_ref().ok_or_else(|| {
        AdapterError::BadRequest("AgentSession missing issue payload".into())
    })?;

    // Resolve the rule by team. The first active rule wins for spawning;
    // multi-listener fan-out is limited to Issue.update close paths.
    let team_id = issue
        .team_id
        .clone()
        .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
        .unwrap_or_default();
    let rule = rules.first().cloned();

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

    let Some(rule) = rule else {
        warn!(
            team_id = %team_id,
            "Linear session created but no active rule found for team — emitting heartbeat only"
        );
        return Ok(());
    };

    let state_clone = state.clone();
    let connection_clone = connection.clone();
    let rule_clone = rule.clone();
    let session_clone = session.clone();
    let api_clone = api.clone();
    tokio::spawn(async move {
        if let Err(e) = provision_session_task(
            &state_clone,
            &api_clone,
            &connection_clone,
            &rule_clone,
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

/// Route an `AgentSessionEvent.prompted` payload to the bound task.
pub async fn handle_session_prompted(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    session_id: &str,
    body: &str,
) -> Result<(), AdapterError> {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
    let bindings = binding_repo
        .find_by_subject(ADAPTER, SUBJECT_KIND, session_id)
        .await?;

    if bindings.is_empty() {
        warn!(session_id, "No Linear binding for prompted session — ignoring");
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
        let external_event_id = format!("linear:prompt:{session_id}:{}", Utc::now().timestamp_millis());
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
            rule.as_ref().map(|r| r.source_matcher.as_str()),
            None,
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

/// Route a `Comment.create` payload to a task that already has an agent-
/// session binding for the comment's issue. No binding → no-op; we only
/// forward comments to issues with an active Moatless task. The agent posts
/// `agentActivity` (not `comment`) outbound, so there is no echo loop.
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

    let binding_repo = PgTaskBindingRepository(state.db.clone());
    let bindings: Vec<TaskBinding> = binding_repo
        .find_active_by_destination_field(ADAPTER, SUBJECT_KIND, "issue_id", issue_id)
        .await?
        .into_iter()
        .filter(|b| b.connection_id == connection.id)
        .collect();

    if bindings.is_empty() {
        debug!(
            issue_id,
            comment_id = %comment.id,
            "Comment.create with no active agent-session binding — ignoring"
        );
        return Ok(());
    }

    let rule = rule_for_binding(state, connection, &bindings[0]).await?;

    for binding in bindings {
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
            &comment.body,
            None,
            None,
            &[],
            &[],
            true,
            Some(&external_event_id),
            Some(ADAPTER_LABEL),
            rule.as_ref().map(|r| r.source_matcher.as_str()),
            None,
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
    rules: &[TaskTriggerRoute],
    envelope_updated_from: Option<&serde_json::Value>,
    issue: &IssuePayload,
) -> Result<(), AdapterError> {
    let state_name = issue.state_name();
    let prev_state_id = envelope_updated_from
        .and_then(|v| v.get("stateId"))
        .and_then(serde_json::Value::as_str);
    let new_state_id = issue.state.as_ref().map(|s| s.id.as_str());

    // Only act on real state transitions.
    if prev_state_id.is_none()
        || new_state_id.is_none()
        || prev_state_id == new_state_id
    {
        return Ok(());
    }

    let Some(rule) = rules.first() else {
        debug!(issue = %issue.identifier, "No Linear rule found for team — ignoring update");
        return Ok(());
    };
    let rule_config = LinearRuleConfig::from_rule(rule);

    // Forward a state-change notification to every bound task (best-effort).
    notify_bindings_of_state_change(
        state,
        connection,
        issue,
        new_state_id.unwrap_or(""),
    )
    .await;

    if rule_config.close_task_on_close
        && rule_config.terminal_states.iter().any(|s| s == state_name)
    {
        close_active_bindings_for_issue(state, api, connection, rule, issue).await?;
    }

    Ok(())
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
        let external_event_id =
            format!("linear:issue-state:{}:{new_state_id}", issue.id);
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
            None,
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
    rule: &TaskTriggerRoute,
    issue: &IssuePayload,
) -> Result<(), AdapterError> {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
    // Look up every active binding tied to this issue.
    let rows = sqlx::query_as::<_, TaskBinding>(
        "SELECT id, task_id, adapter_kind AS provider, subject_kind, subject_key, connection_id, \
                trigger_id, destination_config, metadata, last_external_event_id, \
                origin, created_at, last_event_at \
         FROM task_bindings \
         WHERE adapter_kind = $1 \
           AND subject_kind = $2 \
           AND destination_config->>'issue_id' = $3 \
           AND COALESCE(destination_config->>'pending', 'false') <> 'true'",
    )
    .bind(ADAPTER)
    .bind(SUBJECT_KIND)
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

        if !session_id.is_empty() {
            if let Err(e) = api
                .agent_activity_create(session_id, ActivityKind::Response, &text, false)
                .await
            {
                warn!(session_id, error = %e, "Failed to emit close response activity");
            }
        }

        if let Err(e) = state.sandbox.cleanup(&binding.task_id).await {
            warn!(task_id = %binding.task_id, error = %e, "Failed to cleanup sandbox on Linear close");
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
    rule: &TaskTriggerRoute,
    session: &AgentSession,
) -> Result<(), AdapterError> {
    let issue = session.issue.as_ref().ok_or_else(|| {
        AdapterError::BadRequest("AgentSession missing issue".into())
    })?;

    // Create the task first, then the binding.
    let task_id = create_provider_event_task(state, rule, session).await?;
    provision_task(state, &task_id).await;

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
    };

    PgTaskBindingRepository(state.db.clone())
        .upsert(&binding)
        .await?;

    // Update the session with an externalUrl pointing at the moatless task.
    let task_url = task_url(state, &task_id);
    if let Err(e) = api
        .agent_session_update_external_urls(&session.id, "Moatless task", &task_url)
        .await
    {
        warn!(session_id = %session.id, error = %e, "Failed to update Linear externalUrls");
    }
    if let Some(ref issue) = session.issue {
        if let Err(e) = api
            .attachment_link_url(&issue.id, &task_url, "Moatless task")
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

    // Forward the inbound prompt. On the mention path (real user comment),
    // fetch prior comments on the issue so the agent has the full thread
    // context.
    let prior_comments = if let (Some(issue), Some(comment)) =
        (session.issue.as_ref(), session.comment.as_ref())
    {
        if comment.user.is_some() {
            match api.fetch_issue_comments(&issue.id, 50).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        error = %e,
                        issue_id = %issue.id,
                        "Failed to fetch prior Linear comments — proceeding without"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let inbound_text = build_inbound_text(session, &prior_comments);
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
        Some(rule.source_matcher.as_str()),
        None,
        None,
        false,
        session
            .creator
            .as_ref()
            .map(|c| c.id.as_str()),
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
    Ok(())
}

fn build_inbound_text(
    session: &AgentSession,
    prior_comments: &[LinearIssueComment],
) -> String {
    // 1. Linear's pre-built XML wins when present.
    if let Some(ctx) = session.prompt_context.as_deref().filter(|s| !s.is_empty()) {
        return ctx.to_string();
    }

    let mut text = String::new();

    // 2. Issue block.
    if let Some(issue) = session.issue.as_ref() {
        let _ = write!(text, "## Linear issue {} — {}", issue.identifier, issue.title);
        if let Some(desc) = issue.description.as_deref().filter(|s| !s.is_empty()) {
            let _ = write!(text, "\n\n{desc}");
        }
        if let Some(url) = issue.url.as_deref().filter(|s| !s.is_empty()) {
            let _ = write!(text, "\n\n{url}");
        }
    }

    // 3. Prior comments (mention path only — caller decides; this fn just
    //    renders whatever it's handed).
    let trigger_id = session.comment.as_ref().map(|c| c.id.as_str());
    let prior: Vec<&LinearIssueComment> = prior_comments
        .iter()
        .filter(|c| Some(c.id.as_str()) != trigger_id)
        .collect();
    if !prior.is_empty() {
        text.push_str("\n\n### Previous comments\n");
        for c in prior {
            let name = c.user.as_ref().map_or("user", |u| u.name.as_str());
            let _ = write!(text, "\n**@{name}:**\n\n{}\n", c.body);
        }
    }

    // 4. Triggering @mention — only when the comment has a real user
    //    (system auto-greeting has `comment.user == None`).
    if let Some(comment) = session.comment.as_ref() {
        if let Some(user) = comment.user.as_ref() {
            if !comment.body.is_empty() {
                let author = if user.name.is_empty() {
                    "user"
                } else {
                    user.name.as_str()
                };
                if !text.is_empty() {
                    text.push_str("\n\n---\n\n");
                }
                let _ = write!(
                    text,
                    "**@{author} just mentioned the agent:**\n\n{}",
                    comment.body
                );
            }
        }
    }

    text
}

async fn create_provider_event_task(
    state: &AppState,
    rule: &TaskTriggerRoute,
    session: &AgentSession,
) -> Result<String, AdapterError> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let short_id = generate_short_id(state).await?;

    let issue = session.issue.as_ref().ok_or_else(|| {
        AdapterError::BadRequest("AgentSession missing issue".into())
    })?;
    let preview = build_inbound_text(session, &[]);

    let row = Task {
        task_id: task_id.clone(),
        short_id: Some(short_id),
        created_at: now.clone(),
        last_message_at: now,
        agent_type: AgentType(rule.agent_type.clone()),
        repository_id: rule.repository_id.clone(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: None,
        vibe: None,
        name: rule
            .task_name
            .clone()
            .or_else(|| Some(format!("{} — {}", issue.identifier, issue.title))),
        description: Some(truncate(&preview, 200)),
        site_id: None,
        site_provider: None,
        branch: None,
        created_by: Some(rule.effective_user_id().to_string()),
        visibility: Visibility::Private,
        scope: rule.scope,
        team_id: rule.team_id.clone(),
        owner_user_id: (rule.scope == crate::scope::Scope::User)
            .then(|| rule.effective_user_id().to_string()),
        created_at_db: None,
        updated_at: None,
        parent_task_id: None,
        append_system_prompt: {
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
        },
        sandbox_started_at: None,
        headless: true,
        ttl_minutes: crate::task::models::DEFAULT_TTL_MINUTES,
        inactive_ttl_days: crate::task::models::DEFAULT_INACTIVE_TTL_DAYS,
        task_template_id: None,
        trigger_id: Some(rule.id.clone()),
    };

    state
        .tasks
        .insert_task(&row)
        .await
        .map_err(|e| AdapterError::Internal(format!("Failed to create task: {e}")))?;

    if !rule.tag_ids.is_empty() {
        if let Err(e) = state.tags.add_task_tags(&task_id, &rule.tag_ids).await {
            warn!(task_id = %task_id, error = %e, "Failed to assign trigger tags");
        }
    }

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                moatless_events::TaskSubType::Open,
                Some("claude-code"),
                serde_json::json!({
                    "adapter": ADAPTER_LABEL,
                    "source_matcher": rule.source_matcher,
                }),
            )
            .await;
    }

    info!(task_id = %task_id, rule_id = %rule.id, "Created task from Linear AgentSession");
    Ok(task_id)
}

async fn provision_task(state: &AppState, task_id: &str) {
    match crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        task_id,
    )
    .await
    {
        Ok(request) => match state.sandbox.provision(task_id, &request).await {
            Ok(_) => {
                info!(task_id = %task_id, "Sandbox provisioning started for Linear task");
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for Linear task");
            }
        },
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to build provision request for Linear task");
        }
    }
}

async fn generate_short_id(state: &AppState) -> Result<String, AdapterError> {
    for _ in 0..10 {
        let id = nanoid::nanoid!(8, &nanoid::alphabet::SAFE);
        if !state
            .tasks
            .exists_by_short_id(&id)
            .await
            .map_err(|e| AdapterError::Internal(format!("Short ID check failed: {e}")))?
        {
            return Ok(id);
        }
    }
    Err(AdapterError::Internal(
        "Failed to generate unique short ID".to_string(),
    ))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

pub(crate) fn task_url(state: &AppState, task_id: &str) -> String {
    let base = state
        .config
        .auth
        .frontend_url
        .trim_end_matches('/');
    format!("{base}/tasks/{task_id}")
}

async fn rule_for_binding(
    state: &AppState,
    _connection: &AdapterConnection,
    binding: &TaskBinding,
) -> Result<Option<TaskTriggerRoute>, AdapterError> {
    if let Some(ref trigger_id) = binding.trigger_id {
        Ok(PgTaskTriggerRouteRepository(state.db.clone())
            .find_by_id(trigger_id)
            .await?)
    } else {
        Ok(None)
    }
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
    .bind(&task_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id, error = %e, "notify_linear_status: binding lookup failed");
            return;
        }
    };

    if rows.is_empty() {
        return;
    }

    for binding in rows {
        let Some(session_id) = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        let scope_value: Option<String> = match sqlx::query_scalar(
            "SELECT scope_value FROM integration_connections WHERE id = $1",
        )
        .bind(&binding.connection_id)
        .fetch_optional(&state.db)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    task_id,
                    connection_id = %binding.connection_id,
                    error = %e,
                    "notify_linear_status: connection lookup failed"
                );
                continue;
            }
        };
        let Some(scope_value) = scope_value else {
            continue;
        };

        let api = LinearAgentApi::new(
            default_http_client(),
            state.db.clone(),
            scope_value,
            state.config.linear.oauth_client_id.clone(),
            state.config.linear.oauth_client_secret.clone(),
        );
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
    .bind(&task_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id, error = %e, "notify_linear_action: binding lookup failed");
            return;
        }
    };

    if rows.is_empty() {
        return;
    }

    for binding in rows {
        let Some(session_id) = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        let scope_value: Option<String> = match sqlx::query_scalar(
            "SELECT scope_value FROM integration_connections WHERE id = $1",
        )
        .bind(&binding.connection_id)
        .fetch_optional(&state.db)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    task_id,
                    connection_id = %binding.connection_id,
                    error = %e,
                    "notify_linear_action: connection lookup failed"
                );
                continue;
            }
        };
        let Some(scope_value) = scope_value else {
            continue;
        };

        let api = LinearAgentApi::new(
            default_http_client(),
            state.db.clone(),
            scope_value,
            state.config.linear.oauth_client_id.clone(),
            state.config.linear.oauth_client_secret.clone(),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::linear::webhook_payloads::{
        AgentSessionComment, AgentSessionIssue, LinearActor,
    };
    use moatless_api_schemas::linear::{LinearCommentUser, LinearIssueComment};

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

    fn session_with(comment: Option<AgentSessionComment>, prompt_context: Option<String>) -> AgentSession {
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

    fn prior(id: &str, body: &str, user_name: &str) -> LinearIssueComment {
        LinearIssueComment {
            id: id.into(),
            body: body.into(),
            created_at: chrono::Utc::now(),
            user: Some(LinearCommentUser {
                id: "u".into(),
                name: user_name.into(),
            }),
        }
    }

    #[test]
    fn build_inbound_text_prefers_prompt_context() {
        let session = session_with(
            Some(user_comment("c-1", "ignored", "Alice")),
            Some("<context>winner</context>".into()),
        );
        let prior_comments = vec![prior("c-0", "ignored too", "Bob")];
        let text = build_inbound_text(&session, &prior_comments);
        assert_eq!(text, "<context>winner</context>");
    }

    #[test]
    fn build_inbound_text_falls_back_to_issue_fields() {
        let session = session_with(None, None);
        let text = build_inbound_text(&session, &[]);
        assert!(text.contains("## Linear issue SP-1 — Bug"));
        assert!(text.contains("Long description here."));
        assert!(text.contains("https://linear.app/acme/issue/SP-1"));
        assert!(!text.contains("### Previous comments"));
        assert!(!text.contains("just mentioned the agent"));
    }

    #[test]
    fn build_inbound_text_skips_system_comment() {
        let system_comment = AgentSessionComment {
            id: "sys-1".into(),
            body: "This thread is for an agent session with moatless.".into(),
            user: None,
        };
        let session = session_with(Some(system_comment), None);
        let text = build_inbound_text(&session, &[]);
        assert!(text.contains("## Linear issue SP-1 — Bug"));
        assert!(!text.contains("This thread is for an agent session"));
        assert!(!text.contains("just mentioned the agent"));
    }

    #[test]
    fn build_inbound_text_appends_user_mention() {
        let session = session_with(
            Some(user_comment("c-1", "please look at this", "Alice")),
            None,
        );
        let text = build_inbound_text(&session, &[]);
        assert!(text.contains("## Linear issue SP-1 — Bug"));
        assert!(text.contains("\n\n---\n\n**@Alice just mentioned the agent:**\n\nplease look at this"));
    }

    #[test]
    fn build_inbound_text_renders_prior_comments() {
        let trigger = user_comment("c-trigger", "please fix this", "Alice");
        let session = session_with(Some(trigger), None);
        let prior_comments = vec![
            prior("c-1", "first thought", "Bob"),
            prior("c-trigger", "duplicate of trigger — must be excluded", "Alice"),
            prior("c-2", "second thought", "Carol"),
        ];
        let text = build_inbound_text(&session, &prior_comments);

        assert!(text.contains("## Linear issue SP-1 — Bug"));
        assert!(text.contains("### Previous comments"));
        assert!(text.contains("**@Bob:**\n\nfirst thought"));
        assert!(text.contains("**@Carol:**\n\nsecond thought"));
        assert!(!text.contains("duplicate of trigger"));

        // Trigger mention rendered last, after a separator.
        let prev_idx = text.find("### Previous comments").unwrap();
        let trigger_idx = text.find("just mentioned the agent").unwrap();
        assert!(prev_idx < trigger_idx);
        assert!(text.contains("\n\n---\n\n**@Alice just mentioned the agent:**\n\nplease fix this"));
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
        let text = build_inbound_text(&session, &[]);
        assert!(!text.contains("## Linear issue"));
        assert_eq!(
            text,
            "**@Alice just mentioned the agent:**\n\nhey"
        );
    }
}

