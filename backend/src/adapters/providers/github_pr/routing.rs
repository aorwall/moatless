//! GitHub PR routing on top of provider connections, task loops, and task bindings.

use std::fmt::Write as _;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{
    AdapterConnection, Audience, InboundMessage, RoutingMode, SubscriptionMatch, TaskBinding,
    ADAPTER_EVENT_BINDING_ORIGIN, CONNECTED_THREAD_BINDING_ORIGIN,
};
use crate::adapters::AdapterKind;
use crate::bindings::dao::PgTaskBindingDao;
use crate::bindings::metrics::{inbound_reason, record_inbound};
use crate::bindings::recently_sent::RecentlySent;
use crate::sandbox::lifecycle::ArchiveRetention;
use crate::state::AppState;
use crate::task::creation::TaskCreationError;
use crate::task::dao::TaskPatch;
use crate::task::models::{TaskStatus, Visibility};
use moatless_backend_core::scope::ScopeContext;

use super::subject::GithubPrSubject;
use super::webhook::PrCloseInfo;

/// Adapter value used in adapter connections, trigger sources, and bindings.
const ADAPTER: &str = "github";
/// Subject kind for GitHub PRs (`task_bindings.subject_kind`).
const SUBJECT_KIND: &str = "github_pr";
/// Label written to `task_messages.external_adapter`. Matches the
/// `AdapterKind::GithubPr.as_str()` value so existing UI treats new rows
/// the same as the legacy ones.
const ADAPTER_LABEL: &str = "github_pr";

const ORIGIN_ADAPTER_EVENT: &str = ADAPTER_EVENT_BINDING_ORIGIN;
const ORIGIN_CONNECTED_THREAD: &str = CONNECTED_THREAD_BINDING_ORIGIN;

/// Always prepended to the `append_system_prompt` of tasks created from a
/// GitHub PR event, so loop/trigger prompts don't have to restate the origin.
const PR_ORIGIN_CONTEXT: &str = "This task was created from a GitHub Pull Request.";

