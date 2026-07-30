//! Shared binding runtime backed by adapter connections, triggers, rules,
//! and task bindings.

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, Audience, BindingContext, InboundMessage, LoopContext, RoutingMode,
    SubscriptionMatch, TaskBinding,
};
use crate::bindings::dao::PgTaskBindingRepository;
use crate::bindings::metrics::{inbound_reason, record_inbound};
use crate::bindings::recipients::{resolve_recipients, EventProvenance};
use crate::loops::firing::inbound;
use crate::state::AppState;
use crate::task::creation::TaskCreationError;
use crate::task::models::TaskStatus;
use moatless_backend_core::scope::ScopeContext;

pub use crate::adapters::models::{
    ADAPTER_EVENT_BINDING_ORIGIN as ORIGIN_ADAPTER_EVENT,
    CONNECTED_THREAD_BINDING_ORIGIN as ORIGIN_CONNECTED_THREAD,
};

/// Build a synthetic `LoopContext` from a trigger+route view for provider
/// trait methods that still accept a config-shaped value.
pub fn trigger_context(rule: &SubscriptionMatch, provider_label: &str) -> LoopContext {
    LoopContext {
        id: rule.id.clone(),
        provider: provider_label.to_string(),
        source_id: rule.source_matcher.clone(),
        source_name: rule.source_name.clone(),
        repository_id: rule.repository_id.clone(),
        routing_mode: rule.routing_mode,
        prompt_template: rule.prompt_template.clone(),
        active: rule.active,
        allow_direct_send: rule.allow_direct_send,
        provider_config: rule.provider_config.clone(),
        ongoing_task_id: rule.ongoing_task_id.clone(),
        scope: rule.scope,
        task_name: rule.task_name.clone(),
        tag_ids: rule.tag_ids.clone(),
        run_as_user_id: rule.run_as_user_id.clone(),
        agent_type: rule.agent_type.clone(),
        inactive_ttl_days: rule.inactive_ttl_days,
        created_by: rule.created_by.clone(),
        created_at: rule.created_at,
        updated_at: rule.updated_at,
    }
}

/// Build a provider binding context from a task binding. Never persist this row.
pub fn binding_context(binding: &TaskBinding) -> BindingContext {
    let mut destination_config = binding.destination_config.clone();
    if let Some(obj) = destination_config.as_object_mut() {
        if let Some(ext_id) = binding.last_external_event_id.as_ref() {
            obj.insert("last_external_event_id".into(), serde_json::json!(ext_id));
        }
        if let Some(filter_uid) = binding.metadata.get("filter_user_id").cloned() {
            obj.insert("filter_user_id".into(), filter_uid);
        }
    }

    BindingContext {
        binding_id: binding.id.clone(),
        trigger_id: binding.trigger_id.clone(),
        task_id: binding.task_id.clone(),
        subject_kind: binding.subject_kind.clone(),
        subject_key: binding.subject_key.clone(),
        destination_config,
        metadata: binding.metadata.clone(),
        last_external_event_id: binding.last_external_event_id.clone(),
        last_message_at: binding.last_event_at,
        created_at: binding.created_at,
    }
}

pub async fn resolve_chat_provider_event(
    state: &AppState,
    provider: &str,
    scope_value: &str,
    source_id: &str,
) -> Result<Option<(AdapterConnection, SubscriptionMatch)>, AdapterError> {
    let connection_repo = PgAdapterConnectionRepository(state.db.clone());
    // Look up exactly the requested scope_value. We deliberately do not fall
    // back to the "default" connection when scope_value is unknown — that would
    // silently route a tenant A event to tenant B's connection. Callers that
    // want the default behavior pass "default" explicitly.
    let Some(connection) = connection_repo
        .find_by_scope_value(provider, scope_value)
        .await?
    else {
        warn!(
            provider,
            scope_value, "No provider connection matched incoming event"
        );
        record_inbound(provider, "ignored", inbound_reason::NO_CONNECTION, "", "");
        return Ok(None);
    };

    let rule_repo = state.loops.clone();
    let rule = rule_repo
        .find_subscription_by_source_matcher(provider, source_id)
        .await?
        .or(rule_repo.find_subscription_wildcard(provider).await?);
    let Some(rule) = rule else {
        record_inbound(
            provider,
            "ignored",
            inbound_reason::NO_RULE_MATCH,
            "",
            &connection.id,
        );
        return Ok(None);
    };

    if rule.connection_id != connection.id {
        debug!(
            provider,
            source_id,
            rule_id = %rule.id,
            connection_id = %connection.id,
            "Task trigger belongs to a different connection"
        );
        record_inbound(
            provider,
            "ignored",
            inbound_reason::WRONG_CONNECTION,
            &rule.id,
            &connection.id,
        );
        return Ok(None);
    }

    Ok(Some((connection, rule)))
}

