//! Linear runtime — multi-rule fan-out on top of provider connections,
//! task triggers, and task bindings. Mirrors `github_pr/runtime.rs`
//! with one shape change: `resolve_provider_event` returns a `Vec` of rules so
//! a single team can host several listeners with independent filters.

use std::fmt::Write as _;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::channel::dao::bindings::PgTaskBindingRepository;
use crate::channel::dao::trigger_routes::PgTaskTriggerRouteRepository;
use crate::channel::error::ChannelError;
use crate::channel::models::{
    BindingContext, InboundMessage, ProviderConnection, TaskBinding, TaskTriggerRoute,
    TriggerContext, CONNECTED_THREAD_BINDING_ORIGIN, PROVIDER_EVENT_BINDING_ORIGIN,
};
use crate::channel::provider::ChannelType;
use crate::shared_models::AgentType;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::models::{Task, TaskStatus, Visibility};

use super::provider::IssueCloseInfo;

const PROVIDER: &str = "linear";
const SUBJECT_KIND: &str = "linear_issue";
const CHANNEL_PROVIDER_LABEL: &str = "linear";

const ORIGIN_PROVIDER_EVENT: &str = PROVIDER_EVENT_BINDING_ORIGIN;
const ORIGIN_CONNECTED_THREAD: &str = CONNECTED_THREAD_BINDING_ORIGIN;

/// Generic instructions appended to channel-originated tasks so the agent
/// knows how to reply via `moat reply`. Identical to the GitHub PR copy —
/// kept verbatim so behavior matches what users already see for other
/// providers.
const CHANNEL_REPLY_INSTRUCTIONS: &str = "\
# Channel Reply Instructions

This task was created from a channel message. Send your responses back \
to the channel using the `moat reply` command.

- Use `moat reply --text \"<your response>\"` when this task has exactly one channel.
- If `moat reply` reports multiple targets, list them with \
`moat channels bindings --task $TASK_ID` and target one with \
`moat reply --binding <id> --text \"...\"` (or `--issue <ENG-42>`).
- For the initial message that created this task, always send a reply.
- For follow-up thread messages, reply if you are mentioned or have useful information. \
If the message is not directed at you and you have nothing to add, stay silent (do not send a reply).
- Keep replies concise — the recipient is reading in a messaging app.
- You may send multiple replies for progress updates on long-running tasks.";

/// Build a synthetic `TriggerContext` from an `TaskTriggerRoute` so we can call
/// trait methods (`fetch_context`) that still take a `TriggerContext`.
pub(crate) fn trigger_context(rule: &TaskTriggerRoute) -> TriggerContext {
    TriggerContext {
        id: rule.id.clone(),
        provider: CHANNEL_PROVIDER_LABEL.to_string(),
        source_id: rule.source_matcher.clone(),
        source_name: rule
            .source_name
            .clone()
            .or_else(|| Some(rule.source_matcher.clone())),
        repository_id: rule.repository_id.clone(),
        routing_mode: rule.routing_mode,
        prompt_template: rule.prompt_template.clone(),
        active: rule.active,
        allow_direct_send: rule.allow_direct_send,
        provider_config: rule.provider_config.clone(),
        ongoing_task_id: rule.ongoing_task_id.clone(),
        scope: rule.scope,
        team_id: rule.team_id.clone(),
        task_name: rule.task_name.clone(),
        tag_ids: rule.tag_ids.clone(),
        run_as_user_id: rule.run_as_user_id.clone(),
        agent_type: rule.agent_type.clone(),
        created_by: rule.created_by.clone(),
        created_at: rule.created_at,
        updated_at: rule.updated_at,
    }
}

/// Build a provider binding context from a `TaskBinding`.
pub(crate) fn binding_context(binding: &TaskBinding) -> BindingContext {
    let mut destination_config = binding.destination_config.clone();
    if let Some(obj) = destination_config.as_object_mut() {
        if let Some(ext_id) = binding.last_external_event_id.as_ref() {
            obj.insert("last_external_event_id".into(), serde_json::json!(ext_id));
        }
        if let Some(filter_uid) = binding.metadata.get("filter_user_id").cloned() {
            obj.insert("filter_user_id".into(), filter_uid);
        }
    }
    let issue_id = binding
        .destination_config
        .get("issue_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    BindingContext {
        binding_id: binding.id.clone(),
        trigger_id: binding.trigger_id.clone(),
        task_id: binding.task_id.clone(),
        subject_kind: binding.subject_kind.clone(),
        subject_key: issue_id,
        destination_config,
        metadata: binding.metadata.clone(),
        last_external_event_id: binding.last_external_event_id.clone(),
        last_message_at: binding.last_event_at,
        created_at: binding.created_at,
    }
}

