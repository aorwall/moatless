//! GitHub PR runtime on top of provider connections, task triggers, and task bindings.

use std::fmt::Write as _;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, InboundMessage, TaskBinding, TaskTriggerRoute, ADAPTER_EVENT_BINDING_ORIGIN,
    CONNECTED_THREAD_BINDING_ORIGIN,
};
use crate::adapters::AdapterKind;
use crate::bindings::dao::PgTaskBindingRepository;
use crate::shared_models::AgentType;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::models::{Task, TaskStatus, Visibility};
use crate::triggers::rule_dao::PgTaskTriggerRouteRepository;

use super::adapter::{resolve_user_or_fallback, PrCloseInfo};

/// Adapter value used in adapter connections, trigger sources, and bindings.
const ADAPTER: &str = "github";
/// Subject kind for GitHub PRs (`task_bindings.subject_kind`).
const SUBJECT_KIND: &str = "github_pr";
/// Label written to `task_messages.external_adapter`. Matches the
/// `AdapterKind::GithubPr.as_str()` value so existing UI treats new rows
/// the same as the legacy ones.
const ADAPTER_LABEL: &str = "github_pr";

pub(crate) const ORIGIN_ADAPTER_EVENT: &str = ADAPTER_EVENT_BINDING_ORIGIN;
pub(crate) const ORIGIN_CONNECTED_THREAD: &str = CONNECTED_THREAD_BINDING_ORIGIN;

/// Look up the connection + trigger for an inbound provider event.
pub async fn resolve_provider_event(
    state: &AppState,
    org: &str,
    repo_full_name: &str,
) -> Result<Option<(AdapterConnection, TaskTriggerRoute)>, AdapterError> {
    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_scope_value(ADAPTER, org)
        .await?;
    let Some(connection) = connection else {
        return Ok(None);
    };

    let rule = PgTaskTriggerRouteRepository(state.db.clone())
        .find_by_source_matcher(ADAPTER, repo_full_name)
        .await?;
    let Some(rule) = rule else {
        return Ok(None);
    };

    if rule.connection_id != connection.id {
        debug!(
            source_matcher = %repo_full_name,
            "Task trigger belongs to a different connection — refusing to deliver"
        );
        return Ok(None);
    }

    Ok(Some((connection, rule)))
}