#[allow(clippy::too_many_arguments)]
pub async fn route_inbound_message(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    msg: InboundMessage,
    provider_label: &str,
    subject_kind: &str,
    subject_key: &str,
    destination_config: serde_json::Value,
) -> Result<String, AdapterError> {
    if !rule.active {
        debug!(rule_id = %rule.id, "Ignoring external message: task trigger inactive");
        record_inbound(
            provider_label,
            "ignored",
            inbound_reason::INACTIVE_RULE,
            &rule.id,
            &connection.id,
        );
        return Err(AdapterError::ConfigNotFound(rule.id.clone()));
    }

    let binding_repo = PgTaskBindingRepository(state.db.clone());
    // Provenance: the task an adapter recovered from the event's own body (the
    // `Moatless task:` footer), else the task an already-recorded event
    // identifier names. Never an identity.
    let origin_task_id = match msg.source_task_id.clone() {
        Some(task_id) => Some(task_id),
        None => match msg.external_event_id.as_deref() {
            Some(ext_id) => {
                let candidates = external_id_aliases(state, provider_label, ext_id);
                state
                    .messages
                    .find_task_id_by_any_external_event_id(&candidates)
                    .await?
            }
            None => None,
        },
    };

    let mut bindings = binding_repo
        .find_by_subject(provider_label, subject_kind, subject_key)
        .await?;

    // Skip bindings whose task has closed — a closed task is terminal and must
    // not be resurrected by a new inbound message. Fresh bindings for the
    // originating / ongoing task are re-added active below.
    bindings.retain(|b| b.active);

    if let Some(origin_task_id) = origin_task_id.as_deref() {
        if !bindings.iter().any(|s| s.task_id == origin_task_id) {
            let sub = upsert_binding(
                state,
                connection,
                Some(rule),
                origin_task_id,
                provider_label,
                subject_kind,
                subject_key,
                destination_config.clone(),
                ORIGIN_CONNECTED_THREAD,
                msg.external_event_id.clone(),
            )
            .await?;
            bindings.push(sub);
        }
    }

    if rule.routing_mode == RoutingMode::Ongoing {
        if let Some(task_id) = open_ongoing_task_id(state, rule).await {
            if !bindings.iter().any(|s| s.task_id == task_id) {
                // Pass None so a newly attached binding starts with no cursor —
                // `deliver_to_binding` then fetches the full prior thread as
                // context, and `touch_cursor` advances it after delivery. For an
                // existing binding the COALESCE in the upsert preserves the
                // current cursor.
                let sub = upsert_binding(
                    state,
                    connection,
                    Some(rule),
                    &task_id,
                    provider_label,
                    subject_kind,
                    subject_key,
                    destination_config.clone(),
                    ORIGIN_ADAPTER_EVENT,
                    None,
                )
                .await?;
                bindings.push(sub);
            }
        }
    }

    let mut new_task_binding_id = None;
    if should_create_binding_task(state, rule, &bindings).await? {
        // The earliest binding for this subject (if any) belongs to the task
        // that originated it — typically a `connected_thread` binding placed
        // when an earlier task produced the resource. We link the new task
        // to it as `related_to` (peer sharing the same external artifact),
        // not as a parent.
        let related = bindings
            .first()
            .map(|b| crate::task::relations::RelatedTask {
                related_task_id: b.task_id.clone(),
                context: serde_json::json!({
                    "via": "binding_webhook",
                    "adapter": provider_label,
                    "subject_kind": subject_kind,
                    "subject_key": subject_key,
                    "origin_binding_id": b.id,
                }),
            });
        let config = trigger_context(rule, provider_label);
        // The creation service records the `related_to` relation and provisions
        // the sandbox as part of the owned sequence.
        match inbound::create_task_for_external_message(state, &config, &msg, related).await {
            Ok(outcome) => {
                let task_id = outcome.task.task_id;
                if rule.routing_mode == RoutingMode::Ongoing {
                    state.loops.set_ongoing_task(&rule.id, &task_id).await?;
                }
                // Seed the cursor as None so `fetch_context` retrieves the full
                // prior thread on the first delivery; `touch_cursor` advances it
                // afterward.
                let sub = upsert_binding(
                    state,
                    connection,
                    Some(rule),
                    &task_id,
                    provider_label,
                    subject_kind,
                    subject_key,
                    destination_config.clone(),
                    ORIGIN_ADAPTER_EVENT,
                    None,
                )
                .await?;
                new_task_binding_id = Some(sub.id.clone());
                bindings.push(sub);
                info!(task_id = %task_id, rule_id = %rule.id, provider = provider_label, "Created task from provider event");
            }
            // Permanent failure (e.g. a bad GitHub token). The service already
            // persisted the task as errored + published a `SandboxStatus::Error`
            // event. Skip the new-task binding and its delivery, but fall through
            // to existing-binding delivery and return `Ok` — returning a 5xx here
            // makes sources like Slack redeliver, and every redelivery inserts
            // another errored row.
            Err(TaskCreationError::Preflight(reason)) => {
                warn!(rule_id = %rule.id, provider = provider_label, reason = %reason, "Skipping task creation from provider event: preflight failed");
            }
            // Transient failure (Db / ShortId / RepoNotFound) — propagate so the
            // source retries, exactly as before this migration.
            Err(e) => return Err(AdapterError::TaskCreation(e)),
        }
    }

    fan_out(
        state,
        connection,
        rule,
        &msg,
        Fanout {
            provider_label,
            binding_provider: provider_label,
            subject_kind,
            subject_key,
            bindings,
            author_task_id: origin_task_id.as_deref(),
            new_task_binding_id: new_task_binding_id.as_deref(),
        },
    )
    .await
}