/// Look up every active listener (task trigger) attached to a Linear team
/// under the given workspace connection. Returns the rules — multiple are
/// allowed since migration `0040` dropped the unique constraint on
/// `(provider, source_matcher)`.
pub async fn resolve_provider_event(
    state: &AppState,
    connection: &ProviderConnection,
    team_id: &str,
) -> Result<Vec<TaskTriggerRoute>, ChannelError> {
    let rules = PgTaskTriggerRouteRepository(state.db.clone())
        .list_by_source_matcher(PROVIDER, team_id)
        .await?;

    // Defensive: ignore rules that point at a different workspace connection.
    Ok(rules
        .into_iter()
        .filter(|r| {
            if r.connection_id == connection.id {
                true
            } else {
                debug!(
                    rule_id = %r.id,
                    "Linear task trigger belongs to a different connection — ignoring"
                );
                false
            }
        })
        .collect())
}

/// Route an inbound Linear message produced for a single rule.
pub async fn route_inbound_message(
    state: &AppState,
    connection: &ProviderConnection,
    rule: &TaskTriggerRoute,
    msg: InboundMessage,
) -> Result<(), ChannelError> {
    if !rule.active {
        debug!(
            rule_id = %rule.id,
            source_matcher = %rule.source_matcher,
            "Ignoring Linear message: task trigger inactive"
        );
        return Ok(());
    }

    let subject_key = subject_key_from_thread_properties(&msg.thread_properties)?;
    let binding_repo = PgTaskBindingRepository(state.db.clone());

    let origin_task_id = match msg.channel_external_id.as_deref() {
        Some(ext_id) => {
            let candidates =
                crate::channel::binding_runtime::external_id_aliases(state, "linear", ext_id);
            state
                .messages
                .find_task_id_by_any_channel_external_id(&candidates)
                .await?
        }
        None => None,
    };

    let mut bindings = binding_repo
        .find_by_subject(PROVIDER, SUBJECT_KIND, &subject_key)
        .await?;

    // Attach a binding for the originating task if the agent commented
    // from a task that doesn't yet have one (mirrors GitHub PR behavior).
    if let Some(origin_task_id) = origin_task_id.as_deref() {
        if !bindings.iter().any(|s| s.task_id == origin_task_id) {
            match upsert_binding(
                state,
                connection,
                Some(rule),
                origin_task_id,
                &subject_key,
                &msg.thread_properties,
                ORIGIN_CONNECTED_THREAD,
            )
            .await
            {
                Ok(s) => bindings.push(s),
                Err(e) => warn!(
                    task_id = %origin_task_id,
                    subject_key = %subject_key,
                    error = %e,
                    "Failed to attach binding for origin task"
                ),
            }
        }
    }

    let mut delivery_targets: Vec<(TaskBinding, bool)> = Vec::new();
    for binding in bindings {
        if origin_task_id.as_deref() == Some(binding.task_id.as_str()) {
            debug!(
                task_id = %binding.task_id,
                subject_key = %subject_key,
                "Skipping Linear event for originating task"
            );
            continue;
        }
        delivery_targets.push((binding, false));
    }

    // Has THIS rule already spawned a provider-event task for this issue?
    let has_provider_event_task_for_rule = binding_repo
        .find_by_subject(PROVIDER, SUBJECT_KIND, &subject_key)
        .await?
        .iter()
        .any(|s| {
            s.origin == ORIGIN_PROVIDER_EVENT && s.trigger_id.as_deref() == Some(rule.id.as_str())
        });

    let is_issue_event = msg.channel_external_id.as_deref().is_some_and(|id| {
        id.starts_with("linear:issue:opened:") || id.starts_with("linear:issue:state_changed:")
    });

    if is_issue_event && !has_provider_event_task_for_rule {
        let task_id = create_provider_event_task(state, rule, &msg).await?;
        provision_task(state, &task_id).await;

        let binding = upsert_binding(
            state,
            connection,
            Some(rule),
            &task_id,
            &subject_key,
            &msg.thread_properties,
            ORIGIN_PROVIDER_EVENT,
        )
        .await?;

        info!(
            task_id = %task_id,
            rule_id = %rule.id,
            subject_key = %subject_key,
            "Created provider event task for Linear issue"
        );

        delivery_targets.insert(0, (binding, true));
    }

    let receipts = crate::channel::dao::receipts::PgEventReceiptRepository(state.db.clone());
    for (binding, is_new_task) in delivery_targets {
        if let Some(external_event_id) = msg.channel_external_id.as_deref() {
            let is_first_delivery = receipts
                .record_once(
                    PROVIDER,
                    external_event_id,
                    SUBJECT_KIND,
                    &subject_key,
                    &binding.task_id,
                )
                .await?;
            if !is_first_delivery {
                debug!(
                    task_id = %binding.task_id,
                    external_event_id,
                    "Skipping duplicate Linear event receipt"
                );
                continue;
            }
        }

        deliver_to_binding(state, rule, connection, &binding, &msg, is_new_task, true).await?;
    }

    Ok(())
}

