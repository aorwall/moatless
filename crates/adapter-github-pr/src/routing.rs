//! GitHub PR routing on top of provider connections, task loops, and task
//! bindings.

use std::fmt::Write as _;

use tracing::{debug, info, warn};

use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
use moatless_adapter_model::binding::CONNECTED_THREAD_BINDING_ORIGIN;
use moatless_adapter_model::capabilities::AdapterInboundPresentation;
use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::loops::delivery_only_subscription;
use moatless_adapter_model::metrics::{inbound_reason, record_inbound};
use moatless_ports::task::{CloseReason, EventSubject, InboundTurn, TaskOpenError, TurnOrigin};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::AdapterKind;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::SubscriptionMatch;
use moatless_types::task::TaskStatus;
use moatless_types::task::Visibility;

use super::router::GithubPrInboundRouter;
use super::subject::GithubPrSubject;
use super::webhook::PrCloseInfo;

/// Adapter value used in adapter connections, trigger sources, and bindings.
pub(super) const ADAPTER: &str = AdapterKind::Github.as_str();
pub(super) use super::subject::SUBJECT_KIND;
/// Label written to `task_inbox.external_adapter`. Matches the
/// `AdapterKind::GithubPr.as_str()` value so existing UI treats new rows the
/// same as the legacy ones.
pub(super) const ADAPTER_LABEL: &str = AdapterKind::GithubPr.as_str();

const ORIGIN_ADAPTER_EVENT: &str = ADAPTER_EVENT_BINDING_ORIGIN;
const ORIGIN_CONNECTED_THREAD: &str = CONNECTED_THREAD_BINDING_ORIGIN;

/// Always prepended to the `append_system_prompt` of tasks created from a
/// GitHub PR event, so loop/trigger prompts don't have to restate the origin.
const PR_ORIGIN_CONTEXT: &str = "This task was created from a GitHub Pull Request.";

/// This adapter's [`delivery_only_subscription`]: the Subscription a PR event
/// delivers under when the connection has no matching trigger.
pub(super) fn delivery_only_rule(
    connection: &AdapterConnection,
    repo_full_name: &str,
) -> SubscriptionMatch {
    delivery_only_subscription(connection, AdapterKind::GithubPr, repo_full_name)
}

impl GithubPrInboundRouter {
    /// Route an inbound GitHub PR message to the tasks bound to its pull
    /// request.
    ///
    /// Assembles the bindings — attaching one for the event's author task,
    /// spawning the review task a fresh open PR calls for — but
    /// [`moatless_loops::bindings::runtime::fan_out`] decides which receive the
    /// event, from the audience `adapter.rs` classified it into.
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

        let subject = GithubPrSubject::from_thread_properties(&msg.thread_properties)?;
        let subject_key = subject.key();
        // The repository this event happened in, spelled the way the outbound
        // recorder spells it (`api_routes::note_external_event`), so a task's
        // own claim is found under the same name it was filed under. Read off
        // the subject rather than the trigger, which is configuration.
        let source_name = format!("{}/{}", subject.owner(), subject.repo());
        let binding_repo = self.task_bindings.clone();

        // Provenance: who wrote this event, where it can be named. The
        // `Moatless task:` footer link `<frontend_url>/tasks/<id>` the adapter
        // parsed off the body is authoritative and costs no round-trip; failing
        // that, an event identifier already recorded against a task names it.
        // Nothing here is an identity.
        //
        // The identifier is only asked about on this pull request. A PR-level
        // id names its own subject now (`external_id`), but a row written
        // before it did carries a bare number that every repository with a #607
        // shares, and the answer here attaches a binding — so the question
        // states the subject rather than trusting the string to be unambiguous.
        let origin_task_id = match msg.source_task_id.clone() {
            Some(task_id) => Some(task_id),
            // Every rerun on one commit shares a check-run id, so its
            // `task_inbox` row names the Task it reached, not an author: matching
            // it would skip that Task as a self-echo and re-bind it if unbound.
            None => match msg
                .external_event_id
                .as_deref()
                .filter(|id| !super::external_id::is_check_run(id))
            {
                Some(ext_id) => {
                    let candidates = self.routing.external_id_aliases(ADAPTER_LABEL, ext_id);
                    self.messages
                        .find_author_on_subject(
                            &candidates,
                            EventSubject {
                                adapter: ADAPTER_LABEL,
                                source_name: &source_name,
                                subject_key: &subject_key,
                            },
                        )
                        .await?
                }
                None => None,
            },
        };

