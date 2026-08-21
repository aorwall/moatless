//! GitHub PR routing on top of provider connections, task loops, and task bindings.

use std::fmt::Write as _;

use chrono::Utc;
use tracing::{debug, info, warn};

use moatless_types::task::TaskStatus;
use moatless_types::access_control::ScopeContext;
use moatless_loops::adapters::AdapterInboundPresentation;
use moatless_loops::adapters::AdapterKind;
use moatless_loops::adapters::error::AdapterError;
use moatless_loops::adapters::models::{ADAPTER_EVENT_BINDING_ORIGIN, AdapterConnection, CONNECTED_THREAD_BINDING_ORIGIN, SubscriptionMatch, TaskBinding};
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::RoutingMode;
use moatless_loops::bindings::metrics::{inbound_reason, record_inbound};
use moatless_tasks::task::closer::CloseEvent;
use moatless_tasks::task::creation::TaskCreationError;
use moatless_types::task::Visibility;

use super::router::GithubPrInboundRouter;
use super::subject::GithubPrSubject;
use super::webhook::PrCloseInfo;

/// Adapter value used in adapter connections, trigger sources, and bindings.
pub(super) const ADAPTER: &str = "github";
/// Subject kind for GitHub PRs (`task_bindings.subject_kind`).
pub(super) const SUBJECT_KIND: &str = "github_pr";
/// Label written to `task_messages.external_adapter`. Matches the
/// `AdapterKind::GithubPr.as_str()` value so existing UI treats new rows
/// the same as the legacy ones.
pub(super) const ADAPTER_LABEL: &str = "github_pr";

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
        // Not a real Loop, so it has no configured name — say so rather than
        // borrowing one that would read as a Loop in any UI that shows it.
        name: "delivery-only".to_string(),
        adapter_kind: AdapterKind::GithubPr,
        connection_id: connection.id.clone(),
        source_matcher: repo_full_name.to_string(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        repository_id: String::new(),
        prompt_template: None,
        task_name: None,
        tag_ids: Vec::new(),
        skills: Vec::new(),
        run_as_user_id: connection.owner_user_id.clone(),
        agent_type: moatless_types::agent::AgentType(String::new()),
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

impl GithubPrInboundRouter {
    /// Route an inbound GitHub PR message to the tasks bound to its pull request.
    ///
    /// What is GitHub's about routing is assembling the bindings: attaching one for
    /// a task the event names as its author, and spawning the review task a freshly
    /// opened pull request calls for. Which of them receive the event is not — that
    /// is [`moatless_loops::bindings::runtime::fan_out`], deciding from the audience
    /// `adapter.rs` classified the event into. This function used to answer both,
    /// through a ladder of guards whose last rung compared the sender's identity to
    /// the bound task's owner; a real review reached no task because of what two of
    /// those rungs added up to. Identity plays no part here now.
    pub(super) async fn route_event(
        &self,
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
        let binding_repo = self.task_bindings.clone();

        // Provenance: who wrote this event, where it can be named. The
        // `Moatless task:` footer link `<frontend_url>/tasks/<id>` the adapter
        // parsed off the body is authoritative and costs no round-trip; failing
        // that, an event identifier already recorded against a task names it.
        // Nothing here is an identity.
        let origin_task_id = match msg.source_task_id.clone() {
            Some(task_id) => Some(task_id),
            None => match msg.external_event_id.as_deref() {
                Some(ext_id) => {
                    let candidates = self.routing.external_id_aliases(ADAPTER_LABEL, ext_id);
                    self.messages
                        .find_by_external_event_id(&candidates)
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
                match self
                    .upsert_binding(
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
                // `moat gh pr create` upserts a connected_thread binding for the
                // originating task as part of creating the PR — so by the time
                // pr_opened fires, the earliest binding is the originating
                // task. Link the new review
                // task to it as a `related_to` peer (it shares the PR subject_key,
                // it isn't a child of the originator).
                let related = originator_binding_summary
                    .as_ref()
                    .map(
                        |(task_id, binding_id)| moatless_types::task::RelatedTask {
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
                match self.create_provider_event_task(rule, &msg).await {
                    Ok(task_id) => {
                        if let Some(ref r) = related {
                            moatless_tasks::task::relations::record_related_to(
                                self.task_relations.as_ref(),
                                &task_id,
                                r,
                            )
                            .await;
                        }

                        let binding = self
                            .upsert_binding(
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

        moatless_loops::adapters::subject_events::route_subject_event(
            &self.routing,
            connection,
            &delivery_rule,
            &msg,
            moatless_loops::adapters::subject_events::SubjectEvent {
                adapter_label: ADAPTER_LABEL,
                binding_provider: ADAPTER,
                subject_kind: SUBJECT_KIND,
                subject_key: &subject_key,
                bindings,
                author_task_id: origin_task_id.as_deref(),
                new_task_binding_id: new_task_binding_id.as_deref(),
            },
        )
        .await?;
        Ok(())
    }

    pub(super) async fn close_subject(
        &self,
        rule: &SubscriptionMatch,
        info: &PrCloseInfo,
    ) -> Result<(), AdapterError> {
        let pr_number = info.pr_number;
        let subject = GithubPrSubject::from_source_and_number(&rule.source_matcher, pr_number)?;
        let owner = subject.owner();
        let repo = subject.repo();
        let subject_key = subject.key();

        let bindings_repo = self.task_bindings.clone();
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
            match self.tasks.describe(task_id, &sys).await {
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
            let close_msg = self
                .build_close_inbound_message(owner, repo, rule, info)
                .await;
            if let Err(e) = self.append_to_binding(rule, &binding, &close_msg).await {
                warn!(
                    task_id = %task_id,
                    pr_number,
                    error = %e,
                    "Failed to append PR-close message to task — proceeding with close"
                );
            }

            // Terminate through the shared closer. It persists `Closed` before
            // the slow teardown — the same racing-webhook ordering the inline
            // close kept (a concurrent auto_provision then sees `Closed` and
            // bails) — then removes the sandbox, deactivates the bindings, and
            // publishes the event. The six close-ports live on `TaskCloser`, not
            // on this router; only the enriched close event is github_pr's, so we
            // carry it: the "claude-code" actor and the `{reason, pr_number}`
            // payload the UI renders, byte for byte what the inline close emitted.
            let close_event = CloseEvent::enriched(
                "claude-code",
                serde_json::json!({ "reason": "pr_closed", "pr_number": pr_number }),
            );
            if let Err(e) = self
                .task_closer
                .close_with_event(&sys, task_id, &close_event)
                .await
            {
                warn!(task_id = %task_id, error = %e, "Failed to close task on PR close");
                continue;
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
        &self,
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

        let user_id = super::auth::resolve_sender_user(&info.sender_login, rule, self.users.as_ref()).await;

        InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: rule.source_matcher.clone(),
            thread_id: None, // skip thread-context fetch (matches `opened`)
            message_thread_id: info.pr_number.to_string(),
            user_id,
            author: moatless_loops::adapters::models::author_handle(&info.sender_login),
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
    /// [`moatless_loops::bindings::runtime::upsert_binding_into`]. PR bindings never seed a
    /// cursor, so `last_external_event_id` is always `None`.
    async fn upsert_binding(
        &self,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        task_id: &str,
        thread_properties: &serde_json::Value,
        origin: &str,
    ) -> Result<TaskBinding, AdapterError> {
        let subject = GithubPrSubject::from_thread_properties(thread_properties)?;
        let subject_key = subject.key();
        let destination_config = subject.destination_config(None);

        moatless_loops::bindings::runtime::upsert_binding_into(
            self.task_bindings.as_ref(),
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
        &self,
        rule: &SubscriptionMatch,
        msg: &InboundMessage,
    ) -> Result<String, TaskCreationError> {
        // Compose the shared spine's single task-creation entry point. The only
        // GitHub-specific inputs are the `PR_ORIGIN_CONTEXT` prelude, public
        // visibility (so PR collaborators can open the task), and the
        // `source_matcher` event metadata; `related_to` is recorded by the caller
        // after creation succeeds, and the first message is delivered through the
        // binding.
        let config = moatless_adapter_model::models::subscription_context(rule);
        // Self-resolve: github_pr's reply instructions are its own constant, so
        // read them off this adapter rather than the registry of all providers,
        // and pass them to the firing entry point as a value.
        let reply_instructions = self
            .adapter
            .as_ref()
            .and_then(|adapter| adapter.reply_instructions());
        let outcome = moatless_loops::loops::firing::inbound::create_task_from_loop(
            &self.task_creation,
            &config,
            moatless_loops::loops::firing::inbound::CreateTaskParams {
                name: rule.task_name.clone(),
                description: moatless_loops::bindings::outbound::truncate_text(&msg.text, 200),
                visibility: Visibility::Public,
                origin_prelude: Some(PR_ORIGIN_CONTEXT),
                reply_instructions,
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
        &self,
        rule: &SubscriptionMatch,
        binding: &TaskBinding,
        msg: &InboundMessage,
    ) -> Result<(), AdapterError> {
        self.message_send
            .send_message_flow(
                &binding.task_id,
                &msg.text,
                None,
                None,
                None,
                &msg.file_ids,
                &[],
                false,
                moatless_types::task::ExternalOrigin {
                    event_id: msg.external_event_id.as_deref(),
                    adapter: Some(ADAPTER_LABEL),
                    subject_key: Some(&binding.subject_key),
                    source_name: Some(rule.source_matcher.as_str()),
                    author: msg.author.as_deref(),
                    source_task_id: None,
                },
                false,
                Some(msg.user_id.as_str()),
                &[],
                None,
            )
            .await
            // Logged once at the transport boundary by `AdapterError::into_response`.
            .map_err(AdapterError::MessageDelivery)?;

        let binding_repo = self.task_bindings.clone();
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
}

#[cfg(test)]
mod tests {
    use moatless_types::adapter::AdapterKind;
    use chrono::Utc;
    use moatless_types::access_control::Scope;
    use moatless_loops::adapters::models::AdapterConnection;

    use super::delivery_only_rule;

    #[test]
    fn a_delivery_only_rule_cannot_create_tasks() {
        let now = Utc::now();
        let connection = AdapterConnection {
            id: "pc_org".to_string(),
            adapter_kind: AdapterKind::Github,
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
        assert!(rule.agent_type.as_str().is_empty());
        assert!(rule.prompt_template.is_none());
        assert_eq!(rule.event_filter, serde_json::json!([]));
        assert_eq!(rule.source_matcher, "acme/widget");
        assert_eq!(rule.connection_id, "pc_org");
        assert_eq!(rule.run_as_user_id.as_deref(), Some("u_owner"));
    }

}