/// One event, the bindings on its subject, and what routing already knows about
/// who wrote it.
///
/// Assembling the set is the caller's job because it is the one part of routing
/// that is not shared: a chat adapter spawns a task for a subject nothing is
/// bound to, the GitHub PR adapter spawns one only when a pull request opens.
/// Deciding who among them receives the event is not — that is [`fan_out`].
pub struct Fanout<'a> {
    /// The adapter label the event is delivered and metered under: the value
    /// `task_messages.external_adapter` carries, and the key the adapter
    /// registry answers to for external-id aliases.
    pub provider_label: &'a str,
    /// The provider key the subject's delivery receipts are stored under — the
    /// same key its bindings are. Equal to `provider_label` for every adapter
    /// whose registry name is also its binding provider; GitHub PR binds under
    /// `github` and delivers as `github_pr`.
    pub binding_provider: &'a str,
    pub subject_kind: &'a str,
    pub subject_key: &'a str,
    /// The active bindings on the subject, walked in this order.
    pub bindings: Vec<TaskBinding>,
    /// The task provenance names as the author of the event — a footer, a claim
    /// the task recorded when it acted, or an event identifier already recorded
    /// against it. Never an identity: a task is passed over only where it can
    /// be *named* as the author of what arrived.
    pub author_task_id: Option<&'a str>,
    /// The binding whose task this very event created, if any. Its delivery
    /// does not wake a sandbox that task creation is already provisioning.
    pub new_task_binding_id: Option<&'a str>,
}

