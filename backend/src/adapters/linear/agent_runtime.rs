//! Linear Agents runtime — handles `AgentSessionEvent` and downstream
//! `Issue.update` events.
//!
//! Inbound paths:
//! - `handle_session_created` — mention or assignment. Mention triggers
//!   immediately; assignment is parked or proceeds based on the issue's
//!   current workflow state.
//! - `handle_session_prompted` — follow-up prompt on an existing session.
//!   Routes to `send_message_flow` for the bound task.
//! - `handle_issue_update` — wakes parked sessions when an issue transitions
//!   INTO `trigger_states`, and closes active tasks when it transitions INTO
//!   `terminal_states`.

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

use super::agent_api::{ActivityKind, LinearAgentApi};
use super::config::LinearRuleConfig;
use super::webhook_payloads::{AgentSession, IssuePayload};

const ADAPTER: &str = "linear";
const SUBJECT_KIND: &str = "linear_agent_session";
const ADAPTER_LABEL: &str = "linear";

/// Result of a `handle_session_created` decision — used by the dispatcher to
/// pick the synchronous Linear activity to emit before returning 200.
pub enum SessionCreatedAction {
    /// Mention path or assignment in `trigger_states` — task is being spawned.
    Proceed,
    /// Assignment outside `trigger_states` — session is parked.
    Park,
}

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