/// Build the config-only Subscription used when a connection has no matching
/// trigger.
///
/// This value is never persisted or used to authorize Task creation. Routing
/// owns it because the fallback exists only to keep normalization, metrics, and
/// message labelling consistent for events delivered to existing PR bindings.
pub(super) fn delivery_only_rule(
    connection: &AdapterConnection,
    repo_full_name: &str,
) -> SubscriptionMatch {
    let now = Utc::now();
    SubscriptionMatch {
        id: format!("virtual:{}", connection.id),
        adapter_kind: super::delivery::ADAPTER.to_string(),
        connection_id: connection.id.clone(),
        source_matcher: repo_full_name.to_string(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        repository_id: String::new(),
        prompt_template: None,
        task_name: None,
        tag_ids: Vec::new(),
        run_as_user_id: connection.owner_user_id.clone(),
        agent_type: String::new(),
        inactive_ttl_days: None,
        allow_direct_send: false,
        adapter_config: None,
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

/// Route an inbound GitHub PR message to the tasks bound to its pull request.
///
/// What is GitHub's about routing is assembling the bindings: attaching one for
/// a task the event names as its author, and spawning the review task a freshly
/// opened pull request calls for. Which of them receive the event is not — that
/// is [`crate::bindings::runtime::fan_out`], deciding from the audience
/// `adapter.rs` classified the event into. This function used to answer both,
/// through a ladder of guards whose last rung compared the sender's identity to
/// the bound task's owner; a real review reached no task because of what two of
/// those rungs added up to. Identity plays no part here now.
pub(super) async fn route_event(
    state: &AppState,
    connection: &AdapterConnection,
    rule: Option<&SubscriptionMatch>,
    msg: InboundMessage,
) -> Result<(), AdapterError> {
    if let Some(rule) = rule {
        if !rule.active {
            debug!(
                subscription_id = %rule.id,
                source_matcher = %rule.source_matcher,
                "Ignoring GitHub PR message: task trigger inactive"
            );
            record_inbound(
                ADAPTER_LABEL,
                "ignored",
                inbound_reason::INACTIVE_RULE,
                &rule.id,
                &connection.id,
            );
            return Ok(());
        }
    }

    let subject_key = GithubPrSubject::from_thread_properties(&msg.thread_properties)?.key();
    let binding_repo = PgTaskBindingDao(state.db.clone());

    // Provenance: who wrote this event, where it can be named. The
    // `Moatless task:` footer link `<frontend_url>/tasks/<id>` the adapter
    // parsed off the body is authoritative and costs no round-trip; failing
    // that, an event identifier already recorded against a task names it. A
    // third signal — the commit a task claimed when it pushed — needs the
    // bindings in hand and is resolved below. Nothing here is an identity.
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
    // relation when a brand-new task is spawned from this PR event.
    let originator_binding_summary: Option<(String, String)> =
        bindings.first().map(|b| (b.task_id.clone(), b.id.clone()));

    // Only `pull_request` `opened` / `reopened` events spawn a review task.
    // Comments, reviews, check_runs etc. only deliver to existing bindings.
    // Origin is irrelevant here: an agent-created PR still gets a review task,
    // and a task never receives back an event it is named as the author of.
    let is_pr_open_event = msg.external_event_id.as_deref().is_some_and(|id| {
        id.starts_with("github:pr:opened:") || id.starts_with("github:pr:reopened:")
    });

    // The binding of a task this very event created, so the shared router can
    // tell it apart from one that was already bound.
    let mut new_task_binding_id: Option<String> = None;

    // Spawning a review task needs a real trigger (repo, agent, prompt config
    // a bare connection doesn't carry). Without one we still deliver to any
    // existing bindings above — we just don't create a new task.
    if is_pr_open_event {
        let has_provider_event_task = binding_repo
            .find_provider_event_created_for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
            .await?
            .is_some();
        if let (false, Some(rule)) = (has_provider_event_task, rule) {
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
                        &msg.thread_properties,
                        ORIGIN_ADAPTER_EVENT,
                    )
                    .await?;

                    info!(
                        task_id = %task_id,
                        subscription_id = %rule.id,
                        subject_key = %subject_key,
                        "Created provider event task for GitHub PR binding"
                    );

                    new_task_binding_id = Some(binding.id.clone());
                    bindings.insert(0, binding);
                }
                // Preflight is permanent: a bad token can't be fixed by a GitHub
                // redelivery, and retrying would just accumulate errored rows.
                // The service already marked the task errored with a sandbox
                // error event, so degrade — log and skip the new-task
                // binding/delivery, then fall through to existing-binding
                // delivery below.
                Err(TaskCreationError::Preflight(reason)) => {
                    warn!(
                        subscription_id = %rule.id,
                        subject_key = %subject_key,
                        error = %reason,
                        "GitHub PR review task failed preflight; skipping new-task delivery"
                    );
                }
                // Transient failures (DB blip, short-id exhaustion) must surface
                // so the webhook 5xx's and GitHub redelivers, rather than
                // silently dropping the PR event.
                Err(other) => {
                    return Err(AdapterError::TaskCreation(other));
                }
            }
        } else if rule.is_none() {
            debug!(
                subject_key = %subject_key,
                "PR opened but no trigger for this repo — not creating a review task; \
                 delivering to existing bindings only"
            );
        }
    }

    // A pull request nothing is bound to expects nothing of this event — every
    // task that was working it has closed. Unlike an empty recipient set on a
    // subject that *does* carry bindings, this is not a loss, so it is not a
    // failure.
    if bindings.is_empty() {
        debug!(
            subject_key = %subject_key,
            "No active bindings on this pull request; nothing to deliver"
        );
        return Ok(());
    }

    // A push returns no event identifier of its own, so the task that wrote it
    // is named by the commit it claimed when it pushed. Delivering a push echo
    // is worse than delivering a comment echo — it wakes a stopped sandbox and
    // starts paid work.
    let author_task_id = match origin_task_id {
        Some(task_id) => Some(task_id),
        None => task_that_pushed(
            &state.recently_sent,
            msg.external_event_id.as_deref(),
            &bindings,
        ),
    };

    // Metering and message labelling need a route; a repo with no trigger of
    // its own routes under the delivery-only one owned by routing.
    let delivery_rule = rule.cloned().unwrap_or_else(|| {
        delivery_only_rule(
            connection,
            subject_key
                .split_once('#')
                .map_or(subject_key.as_str(), |(repo, _)| repo),
        )
    });

    crate::adapters::subject_events::route_subject_event(
        state,
        connection,
        &delivery_rule,
        &msg,
        crate::adapters::subject_events::SubjectEvent {
            adapter_label: ADAPTER_LABEL,
            binding_provider: ADAPTER,
            subject_kind: SUBJECT_KIND,
            subject_key: &subject_key,
            bindings,
            author_task_id: author_task_id.as_deref(),
            new_task_binding_id: new_task_binding_id.as_deref(),
        },
    )
    .await?;
    Ok(())
}

