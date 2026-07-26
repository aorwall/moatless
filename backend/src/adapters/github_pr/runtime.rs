//! GitHub PR runtime on top of provider connections, task triggers, and task bindings.

use std::fmt::Write as _;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, InboundMessage, SubscriptionMatch, TaskBinding,
    ADAPTER_EVENT_BINDING_ORIGIN, CONNECTED_THREAD_BINDING_ORIGIN,
};
use crate::adapters::AdapterKind;
use crate::bindings::dao::PgTaskBindingRepository;
use crate::sandbox::lifecycle::ArchiveRetention;
use crate::state::AppState;
use crate::task::creation::TaskCreationError;
use crate::task::dao::TaskPatch;
use crate::task::models::{TaskStatus, Visibility};
use moatless_backend_core::scope::ScopeContext;

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

/// Always prepended to the `append_system_prompt` of tasks created from a
/// GitHub PR event, so loop/trigger prompts don't have to restate the origin.
const PR_ORIGIN_CONTEXT: &str = "This task was created from a GitHub Pull Request.";

/// Look up the connection and (optional) trigger for an inbound provider event.
///
/// The **connection** is required — without it we can't verify the webhook
/// signature or identify the tenant. The **trigger** is optional: when no
/// trigger matches the repo, events are still delivered to any existing PR
/// bindings (the binding a task created when it opened/connected to the PR is
/// itself the opt-in). A trigger is only required to *create* a brand-new task
/// from a PR — [`route_inbound_message`] gates creation on `Some(rule)`.
pub async fn resolve_provider_event(
    state: &AppState,
    org: &str,
    repo_full_name: &str,
) -> Result<Option<(AdapterConnection, Option<SubscriptionMatch>)>, AdapterError> {
    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_scope_value(ADAPTER, org)
        .await?;
    let Some(connection) = connection else {
        return Ok(None);
    };

    let rule = state
        .loops
        .find_subscription_by_source_matcher(ADAPTER, repo_full_name)
        .await?
        .filter(|rule| {
            let same_connection = rule.connection_id == connection.id;
            if !same_connection {
                debug!(
                    source_matcher = %repo_full_name,
                    "Task trigger belongs to a different connection — ignoring trigger; \
                     delivering to existing bindings only"
                );
            }
            same_connection
        });

    Ok(Some((connection, rule)))
}

/// Synthesize a delivery-only trigger route for a connection that has no
/// matching trigger. Used purely as a config bag for
/// [`super::adapter::GithubPrAdapter::parse_webhook`] so an inbound event on a
/// PR a task already connected to is still parsed into an `InboundMessage`.
///
/// It is NEVER persisted and NEVER drives task creation — the real
/// `Option<&SubscriptionMatch>` gates that in [`route_inbound_message`]. An
/// empty `event_filter` lets all supported event types through (including
/// `check_run` failures); `source_matcher` is the repo so message source
/// labels stay correct.
pub fn connection_only_rule(
    connection: &AdapterConnection,
    repo_full_name: &str,
) -> SubscriptionMatch {
    let now = Utc::now();
    SubscriptionMatch {
        id: format!("virtual:{}", connection.id),
        provider: ADAPTER.to_string(),
        connection_id: connection.id.clone(),
        source_matcher: repo_full_name.to_string(),
        source_name: None,
        routing_mode: crate::adapters::models::RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        repository_id: String::new(),
        prompt_template: None,
        task_name: None,
        tag_ids: Vec::new(),
        run_as_user_id: connection.owner_user_id.clone(),
        agent_type: String::new(),
        inactive_ttl_days: None,
        allow_direct_send: false,
        provider_config: None,
        ongoing_task_id: None,
        close_task_on_close: false,
        notify_on_success: false,
        active: true,
        created_by: connection.created_by.clone(),
        scope: connection.scope,
        owner_user_id: connection.owner_user_id.clone(),
        created_at: now,
        updated_at: now,
    }
}