        let mut bindings = binding_repo
            .for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
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
                .adapter_event_binding(ADAPTER, SUBJECT_KIND, &subject_key)
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
                    .map(|(task_id, binding_id)| moatless_types::task::RelatedTask {
                        related_task_id: task_id.clone(),
                        context: serde_json::json!({
                            "via": "github_pr_webhook",
                            "adapter": ADAPTER,
                            "subject_kind": SUBJECT_KIND,
                            "subject_key": subject_key,
                            "origin_binding_id": binding_id,
                        }),
                    });
                // The service owns insert + preflight + provision.
                match self.create_provider_event_task(rule, &msg).await {
                    Ok(task_id) => {
                        if let Some(ref r) = related {
                            self.task_relations.record_related_to(&task_id, r).await;
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
                    Err(TaskOpenError::Refused(reason)) => {
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
                        return Err(AdapterError::task_handoff(
                            "open a task from a provider event",
                            other,
                        ));
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
            .for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
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

            let siblings_count = match bindings_repo.for_task(task_id).await {
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
            let close_reason = CloseReason {
                actor: "claude-code".to_string(),
                payload: serde_json::json!({ "reason": "pr_closed", "pr_number": pr_number }),
            };
            if let Err(e) = self.task_closer.close(task_id, close_reason).await {
                warn!(task_id = %task_id, error = %e, "Failed to close task on PR close");
                continue;
            }

            info!(task_id = %task_id, pr_number, "Task closed due to PR close");
        }

        Ok(())
    }

    /// Build the synthetic `InboundMessage` explaining why a task auto-closed
    /// when its PR was closed/merged.
    ///
    /// Mirrors the shape of a `pull_request.opened` inbound message so the UI
    /// renders it identically.
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

        let user_id = super::auth::resolve_sender_user(
            &info.sender_login,
            info.pr_author_login.as_deref(),
            rule,
            self.users.as_ref(),
        )
        .await;

        InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: rule.source_matcher.clone(),
            thread_id: None, // skip thread-context fetch (matches `opened`)
            message_thread_id: info.pr_number.to_string(),
            user_id,
            author: moatless_types::adapter::author_handle(&info.sender_login),
            sender_external_id: None,
            text,
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": owner,
                "repo": repo,
                "pr_number": info.pr_number,
            }),
            adapter_context: serde_json::json!({}),
            external_event_id: Some(super::external_id::pr_event(
                "closed",
                &GithubPrSubject::new(owner, repo, info.pr_number),
                None,
            )),
            source_task_id: None,
            audience: Audience::Everyone,
        }
    }

    /// Upsert a `github_pr` binding through the shared spine.
    ///
    /// Parses PR coordinates from `thread_properties` into the destination
    /// config; PR bindings never seed a cursor, so `last_external_event_id` is
    /// always `None`.
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

        self.task_bindings
            .bind(
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

    /// Translate a GitHub PR provider event into a [`NewTaskRequest`] and hand
    /// it to the task band's own opener; the binding upsert and first-message
    /// delivery stay with the caller ([`route_event`]).
    ///
    /// Returns the raw [`TaskOpenError`] so the caller can distinguish a
    /// permanent failure (`Preflight`) from a transient one that should surface
    /// as a 5xx so GitHub retries.
    async fn create_provider_event_task(
        &self,
        rule: &SubscriptionMatch,
        msg: &InboundMessage,
    ) -> Result<String, TaskOpenError> {
        // Compose the shared spine's single task-creation entry point. The only
        // GitHub-specific inputs are the `PR_ORIGIN_CONTEXT` prelude, public
        // visibility (so PR collaborators can open the task), and the
        // `source_matcher` event metadata; `related_to` is recorded by the caller
        // after creation succeeds, and the first message is delivered through the
        // binding.
        let config = moatless_adapter_model::loops::subscription_context(rule);
        // Self-resolve: github_pr's reply instructions are its own constant, so
        // read them off this adapter rather than the registry of all providers,
        // and pass them to the firing entry point as a value.
        let reply_instructions = match self.adapter.as_ref() {
            Some(adapter) => adapter.reply_instructions().await,
            None => None,
        };
        let outcome = moatless_loops::loops::firing::inbound::create_task_from_loop(
            self.task_creation.as_ref(),
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

        let task_id = outcome.task_id;
        info!(task_id = %task_id, subscription_id = %rule.id, "Created task from GitHub PR provider event");
        Ok(task_id)
    }

    /// Append a message to a bound task's chat without waking it.
    ///
    /// The one GitHub PR delivery that does not route: the PR-close notice must
    /// land while the task is still `Open` (`send.rs` drops inbound messages on
    /// a `Closed` task) without waking an agent on a sandbox about to be torn
    /// down. Everything else goes through [`route_event`].
    async fn append_to_binding(
        &self,
        rule: &SubscriptionMatch,
        binding: &TaskBinding,
        msg: &InboundMessage,
    ) -> Result<(), AdapterError> {
        self.message_send
            .accept(InboundTurn {
                task_id: binding.task_id.clone(),
                text: msg.text.clone(),
                file_ids: msg.file_ids.clone(),
                resume: false,
                origin: TurnOrigin {
                    event_id: msg.external_event_id.clone(),
                    adapter: Some(ADAPTER_LABEL.to_string()),
                    subject_key: Some(binding.subject_key.clone()),
                    source_name: Some(rule.source_matcher.clone()),
                    author: msg.author.clone(),
                    sender_id: None,
                },
                sent_by: Some(msg.user_id.clone()),
                skills: Vec::new(),
            })
            .await
            // Logged once at the transport boundary by `AdapterError::into_response`.
            .map_err(|e| AdapterError::task_handoff("deliver a provider turn to a task", e))?;

        if let Err(e) = self
            .task_bindings
            .record_inbound_event(binding, msg.external_event_id.as_deref())
            .await
        {
            warn!(
                binding_id = %binding.id,
                error = %e,
                "Failed to record the inbound event on binding"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterConnection;
    use moatless_types::adapter::AdapterKind;

    use super::delivery_only_rule;

    fn connection(owner_user_id: Option<&str>) -> AdapterConnection {
        let now = Utc::now();
        AdapterConnection {
            id: "pc_org".to_string(),
            adapter_kind: AdapterKind::Github,
            connection_kind: "org_webhook".to_string(),
            scope_value: "acme".to_string(),
            credential_ref: None,
            webhook_secret: Some("s3cr3t".to_string()),
            transport_config: serde_json::json!({}),
            created_by: "u_installer".to_string(),
            scope: Scope::User,
            owner_user_id: owner_user_id.map(str::to_string),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn a_delivery_only_rule_cannot_create_tasks() {
        let rule = delivery_only_rule(&connection(Some("u_owner")), "acme/widget");

        assert!(rule.active);
        assert!(rule.workspace_id.is_empty());
        assert!(rule.prompt_template.is_none());
        assert_eq!(rule.event_filter, serde_json::json!([]));
        assert_eq!(rule.source_matcher, "acme/widget");
        assert_eq!(rule.connection_id, "pc_org");
        assert_eq!(rule.run_as_user_id.as_deref(), Some("u_owner"));
    }

    #[test]
    fn an_unowned_connection_still_names_a_run_as_user() {
        let rule = delivery_only_rule(&connection(None), "acme/widget");

        assert_eq!(
            rule.effective_user_id(),
            "u_installer",
            "an org connection owned by nobody must not panic every reader"
        );
    }
}