pub(super) async fn close_subject(
    state: &AppState,
    rule: &SubscriptionMatch,
    info: &PrCloseInfo,
) -> Result<(), AdapterError> {
    let pr_number = info.pr_number;
    let subject = GithubPrSubject::from_source_and_number(&rule.source_matcher, pr_number)?;
    let owner = subject.owner();
    let repo = subject.repo();
    let subject_key = subject.key();

    let bindings_repo = PgTaskBindingDao(state.db.clone());
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
                && b.loop_id.as_deref() == Some(rule.id.as_str())
        })
        .collect::<Vec<_>>();

    for binding in bindings {
        let task_id = &binding.task_id;

        #[allow(clippy::disallowed_methods)]
        // reason: GitHub PR close handler runs as background system, no caller identity
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
        if let Err(e) = append_to_binding(state, rule, &binding, &close_msg).await {
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

    let user_id = super::auth::resolve_sender_user(&info.sender_login, rule, &state.db).await;

    InboundMessage {
        adapter_kind: AdapterKind::GithubPr,
        source_id: rule.source_matcher.clone(),
        thread_id: None, // skip thread-context fetch (matches `opened`)
        message_thread_id: info.pr_number.to_string(),
        user_id,
        author: crate::adapters::models::author_handle(&info.sender_login),
        text,
        file_ids: vec![],
        thread_properties: serde_json::json!({
            "owner": owner,
            "repo": repo,
            "pr_number": info.pr_number,
        }),
        adapter_context: serde_json::json!({}),
        external_event_id: Some(super::external_id::pr_event("closed", info.pr_number, None)),
        source_task_id: None,
        audience: Audience::Everyone,
    }
}

/// Upsert a `github_pr` binding through the shared spine. Parses the PR
/// coordinates out of `thread_properties` into the `{owner, repo, pr_number}`
/// destination config, then delegates the row build + persistence to
/// [`crate::bindings::runtime::upsert_binding`]. PR bindings never seed a
/// cursor, so `last_external_event_id` is always `None`.
async fn upsert_binding(
    state: &AppState,
    connection: &AdapterConnection,
    rule: Option<&SubscriptionMatch>,
    task_id: &str,
    thread_properties: &serde_json::Value,
    origin: &str,
) -> Result<TaskBinding, AdapterError> {
    let subject = GithubPrSubject::from_thread_properties(thread_properties)?;
    let subject_key = subject.key();
    let destination_config = subject.destination_config(None);

    crate::bindings::runtime::upsert_binding(
        state,
        connection,
        rule,
        task_id,
        ADAPTER,
        SUBJECT_KIND,
        &subject_key,
        destination_config,
        origin,
        None,
    )
    .await
}

/// Translate a GitHub PR provider event into a [`NewTaskRequest`] and hand it to
/// the shared [`TaskCreationService`]. The service owns short-id generation,
/// preflight, insert, tags, provision, and the task event; this function only
/// maps the subscription + inbound message onto the normalised request.
///
/// The binding upsert and first-message delivery stay with the caller
/// ([`route_event`]), so no first message is passed here.
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
    let config = crate::bindings::runtime::subscription_context(rule, ADAPTER_LABEL);
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
    info!(task_id = %task_id, subscription_id = %rule.id, "Created task from GitHub PR provider event");
    Ok(task_id)
}