/// Close every task whose binding was spawned by THIS rule and points at
/// the given issue. Other listeners' tasks are left alone.
pub async fn close_tasks_for_issue(
    state: &AppState,
    connection: &ProviderConnection,
    rule: &TaskTriggerRoute,
    info: &IssueCloseInfo,
) -> Result<(), ChannelError> {
    let subject_key = info.issue_id.clone();
    let bindings_repo = PgTaskBindingRepository(state.db.clone());
    let bindings = bindings_repo
        .find_by_subject(PROVIDER, SUBJECT_KIND, &subject_key)
        .await?
        .into_iter()
        .filter(|s| s.trigger_id.as_deref() == Some(rule.id.as_str()))
        .collect::<Vec<_>>();

    for binding in bindings {
        let task_id = &binding.task_id;

        match state.tasks.find_by_id_internal(task_id).await {
            Ok(Some(task)) if task.status == TaskStatus::Closed => {
                debug!(task_id, issue = %info.identifier, "Task already closed, skipping");
                continue;
            }
            Ok(None) => {
                warn!(task_id, issue = %info.identifier, "Task not found for issue close");
                continue;
            }
            Err(e) => {
                warn!(task_id, issue = %info.identifier, error = %e, "Failed to fetch task for issue close");
                continue;
            }
            _ => {}
        }

        // Bail out of the auto-close if the task is also subscribed elsewhere
        // (e.g. a GitHub PR connected to the same task) — same guard as the
        // GitHub PR runtime.
        let siblings_count = match bindings_repo.find_by_task_id(task_id).await {
            Ok(rows) => rows.len(),
            Err(e) => {
                warn!(task_id = %task_id, issue = %info.identifier, error = %e, "Failed to look up sibling bindings — skipping auto-close");
                continue;
            }
        };
        if siblings_count > 1 {
            info!(
                task_id = %task_id,
                issue = %info.identifier,
                other_binding_count = siblings_count - 1,
                "Linear issue closed but task has other connected bindings — leaving task open"
            );
            continue;
        }

        let close_msg = build_close_inbound_message(rule, info);
        if let Err(e) =
            deliver_to_binding(state, rule, connection, &binding, &close_msg, false, false).await
        {
            warn!(
                task_id = %task_id,
                issue = %info.identifier,
                error = %e,
                "Failed to append issue-close message to task — proceeding with close"
            );
        }

        if let Err(e) = state.sandbox.cleanup(task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to cleanup sandbox on Linear close");
        }

        if let Err(e) = state
            .tasks
            .update_task(
                task_id,
                &TaskPatch {
                    status: Some(TaskStatus::Closed),
                    ..Default::default()
                },
            )
            .await
        {
            warn!(task_id = %task_id, error = %e, "Failed to close task on Linear issue close");
            continue;
        }

        if let Some(ref events) = state.events {
            events
                .publish_task_event(
                    task_id,
                    moatless_events::TaskSubType::Closed,
                    Some("claude-code"),
                    serde_json::json!({
                        "reason": "linear_state_done",
                        "issue_identifier": info.identifier,
                        "state_name": info.state_name,
                    }),
                )
                .await;
        }

        info!(task_id = %task_id, issue = %info.identifier, "Task closed due to Linear issue terminal state");
    }

    Ok(())
}