/// Route an inbound GitHub PR message to one or more tasks via bindings.
pub async fn route_inbound_message(
    state: &AppState,
    connection: &AdapterConnection,
    rule: Option<&SubscriptionMatch>,
    msg: InboundMessage,
) -> Result<(), AdapterError> {
    if let Some(rule) = rule {
        if !rule.active {
            debug!(
                rule_id = %rule.id,
                source_matcher = %rule.source_matcher,
                "Ignoring GitHub PR message: task trigger inactive"
            );
            return Ok(());
        }
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
                rule,
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
    let originator_binding_summary: Option<(String, String)> =
        bindings.first().map(|b| (b.task_id.clone(), b.id.clone()));

    // CI notifications (check_run / check_suite / workflow_run) are synthetic
    // provider signals, never authored by the task agent — so the owner-identity
    // self-echo heuristic below must not apply to them. Without this exemption a
    // build-failure event whose webhook `sender` is the human who pushed the
    // commit resolves to that person's user id, which matches the `created_by`/
    // `owner_user_id` of *their own* PR task, and the notification is silently
    // dropped for exactly the task that most needs it. The cheap origin-task and
    // event-receipt dedup paths still guard against genuine echoes.
    let ci_self_echo_exempt = msg
        .external_event_id
        .as_deref()
        .is_some_and(is_ci_notification);

    // Review comments are thread-scoped for the reviewer (`ingress`): it should
    // only hear replies within threads it started, not every inline comment on
    // the PR. Resolve the task that authored the comment being replied to (via
    // the `github:rc:<in_reply_to_id>` we recorded when it posted). If that
    // resolves to a reviewer binding's own task, the reply belongs to its
    // thread and is delivered; otherwise it is skipped for that binding. The PR
    // author (`connected_thread`) is unaffected and still gets every review
    // comment to address. A brand-new top-level comment (no `in_reply_to_id`)
    // resolves to `None`, so it reaches no reviewer.
    let review_comment_reply_owner: Option<String> = if msg
        .external_event_id
        .as_deref()
        .is_some_and(is_review_comment)
    {
        match msg
            .thread_properties
            .get("in_reply_to_id")
            .and_then(serde_json::Value::as_u64)
        {
            Some(reply_id) => {
                let candidates = crate::bindings::runtime::external_id_aliases(
                    state,
                    ADAPTER_LABEL,
                    &super::external_id::review_comment(reply_id),
                );
                state
                    .messages
                    .find_task_id_by_any_external_event_id(&candidates)
                    .await?
            }
            None => None,
        }
    } else {
        None
    };

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

        // (1b) Reviewer thread-scoping: an `ingress` binding only receives a
        // review comment that is a reply within a thread it started.
        if binding.origin == ORIGIN_ADAPTER_EVENT
            && msg
                .external_event_id
                .as_deref()
                .is_some_and(is_review_comment)
            && review_comment_reply_owner.as_deref() != Some(binding.task_id.as_str())
        {
            debug!(
                task_id = %binding.task_id,
                subject_key = %subject_key,
                "Skipping review comment for reviewer: not a reply to its own thread"
            );
            continue;
        }

        // (2) Owner-identity check: catches out-of-band agent comments
        // (e.g. `gh pr comment` bypassing `moat reply`, which leaves no marker
        // and no recorded external_event_id). Per-binding DB fetch; typical
        // PRs have 1-2 bindings. Skipped for CI notifications (see above).
        #[allow(clippy::disallowed_methods)] // reason: GitHub PR webhook routing has no caller identity
        let sys = ScopeContext::system();
        match state.tasks.find_by_id(&binding.task_id, &sys).await {
            Ok(Some(task))
                if !ci_self_echo_exempt
                    && is_authored_by_task_owner(
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

    // Spawning a review task needs a real trigger (repo, agent, prompt config
    // a bare connection doesn't carry). Without one we still deliver to any
    // existing bindings above — we just don't create a new task.
    if is_pr_open_event && !has_provider_event_task {
        if let Some(rule) = rule {
            // The eager `gh pr create` hook (see `github_pr::shell_reconcile`)
            // upserts a connected_thread binding for the originating task at the
            // moment the agent created the PR — so by the time pr_opened fires,
            // the earliest binding is the originating task. Link the new review
            // task to it as a `related_to` peer (it shares the PR subject_key,
            // it isn't a child of the originator).
            let related = originator_binding_summary
                .as_ref()
                .map(
                    |(task_id, binding_id)| crate::task::relations::RelatedTask {
                        related_task_id: task_id.clone(),
                        context: serde_json::json!({
                            "via": "github_pr_webhook",
                            "adapter": ADAPTER,
                            "subject_kind": SUBJECT_KIND,
                            "subject_key": subject_key,
                            "origin_binding_id": binding_id,
                        }),
                    },
                );
            // The service owns insert + preflight + provision.
            match create_provider_event_task(state, rule, &msg).await {
                Ok(task_id) => {
                    if let Some(ref r) = related {
                        crate::task::relations::record_related_to(
                            state.task_relations.as_ref(),
                            &task_id,
                            r,
                        )
                        .await;
                    }

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
                // Preflight is permanent: a bad token can't be fixed by a GitHub
                // redelivery, and retrying would just accumulate errored rows.
                // The service already marked the task errored with a sandbox
                // error event, so degrade — log and skip the new-task
                // binding/delivery, then fall through to existing-binding
                // delivery below.
                Err(TaskCreationError::Preflight(reason)) => {
                    warn!(
                        rule_id = %rule.id,
                        subject_key = %subject_key,
                        error = %reason,
                        "GitHub PR review task failed preflight; skipping new-task delivery"
                    );
                }
                // Transient failures (DB blip, short-id exhaustion) must surface
                // so the webhook 5xx's and GitHub redelivers, rather than
                // silently dropping the PR event.
                Err(other) => {
                    return Err(AdapterError::Internal(format!(
                        "Failed to create GitHub PR review task: {other}"
                    )));
                }
            }
        } else {
            debug!(
                subject_key = %subject_key,
                "PR opened but no trigger for this repo — not creating a review task; \
                 delivering to existing bindings only"
            );
        }
    }

    // Health signals (CI check / suite results routed as messages) belong to
    // the PR author. Restrict delivery to `connected_thread` bindings, falling
    // back to all targets when no author task is bound (see `author_only_target`).
    if msg
        .external_event_id
        .as_deref()
        .is_some_and(is_author_only_signal)
    {
        let has_author = delivery_targets
            .iter()
            .any(|(b, _)| b.origin == ORIGIN_CONNECTED_THREAD);
        delivery_targets.retain(|(b, _)| author_only_target(&b.origin, has_author));
    }

    let receipts = crate::loops::firing::receipts::PgEventReceiptRepository(state.db.clone());
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

/// Persist a PR's mergeable state onto its bindings and, on an open PR
/// transitioning *into* conflict, deliver a one-shot conflict notification.
///
/// Best-effort: called from a spawned task, all failures are logged, never
/// propagated. The mergeable state is written to **every** binding (so a
/// resolved conflict also updates the column back to `clean`); the notification
/// only fires for **active** bindings whose previously-recorded state was not
/// already `dirty` — that transition check is the dedup, so a PR that stays
/// conflicted across many events is announced once, not on every event.
///
/// Delivery uses [`deliver_to_binding`] directly rather than
/// [`route_inbound_message`] precisely so the synthetic alert bypasses the
/// owner self-echo filter (the author resolves to the task owner, but this is a
/// provider notification, not the task's own comment echoed back).
pub async fn record_merge_state_and_notify(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    subject_key: &str,
    mergeable: super::signals::MergeableState,
    pr_is_open: bool,
    conflict_msg: &InboundMessage,
) {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
    let bindings = match binding_repo
        .find_by_subject(ADAPTER, SUBJECT_KIND, subject_key)
        .await
    {
        Ok(bindings) => bindings,
        Err(e) => {
            warn!(subject_key, error = %e, "Failed to load bindings for mergeable-state update");
            return;
        }
    };

    // Conflict notifications are an author-only health signal. Deliver only to
    // the PR author's `connected_thread` binding (falling back to all active
    // bindings when no author task is bound). Mergeable state is still persisted
    // to *every* binding below regardless — only the notification is scoped.
    let has_author = bindings
        .iter()
        .any(|b| b.active && b.origin == CONNECTED_THREAD_BINDING_ORIGIN);

    for binding in bindings {
        let was_dirty = binding.external_mergeable_state.as_deref() == Some("dirty");

        if let Err(e) = binding_repo
            .update_mergeable_state(&binding.id, Some(mergeable.as_str()))
            .await
        {
            warn!(binding_id = %binding.id, error = %e, "Failed to persist mergeable state");
        }

        // Notify only on a fresh conflict on an open PR, for a live binding that
        // should receive the author-only signal.
        if pr_is_open
            && binding.active
            && mergeable.is_conflicted()
            && !was_dirty
            && author_only_target(&binding.origin, has_author)
        {
            info!(
                task_id = %binding.task_id,
                subject_key,
                "Delivering merge-conflict notification"
            );
            if let Err(e) = deliver_to_binding(
                state,
                Some(rule),
                connection,
                &binding,
                conflict_msg,
                false,
                true,
            )
            .await
            {
                warn!(
                    task_id = %binding.task_id,
                    error = %e,
                    "Failed to deliver merge-conflict notification"
                );
            }
        }
    }
}

/// Deliver a post-merge CI notification (base-branch `workflow_run` result) to
/// every task still connected to the merged PR.
///
/// Delivery goes through [`deliver_to_binding`], which does not apply the owner
/// self-echo filter — the notice is a synthetic provider signal, not the task's
/// own echo. Only **active** bindings are notified: a task auto-closed on merge
/// is terminal (its sandbox is torn down and there is no reopen path), so
/// reviving it to deliver an FYI is unsafe and intentionally skipped. The
/// PR-author's connected task typically survives the merge and is the intended
/// recipient. Deduped per run id downstream via the message `external_event_id`.
pub async fn deliver_post_merge_ci(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
    subject_key: &str,
    notice: &InboundMessage,
) {
    let binding_repo = PgTaskBindingRepository(state.db.clone());
    let bindings = match binding_repo
        .find_by_subject(ADAPTER, SUBJECT_KIND, subject_key)
        .await
    {
        Ok(bindings) => bindings,
        Err(e) => {
            warn!(subject_key, error = %e, "Failed to load bindings for post-merge CI notice");
            return;
        }
    };

    // Post-merge CI is an author-only health signal. Deliver only to the PR
    // author's `connected_thread` binding (falling back to all active bindings
    // when no author task is bound — e.g. a human-opened, human-merged PR).
    let has_author = bindings
        .iter()
        .any(|b| b.active && b.origin == CONNECTED_THREAD_BINDING_ORIGIN);

    for binding in bindings {
        if !binding.active {
            debug!(
                task_id = %binding.task_id,
                subject_key,
                "Skipping post-merge CI notice: binding inactive (task closed on merge)"
            );
            continue;
        }
        if !author_only_target(&binding.origin, has_author) {
            debug!(
                task_id = %binding.task_id,
                subject_key,
                "Skipping post-merge CI notice: reviewer binding, author task present"
            );
            continue;
        }
        info!(
            task_id = %binding.task_id,
            subject_key,
            "Delivering post-merge CI notification"
        );
        if let Err(e) =
            deliver_to_binding(state, Some(rule), connection, &binding, notice, false, true).await
        {
            warn!(
                task_id = %binding.task_id,
                error = %e,
                "Failed to deliver post-merge CI notification"
            );
        }
    }
}

pub async fn close_tasks_for_pr(
    state: &AppState,
    connection: &AdapterConnection,
    rule: &SubscriptionMatch,
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

        #[allow(clippy::disallowed_methods)] // reason: GitHub PR close handler runs as background system, no caller identity
        let sys = ScopeContext::system();
        match state.tasks.find_by_id(task_id, &sys).await {
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

        // Append the close notice to the task chat while the task is still
        // `Open`. The `send.rs` closed-task guard drops inbound external
        // messages on `Closed` tasks, so the notice MUST be delivered before
        // the status flip below — otherwise it would silently vanish.
        // `resume = false` only persists the message; it never provisions or
        // wakes the agent on a sandbox we're about to tear down.
        let close_msg = build_close_inbound_message(state, owner, repo, rule, info).await;
        if let Err(e) = deliver_to_binding(
            state,
            Some(rule),
            connection,
            &binding,
            &close_msg,
            false,
            false,
        )
        .await
        {
            warn!(
                task_id = %task_id,
                pr_number,
                error = %e,
                "Failed to append PR-close message to task — proceeding with close"
            );
        }

        // Commit `Closed` BEFORE the slow teardown. The original code wrote the
        // status last (after `cleanup`), leaving a window where a webhook racing
        // the close still saw `open` and could re-provision the task. Persisting
        // `Closed` first means any concurrent `auto_provision` /
        // `get_pending_messages` (gated by Fix B) sees `Closed` and bails.
        if let Err(e) = state
            .tasks
            .update_task(
                task_id,
                &TaskPatch {
                    status: Some(TaskStatus::Closed),
                    ..Default::default()
                },
                &sys,
            )
            .await
        {
            warn!(task_id = %task_id, error = %e, "Failed to close task on PR close");
            continue;
        }

        // Remove the sandbox (best-effort, `Closed` already committed so a
        // teardown failure never leaves a non-terminal task). Lifecycle issues
        // the graceful agent stop and retains S3 archives per config.
        let retention = ArchiveRetention::on_close(state.config.sandbox.retain_storage_on_close);
        if let Err(e) = state.sandbox.remove(task_id, retention).await {
            warn!(task_id = %task_id, error = %e, "Failed to remove sandbox on PR close");
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
    rule: &SubscriptionMatch,
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

/// Upsert a `github_pr` binding through the shared spine. Parses the PR
/// coordinates out of `thread_properties` into the `{owner, repo, pr_number}`
/// destination config, then delegates the row build + persistence to
/// [`crate::bindings::runtime::upsert_binding`]. PR bindings never seed a
/// cursor, so `last_external_event_id` is always `None`.
pub async fn upsert_binding(
    state: &AppState,
    connection: &AdapterConnection,
    rule: Option<&SubscriptionMatch>,
    task_id: &str,
    subject_key: &str,
    thread_properties: &serde_json::Value,
    origin: &str,
) -> Result<TaskBinding, AdapterError> {
    let (owner, repo, pr_number) = parse_pr_destination_config(thread_properties)?;
    let destination_config = serde_json::json!({
        "owner": owner,
        "repo": repo,
        "pr_number": pr_number,
    });

    crate::bindings::runtime::upsert_binding(
        state,
        connection,
        rule,
        task_id,
        ADAPTER,
        SUBJECT_KIND,
        subject_key,
        destination_config,
        origin,
        None,
    )
    .await
}

/// Translate a GitHub PR provider event into a [`NewTaskRequest`] and hand it to
/// the shared [`TaskCreationService`]. The service owns short-id generation,
/// preflight, insert, tags, provision, and the task event; this function only
/// maps the trigger route + inbound message onto the normalised request.
///
/// The binding upsert and first-message delivery stay with the caller
/// ([`route_inbound_message`]), so no first message is passed here.
///
/// Returns the raw [`TaskCreationError`] so the caller can distinguish a
/// permanent failure (`Preflight` — a bad token that a webhook redelivery
/// can't fix) from a transient one (a DB blip, short-id exhaustion) that
/// should surface as a 5xx so GitHub retries.
async fn create_provider_event_task(
    state: &AppState,
    rule: &SubscriptionMatch,
    msg: &InboundMessage,
) -> Result<String, TaskCreationError> {
    // Compose the shared spine's single task-creation entry point. The only
    // GitHub-specific inputs are the `PR_ORIGIN_CONTEXT` prelude, public
    // visibility (so PR collaborators can open the task), and the
    // `source_matcher` event metadata; `related_to` is recorded by the caller
    // after creation succeeds, and the first message is delivered through the
    // binding.
    let config = crate::bindings::runtime::trigger_context(rule, ADAPTER_LABEL);
    let outcome = crate::loops::firing::inbound::create_task_from_loop(
        state,
        &config,
        crate::loops::firing::inbound::CreateTaskParams {
            name: rule.task_name.clone(),
            description: crate::bindings::outbound::truncate_text(&msg.text, 200),
            visibility: Visibility::Public,
            origin_prelude: Some(PR_ORIGIN_CONTEXT),
            related_to: None,
            event_metadata: serde_json::json!({
                "adapter": ADAPTER_LABEL,
                "source_matcher": rule.source_matcher,
            }),
        },
    )
    .await?;

    let task_id = outcome.task.task_id;
    info!(task_id = %task_id, rule_id = %rule.id, "Created task from GitHub PR provider event");
    Ok(task_id)
}

async fn deliver_to_binding(
    state: &AppState,
    rule: Option<&SubscriptionMatch>,
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

    // Source label for the message row. Falls back to the repo portion of the
    // subject key ("owner/repo#N" -> "owner/repo") when there is no trigger.
    let source_matcher = rule.map_or_else(
        || {
            binding
                .subject_key
                .split_once('#')
                .map_or_else(|| binding.subject_key.clone(), |(repo, _)| repo.to_string())
        },
        |r| r.source_matcher.clone(),
    );

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
        Some(source_matcher.as_str()),
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

/// Returns `true` for CI-status notifications — `check_run` (`github:cr:`),
/// `check_suite` (`github:cs:`), and post-merge `workflow_run` (`github:wfrun:`).
/// These are synthetic provider signals, never authored by a task agent, so the
/// owner-identity self-echo filter must not suppress them.
pub(crate) fn is_ci_notification(external_event_id: &str) -> bool {
    external_event_id.starts_with("github:cr:")
        || external_event_id.starts_with("github:cs:")
        || external_event_id.starts_with("github:wfrun:")
}

/// Returns `true` for signals about the PR's *health* rather than its content —
/// CI results (`github:cr:` / `github:cs:` / `github:wfrun:`) and merge
/// conflicts (`github:conflict:`). Whether a PR builds, merges cleanly, and
/// survives the base branch is the PR **author's** concern, not the reviewer's,
/// so these are delivered only to the author's `connected_thread` binding (see
/// [`author_only_target`]).
pub(crate) fn is_author_only_signal(external_event_id: &str) -> bool {
    is_ci_notification(external_event_id) || external_event_id.starts_with("github:conflict:")
}

/// Whether a binding with `origin` should receive an author-only health signal,
/// given whether any author (`connected_thread`) binding exists for the PR.
///
/// The PR author (`connected_thread`) always receives it. The reviewer
/// (`ingress`) receives it only as a fallback when there is no author task
/// bound — e.g. a human-opened PR whose only bound task is the reviewer — so a
/// conflict / CI failure on such a PR still reaches *some* task rather than
/// vanishing.
pub(crate) fn author_only_target(origin: &str, has_author_binding: bool) -> bool {
    if has_author_binding {
        origin == CONNECTED_THREAD_BINDING_ORIGIN
    } else {
        true
    }
}

/// Returns `true` for a GitHub PR review-comment signal (`github:rc:`). Review
/// comments are thread-scoped for the reviewer (`ingress`): it only hears
/// replies within threads it started (see the delivery loop in
/// [`route_inbound_message`]).
pub(crate) fn is_review_comment(external_event_id: &str) -> bool {
    external_event_id.starts_with("github:rc:")
}

#[cfg(test)]
mod tests {
    mod connection_only_rule {
        use super::super::connection_only_rule;
        use crate::adapters::models::AdapterConnection;
        use chrono::Utc;
        use moatless_backend_core::scope::Scope;

        fn connection() -> AdapterConnection {
            let now = Utc::now();
            AdapterConnection {
                id: "pc_org".to_string(),
                provider: "github".to_string(),
                connection_kind: "org_webhook".to_string(),
                scope_value: "acme".to_string(),
                credential_ref: None,
                webhook_secret: Some("s3cr3t".to_string()),
                transport_config: serde_json::json!({}),
                created_by: "u_owner".to_string(),
                scope: Scope::User,
                owner_user_id: Some("u_owner".to_string()),
                created_at: now,
                updated_at: now,
            }
        }

        #[test]
        fn is_delivery_only_and_cannot_create_tasks() {
            let rule = connection_only_rule(&connection(), "acme/widget");

            // Active so delivery isn't suppressed, but carries no repo/agent
            // config — so `route_inbound_message` can never spawn a task from it.
            assert!(rule.active);
            assert!(rule.repository_id.is_empty());
            assert!(rule.agent_type.is_empty());
            assert!(rule.prompt_template.is_none());

            // Empty event_filter = every supported event type passes (incl. check_run).
            assert_eq!(rule.event_filter, serde_json::json!([]));

            // Source label + connection linkage stay correct for message rows.
            assert_eq!(rule.source_matcher, "acme/widget");
            assert_eq!(rule.connection_id, "pc_org");
            assert_eq!(rule.run_as_user_id.as_deref(), Some("u_owner"));
        }
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

    mod ci_notification {
        use super::super::is_ci_notification;
        use crate::adapters::github_pr::external_id;

        #[test]
        fn check_run_and_suite_and_workflow_run_are_ci() {
            assert!(is_ci_notification(&external_id::check_run(123)));
            assert!(is_ci_notification(&external_id::check_suite(456)));
            assert!(is_ci_notification("github:wfrun:789"));
        }

        #[test]
        fn comments_reviews_and_conflicts_are_not_ci() {
            assert!(!is_ci_notification(&external_id::issue_comment(1)));
            assert!(!is_ci_notification(&external_id::review_comment(2)));
            assert!(!is_ci_notification(&external_id::review(3)));
            assert!(!is_ci_notification(&external_id::pr_opened(4)));
            assert!(!is_ci_notification(&external_id::merge_conflict(5, "abc")));
            assert!(!is_ci_notification(""));
        }
    }

    mod origin_routing {
        use super::super::{author_only_target, is_author_only_signal, is_review_comment};
        use crate::adapters::github_pr::external_id;
        use crate::adapters::models::{
            ADAPTER_EVENT_BINDING_ORIGIN, CONNECTED_THREAD_BINDING_ORIGIN,
        };

        #[test]
        fn ci_and_conflict_signals_are_author_only() {
            assert!(is_author_only_signal(&external_id::check_run(1)));
            assert!(is_author_only_signal(&external_id::check_suite(2)));
            assert!(is_author_only_signal(&external_id::workflow_run(3)));
            assert!(is_author_only_signal(&external_id::merge_conflict(
                4, "sha"
            )));
        }

        #[test]
        fn content_signals_are_not_author_only() {
            assert!(!is_author_only_signal(&external_id::issue_comment(1)));
            assert!(!is_author_only_signal(&external_id::review_comment(2)));
            assert!(!is_author_only_signal(&external_id::review(3)));
            assert!(!is_author_only_signal(&external_id::pr_opened(4)));
            assert!(!is_author_only_signal(""));
        }

        #[test]
        fn author_binding_always_receives_author_only_signal() {
            assert!(author_only_target(CONNECTED_THREAD_BINDING_ORIGIN, true));
            assert!(author_only_target(CONNECTED_THREAD_BINDING_ORIGIN, false));
        }

        #[test]
        fn reviewer_binding_receives_only_as_fallback() {
            // Author task bound → reviewer is excluded.
            assert!(!author_only_target(ADAPTER_EVENT_BINDING_ORIGIN, true));
            // No author task bound → reviewer is the fallback recipient.
            assert!(author_only_target(ADAPTER_EVENT_BINDING_ORIGIN, false));
        }

        #[test]
        fn review_comment_ids_are_recognized() {
            assert!(is_review_comment(&external_id::review_comment(7)));
            assert!(!is_review_comment(&external_id::issue_comment(7)));
            assert!(!is_review_comment(&external_id::review(7)));
            assert!(!is_review_comment(""));
        }
    }
}