/// Route an inbound GitHub PR message to one or more tasks via bindings.
pub async fn route_inbound_message(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &TaskTriggerRoute,
    msg: InboundMessage,
) -> Result<(), AdapterError> {
    if !rule.active {
        debug!(
            rule_id = %rule.id,
            source_matcher = %rule.source_matcher,
            "Ignoring GitHub PR message: task trigger inactive"
        );
        return Ok(());
    }

    let subject_key = subject_key_from_thread_properties(&msg.thread_properties)?;
    let binding_repo = PgTaskBindingRepository(state.db.clone());

    // Prefer the source task id carried on the inbound message (parsed from
    // the `Moatless task:` footer link `<frontend_url>/tasks/<id>` on the
    // comment body) — it is authoritative and avoids a DB round-trip. Fall back
    // to the external_event_id lookup for paths that don't carry the footer
    // (e.g. human comments, PR opened events). These two signals drive both
    // the pre-loop binding upsert and the cheap self-echo skip below.
    //
    // A third self-echo signal — owner-identity match between `msg.user_id`
    // and the task's `created_by` / `owner_user_id` — runs per-binding inside
    // the delivery loop. It catches out-of-band agent comments (e.g. `gh pr
    // comment` bypassing `moat reply`) that leave no marker and no recorded
    // external_event_id. It is delivery-only and intentionally does not feed
    // `origin_task_id`, which must remain task-id authoritative for the
    // upsert below.
    let origin_task_id = match msg.source_task_id.clone() {
        Some(task_id) => Some(task_id),
        None => match msg.external_event_id.as_deref() {
            Some(ext_id) => {
                let candidates =
                    crate::bindings::runtime::external_id_aliases(state, ADAPTER_LABEL, ext_id);
                state
                    .messages
                    .find_task_id_by_any_external_event_id(&candidates)
                    .await?
            }
            None => None,
        },
    };

    let mut bindings = binding_repo
        .find_by_subject(ADAPTER, SUBJECT_KIND, &subject_key)
        .await?;

    // Skip bindings whose task has closed — a closed task is terminal and must
    // not be resurrected by a new PR comment. (A fresh binding for the
    // originating task is re-added active just below.)
    bindings.retain(|b| b.active);

    // If the originating task isn't subscribed yet (e.g. agent ran
    // `gh pr comment` from a task that never connected to the PR), attach a
    // binding so future events fan out to it too.
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
                Err(e) => {
                    warn!(
                        task_id = %origin_task_id,
                        subject_key = %subject_key,
                        error = %e,
                        "Failed to attach binding for origin task"
                    );
                }
            }
        }
    }

    // Capture the earliest binding (if any) so we can wire a `related_to`
    // relation when a brand-new task is spawned from this PR event. Must
    // happen before the loop below consumes `bindings`.
    let originator_binding_summary: Option<(String, String)> = bindings
        .first()
        .map(|b| (b.task_id.clone(), b.id.clone()));

    let mut delivery_targets: Vec<(TaskBinding, bool)> = Vec::new();
    for binding in bindings {
        // (1) Cheap check: marker / external_event_id resolved the origin task id.
        if origin_task_id.as_deref() == Some(binding.task_id.as_str()) {
            debug!(
                task_id = %binding.task_id,
                subject_key = %subject_key,
                "Skipping GitHub event for originating task"
            );
            continue;
        }

        // (2) Owner-identity check: catches out-of-band agent comments
        // (e.g. `gh pr comment` bypassing `moat reply`, which leaves no marker
        // and no recorded external_event_id). Per-binding DB fetch; typical
        // PRs have 1-2 bindings.
        match state.tasks.find_by_id_internal(&binding.task_id).await {
            Ok(Some(task))
                if is_authored_by_task_owner(
                    &msg.user_id,
                    task.created_by.as_deref(),
                    task.owner_user_id.as_deref(),
                ) =>
            {
                debug!(
                    task_id = %binding.task_id,
                    user_id = %msg.user_id,
                    subject_key = %subject_key,
                    "Skipping GitHub event: sender matches task owner (self-echo)"
                );
                continue;
            }
            // Task gone (Ok(None)) or owner mismatch — deliver. A missing task
            // will surface more clearly inside send_message_flow.
            Ok(_) => {}
            // Fail-open: a DB blip must not silently drop a legitimate event.
            Err(e) => {
                warn!(
                    task_id = %binding.task_id,
                    error = %e,
                    "Failed to fetch task for self-echo check; delivering anyway"
                );
            }
        }

        delivery_targets.push((binding, false));
    }

    let has_provider_event_task = binding_repo
        .find_provider_event_created_for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
        .await?
        .is_some();

    // Only `pull_request` `opened` / `reopened` events spawn a review task.
    // Comments, reviews, check_runs etc. only deliver to existing bindings.
    // Origin is irrelevant here: an agent-created PR still gets a review task,
    // and self-echo is filtered out of `delivery_targets` above.
    let is_pr_open_event = msg.external_event_id.as_deref().is_some_and(|id| {
        id.starts_with("github:pr:opened:") || id.starts_with("github:pr:reopened:")
    });

    if is_pr_open_event && !has_provider_event_task {
        // The eager `gh pr create` hook (see backend/src/message/hooks.rs)
        // upserts a connected_thread binding for the originating task at the
        // moment the agent created the PR — so by the time pr_opened fires,
        // the earliest binding is the originating task. Link the new review
        // task to it as a `related_to` peer (it shares the PR subject_key,
        // it isn't a child of the originator).
        let related = originator_binding_summary.as_ref().map(|(task_id, binding_id)| {
            crate::task::relations::RelatedTask {
                related_task_id: task_id.clone(),
                context: serde_json::json!({
                    "via": "github_pr_webhook",
                    "adapter": ADAPTER,
                    "subject_kind": SUBJECT_KIND,
                    "subject_key": subject_key,
                    "origin_binding_id": binding_id,
                }),
            }
        });
        let task_id = create_provider_event_task(state, rule, &msg).await?;
        if let Some(ref r) = related {
            crate::task::relations::record_related_to(state.task_relations.as_ref(), &task_id, r)
                .await;
        }
        provision_task(state, &task_id).await;

        let binding = upsert_binding(
            state,
            connection,
            Some(rule),
            &task_id,
            &subject_key,
            &msg.thread_properties,
            ORIGIN_ADAPTER_EVENT,
        )
        .await?;

        info!(
            task_id = %task_id,
            rule_id = %rule.id,
            subject_key = %subject_key,
            "Created provider event task for GitHub PR binding"
        );

        delivery_targets.insert(0, (binding, true));
    }

    let receipts = crate::triggers::receipts::PgEventReceiptRepository(state.db.clone());
    for (binding, is_new_task) in delivery_targets {
        if let Some(external_event_id) = msg.external_event_id.as_deref() {
            let is_first_delivery = receipts
                .record_once(
                    ADAPTER,
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
                    "Skipping duplicate GitHub event receipt"
                );
                continue;
            }
        }

        deliver_to_binding(state, rule, connection, &binding, &msg, is_new_task, true).await?;
    }

    Ok(())
}

