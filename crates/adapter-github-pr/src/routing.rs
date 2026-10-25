//! GitHub PR routing on top of provider connections, task loops, and task
//! bindings.

use std::fmt::Write as _;

use tracing::{debug, info, warn};

use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
use moatless_adapter_model::binding::CONNECTED_THREAD_BINDING_ORIGIN;
use moatless_adapter_model::binding::{
    BindReason, BindScope, closes_on_subject_close, hears_only_replies,
};
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

pub(super) const ORIGIN_ADAPTER_EVENT: &str = ADAPTER_EVENT_BINDING_ORIGIN;
const ORIGIN_CONNECTED_THREAD: &str = CONNECTED_THREAD_BINDING_ORIGIN;

/// Always prepended to the `append_system_prompt` of tasks created from a
/// GitHub PR event, so loop/trigger prompts don't have to restate the origin.
pub(super) const PR_ORIGIN_CONTEXT: &str = "This task was created from a GitHub Pull Request.";

/// This adapter's [`delivery_only_subscription`]: the Subscription a PR event
/// delivers under when the connection has no matching trigger.
pub(super) fn delivery_only_rule(
    connection: &AdapterConnection,
    repo_full_name: &str,
) -> SubscriptionMatch {
    delivery_only_subscription(connection, AdapterKind::GithubPr, repo_full_name)
}

/// The source matcher of an org's check-run trigger loop, which matches every
/// repository of `org` that has no loop of its own.
pub(super) fn org_wildcard(org: &str) -> String {
    format!("{org}/*")
}

/// `rule` as it applies to `repo_full_name`. An org wildcard loop's matcher is
/// replaced by the repository: everything downstream reads `source_matcher`
/// as the event's `owner/repo`, and would otherwise fail to parse `owner/*`.
pub(super) fn for_repository(
    mut rule: SubscriptionMatch,
    repo_full_name: &str,
) -> SubscriptionMatch {
    if rule.source_matcher.ends_with("/*") {
        rule.source_matcher = repo_full_name.to_string();
    }
    rule
}

/// Whether a closing pull request closes `binding`'s Task: one `rule` created,
/// or one whose active Binding opted in. A Task `moat gh pr create` connected
/// existed before the PR and must not close by default.
pub(super) fn closes_with_pr(
    rule: &SubscriptionMatch,
    trigger_closes: bool,
    binding: &TaskBinding,
) -> bool {
    let created_by_trigger = binding.origin == ADAPTER_EVENT_BINDING_ORIGIN
        && binding.loop_id.as_deref() == Some(rule.id.as_str());
    (trigger_closes && created_by_trigger) || (binding.active && closes_on_subject_close(binding))
}

/// The `metadata` key [`GithubPrInboundRouter::close_subject`] sets on every
/// Binding of a closed pull request, and a `reopened` event clears.
const PR_CLOSED_KEY: &str = "pr_closed";