/// Append a message to a bound task's chat without waking it.
///
/// The one GitHub PR delivery that does not route. The PR-close notice has to
/// land while the task is still `Open` — `send.rs` drops inbound external
/// messages on a `Closed` task — and it must not provision or wake an agent on
/// a sandbox that is about to be torn down, which is what routing would do.
/// Every event that is news rather than an epitaph goes through
/// [`route_event`].
async fn append_to_binding(
    state: &AppState,
    rule: &SubscriptionMatch,
    binding: &TaskBinding,
    msg: &InboundMessage,
) -> Result<(), AdapterError> {
    crate::message::send::send_message_flow(
        state,
        &binding.task_id,
        &msg.text,
        None,
        None,
        None,
        &msg.file_ids,
        &[],
        false,
        crate::message::send::ExternalOrigin {
            event_id: msg.external_event_id.as_deref(),
            adapter: Some(ADAPTER_LABEL),
            subject_key: Some(&binding.subject_key),
            source_name: Some(rule.source_matcher.as_str()),
            author: msg.author.as_deref(),
            source_task_id: None,
        },
        false,
        Some(msg.user_id.as_str()),
        None,
    )
    .await
    // Logged once at the transport boundary by `AdapterError::into_response`.
    .map_err(AdapterError::MessageDelivery)?;

    let binding_repo = PgTaskBindingDao(state.db.clone());
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

    Ok(())
}

/// The task that claimed the commit this event carries, if any.
///
/// Only a `pull_request.synchronize` notification carries one; every other
/// event names no commit and so names no task. The webhook carries the head
/// commit in full while the claim recorded from `git push` output is
/// abbreviated, so [`RecentlySent`] matches the two on their leading
/// characters.
///
/// This is provenance, not a guard: it says who wrote the push, and the shared
/// router draws the same conclusion from it as from a footer or a recorded
/// event identifier. Only tasks already bound to the pull request are
/// considered, because only they could receive the push in the first place.
fn task_that_pushed(
    recently_sent: &RecentlySent,
    external_event_id: Option<&str>,
    bindings: &[TaskBinding],
) -> Option<String> {
    let sha = external_event_id.and_then(super::external_id::synchronize_sha)?;
    bindings
        .iter()
        .find(|binding| recently_sent.contains_commit(&binding.task_id, sha))
        .map(|binding| binding.task_id.clone())
}

#[cfg(test)]
mod tests {
    use crate::adapters::models::AdapterConnection;
    use chrono::Utc;
    use moatless_backend_core::scope::Scope;

    use super::delivery_only_rule;

    #[test]
    fn a_delivery_only_rule_cannot_create_tasks() {
        let now = Utc::now();
        let connection = AdapterConnection {
            id: "pc_org".to_string(),
            adapter_kind: "github".to_string(),
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
        };

        let rule = delivery_only_rule(&connection, "acme/widget");

        assert!(rule.active);
        assert!(rule.repository_id.is_empty());
        assert!(rule.agent_type.is_empty());
        assert!(rule.prompt_template.is_none());
        assert_eq!(rule.event_filter, serde_json::json!([]));
        assert_eq!(rule.source_matcher, "acme/widget");
        assert_eq!(rule.connection_id, "pc_org");
        assert_eq!(rule.run_as_user_id.as_deref(), Some("u_owner"));
    }