pub async fn close_tasks_for_pr(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &TaskTriggerRoute,
    info: &PrCloseInfo,
) -> Result<(), AdapterError> {
    let (owner, repo) = rule.source_matcher.split_once('/').ok_or_else(|| {
        AdapterError::BadRequest(format!(
            "Invalid GitHub source_matcher: {}",
            rule.source_matcher
        ))
    })?;
    let pr_number = info.pr_number;
    let subject_key = format!("{owner}/{repo}#{pr_number}");

    let bindings_repo = PgTaskBindingRepository(state.db.clone());
    // Only auto-close tasks that were *created by* this PR trigger
    // (origin = ingress). Tasks merely connected to the PR (origin =
    // connected_thread, e.g. an agent on a UI-created task ran `gh pr create`)
    // pre-existed the PR and must not be closed when the PR closes.
    let bindings = bindings_repo
        .find_by_subject(ADAPTER, SUBJECT_KIND, &subject_key)
        .await?
        .into_iter()
        .filter(|b| {
            b.origin == ADAPTER_EVENT_BINDING_ORIGIN
                && b.trigger_id.as_deref() == Some(rule.id.as_str())
        })
        .collect::<Vec<_>>();

    for binding in bindings {
        let task_id = &binding.task_id;

        match state.tasks.find_by_id_internal(task_id).await {
            Ok(Some(task)) if task.status == TaskStatus::Closed => {
                debug!(task_id, pr_number, "Task already closed, skipping");
                continue;
            }
            Ok(None) => {
                warn!(task_id, pr_number, "Task not found for PR close");
                continue;
            }
            Err(e) => {
                warn!(task_id, pr_number, error = %e, "Failed to fetch task for PR close");
                continue;
            }
            _ => {}
        }

        let siblings_count = match bindings_repo.find_by_task_id(task_id).await {
            Ok(rows) => rows.len(),
            Err(e) => {
                warn!(task_id = %task_id, pr_number, error = %e, "Failed to look up sibling bindings — skipping auto-close");
                continue;
            }
        };
        if siblings_count > 1 {
            info!(
                task_id = %task_id,
                pr_number,
                other_binding_count = siblings_count - 1,
                "PR closed but task has other connected bindings — leaving task open"
            );
            continue;
        }

        // Append the close notice to the task chat *before* tearing down the
        // sandbox + flipping status, so the message is the final visible one
        // and `send_message_flow`'s `last_message_at` write happens while the
        // task is still `Open`. `resume = false` keeps us from briefly waking
        // the agent on a sandbox we're about to clean up.
        let close_msg = build_close_inbound_message(state, owner, repo, rule, info).await;
        if let Err(e) =
            deliver_to_binding(state, rule, connection, &binding, &close_msg, false, false).await
        {
            warn!(
                task_id = %task_id,
                pr_number,
                error = %e,
                "Failed to append PR-close message to task — proceeding with close"
            );
        }

        if let Err(e) = state.sandbox.cleanup(task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to cleanup sandbox on PR close");
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
            warn!(task_id = %task_id, error = %e, "Failed to close task on PR close");
            continue;
        }

        // A closed task is terminal — deactivate its bindings so later comments
        // on the PR don't resurrect it. Best-effort.
        if let Err(e) = bindings_repo.deactivate_by_task(task_id).await {
            warn!(task_id = %task_id, error = %e, "Failed to deactivate bindings on PR close");
        }

        if let Some(ref events) = state.events {
            events
                .publish_task_event(
                    task_id,
                    moatless_events::TaskSubType::Closed,
                    Some("claude-code"),
                    serde_json::json!({"reason": "pr_closed", "pr_number": pr_number}),
                )
                .await;
        }

        info!(task_id = %task_id, pr_number, "Task closed due to PR close");
    }

    Ok(())
}