fn build_close_inbound_message(rule: &TaskTriggerRoute, info: &IssueCloseInfo) -> InboundMessage {
    let mut text = format!(
        "## Linear issue {} transitioned to {}\n\nClosing this task.",
        info.identifier, info.state_name,
    );
    if !info.url.is_empty() {
        let _ = write!(text, "\n\n{}", info.url);
    }

    InboundMessage {
        provider: ChannelType::Linear,
        source_id: rule.source_matcher.clone(),
        thread_id: None,
        message_thread_id: info.issue_id.clone(),
        user_id: rule.effective_user_id().to_string(),
        text,
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "issue_id": info.issue_id,
            "identifier": info.identifier,
            "team_id": rule.source_matcher,
            "state_name": info.state_name,
        }),
        provider_context: serde_json::json!({}),
        channel_external_id: Some(format!("linear:issue:closed:{}", info.issue_id)),
    }
}

pub async fn upsert_binding(
    state: &AppState,
    connection: &ProviderConnection,
    rule: Option<&TaskTriggerRoute>,
    task_id: &str,
    subject_key: &str,
    thread_properties: &serde_json::Value,
    origin: &str,
) -> Result<TaskBinding, ChannelError> {
    let issue_id = thread_properties
        .get("issue_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ChannelError::BadRequest("Missing issue_id in Linear thread properties".into())
        })?
        .to_string();
    let identifier = thread_properties
        .get("identifier")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let team_id = thread_properties
        .get("team_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();

    let now = Utc::now();
    let filter_uid = thread_properties
        .get("filter_user_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let binding = TaskBinding {
        id: format!("ts_{}", uuid::Uuid::new_v4()),
        task_id: task_id.to_string(),
        provider: PROVIDER.to_string(),
        subject_kind: SUBJECT_KIND.to_string(),
        subject_key: subject_key.to_string(),
        connection_id: connection.id.clone(),
        trigger_id: rule.map(|r| r.id.clone()),
        destination_config: serde_json::json!({
            "issue_id": issue_id,
            "identifier": identifier,
            "team_id": team_id,
        }),
        metadata: serde_json::json!({
            "filter_user_id": filter_uid,
        }),
        last_external_event_id: None,
        origin: origin.to_string(),
        created_at: now,
        last_event_at: now,
    };

    Ok(PgTaskBindingRepository(state.db.clone())
        .upsert(&binding)
        .await?)
}

async fn create_provider_event_task(
    state: &AppState,
    rule: &TaskTriggerRoute,
    msg: &InboundMessage,
) -> Result<String, ChannelError> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let short_id = generate_short_id(state).await?;

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
        name: rule.task_name.clone(),
        description: Some(truncate(&msg.text, 200)),
        site_id: None,
        site_provider: None,
        branch: None,
        created_by: Some(rule.effective_user_id().to_string()),
        visibility: Visibility::Private,
        scope: rule.scope,
        team_id: rule.team_id.clone(),
        created_at_db: None,
        updated_at: None,
        parent_task_id: None,
        append_system_prompt: {
            let mut parts = Vec::new();
            if let Some(ref tpl) = rule.prompt_template {
                parts.push(tpl.to_string());
            }
            parts.push(CHANNEL_REPLY_INSTRUCTIONS.to_string());
            if let Some(providers) = state.channel_providers.as_ref() {
                if let Some(provider) = providers.get_by_name(CHANNEL_PROVIDER_LABEL) {
                    if let Some(extra) = provider.reply_instructions() {
                        parts.push(extra);
                    }
                }
            }
            Some(parts.join("\n\n"))
        },
        sandbox_started_at: None,
        headless: true,
        ttl_minutes: crate::task::models::DEFAULT_TTL_MINUTES,
    };

    state
        .tasks
        .insert_task(&row)
        .await
        .map_err(|e| ChannelError::Internal(format!("Failed to create task: {e}")))?;

    if !rule.tag_ids.is_empty() {
        if let Err(e) = state.tags.add_task_tags(&task_id, &rule.tag_ids).await {
            warn!(task_id = %task_id, error = %e, "Failed to assign channel tags to task");
        }
    }

    if let Some(ref events) = state.events {
        events
            .publish_task_event(
                &task_id,
                moatless_events::TaskSubType::Open,
                Some("claude-code"),
                serde_json::json!({
                    "channel": CHANNEL_PROVIDER_LABEL,
                    "source_matcher": rule.source_matcher,
                }),
            )
            .await;
    }

    info!(task_id = %task_id, rule_id = %rule.id, "Created task from Linear provider event");
    Ok(task_id)
}