    mod push_claim {
        use super::super::task_that_pushed;
        use crate::adapters::github_pr::external_id;
        use crate::adapters::models::{TaskBinding, CONNECTED_THREAD_BINDING_ORIGIN};
        use crate::bindings::recently_sent::RecentlySent;
        use chrono::Utc;

        const FULL_SHA: &str = "7fc0612a1b2c3d4e5f60718293a4b5c6d7e8f901";

        /// Only the task id and the fact of being bound matter here.
        fn bound(task_ids: &[&str]) -> Vec<TaskBinding> {
            let now = Utc::now();
            task_ids
                .iter()
                .map(|task_id| TaskBinding {
                    id: format!("ts_{task_id}"),
                    task_id: (*task_id).to_string(),
                    adapter_kind: "github".to_string(),
                    subject_kind: "github_pr".to_string(),
                    subject_key: "acme/widget#42".to_string(),
                    connection_id: "pc_org".to_string(),
                    loop_id: None,
                    destination_config: serde_json::json!({}),
                    metadata: serde_json::json!({}),
                    last_external_event_id: None,
                    origin: CONNECTED_THREAD_BINDING_ORIGIN.to_string(),
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
                })
                .collect()
        }

        #[test]
        fn sync_event_for_a_claimed_commit_names_the_task_that_pushed() {
            let recently_sent = RecentlySent::new();
            // What `git push` printed — abbreviated.
            recently_sent.note_commit("task-1", "7fc0612");
            // What the webhook carries — full length.
            let sync = external_id::pr_synchronize(42, FULL_SHA);
            assert_eq!(
                task_that_pushed(&recently_sent, Some(&sync), &bound(&["task-2", "task-1"])),
                Some("task-1".to_string())
            );
        }

        #[test]
        fn another_tasks_push_and_another_commit_name_nobody() {
            let recently_sent = RecentlySent::new();
            recently_sent.note_commit("task-1", "7fc0612");
            let sync = external_id::pr_synchronize(42, FULL_SHA);
            assert_eq!(
                task_that_pushed(&recently_sent, Some(&sync), &bound(&["task-2"])),
                None
            );

            let other = external_id::pr_synchronize(42, "deadbee1a1b2c3d4e5f60718293a4b5c6d7e8f90");
            assert_eq!(
                task_that_pushed(&recently_sent, Some(&other), &bound(&["task-1"])),
                None
            );
        }

        #[test]
        fn degenerate_claim_never_matches_a_sync_event() {
            let recently_sent = RecentlySent::new();
            recently_sent.note_commit("task-1", "");
            recently_sent.note_commit("task-1", "7fc");
            let sync = external_id::pr_synchronize(42, FULL_SHA);
            assert_eq!(
                task_that_pushed(&recently_sent, Some(&sync), &bound(&["task-1"])),
                None
            );
        }

        #[test]
        fn only_sync_events_carry_a_commit_to_match() {
            let recently_sent = RecentlySent::new();
            recently_sent.note_commit("task-1", "7fc0612");
            let bindings = bound(&["task-1"]);
            for id in [
                external_id::pr_opened(42),
                external_id::issue_comment(7),
                external_id::review_comment(7),
                String::new(),
            ] {
                assert_eq!(
                    task_that_pushed(&recently_sent, Some(&id), &bindings),
                    None,
                    "{id} names no commit"
                );
            }
            assert_eq!(task_that_pushed(&recently_sent, None, &bindings), None);
        }

        /// A forced push prints a different range marker but the same commit,
        /// so the claim recorded from it matches the same way a fast-forward
        /// one does — the router never sees the difference.
        #[test]
        fn a_forced_push_is_claimed_like_a_fast_forward_one() {
            let recently_sent = RecentlySent::new();
            recently_sent.note_commit("task-1", "deadbee");
            let sync = external_id::pr_synchronize(42, "deadbee1a1b2c3d4e5f60718293a4b5c6d7e8f90");
            assert_eq!(
                task_that_pushed(&recently_sent, Some(&sync), &bound(&["task-1"])),
                Some("task-1".to_string())
            );
        }
    }
}