/// Build the synthetic `InboundMessage` that gets persisted into a task's
/// chat to explain why the task auto-closed when its PR was closed/merged.
///
/// Mirrors the shape of a `pull_request.opened` inbound message so the UI
/// renders it identically (same `display (login):` sender prefix, same
/// source pill, same metadata).
async fn build_close_inbound_message(
    state: &AppState,
    owner: &str,
    repo: &str,
    rule: &TaskTriggerRoute,
    info: &PrCloseInfo,
) -> InboundMessage {
    let status = if info.merged {
        "Merged"
    } else {
        "Closed without merging"
    };

    let mut text = format!(
        "## Pull Request Closed: #{pr_number}\n\n\
         **Title:** {title}\n\
         **Status:** {status}\n\
         **Closed by:** @{login}\n",
        pr_number = info.pr_number,
        title = info.title,
        login = info.sender_login,
    );
    if !info.html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {}", info.html_url);
    }
    text.push_str("\n_Task auto-closed because the PR was closed._");

    let user_id = resolve_user_or_fallback(&info.sender_login, rule, &state.db).await;

    InboundMessage {
        provider: AdapterKind::GithubPr,
        source_id: rule.source_matcher.clone(),
        thread_id: None, // skip thread-context fetch (matches `opened`)
        message_thread_id: info.pr_number.to_string(),
        user_id,
        text,
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "pr_number": info.pr_number,
        }),
        provider_context: serde_json::json!({}),
        external_event_id: Some(super::external_id::pr_event("closed", info.pr_number, None)),
        source_task_id: None,
    }
}

pub async fn upsert_binding(
    state: &AppState,
    connection: &AdapterConnection,
    rule: Option<&TaskTriggerRoute>,
    task_id: &str,
    subject_key: &str,
    thread_properties: &serde_json::Value,
    origin: &str,
) -> Result<TaskBinding, AdapterError> {
    let (owner, repo, pr_number) = parse_pr_destination_config(thread_properties)?;
    let now = Utc::now();

    let filter_uid = thread_properties
        .get("filter_user_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let destination_config = serde_json::json!({
        "owner": owner,
        "repo": repo,
        "pr_number": pr_number,
    });

    let binding = TaskBinding {
        id: format!("ts_{}", uuid::Uuid::new_v4()),
        task_id: task_id.to_string(),
        provider: ADAPTER.to_string(),
        subject_kind: SUBJECT_KIND.to_string(),
        subject_key: subject_key.to_string(),
        connection_id: connection.id.clone(),
        trigger_id: rule.map(|r| r.id.clone()),
        destination_config,
        metadata: serde_json::json!({
            "filter_user_id": filter_uid,
        }),
        last_external_event_id: None,
        origin: origin.to_string(),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    };

    Ok(PgTaskBindingRepository(state.db.clone())
        .upsert(&binding)
        .await?)
}

async fn create_provider_event_task(
    state: &AppState,
    rule: &TaskTriggerRoute,
    msg: &InboundMessage,
) -> Result<String, AdapterError> {
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
        // Tasks auto-created from a GitHub PR webhook default to public so the
        // people collaborating in the PR thread can open the task.
        visibility: Visibility::Public,
        scope: rule.scope,
        team_id: rule.team_id.clone(),
        owner_user_id: (rule.scope == crate::scope::Scope::User)
            .then(|| rule.effective_user_id().to_string()),
        created_at_db: None,
        updated_at: None,
        append_system_prompt: {
            let mut parts = Vec::new();
            if let Some(ref tpl) = rule.prompt_template {
                parts.push(tpl.to_string());
            }
            // Universal reply guidance now lives in the base sandbox-tools prompt.
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
        inactive_ttl_days: rule
            .inactive_ttl_days
            .unwrap_or(crate::task::models::DEFAULT_INACTIVE_TTL_DAYS),
        task_template_id: None,
        trigger_id: Some(rule.id.clone()),
        disabled_servers: None,
    };

    state
        .tasks
        .insert_task(&row)
        .await
        .map_err(|e| AdapterError::Internal(format!("Failed to create task: {e}")))?;

    if !rule.tag_ids.is_empty() {
        if let Err(e) = state.tags.add_task_tags(&task_id, &rule.tag_ids).await {
            warn!(task_id = %task_id, error = %e, "Failed to assign trigger tags to task");
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

    info!(task_id = %task_id, rule_id = %rule.id, "Created task from GitHub PR provider event");
    Ok(task_id)
}

async fn deliver_to_binding(
    state: &AppState,
    rule: &TaskTriggerRoute,
    _connection: &AdapterConnection,
    binding: &TaskBinding,
    msg: &InboundMessage,
    is_new_task: bool,
    resume: bool,
) -> Result<(), AdapterError> {
    // Each PR comment/review streams in as its own message — we no longer
    // backfill the prior comment history. The old `fetch_context` backfill
    // duplicated context and re-sent sibling tasks' comments on every event;
    // the agent can read earlier discussion via its GitHub tooling when needed.
    let mut message_text = msg.text.clone();

    if msg.thread_id.is_some() {
        if let Some(providers) = state.adapters.as_ref() {
            if let Some(provider) = providers.get_by_name(ADAPTER_LABEL) {
                if let Some(hint) = provider.thread_reply_hint(msg) {
                    message_text = format!("{hint}\n\n{message_text}");
                }
            }
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
        msg.external_event_id.as_deref(),
        Some(ADAPTER_LABEL),
        Some(&binding.subject_key),
        Some(rule.source_matcher.as_str()),
        None,
        None,
        false,
        Some(msg.user_id.as_str()),
    )
    .await
    {
        warn!(task_id = %binding.task_id, error = %e, "Failed to send github external message to task");
        return Err(AdapterError::Internal(format!(
            "Failed to send message: {e}"
        )));
    }

    let binding_repo = PgTaskBindingRepository(state.db.clone());
    if let Some(ref ext_id) = msg.external_event_id {
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
                "Failed to ensure sandbox for github external message"
            );
        }
    }

    Ok(())
}

fn subject_key_from_thread_properties(
    thread_properties: &serde_json::Value,
) -> Result<String, AdapterError> {
    let (owner, repo, pr_number) = parse_pr_destination_config(thread_properties)?;
    Ok(format!("{owner}/{repo}#{pr_number}"))
}

fn parse_pr_destination_config(
    value: &serde_json::Value,
) -> Result<(String, String, u64), AdapterError> {
    let owner = value
        .get("owner")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AdapterError::BadRequest("Missing owner in GitHub PR destination".into()))?
        .to_string();
    let repo = value
        .get("repo")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AdapterError::BadRequest("Missing repo in GitHub PR destination".into()))?
        .to_string();
    let pr_number = value
        .get("pr_number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            AdapterError::BadRequest("Missing pr_number in GitHub PR destination".into())
        })?;
    Ok((owner, repo, pr_number))
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
                info!(task_id = %task_id, "Sandbox provisioning started for GitHub PR task");
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for GitHub PR task");
            }
        },
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to build provision request for GitHub PR task");
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