fn pr_is_closed(binding: &TaskBinding) -> bool {
    binding
        .metadata
        .get(PR_CLOSED_KEY)
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// How many of the Task's Bindings, `binding` included, keep it open when
/// `binding`'s pull request closes.
///
/// Every row counts for a Task that did not opt in. An opted-in Task stays open
/// only for another active pull request not yet closed: a Slack or Linear
/// thread it was started from must not keep it open forever.
fn bindings_holding_open(binding: &TaskBinding, rows: &[TaskBinding]) -> usize {
    if !closes_on_subject_close(binding) {
        return rows.len();
    }
    let open_prs = rows.iter().filter(|row| {
        row.id != binding.id
            && row.active
            && row.adapter_kind.as_str() == ADAPTER
            && row.subject_kind == SUBJECT_KIND
            && !pr_is_closed(row)
    });
    1 + open_prs.count()
}

/// Whether a pull request opening creates a Task under `rule`. Under a
/// `create_on: check_run` loop only a trigger check run does.
fn opening_creates_task(rule: &SubscriptionMatch) -> bool {
    super::config::GithubPrRuleConfig::from_rule(rule).create_on
        == super::config::CreateOn::PrOpened
}

/// What the event says of the task named as its author: that it opened the
/// pull request or pushed to it, or else commented, with what a reply to that
/// comment carries.
fn origin_scope(msg: &InboundMessage) -> BindScope {
    let event_id = msg.external_event_id.as_deref().unwrap_or_default();
    if super::external_id::is_opened(event_id) {
        return BindScope::because(BindReason::Creator);
    }
    if super::external_id::is_synchronize(event_id) {
        return BindScope::because(BindReason::Contributor);
    }
    let reply_keys = msg
        .adapter_context
        .get(super::reply_keys::OPENS_KEY)
        .and_then(serde_json::Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    BindScope {
        reason: Some(BindReason::Commenter),
        reply_keys,
        close_on_subject_close: false,
    }
}

impl GithubPrInboundRouter {
    /// Route an inbound GitHub PR message to tasks bound to its pull request.
    ///
    /// Assembles the bindings — attaching the author's task, spawning the
    /// review task a fresh open PR calls for — but
    /// [`moatless_loops::bindings::runtime::fan_out`] decides who receives it,
    /// by audience.
    pub(super) async fn route_event(
        &self,
        connection: &AdapterConnection,
        rule: Option<&SubscriptionMatch>,
        mut msg: InboundMessage,
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

        // Provenance: who wrote this, via the `Moatless task:` footer link the
        // adapter parsed off the body, else a recorded event id — never an
        // identity. Asked only on this PR: an old row's bare number can repeat
        // across repos, so this scopes the lookup to the subject, not the
        // string alone.
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

        // Tasks post as their owner, so a sender match on a task's own
        // comment would make every task of that owner answer a commenter
        // who mentioned the owner.
        if origin_task_id.is_some() {
            msg.replies_to
                .retain(|key| !super::reply_keys::is_addressed(key));
        }

        let mut bindings = binding_repo
            .for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
            .await?;

        // Skip bindings whose task has closed — a closed task is terminal and must
        // not be resurrected by a new PR comment. (A fresh binding for the
        // originating task is re-added active just below.)
        bindings.retain(|b| b.active);

        // If the originating task isn't subscribed yet (e.g. agent ran
        // `gh pr comment` from a task that never connected to the PR), attach a
        // binding so future events fan out to it too. A bound task's comment
        // still writes, to record what a reply to it will carry.
        if let Some(origin_task_id) = origin_task_id.as_deref() {
            let scope = origin_scope(&msg);
            let bound_at = bindings.iter().position(|s| s.task_id == origin_task_id);
            let widens = |bound: &TaskBinding| {
                !scope.reply_keys.is_empty()
                    || (hears_only_replies(bound) && scope.reason > Some(BindReason::Commenter))
            };
            if bound_at.is_none_or(|at| widens(&bindings[at])) {
                match self
                    .upsert_binding(
                        connection,
                        rule,
                        origin_task_id,
                        &msg.thread_properties,
                        ORIGIN_CONNECTED_THREAD,
                        &scope,
                    )
                    .await
                {
                    Ok(s) => match bound_at {
                        Some(at) => bindings[at] = s,
                        None => bindings.push(s),
                    },
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

        if msg
            .external_event_id
            .as_deref()
            .is_some_and(super::external_id::is_reopened)
        {
            self.clear_pr_closed(&bindings).await;
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
            super::external_id::is_opened(id) || super::external_id::is_reopened(id)
        }) && rule.is_none_or(opening_creates_task);

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
                // `moat gh pr create` upserts a connected_thread binding for
                // the originating task when creating the PR, so by `pr_opened`
                // the earliest binding is the originator. Link the new review
                // task to it as a `related_to` peer — it shares the
                // subject_key, not a parent/child.
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
                                &BindScope::default(),
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
                    // Preflight is permanent: no redelivery fixes a bad token,
                    // and retrying only accumulates errored rows. The service
                    // already marked the task errored, so this degrades: skip
                    // the new-task binding/delivery and fall through to the
                    // existing bindings.
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

        // A Binding whose watch was turned off in T3 stays linked and keeps its
        // status, but no event on the pull request wakes its Task. Filtered
        // here, after the origin attach above, so a muted origin is not bound
        // a second time.
        let bound = bindings.len();
        bindings.retain(|b| b.watching);
        if bindings.is_empty() && bound > 0 {
            debug!(
                subject_key = %subject_key,
                muted = bound,
                "Every binding on this pull request has its watch off; nothing to deliver"
            );
            return Ok(());
        }

        // Nothing bound to this PR means nothing is owed: every task working
        // it has closed. Unlike an empty recipient set on a subject that does
        // carry bindings, this is not a loss.
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

    /// Close the Tasks [`closes_with_pr`] picks for this pull request.
    pub(super) async fn close_subject(
        &self,
        rule: &SubscriptionMatch,
        info: &PrCloseInfo,
        trigger_closes: bool,
    ) -> Result<(), AdapterError> {
        let pr_number = info.pr_number;
        let subject = GithubPrSubject::from_source_and_number(&rule.source_matcher, pr_number)?;
        let owner = subject.owner();
        let repo = subject.repo();
        let subject_key = subject.key();

        let bindings_repo = self.task_bindings.clone();
        let all = bindings_repo
            .for_subject(ADAPTER, SUBJECT_KIND, &subject_key)
            .await?;
        // Every Binding is marked, opted in or not, and before any Task's
        // siblings are read: an unmarked closed PR would hold an opted-in Task
        // open forever, and of two PRs closing at once one must see the other.
        let mark = serde_json::json!({ PR_CLOSED_KEY: true });
        for binding in &all {
            if let Err(e) = bindings_repo.merge_metadata(&binding.id, &mark).await {
                warn!(binding_id = %binding.id, error = %e, "Failed to mark the pull request closed on its binding");
            }
        }
        let bindings = all
            .into_iter()
            .filter(|b| closes_with_pr(rule, trigger_closes, b))
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
                Ok(rows) => bindings_holding_open(&binding, &rows),
                Err(e) => {
                    warn!(task_id = %task_id, pr_number, error = %e, "Failed to look up sibling bindings — skipping auto-close");
                    continue;
                }
            };

            self.notify_and_maybe_close_for_pr(rule, info, owner, repo, &binding, siblings_count)
                .await;
        }

        Ok(())
    }

    /// Clear [`PR_CLOSED_KEY`] from the Bindings of a reopened pull request, so
    /// it holds its opted-in Task open again.
    async fn clear_pr_closed(&self, bindings: &[TaskBinding]) {
        let clear = serde_json::json!({ PR_CLOSED_KEY: false });
        for binding in bindings.iter().filter(|b| pr_is_closed(b)) {
            if let Err(e) = self.task_bindings.merge_metadata(&binding.id, &clear).await {
                warn!(binding_id = %binding.id, error = %e, "Failed to clear the closed mark on a reopened pull request");
            }
        }
    }

    /// Deliver the PR-close notice, then close the Task — unless a sibling
    /// Binding survives it.
    ///
    /// The notice runs first: `send.rs` drops messages to `Closed` tasks. It
    /// wakes only a spared opted-in Task, whose merged notice `auto_close_owns`
    /// withheld; a closing Task's sandbox is being torn down.
    async fn notify_and_maybe_close_for_pr(
        &self,
        rule: &SubscriptionMatch,
        info: &PrCloseInfo,
        owner: &str,
        repo: &str,
        binding: &TaskBinding,
        siblings_count: usize,
    ) {
        let task_id = &binding.task_id;
        let pr_number = info.pr_number;

        let closing = siblings_count <= 1;
        let close_msg = self
            .build_close_inbound_message(owner, repo, rule, info, closing)
            .await;
        let resume = !closing && closes_on_subject_close(binding);
        if let Err(e) = self
            .append_to_binding(rule, binding, &close_msg, resume)
            .await
        {
            warn!(
                task_id = %task_id,
                pr_number,
                error = %e,
                "Failed to append PR-close message to task — proceeding with close"
            );
        }

        if !closing {
            info!(
                task_id = %task_id,
                pr_number,
                other_binding_count = siblings_count - 1,
                "PR closed but task has other connected bindings — leaving task open"
            );
            return;
        }

        // Terminate through the shared closer: it persists `Closed` before
        // the slow teardown, so a racing `auto_provision` sees `Closed` and
        // bails, then removes the sandbox, deactivates bindings, and
        // publishes the event. The close-ports live on `TaskCloser`, not
        // this router; only the close event below is github_pr's own.
        let close_reason = CloseReason {
            actor: "claude-code".to_string(),
            payload: serde_json::json!({ "reason": "pr_closed", "pr_number": pr_number }),
        };
        if let Err(e) = self.task_closer.close(task_id, close_reason).await {
            warn!(task_id = %task_id, error = %e, "Failed to close task on PR close");
            return;
        }

        info!(task_id = %task_id, pr_number, "Task closed due to PR close");
    }

    /// Build the synthetic `InboundMessage` reporting that a task's PR
    /// closed/merged, with a footer naming whether this Task auto-closes for
    /// it or stays open for a surviving sibling Binding.
    ///
    /// Mirrors the shape of a `pull_request.opened` inbound message so the UI
    /// renders it identically.
    async fn build_close_inbound_message(
        &self,
        owner: &str,
        repo: &str,
        rule: &SubscriptionMatch,
        info: &PrCloseInfo,
        closing: bool,
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
        text.push_str(if closing {
            "\n_Task auto-closed because the PR was closed._"
        } else {
            "\n_Task left open: it has other connected bindings._"
        });

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
            replies_to: Vec::new(),
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
        scope: &BindScope,
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
                scope,
            )
            .await
    }

    /// Translate a GitHub PR provider event into a [`NewTaskRequest`] for the
    /// task opener; binding upsert and first-message delivery stay with the
    /// caller ([`route_event`]).
    ///
    /// Returns the raw [`TaskOpenError`]: a permanent failure (`Preflight`) vs.
    /// a transient one that should 5xx so GitHub retries.
    async fn create_provider_event_task(
        &self,
        rule: &SubscriptionMatch,
        msg: &InboundMessage,
    ) -> Result<String, TaskOpenError> {
        // Compose the shared spine's task-creation entry point. GitHub-specific
        // inputs: the `PR_ORIGIN_CONTEXT` prelude, public visibility (so PR
        // collaborators can open the task), and `source_matcher` metadata.
        // `related_to` is recorded by the caller after creation; the first
        // message is delivered through the binding.
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
        resume: bool,
    ) -> Result<(), AdapterError> {
        self.message_send
            .accept(InboundTurn {
                task_id: binding.task_id.clone(),
                text: msg.text.clone(),
                file_ids: msg.file_ids.clone(),
                resume,
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
pub(crate) mod tests {
    use chrono::Utc;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterConnection;
    use moatless_types::adapter::AdapterKind;

    use super::delivery_only_rule;
    use super::origin_scope;
    use moatless_adapter_model::binding::BindReason;
    use moatless_types::adapter::{Audience, InboundMessage};

    fn authored(external_event_id: &str, adapter_context: serde_json::Value) -> InboundMessage {
        InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: "acme/widgets".into(),
            thread_id: None,
            message_thread_id: "7".into(),
            user_id: "u_1".into(),
            author: None,
            sender_external_id: None,
            text: String::new(),
            file_ids: vec![],
            thread_properties: serde_json::json!({}),
            adapter_context,
            external_event_id: Some(external_event_id.into()),
            source_task_id: Some("t_1".into()),
            audience: Audience::Everyone,
            replies_to: Vec::new(),
        }
    }

    /// The event a task is named as the author of says why it is bound: the
    /// pull request it opened or pushed to is its work; a comment is not.
    #[test]
    fn the_authored_event_names_the_bind_reason() {
        assert_eq!(
            origin_scope(&authored(
                "github:pr:opened:acme/widgets#7",
                serde_json::json!({})
            ))
            .reason,
            Some(BindReason::Creator)
        );
        assert_eq!(
            origin_scope(&authored(
                "github:pr:synchronize:acme/widgets#7:abc",
                serde_json::json!({})
            ))
            .reason,
            Some(BindReason::Contributor)
        );
        let comment = origin_scope(&authored(
            "github:ic:5",
            serde_json::json!({ crate::reply_keys::OPENS_KEY: ["github:ic:5"] }),
        ));
        assert_eq!(comment.reason, Some(BindReason::Commenter));
        assert_eq!(comment.reply_keys, vec!["github:ic:5".to_string()]);
    }

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

    /// Builds a full `GithubPrInboundRouter` for the method below, the one
    /// that reads every field. Every port it skips is wired to a lazily
    /// connected pool or an unused mock — no live Postgres needed.
    pub(crate) mod notify_and_maybe_close_tests {
        use std::sync::Arc;

        use async_trait::async_trait;
        use chrono::Utc;
        use sqlx::postgres::PgPoolOptions;

        use moatless_ports::identity::MockProviderIdentityLink;
        use moatless_ports::sandbox::SandboxWaker;
        use moatless_ports::task::{
            MockMessageAcceptor, MockRelationRecorder, MockTaskFinisher, MockTaskOpener,
            MockTaskReader, MockTranscriptAppend,
        };
        use moatless_types::access_control::Scope;
        use moatless_types::adapter::MessageBufferConfig;
        use moatless_types::agent::AgentProfile;
        use moatless_types::loops::RoutingMode;

        use super::super::*;
        use crate::router::MockReviewGuideTasks;

        /// Never dispatched by `notify_and_maybe_close_for_pr`: the field only
        /// exists to satisfy `RoutingInputs`, which this test's unused
        /// `RoutingService` needs to type-check.
        struct UnusedWaker;

        #[async_trait]
        impl SandboxWaker for UnusedWaker {
            async fn wake(&self, _task_id: &str) -> Result<(), String> {
                panic!("not exercised by notify_and_maybe_close_for_pr")
            }
        }

        /// A pool that never connects: every field unread by
        /// `notify_and_maybe_close_for_pr` stays this cheap. Its one write,
        /// `record_inbound_event` inside `append_to_binding`, already
        /// tolerates failure (`warn!` then `Ok(())` either way), so a refused
        /// loopback connection is as good as a real table.
        fn unconnected_pool() -> sqlx::PgPool {
            PgPoolOptions::new()
                .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/postgres")
                .expect("lazy connect never dials out")
        }

        fn rule(close_task_on_close: bool) -> SubscriptionMatch {
            SubscriptionMatch {
                name: "test-loop".to_string(),
                id: "ir_1".into(),
                adapter_kind: AdapterKind::Github,
                connection_id: "pc_test".into(),
                source_matcher: "soaplabs/repo".into(),
                source_name: None,
                routing_mode: RoutingMode::BySubject,
                event_filter: serde_json::json!([]),
                workspace_id: "ws-1".into(),
                prompt_template: None,
                task_name: None,
                tag_ids: vec![],
                memory_preload: Vec::new(),
                skills: Vec::new(),
                run_as_user_id: Some("alice".into()),
                agent_profile: AgentProfile::ClaudeCode,
                model: None,
                close_after_settled_days: None,
                settle_after_hours: None,
                node_selector: None,
                stop_when_done: None,
                message_buffer_seconds: None,
                allow_direct_send: false,
                adapter_config: None,
                ongoing_task_id: None,
                close_task_on_close,
                notify_on_success: false,
                active: true,
                created_by: "alice".into(),
                scope: Scope::User,
                owner_user_id: Some("alice".into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }

        fn binding() -> TaskBinding {
            TaskBinding {
                id: "tb_1".into(),
                task_id: "task-1".into(),
                adapter_kind: AdapterKind::GithubPr,
                subject_kind: SUBJECT_KIND.into(),
                subject_key: "soaplabs/repo#42".into(),
                connection_id: "pc_test".into(),
                loop_id: Some("ir_1".into()),
                destination_config: serde_json::json!({}),
                metadata: serde_json::json!({}),
                last_external_event_id: None,
                origin: ADAPTER_EVENT_BINDING_ORIGIN.into(),
                created_at: Utc::now(),
                last_event_at: Utc::now(),
                external_status: None,
                external_state_kind: None,
                external_status_raw: None,
                external_review_decision: None,
                external_mergeable_state: None,
                external_status_updated_at: None,
                active: true,
                deactivated_at: None,
                watching: true,
            }
        }

        fn close_info() -> PrCloseInfo {
            PrCloseInfo {
                pr_number: 42,
                merged: false,
                title: "feat: land it".into(),
                html_url: "https://github.com/soaplabs/repo/pull/42".into(),
                sender_login: "octocat".into(),
                pr_author_login: None,
            }
        }

        /// `message_send` and `task_closer` carry the test's assertions; every
        /// other port is real-but-unqueried (the DB ones) or a mock with no
        /// expectations (never called on this path).
        fn router(
            message_send: MockMessageAcceptor,
            task_closer: MockTaskFinisher,
        ) -> GithubPrInboundRouter {
            router_with(
                message_send,
                task_closer,
                MockTaskReader::new(),
                MockReviewGuideTasks::new(),
            )
        }

        pub(crate) fn router_with(
            message_send: MockMessageAcceptor,
            task_closer: MockTaskFinisher,
            tasks: MockTaskReader,
            review_guides: MockReviewGuideTasks,
        ) -> GithubPrInboundRouter {
            let pool = unconnected_pool();
            let connections = moatless_adapter_model::compose::adapter_connections(pool.clone());
            let loop_graph = moatless_loops::compose::loop_graph(pool);

            let mut users = MockProviderIdentityLink::new();
            users.expect_user_with_login().returning(|_, _| Ok(None));

            let routing = loop_graph.routing(moatless_loops::compose::RoutingInputs {
                adapter_connections: connections.routing(),
                messages: Arc::new(MockTranscriptAppend::new()),
                tasks: Arc::new(MockTaskReader::new()),
                task_creation: Arc::new(MockTaskOpener::new()),
                message_send: Arc::new(MockMessageAcceptor::new()),
                sandbox_waker: Arc::new(UnusedWaker),
                adapters: None,
                message_buffer: MessageBufferConfig::default(),
            });

            GithubPrInboundRouter::new(
                connections.routing(),
                loop_graph.loop_reader(),
                loop_graph.bindings(),
                Arc::new(users),
                routing,
                Arc::new(MockTranscriptAppend::new()),
                Arc::new(MockRelationRecorder::new()),
                Arc::new(tasks),
                Arc::new(MockTaskOpener::new()),
                Arc::new(message_send),
                Arc::new(task_closer),
                Arc::new(review_guides),
                Arc::new(crate::check_run_trigger::MockTriggerWorkspaces::new()),
                None,
            )
        }

        /// A trigger's close reaches the Tasks it created; a Task connected to
        /// the pull request closes only when its Binding opted in.
        #[test]
        fn a_pr_close_reaches_the_triggers_tasks_and_opted_in_bindings() {
            let rule = rule(true);
            let created = binding();
            assert!(closes_with_pr(&rule, true, &created));
            assert!(!closes_with_pr(&rule, false, &created));

            let mut connected = TaskBinding {
                origin: CONNECTED_THREAD_BINDING_ORIGIN.into(),
                loop_id: Some("ir_1".into()),
                ..binding()
            };
            assert!(!closes_with_pr(&rule, true, &connected));
            connected.metadata = serde_json::json!({ "close_on_subject_close": true });
            assert!(closes_with_pr(&rule, false, &connected));
            connected.active = false;
            assert!(!closes_with_pr(&rule, true, &connected));
        }

        /// The bug this guards: a spared Task must still hear its PR closed
        /// — `close_subject` used to build and discard the notice before the
        /// sibling check ran — and the text must say it stayed open, not
        /// that it auto-closed, which a call-count assertion cannot see.
        #[tokio::test]
        async fn a_spared_task_still_receives_the_close_notice() {
            let mut message_send = MockMessageAcceptor::new();
            message_send
                .expect_accept()
                .withf(|turn: &InboundTurn| {
                    turn.text
                        .contains("Task left open: it has other connected bindings.")
                        && !turn.text.contains("auto-closed")
                })
                .times(1)
                .returning(|_| Ok(()));
            let mut task_closer = MockTaskFinisher::new();
            task_closer.expect_close().never();

            let router = router(message_send, task_closer);
            router
                .notify_and_maybe_close_for_pr(
                    &rule(true),
                    &close_info(),
                    "soaplabs",
                    "repo",
                    &binding(),
                    2, // a sibling Binding survives the close
                )
                .await;
        }

        /// An opted-in Task closes with its last open pull request, whatever
        /// else it is bound to; one that did not opt in counts every Binding.
        #[test]
        fn only_an_open_pull_request_holds_an_opted_in_task_open() {
            let pr = |id: &str, key: &str, metadata: serde_json::Value| TaskBinding {
                id: id.into(),
                adapter_kind: AdapterKind::Github,
                subject_key: key.into(),
                origin: CONNECTED_THREAD_BINDING_ORIGIN.into(),
                metadata,
                ..binding()
            };
            let opted = serde_json::json!({ "close_on_subject_close": true });
            let closing = pr("tb_a", "soaplabs/repo#1", opted.clone());
            let slack = TaskBinding {
                id: "tb_slack".into(),
                adapter_kind: AdapterKind::Slack,
                subject_kind: "thread".into(),
                ..binding()
            };
            let closed = pr(
                "tb_b",
                "soaplabs/repo#2",
                serde_json::json!({ "pr_closed": true }),
            );
            let open = pr("tb_c", "soaplabs/repo#3", serde_json::json!({}));
            let reopened = pr(
                "tb_d",
                "soaplabs/repo#4",
                serde_json::json!({ "pr_closed": false }),
            );

            let rows = [closing.clone(), slack.clone(), closed.clone()];
            assert_eq!(bindings_holding_open(&closing, &rows), 1);
            let rows = [closing.clone(), slack.clone(), closed, open, reopened];
            assert_eq!(bindings_holding_open(&closing, &rows), 3);

            let not_opted = pr("tb_a", "soaplabs/repo#1", serde_json::json!({}));
            assert_eq!(
                bindings_holding_open(&not_opted, &[not_opted.clone(), slack]),
                2
            );
        }

        /// An opted-in Task spared by a sibling Binding is woken by the close
        /// notice, since the merged notice skipped it; a trigger's Task is not.
        #[tokio::test]
        async fn a_spared_opted_in_task_is_woken_by_the_close_notice() {
            for (opted_in, wakes) in [(true, true), (false, false)] {
                let mut message_send = MockMessageAcceptor::new();
                message_send
                    .expect_accept()
                    .withf(move |turn: &InboundTurn| turn.resume == wakes)
                    .times(1)
                    .returning(|_| Ok(()));
                let mut task_closer = MockTaskFinisher::new();
                task_closer.expect_close().never();
                let mut bound = binding();
                if opted_in {
                    bound.origin = CONNECTED_THREAD_BINDING_ORIGIN.into();
                    bound.metadata = serde_json::json!({ "close_on_subject_close": true });
                }

                router(message_send, task_closer)
                    .notify_and_maybe_close_for_pr(
                        &rule(true),
                        &close_info(),
                        "soaplabs",
                        "repo",
                        &bound,
                        2,
                    )
                    .await;
            }
        }

        /// The Task with no surviving sibling still gets both: the notice,
        /// worded as a close, and the close itself.
        #[tokio::test]
        async fn a_sole_binding_is_notified_and_closed() {
            let mut message_send = MockMessageAcceptor::new();
            message_send
                .expect_accept()
                .withf(|turn: &InboundTurn| {
                    turn.text
                        .contains("Task auto-closed because the PR was closed.")
                        && !turn.text.contains("left open")
                })
                .times(1)
                .returning(|_| Ok(()));
            let mut task_closer = MockTaskFinisher::new();
            task_closer.expect_close().times(1).returning(|_, _| Ok(()));

            let router = router(message_send, task_closer);
            router
                .notify_and_maybe_close_for_pr(
                    &rule(true),
                    &close_info(),
                    "soaplabs",
                    "repo",
                    &binding(),
                    1, // no sibling — the only Binding on this subject
                )
                .await;
        }
    }

    #[test]
    fn an_org_wildcard_rule_names_the_events_repository() {
        let mut rule = delivery_only_rule(&connection(None), "acme/*");
        rule.source_matcher = super::org_wildcard("acme");
        assert_eq!(
            super::for_repository(rule.clone(), "acme/widget").source_matcher,
            "acme/widget"
        );
        rule.source_matcher = "acme/gadget".to_string();
        assert_eq!(
            super::for_repository(rule, "acme/widget").source_matcher,
            "acme/gadget",
            "an exact rule keeps its own matcher"
        );
    }

    #[test]
    fn a_pr_opening_creates_nothing_under_a_check_run_loop() {
        let mut rule = delivery_only_rule(&connection(None), "acme/widget");
        assert!(super::opening_creates_task(&rule));
        rule.adapter_config = Some(serde_json::json!({"create_on": "check_run"}));
        assert!(!super::opening_creates_task(&rule));
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