/// Deliver one event to the bindings on its subject that should receive it.
///
/// The single place the delivery decision is made, for every adapter. It
/// resolves the audience through [`resolve_recipients`], records why each
/// binding it passes over was passed over, checks the redelivery receipt, and
/// fails loudly when a subject that had bindings ends up with no recipient at
/// all.
///
/// Returns the last task delivered to — or accounted for, where a skip means
/// the task already holds the event.
pub async fn fan_out(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    msg: &InboundMessage,
    subject: Fanout<'_>,
) -> Result<String, AdapterError> {
    let Fanout {
        provider_label,
        binding_provider,
        subject_kind,
        subject_key,
        bindings,
        author_task_id,
        new_task_binding_id,
    } = subject;

    let parent_author_task_id = parent_author_task_id(state, provider_label, &msg.audience).await?;

    // The single place the delivery decision is made. Membership is by binding
    // id so a task bound twice to the same subject is judged once per binding,
    // exactly as the loop below walks it.
    let recipient_ids: std::collections::HashSet<String> = resolve_recipients(
        &msg.audience,
        &bindings,
        EventProvenance {
            author_task_id,
            parent_author_task_id: parent_author_task_id.as_deref(),
        },
    )
    .into_iter()
    .map(|binding| binding.id.clone())
    .collect();

    // How many bindings the subject carried when the walk began, and why each
    // one that got nothing got nothing. A subject with no bindings expects
    // nothing of this event; one whose every binding was passed over expected
    // something and no task got it.
    let active_bindings = bindings.len();
    let mut passed_over: Vec<(String, &'static str)> = Vec::new();
    let mut delivered_task_id = None;
    let receipts = crate::loops::firing::receipts::PgEventReceiptRepository(state.db.clone());
    for binding in bindings {
        // Two ways to be passed over, and only one of them means this task
        // already has the event.
        //
        // It wrote the event: either provenance names it, or it posted under
        // the binding's configured filter identity. The identity filter stays
        // out of the resolver — it is the one place routing may look at *who*
        // posted, and the resolver is deliberately never given an identity.
        // Either way the task holds the event, so the skip is accounted for.
        if author_task_id == Some(binding.task_id.as_str())
            || should_filter_self_echo(&binding, &msg.user_id)
        {
            debug!(
                task_id = %binding.task_id,
                subject_key,
                provider = provider_label,
                "Skipping binding: the task authored this event"
            );
            record_inbound(
                provider_label,
                "ignored",
                inbound_reason::SELF_ECHO,
                &rule.id,
                &connection.id,
            );
            delivered_task_id = Some(binding.task_id.clone());
            passed_over.push((binding.task_id, inbound_reason::SELF_ECHO));
            continue;
        }

        // The audience named someone else. This deliberately does *not* mark
        // the event as delivered: nothing holds it on this binding's account,
        // and `delivered_task_id` is the whole of the empty-result signal once
        // the GitHub PR router's own ledger is gone. Marking here would make a
        // review that reached no task report as a successful delivery, which is
        // precisely the fault the signal exists to catch.
        if !recipient_ids.contains(&binding.id) {
            debug!(
                task_id = %binding.task_id,
                subject_key,
                provider = provider_label,
                "Skipping binding: not in this event's audience"
            );
            record_inbound(
                provider_label,
                "ignored",
                inbound_reason::NOT_IN_AUDIENCE,
                &rule.id,
                &connection.id,
            );
            passed_over.push((binding.task_id, inbound_reason::NOT_IN_AUDIENCE));
            continue;
        }

        if let Some(external_event_id) = msg.external_event_id.as_deref() {
            let is_first_delivery = receipts
                .record_once(
                    binding_provider,
                    external_event_id,
                    subject_kind,
                    subject_key,
                    &binding.task_id,
                )
                .await?;
            if !is_first_delivery {
                debug!(
                    task_id = %binding.task_id,
                    external_event_id,
                    provider = provider_label,
                    "Skipping duplicate external event receipt"
                );
                record_inbound(
                    provider_label,
                    "ignored",
                    inbound_reason::DUPLICATE_RECEIPT,
                    &rule.id,
                    &connection.id,
                );
                delivered_task_id = Some(binding.task_id.clone());
                passed_over.push((binding.task_id, inbound_reason::DUPLICATE_RECEIPT));
                continue;
            }
        }

        let this_is_new = new_task_binding_id == Some(binding.id.as_str());
        match deliver_to_binding(state, rule, &binding, msg, provider_label, this_is_new).await {
            Ok(()) => {
                let reason = if this_is_new {
                    inbound_reason::NEW_TASK
                } else {
                    inbound_reason::EXISTING_TASK
                };
                record_inbound(provider_label, "success", reason, &rule.id, &connection.id);
            }
            Err(e) => {
                record_inbound(
                    provider_label,
                    "error",
                    inbound_reason::DELIVER_FAILED,
                    &rule.id,
                    &connection.id,
                );
                return Err(e);
            }
        }
        delivered_task_id = Some(binding.task_id);
    }

    if let Some(task_id) = delivered_task_id {
        return Ok(task_id);
    }

    // Nothing was bound to the subject, so nothing was expected of this event.
    // The word stays what it has always been for this case — the caller decides
    // whether a subject with nothing on it is a failure of its own.
    if passed_over.is_empty() {
        return Err(AdapterError::ConfigNotFound(rule.id.clone()));
    }

    // Something *was* expected and no task got it. This used to be a `debug!`
    // and a bare `Ok(())` on the GitHub PR path, which is how a real review
    // reached nobody and looked exactly like a quiet day.
    let reasons = passed_over
        .iter()
        .map(|(task_id, reason)| format!("{task_id}={reason}"))
        .collect::<Vec<_>>()
        .join(", ");
    warn!(
        subject_key,
        external_event_id = msg.external_event_id.as_deref().unwrap_or_default(),
        provider = provider_label,
        active_bindings,
        passed_over = %reasons,
        "Event reached no task: every binding on the subject was passed over"
    );
    record_inbound(
        provider_label,
        "error",
        inbound_reason::NO_RECIPIENTS,
        &rule.id,
        &connection.id,
    );
    // Permanent by classification (`adapters/delivery`): the same event walking the
    // same bindings is passed over the same way, so retrying only burns the
    // budget. The record lands in `failed` carrying this message, which is the
    // point — the event is not lost quietly.
    Err(AdapterError::NotFound(format!(
        "no task binding on {subject_key} could receive this {provider_label} event \
         ({active_bindings} passed over: {reasons})"
    )))
}

/// The task that authored the parent event an [`Audience::ParentAuthor`] names,
/// or `None` for every other audience and for a parent no task authored.
///
/// This lookup sits above [`resolve_recipients`] because the resolver performs
/// no input or output of its own — see [`crate::bindings::recipients`]. An
/// audience that names no parent asks nothing of the database. It is `pub`
/// because an adapter that still walks its own delivery loop resolves the same
/// parent for the same resolver; the rule is written once here rather than once
/// per adapter.
pub async fn parent_author_task_id(
    state: &AppState,
    provider_label: &str,
    audience: &Audience,
) -> Result<Option<String>, AdapterError> {
    let Audience::ParentAuthor {
        parent_external_event_id: Some(parent_id),
    } = audience
    else {
        return Ok(None);
    };
    let candidates = external_id_aliases(state, provider_label, parent_id);
    Ok(state
        .messages
        .find_task_id_by_any_external_event_id(&candidates)
        .await?)
}

pub fn external_id_aliases(
    state: &AppState,
    provider_label: &str,
    external_id: &str,
) -> Vec<String> {
    state
        .adapters
        .as_ref()
        .and_then(|providers| providers.inbound_presentation_by_name(provider_label))
        .map_or_else(
            || vec![external_id.to_string()],
            |presentation| presentation.external_id_aliases(external_id),
        )
}

async fn should_create_binding_task(
    state: &AppState,
    rule: &SubscriptionMatch,
    bindings: &[TaskBinding],
) -> Result<bool, AdapterError> {
    if rule.routing_mode != RoutingMode::Ongoing {
        return Ok(bindings.is_empty());
    }

    if !bindings.is_empty() {
        return Ok(false);
    }
    Ok(open_ongoing_task_id(state, rule).await.is_none())
}

async fn open_ongoing_task_id(state: &AppState, rule: &SubscriptionMatch) -> Option<String> {
    let task_id = rule.ongoing_task_id.as_deref()?;
    #[allow(clippy::disallowed_methods)] // reason: event routing is a background path with no caller identity
    let sys = ScopeContext::system();
    match state.tasks.find_by_id(task_id, &sys).await {
        Ok(Some(task)) if task.status == TaskStatus::Open => Some(task.task_id),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_binding(
    state: &AppState,
    connection: &AdapterConnection,
    rule: Option<&SubscriptionMatch>,
    task_id: &str,
    provider: &str,
    subject_kind: &str,
    subject_key: &str,
    destination_config: serde_json::Value,
    origin: &str,
    last_external_event_id: Option<String>,
) -> Result<TaskBinding, AdapterError> {
    let now = Utc::now();
    let filter_uid = destination_config
        .get("filter_user_id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let binding = TaskBinding {
        id: format!("ts_{}", uuid::Uuid::new_v4()),
        task_id: task_id.to_string(),
        provider: provider.to_string(),
        subject_kind: subject_kind.to_string(),
        subject_key: subject_key.to_string(),
        connection_id: connection.id.clone(),
        trigger_id: rule.map(|r| r.id.clone()),
        destination_config,
        metadata: serde_json::json!({ "filter_user_id": filter_uid }),
        last_external_event_id,
        origin: origin.to_string(),
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
    Ok(PgTaskBindingRepository(state.db.clone())
        .upsert(&binding)
        .await?)
}

async fn deliver_to_binding(
    state: &AppState,
    rule: &SubscriptionMatch,
    binding: &TaskBinding,
    msg: &InboundMessage,
    provider_label: &str,
    is_new_task: bool,
) -> Result<(), AdapterError> {
    let config = trigger_context(rule, provider_label);
    let thread = binding_context(binding);

    let thread_context = if msg.thread_id.is_some() {
        if let Some(ref providers) = state.adapters {
            if let Some(presentation) = providers.inbound_presentation_by_name(provider_label) {
                let mut exclude_ids = state
                    .messages
                    .get_external_event_ids(&binding.task_id)
                    .await
                    .unwrap_or_default();
                if let Some(ref ext_id) = msg.external_event_id {
                    exclude_ids.push(ext_id.clone());
                }
                presentation
                    .fetch_context(&thread, &config, 20, &exclude_ids)
                    .await
                    .unwrap_or_else(|e| {
                        warn!(provider = provider_label, error = %e, "Failed to fetch thread context");
                        None
                    })
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut message_text = msg.text.clone();

    if let Some(prefix) = state
        .adapters
        .as_ref()
        .and_then(|providers| providers.inbound_presentation_by_name(provider_label))
        .and_then(|presentation| presentation.inbound_message_prefix(msg))
    {
        message_text = format!("{prefix}\n\n{message_text}");
    }

    if msg.thread_id.is_some() {
        if let Some(providers) = state.adapters.as_ref() {
            if let Some(presentation) = providers.inbound_presentation_by_name(provider_label) {
                if let Some(hint) = presentation.thread_reply_hint(msg) {
                    message_text = format!("{hint}\n\n{message_text}");
                }
            }
        }
    }

    if let Some(ctx) = thread_context {
        if binding.last_external_event_id.is_some() {
            message_text = format!("{ctx}\n\n---\n\n{message_text}");
        } else {
            message_text = format!(
                "Thread context (previous messages):\n\n{ctx}\n\n---\n\nNew message:\n\n{message_text}"
            );
        }
    }

    crate::message::send::send_message_flow(
        state,
        &binding.task_id,
        &message_text,
        None,
        None,
        &msg.file_ids,
        &[],
        true,
        msg.external_event_id.as_deref(),
        Some(provider_label),
        Some(&binding.subject_key),
        // The source's own name where the trigger carries one, else the matcher
        // that identified it — a repository full name, a channel id. A label a
        // reader recognises beats no label at all.
        Some(
            config
                .source_name
                .as_deref()
                .unwrap_or(config.source_id.as_str()),
        ),
        None,
        false,
        Some(msg.user_id.as_str()),
    )
    .await
    .map_err(AdapterError::MessageDelivery)?;

    let repo = PgTaskBindingRepository(state.db.clone());
    if let Some(ref ext_id) = msg.external_event_id {
        repo.touch_cursor(&binding.id, ext_id).await?;
    } else {
        repo.touch_last_event(&binding.id).await?;
    }

    if !is_new_task {
        if let Err(e) =
            crate::sandbox::notify::ensure_sandbox_and_notify(state, &binding.task_id).await
        {
            warn!(task_id = %binding.task_id, error = %e, "Failed to ensure sandbox for external message");
        }
    }

    Ok(())
}

fn should_filter_self_echo(binding: &TaskBinding, user_id: &str) -> bool {
    binding
        .metadata
        .get("filter_user_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|filter| filter == user_id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_adapter_files_do_not_own_adapter_subject_or_external_id_builders() {
        let shared_files = [
            ("runtime.rs", include_str!("runtime.rs")),
            ("recipients.rs", include_str!("recipients.rs")),
            (
                "../loops/firing/listen.rs",
                include_str!("../loops/firing/listen.rs"),
            ),
            ("outbound.rs", include_str!("outbound.rs")),
            ("../adapters/mod.rs", include_str!("../adapters/mod.rs")),
            ("dao.rs", include_str!("dao.rs")),
        ];
        let forbidden = [
            concat!("slack_", "thread"),
            concat!("telegram_", "thread"),
            concat!("github", "_pr_event"),
            concat!("github", "_issue_comment"),
            concat!("github", "_review_comment"),
            concat!("telegram", "_msg"),
            concat!("slack", "_ts"),
        ];

        for (file, source) in shared_files {
            for pattern in forbidden {
                assert!(
                    !source.contains(pattern),
                    "{file} should not contain provider-specific builder/subject pattern {pattern}"
                );
            }
        }
    }
}

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;

    use axum::http::StatusCode;
    use axum::response::IntoResponse as _;

    use crate::adapters::error::AdapterError;
    use crate::adapters::test_support::{response_snapshot, source_chain};

    /// These assert *reachability* of the causal error, not the depth of the
    /// chain. Depth is a property of how the wrapped type declares its own
    /// variants — `TaskCreationError::Db` is `#[error(transparent)]` and so
    /// collapses into a single link, while `MessageError::Db` has its own
    /// `Display` and adds one. Neither type belongs to this module, so pinning
    /// a link count here would make these tests fail on an unrelated change to
    /// a `#[error(...)]` attribute elsewhere. What the ticket requires is that
    /// the cause survives `Error::source` while the HTTP body stays redacted.
    #[tokio::test]
    async fn task_creation_keeps_its_cause_and_redacts() {
        let err = AdapterError::TaskCreation(crate::task::creation::TaskCreationError::Db(
            sqlx::Error::PoolClosed,
        ));

        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<crate::task::creation::TaskCreationError>())
            .is_some());
        let chain = source_chain(&err);
        assert!(
            chain.iter().any(|link| link.contains("closed pool")),
            "sqlx cause must stay reachable: {chain:?}"
        );

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }

    #[tokio::test]
    async fn message_delivery_keeps_its_cause_and_redacts() {
        let err = AdapterError::MessageDelivery(crate::message::error::MessageError::Db(
            sqlx::Error::PoolClosed,
        ));

        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<crate::message::error::MessageError>())
            .is_some());
        let chain = source_chain(&err);
        assert!(
            chain.iter().any(|link| link.contains("closed pool")),
            "sqlx cause must stay reachable: {chain:?}"
        );

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }
}