/// Returns `true` when `user_id` matches either identity field on the task's
/// owner, indicating the inbound message was authored by the task's own bot
/// identity and should not be echoed back to it.
///
/// `created_by` covers all trigger-spawned tasks; `owner_user_id` covers
/// user-scoped tasks. Matching either is sufficient. An empty `user_id`
/// never matches.
pub(crate) fn is_authored_by_task_owner(
    user_id: &str,
    created_by: Option<&str>,
    owner_user_id: Option<&str>,
) -> bool {
    if user_id.is_empty() {
        return false;
    }
    created_by == Some(user_id) || owner_user_id == Some(user_id)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let cutoff = max.saturating_sub(3);
    let safe = s
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= cutoff)
        .last()
        .unwrap_or(0);
    format!("{}...", &s[..safe])
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_under_max_returns_input() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_over_max_appends_ellipsis() {
        assert_eq!(truncate("abcdefghij", 8), "abcde...");
    }

    #[test]
    fn truncate_respects_utf8_char_boundaries() {
        // Reproduces PR #621 panic: byte 197 fell inside '→' (3 bytes).
        let mut s = String::new();
        for _ in 0..195 {
            s.push('a');
        }
        s.push_str("→ main");
        let out = truncate(&s, 200);
        assert!(out.is_char_boundary(out.len() - 3));
        assert!(out.ends_with("..."));
    }

    #[test]
    fn truncate_at_exact_multibyte_boundary() {
        let s = "aaaa→bbbb";
        let out = truncate(s, 7);
        assert!(out.ends_with("..."));
        assert!(out.len() <= 7);
    }

    mod self_echo_filter {
        use super::super::is_authored_by_task_owner;

        #[test]
        fn matches_created_by() {
            assert!(is_authored_by_task_owner(
                "bot:reviewer",
                Some("bot:reviewer"),
                None,
            ));
        }

        #[test]
        fn matches_owner_user_id() {
            assert!(is_authored_by_task_owner(
                "bot:reviewer",
                None,
                Some("bot:reviewer"),
            ));
        }

        #[test]
        fn matches_when_both_set() {
            assert!(is_authored_by_task_owner(
                "bot:reviewer",
                Some("bot:reviewer"),
                Some("bot:reviewer"),
            ));
        }

        #[test]
        fn mismatched_identity_does_not_match() {
            assert!(!is_authored_by_task_owner(
                "human-1",
                Some("bot:reviewer"),
                None,
            ));
        }

        #[test]
        fn empty_user_id_never_matches() {
            assert!(!is_authored_by_task_owner(
                "",
                Some("bot:reviewer"),
                Some("bot:reviewer"),
            ));
        }

        #[test]
        fn both_none_never_matches() {
            assert!(!is_authored_by_task_owner("bot:reviewer", None, None));
        }
    }
}