/// Process an `AgentSessionEvent.created` payload. Decides between the
/// proceed-now path (mention or assignment-with-state-in-trigger-states) and
/// the park-and-wait path (assignment outside trigger states).
///
/// Performs synchronous Linear API calls (`agentActivityCreate` +
/// `agentSessionUpdate`) before spawning the long-running task creation so
/// the webhook response stays inside Linear's 10-second window.
pub async fn handle_session_created(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rules: &[TaskTriggerRoute],
    session: &AgentSession,
) -> Result<SessionCreatedAction, AdapterError> {
    let issue = session.issue.as_ref().ok_or_else(|| {
        AdapterError::BadRequest("AgentSession missing issue payload".into())
    })?;

    // Mention path: any comment present → proceed regardless of state.
    let is_mention = session.comment.is_some();

    // Resolve the rule by team. We always proceed even with multiple rules:
    // the first active one wins for spawning. Multi-listener fan-out is
    // limited to Issue.update wake/close paths (where state changes drive
    // routing independently).
    let team_id = issue
        .team_id
        .clone()
        .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
        .unwrap_or_default();
    let rule = rules.first().cloned();

    let rule_config = rule.as_ref().map_or_else(
        || LinearRuleConfig {
            require_labels: vec![],
            trigger_states: vec!["In Progress".to_string()],
            terminal_states: vec![
                "Done".to_string(),
                "Cancelled".to_string(),
                "Canceled".to_string(),
                "Duplicate".to_string(),
            ],
            close_task_on_close: true,
        },
        LinearRuleConfig::from_rule,
    );

    let state_name = issue.state.as_ref().map_or("", |s| s.name.as_str());
    let in_trigger_state = rule_config
        .trigger_states
        .iter()
        .any(|s| s == state_name);

    let should_proceed = is_mention || in_trigger_state;

    if !should_proceed {
        // Assignment with state outside trigger_states — park.
        let states = rule_config.trigger_states.join(", ");
        let prompt = if states.is_empty() {
            "I'm assigned. Move the issue to a configured trigger state to begin.".to_string()
        } else {
            format!(
                "I'm assigned. Move the issue to one of `{states}` and I'll start work."
            )
        };
        if let Err(e) = api
            .agent_activity_create(&session.id, ActivityKind::Elicitation, &prompt)
            .await
        {
            warn!(session_id = %session.id, error = %e, "Failed to emit Linear elicitation activity");
        }
        if let Some(rule) = rule.as_ref() {
            if let Err(e) =
                upsert_pending_binding(state, connection, rule, issue, &session.id).await
            {
                warn!(
                    session_id = %session.id,
                    error = %e,
                    "Failed to upsert pending Linear binding"
                );
            }
        }
        return Ok(SessionCreatedAction::Park);
    }

    // Proceed: emit a heartbeat thought synchronously so Linear sees activity
    // within the 10s window. The actual sandbox provisioning happens in a
    // spawned task.
    if let Err(e) = api
        .agent_activity_create(
            &session.id,
            ActivityKind::Thought,
            "Spinning up sandbox…",
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
        return Ok(SessionCreatedAction::Proceed);
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

    Ok(SessionCreatedAction::Proceed)
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

    // Optimistically ack receipt with a thought heartbeat so Linear doesn't
    // hang on a slow `send_message_flow`.
    if let Err(e) = api
        .agent_activity_create(session_id, ActivityKind::Thought, "Working on it…")
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

/// Handle an `Issue.update` payload. Two distinct flows:
/// - State INTO `trigger_states` AND a pending binding exists → wake.
/// - State INTO `terminal_states` AND an active binding exists → close.
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

    if rule_config.trigger_states.iter().any(|s| s == state_name) {
        wake_pending_bindings_for_issue(state, api, connection, rule, issue).await?;
    }

    if rule_config.close_task_on_close
        && rule_config.terminal_states.iter().any(|s| s == state_name)
    {
        close_active_bindings_for_issue(state, api, connection, rule, issue).await?;
    }

    Ok(())
}

async fn wake_pending_bindings_for_issue(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &TaskTriggerRoute,
    issue: &IssuePayload,
) -> Result<(), AdapterError> {
    // Pending bindings are keyed by session_id (subject_key), so we look them
    // up via the issue_id stored in destination_config.
    let rows = sqlx::query_as::<_, TaskBinding>(
        "SELECT id, task_id, provider, subject_kind, subject_key, connection_id, \
                trigger_id, destination_config, metadata, last_external_event_id, \
                origin, created_at, last_event_at \
         FROM task_bindings \
         WHERE provider = $1 \
           AND subject_kind = $2 \
           AND destination_config->>'issue_id' = $3 \
           AND destination_config->>'pending' = 'true'",
    )
    .bind(ADAPTER)
    .bind(SUBJECT_KIND)
    .bind(&issue.id)
    .fetch_all(&state.db)
    .await
    .map_err(AdapterError::Db)?;

    if rows.is_empty() {
        return Ok(());
    }

    for binding in rows {
        let session_id = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);

        // Clear the pending flag and synthesize a task.
        let mut new_config = binding.destination_config.clone();
        if let Some(obj) = new_config.as_object_mut() {
            obj.remove("pending");
        }

        if let Some(ref sid) = session_id {
            if let Err(e) = api
                .agent_activity_create(sid, ActivityKind::Thought, "Spinning up sandbox…")
                .await
            {
                warn!(session_id = %sid, error = %e, "Failed to emit wake thought");
            }
        }

        // Create the task now that we're unblocked.
        let synthetic_session = AgentSession {
            id: session_id.clone().unwrap_or_default(),
            issue: Some(super::webhook_payloads::AgentSessionIssue {
                id: issue.id.clone(),
                identifier: issue.identifier.clone(),
                title: issue.title.clone(),
                description: None,
                url: issue.url.clone(),
                team_id: issue.team_id.clone(),
                team: issue.team.clone(),
                state: issue.state.clone(),
            }),
            comment: None,
            creator: None,
            prompt_context: None,
        };

        if let Err(e) = provision_session_task_with_existing_binding(
            state,
            api,
            connection,
            rule,
            &synthetic_session,
            &binding.id,
            &new_config,
        )
        .await
        {
            warn!(
                binding_id = %binding.id,
                error = %e,
                "Failed to provision Linear task from woken binding"
            );
        }
    }

    Ok(())
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
        "SELECT id, task_id, provider, subject_kind, subject_key, connection_id, \
                trigger_id, destination_config, metadata, last_external_event_id, \
                origin, created_at, last_event_at \
         FROM task_bindings \
         WHERE provider = $1 \
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
                .agent_activity_create(session_id, ActivityKind::Response, &text)
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

/// Upsert a "pending" binding that records a parked session.
async fn upsert_pending_binding(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &TaskTriggerRoute,
    issue: &super::webhook_payloads::AgentSessionIssue,
    session_id: &str,
) -> Result<(), AdapterError> {
    // Pending bindings have no task yet — use a placeholder task_id that
    // we'll overwrite when the session wakes. Encode session_id so the wake
    // path can find this row by `destination_config->>'issue_id'`.
    let team_id = issue
        .team_id
        .clone()
        .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
        .unwrap_or_default();
    let destination_config = serde_json::json!({
        "session_id": session_id,
        "issue_id": issue.id,
        "identifier": issue.identifier,
        "team_id": team_id,
        "issue_url": issue.url,
        "pending": true,
    });

    let now = Utc::now();
    let placeholder_task_id = format!("pending-linear-{session_id}");

    sqlx::query(
        r#"INSERT INTO task_bindings
           (id, task_id, provider, subject_kind, subject_key, connection_id,
            trigger_id, destination_config, metadata, last_external_event_id,
            origin, created_at, last_event_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, $10, $11, $11)
           ON CONFLICT (provider, subject_kind, subject_key) DO UPDATE SET
               destination_config = EXCLUDED.destination_config,
               trigger_id = EXCLUDED.trigger_id,
               connection_id = EXCLUDED.connection_id,
               last_event_at = EXCLUDED.last_event_at"#,
    )
    .bind(format!("ts_{}", uuid::Uuid::new_v4()))
    .bind(placeholder_task_id)
    .bind(ADAPTER)
    .bind(SUBJECT_KIND)
    .bind(session_id)
    .bind(&connection.id)
    .bind(&rule.id)
    .bind(&destination_config)
    .bind(serde_json::json!({}))
    .bind(ADAPTER_EVENT_BINDING_ORIGIN)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(AdapterError::Db)?;
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

async fn provision_session_task_with_existing_binding(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    rule: &TaskTriggerRoute,
    session: &AgentSession,
    binding_id: &str,
    cleared_config: &serde_json::Value,
) -> Result<(), AdapterError> {
    let task_id = create_provider_event_task(state, rule, session).await?;
    provision_task(state, &task_id).await;

    let now = Utc::now();
    sqlx::query(
        "UPDATE task_bindings \
         SET task_id = $1, destination_config = $2, last_event_at = $3 \
         WHERE id = $4",
    )
    .bind(&task_id)
    .bind(cleared_config)
    .bind(now)
    .bind(binding_id)
    .execute(&state.db)
    .await
    .map_err(AdapterError::Db)?;

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

    let inbound_text = build_inbound_text(session, &[]);
    let external_event_id = format!("linear:session:wake:{}", session.id);
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
        None,
    )
    .await
    {
        warn!(
            task_id = %task_id,
            session_id = %session.id,
            error = %e,
            "Failed to forward Linear wake prompt to task"
        );
    }
    let _ = connection;
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