async fn deliver_to_binding(
    state: &AppState,
    rule: &TaskTriggerRoute,
    _connection: &ProviderConnection,
    binding: &TaskBinding,
    msg: &InboundMessage,
    is_new_task: bool,
    resume: bool,
) -> Result<(), ChannelError> {
    let synthetic_cfg = trigger_context(rule);
    let synthetic_thr = binding_context(binding);

    let thread_context = if msg.thread_id.is_some() {
        if let Some(providers) = state.channel_providers.as_ref() {
            if let Some(provider) = providers.get_by_name(CHANNEL_PROVIDER_LABEL) {
                let mut exclude_ids = match state
                    .messages
                    .get_channel_external_ids(&binding.task_id)
                    .await
                {
                    Ok(ids) => ids,
                    Err(e) => {
                        warn!(error = %e, "Failed to fetch channel external IDs, fetching full context");
                        vec![]
                    }
                };
                if let Some(ref ext_id) = msg.channel_external_id {
                    exclude_ids.push(ext_id.clone());
                }

                match provider
                    .fetch_context(&synthetic_thr, &synthetic_cfg, 20, &exclude_ids)
                    .await
                {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        warn!(error = %e, "Failed to fetch Linear context");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let has_cursor = binding.last_external_event_id.is_some();

    let mut message_text = msg.text.clone();
    let sender_label = match state.users.get_user(&msg.user_id).await {
        Ok(Some(user)) => {
            let display = user.name.as_deref().unwrap_or(&user.login);
            Some(format!("{display} ({login})", login = user.login))
        }
        _ => None,
    };
    if let Some(label) = sender_label {
        message_text = format!("{label}: {message_text}");
    }

    if msg.thread_id.is_some() {
        if let Some(providers) = state.channel_providers.as_ref() {
            if let Some(provider) = providers.get_by_name(CHANNEL_PROVIDER_LABEL) {
                if let Some(hint) = provider.thread_reply_hint(msg) {
                    message_text = format!("{hint}\n\n{message_text}");
                }
            }
        }
    }

    if let Some(ctx) = thread_context {
        if has_cursor {
            message_text = format!("{ctx}\n\n---\n\n{message_text}");
        } else {
            message_text = format!(
                "Thread context (previous messages):\n\n{ctx}\n\n---\n\nNew message:\n\n{message_text}"
            );
        }
    }

    if let Err(e) = crate::message::send::send_message_flow(
        state,
        &binding.task_id,
        &message_text,
        None,
        None,
        &msg.file_ids,
        &[],
        resume,
        msg.channel_external_id.as_deref(),
        Some(CHANNEL_PROVIDER_LABEL),
        Some(rule.source_matcher.as_str()),
        None,
        None,
        false,
    )
    .await
    {
        warn!(task_id = %binding.task_id, error = %e, "Failed to send Linear channel message to task");
        return Err(ChannelError::Internal(format!(
            "Failed to send message: {e}"
        )));
    }

    let binding_repo = PgTaskBindingRepository(state.db.clone());
    if let Some(ref ext_id) = msg.channel_external_id {
        if let Err(e) = binding_repo.touch_cursor(&binding.id, ext_id).await {
            warn!(
                binding_id = %binding.id,
                error = %e,
                "Failed to update last_external_event_id on binding"
            );
        }
    } else if let Err(e) = binding_repo.touch_last_event(&binding.id).await {
        warn!(
            binding_id = %binding.id,
            error = %e,
            "Failed to bump last_event_at on binding"
        );
    }

    if !is_new_task && resume {
        if let Err(e) =
            crate::sandbox::notify::ensure_sandbox_and_notify(state, &binding.task_id).await
        {
            warn!(
                task_id = %binding.task_id,
                error = %e,
                "Failed to ensure sandbox for Linear channel message"
            );
        }
    }

    Ok(())
}

fn subject_key_from_thread_properties(
    thread_properties: &serde_json::Value,
) -> Result<String, ChannelError> {
    thread_properties
        .get("issue_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ChannelError::BadRequest("Missing issue_id in Linear thread properties".into())
        })
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

async fn generate_short_id(state: &AppState) -> Result<String, ChannelError> {
    for _ in 0..10 {
        let id = nanoid::nanoid!(8, &nanoid::alphabet::SAFE);
        if !state
            .tasks
            .exists_by_short_id(&id)
            .await
            .map_err(|e| ChannelError::Internal(format!("Short ID check failed: {e}")))?
        {
            return Ok(id);
        }
    }
    Err(ChannelError::Internal(
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
