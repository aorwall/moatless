//! Shared binding runtime backed by adapter connections, loops, subscriptions,
//! and task bindings.

use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::adapters::AdapterInboundPresentation;
use crate::adapters::Adapters;
use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::delivery::AdapterDeliveryFailure;
use crate::adapters::error::{AdapterError, SharedAdapterError};
use crate::bindings::metrics::{inbound_reason, record_inbound};
use crate::bindings::recipients::{EventProvenance, resolve_recipients};
use crate::loops::dao::LoopDao;
use crate::loops::firing::inbound;
use crate::loops::firing::receipts::EventReceiptDao;
use moatless_adapter_model::models::{parse_adapter_kind, AdapterConnection, SubscriptionMatch, TaskBinding, binding_context, subscription_context};
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::RoutingMode;
use moatless_types::task::TaskStatus;
use moatless_types::access_control::ScopeContext;
use moatless_backend_core::task::TranscriptAppend;
use moatless_tasks::message::send::MessageSendService;
use crate::bindings::dao::TaskBindingDao;
use moatless_tasks::task::creation::{TaskCreationError, TaskCreationService};
use moatless_backend_core::task::TaskReader;
use moatless_backend_core::sandbox::{SandboxWaker, wake_for_message};

pub use moatless_adapter_model::models::{ADAPTER_EVENT_BINDING_ORIGIN as ORIGIN_ADAPTER_EVENT, CONNECTED_THREAD_BINDING_ORIGIN as ORIGIN_CONNECTED_THREAD};

/// The inbound-routing subgraph, owning the ports it reads so the routing logic
/// names none of them through `AppState`.
///
/// Held on `AppState` as `state.routing` and built once at the composition root.
/// The durable inbound-delivery handler holds a single `Arc<RoutingService>`
/// rather than a field per port, and `subject_events` routes through it too. The
/// provider-facing operations (`parent_author_task_id`, `external_id_aliases`,
/// `upsert_binding`) are reached through the ports they read directly — the
/// `pub(crate)` methods on `RoutingService` and the `upsert_binding_into` core
/// below — so provider callers pass those ports and this module names no
/// `AppState`. See `docs/deferred-changes.md`.
pub struct RoutingService {
    adapter_connections: Arc<dyn AdapterConnectionDao>,
    loops: Arc<LoopDao>,
    task_bindings: Arc<dyn TaskBindingDao>,
    messages: Arc<dyn TranscriptAppend>,
    tasks: Arc<dyn TaskReader>,
    task_creation: Arc<TaskCreationService>,
    message_send: Arc<MessageSendService>,
    event_receipts: Arc<dyn EventReceiptDao>,
    sandbox_waker: Arc<dyn SandboxWaker>,
    adapters: Option<Arc<Adapters>>,
}

/// The connection or Loop a record was admitted against is gone.
///
/// A synthesized condition, not a wrapped failure: there is no underlying
/// error to retain, and no retry can bring the row back.
#[derive(Debug, thiserror::Error)]
#[error("the {what} (`{id}`) this delivery was admitted for is no longer available")]
struct Vanished {
    what: &'static str,
    id: String,
}

impl RoutingService {
    #[expect(
        clippy::too_many_arguments,
        reason = "ten independent ports the inbound-routing subgraph reads; they \
                  are exactly the AppState fields this service absorbs, wired in \
                  from the composition root, and there is no narrower grouping \
                  that is not itself an AppState-shaped bag"
    )]
    pub fn new(
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<LoopDao>,
        task_bindings: Arc<dyn TaskBindingDao>,
        messages: Arc<dyn TranscriptAppend>,
        tasks: Arc<dyn TaskReader>,
        task_creation: Arc<TaskCreationService>,
        message_send: Arc<MessageSendService>,
        event_receipts: Arc<dyn EventReceiptDao>,
        sandbox_waker: Arc<dyn SandboxWaker>,
        adapters: Option<Arc<Adapters>>,
    ) -> Self {
        Self {
            adapter_connections,
            loops,
            task_bindings,
            messages,
            tasks,
            task_creation,
            message_send,
            event_receipts,
            sandbox_waker,
            adapters,
        }
    }

    /// Re-read the connection and Loop a durable inbound delivery was admitted
    /// against.
    ///
    /// Read at execution time rather than captured at admission so the work runs
    /// against the current configuration, and so a deleted Loop ends the record
    /// instead of routing through a stale copy of itself.
    pub async fn resolve_connection_and_subscription(
        &self,
        connection_id: &str,
        loop_id: &str,
    ) -> Result<(AdapterConnection, SubscriptionMatch), AdapterDeliveryFailure> {
        // reason: a durable Adapter delivery runs in the worker with no user session;
        // the connection is named by an already-admitted record, not by caller
        // input.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let connection = self
            .adapter_connections
            .find_by_id(connection_id, &sys_ctx)
            .await
            .map_err(AdapterDeliveryFailure::retryable)?
            .ok_or_else(|| {
                AdapterDeliveryFailure::permanent(Vanished {
                    what: "adapter connection",
                    id: connection_id.to_string(),
                })
            })?;

        let subscription = self
            .loops
            .find_runnable_subscription_by_id(loop_id)
            .await
            .map_err(AdapterDeliveryFailure::retryable)?
            .ok_or_else(|| {
                AdapterDeliveryFailure::permanent(Vanished {
                    what: "Loop",
                    id: loop_id.to_string(),
                })
            })?;

        Ok((connection, subscription))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one inbound event plus the routing context it arrived with — \
                  connection, subscription, labels, subject, and destination \
                  config — none of which is derivable from another"
    )]
    pub(crate) async fn route_inbound_message(
        &self,
        connection: &AdapterConnection,
        subscription: &SubscriptionMatch,
        msg: InboundMessage,
        adapter_label: &str,
        subject_kind: &str,
        subject_key: &str,
        destination_config: serde_json::Value,
    ) -> Result<String, AdapterError> {
        if !subscription.active {
            debug!(subscription_id = %subscription.id, "Ignoring external message: task trigger inactive");
            record_inbound(
                adapter_label,
                "ignored",
                inbound_reason::INACTIVE_RULE,
                &subscription.id,
                &connection.id,
            );
            return Err(SharedAdapterError::ConfigNotFound(subscription.id.clone()).into());
        }

        let binding_repo = self.task_bindings.clone();
        // Provenance: the task an adapter recovered from the event's own body (the
        // `Moatless task:` footer), else the task an already-recorded event
        // identifier names. Never an identity.
        let origin_task_id = match msg.source_task_id.clone() {
            Some(task_id) => Some(task_id),
            None => match msg.external_event_id.as_deref() {
                Some(ext_id) => {
                    let candidates = self.external_id_aliases(adapter_label, ext_id);
                    self.messages
                        .find_by_external_event_id(&candidates)
                        .await?
                }
                None => None,
            },
        };

        let mut bindings = binding_repo
            .find_by_subject(adapter_label, subject_kind, subject_key)
            .await?;

        // Skip bindings whose task has closed — a closed task is terminal and must
        // not be resurrected by a new inbound message. Fresh bindings for the
        // originating / ongoing task are re-added active below.
        bindings.retain(|b| b.active);

        if let Some(origin_task_id) = origin_task_id.as_deref() {
            if !bindings.iter().any(|s| s.task_id == origin_task_id) {
                let sub = upsert_binding_into(
                    self.task_bindings.as_ref(),
                    connection,
                    Some(subscription),
                    origin_task_id,
                    adapter_label,
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

        if subscription.routing_mode == RoutingMode::Ongoing {
            if let Some(task_id) = self.open_ongoing_task_id(subscription).await {
                if !bindings.iter().any(|s| s.task_id == task_id) {
                    // Pass None so a newly attached binding starts with no cursor —
                    // `deliver_to_binding` then fetches the full prior thread as
                    // context, and `touch_cursor` advances it after delivery. For an
                    // existing binding the COALESCE in the upsert preserves the
                    // current cursor.
                    let sub = upsert_binding_into(
                        self.task_bindings.as_ref(),
                        connection,
                        Some(subscription),
                        &task_id,
                        adapter_label,
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
        if self
            .should_create_binding_task(subscription, &bindings)
            .await?
        {
            // Reading the bindings above and creating below are two statements,
            // not one transaction, and the backend runs more than one replica.
            // Two events for the same subject arriving together therefore both
            // read no binding and both create — which is not hypothetical: an
            // @mention reaches Slack's adapter as two events carrying the same
            // message ts, ~300ms apart, and produced two tasks and two replies.
            //
            // The claim is the decision. Exactly one delivery lands the row;
            // the rest are told to retry, and by then the winner's binding is
            // there to receive them. A duplicate event is then dropped by the
            // event-receipt table, and a genuinely different message is
            // delivered to the task the winner made. Nothing is lost either way.
            if !self
                .event_receipts
                .claim_task_creation(
                    &subscription.id,
                    subject_kind,
                    subject_key,
                    adapter_label,
                    msg.external_event_id.as_deref(),
                )
                .await?
            {
                debug!(
                    subscription_id = %subscription.id,
                    subject_key,
                    provider = adapter_label,
                    "Lost the task-creation claim for this subject; retrying to reach the winner"
                );
                record_inbound(
                    adapter_label,
                    "retry",
                    inbound_reason::CREATION_CLAIMED,
                    &subscription.id,
                    &connection.id,
                );
                return Err(AdapterError::CreationClaimed {
                    subject_kind: subject_kind.to_string(),
                    subject_key: subject_key.to_string(),
                });
            }

            // The earliest binding for this subject (if any) belongs to the task
            // that originated it — typically a `connected_thread` binding placed
            // when an earlier task produced the resource. We link the new task
            // to it as `related_to` (peer sharing the same external artifact),
            // not as a parent.
            let related = bindings
                .first()
                .map(|b| moatless_types::task::RelatedTask {
                    related_task_id: b.task_id.clone(),
                    context: serde_json::json!({
                        "via": "binding_webhook",
                        "adapter": adapter_label,
                        "subject_kind": subject_kind,
                        "subject_key": subject_key,
                        "origin_binding_id": b.id,
                    }),
                });
            let config = subscription_context(subscription);
            // Resolve the bound adapter's reply instructions from the registry
            // and pass them to the firing entry point as a value.
            let reply_instructions = self
                .adapters
                .as_ref()
                .and_then(|providers| providers.inbound_presentation_by_name(&config.adapter_kind))
                .and_then(AdapterInboundPresentation::reply_instructions);
            // The creation service records the `related_to` relation and provisions
            // the sandbox as part of the owned sequence.
            match inbound::create_task_for_external_message(
                &self.task_creation,
                &config,
                &msg,
                related,
                reply_instructions,
            )
            .await
            {
                Ok(outcome) => {
                    let task_id = outcome.task.task_id;
                    if subscription.routing_mode == RoutingMode::Ongoing {
                        self.loops
                            .set_ongoing_task(&subscription.id, &task_id)
                            .await?;
                    }
                    // Seed the cursor as None so `fetch_context` retrieves the full
                    // prior thread on the first delivery; `touch_cursor` advances it
                    // afterward.
                    let sub = upsert_binding_into(
                        self.task_bindings.as_ref(),
                        connection,
                        Some(subscription),
                        &task_id,
                        adapter_label,
                        subject_kind,
                        subject_key,
                        destination_config.clone(),
                        ORIGIN_ADAPTER_EVENT,
                        None,
                    )
                    .await?;
                    new_task_binding_id = Some(sub.id.clone());
                    bindings.push(sub);
                    // Point the claim at the task it produced. A later delivery
                    // that loses the claim can then be told what won it.
                    self.event_receipts
                        .record_claimed_task(
                            &subscription.id,
                            subject_kind,
                            subject_key,
                            &task_id,
                        )
                        .await?;
                    info!(task_id = %task_id, subscription_id = %subscription.id, provider = adapter_label, "Created task from provider event");
                }
                // Permanent failure (e.g. a bad GitHub token). The service already
                // persisted the task as errored + published a `SandboxStatus::Error`
                // event. Skip the new-task binding and its delivery, but fall through
                // to existing-binding delivery and return `Ok` — returning a 5xx here
                // makes sources like Slack redeliver, and every redelivery inserts
                // another errored row.
                Err(TaskCreationError::Preflight(reason)) => {
                    // The claim bought the right to create and no task came of
                    // it. Hand it back, or every later event for this subject
                    // loses a claim held by nothing and retries until its
                    // budget runs out.
                    self.event_receipts
                        .release_claim(&subscription.id, subject_kind, subject_key)
                        .await?;
                    warn!(subscription_id = %subscription.id, provider = adapter_label, reason = %reason, "Skipping task creation from provider event: preflight failed");
                }
                // Transient failure (Db / ShortId / RepoNotFound) — propagate so the
                // source retries, exactly as before this migration.
                Err(e) => {
                    // Transient, so the sender retries — but the claim must not
                    // outlive the attempt that took it, or the retry loses to
                    // itself.
                    self.event_receipts
                        .release_claim(&subscription.id, subject_kind, subject_key)
                        .await?;
                    return Err(AdapterError::TaskCreation(e));
                }
            }
        }

        self.fan_out(
            connection,
            subscription,
            &msg,
            Fanout {
                adapter_label,
                binding_provider: adapter_label,
                subject_kind,
                subject_key,
                bindings,
                author_task_id: origin_task_id.as_deref(),
                new_task_binding_id: new_task_binding_id.as_deref(),
            },
        )
        .await
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
    pub(crate) async fn fan_out(
        &self,
        connection: &AdapterConnection,
        subscription: &SubscriptionMatch,
        msg: &InboundMessage,
        subject: Fanout<'_>,
    ) -> Result<String, AdapterError> {
        let Fanout {
            adapter_label,
            binding_provider,
            subject_kind,
            subject_key,
            bindings,
            author_task_id,
            new_task_binding_id,
        } = subject;

        let parent_author_task_id = self
            .parent_author_task_id(adapter_label, &msg.audience)
            .await?;

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
        let receipts = self.event_receipts.clone();
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
                    provider = adapter_label,
                    "Skipping binding: the task authored this event"
                );
                record_inbound(
                    adapter_label,
                    "ignored",
                    inbound_reason::SELF_ECHO,
                    &subscription.id,
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
                    provider = adapter_label,
                    "Skipping binding: not in this event's audience"
                );
                record_inbound(
                    adapter_label,
                    "ignored",
                    inbound_reason::NOT_IN_AUDIENCE,
                    &subscription.id,
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
                        provider = adapter_label,
                        "Skipping duplicate external event receipt"
                    );
                    record_inbound(
                        adapter_label,
                        "ignored",
                        inbound_reason::DUPLICATE_RECEIPT,
                        &subscription.id,
                        &connection.id,
                    );
                    delivered_task_id = Some(binding.task_id.clone());
                    passed_over.push((binding.task_id, inbound_reason::DUPLICATE_RECEIPT));
                    continue;
                }
            }

            let this_is_new = new_task_binding_id == Some(binding.id.as_str());
            match self
                .deliver_to_binding(subscription, &binding, msg, adapter_label, this_is_new)
                .await
            {
                Ok(()) => {
                    let reason = if this_is_new {
                        inbound_reason::NEW_TASK
                    } else {
                        inbound_reason::EXISTING_TASK
                    };
                    record_inbound(
                        adapter_label,
                        "success",
                        reason,
                        &subscription.id,
                        &connection.id,
                    );
                }
                Err(e) => {
                    record_inbound(
                        adapter_label,
                        "error",
                        inbound_reason::DELIVER_FAILED,
                        &subscription.id,
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
            return Err(SharedAdapterError::ConfigNotFound(subscription.id.clone()).into());
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
            provider = adapter_label,
            active_bindings,
            passed_over = %reasons,
            "Event reached no task: every binding on the subject was passed over"
        );
        record_inbound(
            adapter_label,
            "error",
            inbound_reason::NO_RECIPIENTS,
            &subscription.id,
            &connection.id,
        );
        // Permanent by classification (`adapters/delivery`): the same event walking the
        // same bindings is passed over the same way, so retrying only burns the
        // budget. The record lands in `failed` carrying this message, which is the
        // point — the event is not lost quietly.
        Err(SharedAdapterError::NotFound(format!(
            "no task binding on {subject_key} could receive this {adapter_label} event \
             ({active_bindings} passed over: {reasons})"
        ))
        .into())
    }

    /// The task that authored the parent event an [`Audience::ParentAuthor`] names,
    /// or `None` for every other audience and for a parent no task authored.
    ///
    /// This lookup sits above [`resolve_recipients`] because the resolver performs
    /// no input or output of its own — see [`crate::bindings::recipients`]. An
    /// audience that names no parent asks nothing of the database.
    pub async fn parent_author_task_id(
        &self,
        adapter_label: &str,
        audience: &Audience,
    ) -> Result<Option<String>, AdapterError> {
        let Audience::ParentAuthor {
            parent_external_event_id: Some(parent_id),
        } = audience
        else {
            return Ok(None);
        };
        let candidates = self.external_id_aliases(adapter_label, parent_id);
        Ok(self
            .messages
            .find_by_external_event_id(&candidates)
            .await?)
    }

    pub fn external_id_aliases(&self, adapter_label: &str, external_id: &str) -> Vec<String> {
        self.adapters
            .as_ref()
            .and_then(|providers| providers.inbound_presentation_by_name(adapter_label))
            .map_or_else(
                || vec![external_id.to_string()],
                |presentation| presentation.external_id_aliases(external_id),
            )
    }

    async fn should_create_binding_task(
        &self,
        subscription: &SubscriptionMatch,
        bindings: &[TaskBinding],
    ) -> Result<bool, AdapterError> {
        if subscription.routing_mode != RoutingMode::Ongoing {
            return Ok(bindings.is_empty());
        }

        if !bindings.is_empty() {
            return Ok(false);
        }
        Ok(self.open_ongoing_task_id(subscription).await.is_none())
    }

    async fn open_ongoing_task_id(&self, subscription: &SubscriptionMatch) -> Option<String> {
        let task_id = subscription.ongoing_task_id.as_deref()?;
        #[allow(clippy::disallowed_methods)]
        // reason: event routing is a background path with no caller identity
        let sys = ScopeContext::system();
        match self.tasks.describe(task_id, &sys).await {
            Ok(Some(task)) if task.status == TaskStatus::Open => Some(task.task_id),
            _ => None,
        }
    }

    async fn deliver_to_binding(
        &self,
        subscription: &SubscriptionMatch,
        binding: &TaskBinding,
        msg: &InboundMessage,
        adapter_label: &str,
        is_new_task: bool,
    ) -> Result<(), AdapterError> {
        let config = subscription_context(subscription);
        let thread = binding_context(binding);

        let thread_context = if msg.thread_id.is_some() {
            if let Some(ref providers) = self.adapters {
                if let Some(presentation) = providers.inbound_presentation_by_name(adapter_label) {
                    let mut exclude_ids = self
                        .messages
                        .external_event_ids(&binding.task_id)
                        .await
                        .unwrap_or_default();
                    if let Some(ref ext_id) = msg.external_event_id {
                        exclude_ids.push(ext_id.clone());
                    }
                    presentation
                        .fetch_context(&thread, &config, 20, &exclude_ids)
                        .await
                        .unwrap_or_else(|e| {
                            warn!(provider = adapter_label, error = %e, "Failed to fetch thread context");
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

        if let Some(prefix) = self
            .adapters
            .as_ref()
            .and_then(|providers| providers.inbound_presentation_by_name(adapter_label))
            .and_then(|presentation| presentation.inbound_message_prefix(msg))
        {
            message_text = format!("{prefix}\n\n{message_text}");
        }

        if msg.thread_id.is_some() {
            if let Some(providers) = self.adapters.as_ref() {
                if let Some(presentation) = providers.inbound_presentation_by_name(adapter_label) {
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

        self.message_send
            .send_message_flow(
                &binding.task_id,
                &message_text,
                None,
                None,
                None,
                &msg.file_ids,
                &[],
                true,
                moatless_types::task::ExternalOrigin {
                    event_id: msg.external_event_id.as_deref(),
                    adapter: Some(adapter_label),
                    subject_key: Some(&binding.subject_key),
                    // The source's own name where the subscription carries one, else the
                    // matcher that identified it — a repository full name, a channel id.
                    // A label a reader recognises beats no label at all.
                    source_name: Some(
                        config
                            .source_name
                            .as_deref()
                            .unwrap_or(config.source_id.as_str()),
                    ),
                    author: msg.author.as_deref(),
                    sender_id: msg.sender_external_id.as_deref(),
                    source_task_id: None,
                },
                false,
                Some(msg.user_id.as_str()),
                // A `routing_mode: ongoing` Loop fires into a task that is already
                // open, so the delivery resumes and the task's own skills are held
                // back. The Loop still means them for every firing; send them here.
                &config.skills,
                None,
            )
            .await
            .map_err(AdapterError::MessageDelivery)?;

        let repo = self.task_bindings.clone();
        if let Some(ref ext_id) = msg.external_event_id {
            repo.touch_cursor(&binding.id, ext_id).await?;
        } else {
            repo.touch_last_event(&binding.id).await?;
        }

        if !is_new_task {
            if let Some(e) = wake_for_message(self.sandbox_waker.as_ref(), &binding.task_id).await {
                warn!(task_id = %binding.task_id, error = %e, "Failed to ensure sandbox for external message");
            }
        }

        Ok(())
    }
}

pub async fn resolve_chat_provider_event(
    adapter_connections: &dyn AdapterConnectionDao,
    loops: &LoopDao,
    provider: &str,
    scope_value: &str,
    source_id: &str,
) -> Result<Option<(AdapterConnection, SubscriptionMatch)>, AdapterError> {
    let connection_repo = adapter_connections;
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

    let subscription_repo = loops;
    let subscription = subscription_repo
        .find_subscription_by_source_matcher(provider, source_id)
        .await?
        .or(subscription_repo
            .find_subscription_wildcard(provider)
            .await?);
    let Some(subscription) = subscription else {
        record_inbound(
            provider,
            "ignored",
            inbound_reason::NO_RULE_MATCH,
            "",
            &connection.id,
        );
        return Ok(None);
    };

    if subscription.connection_id != connection.id {
        debug!(
            provider,
            source_id,
            subscription_id = %subscription.id,
            connection_id = %connection.id,
            "Task trigger belongs to a different connection"
        );
        record_inbound(
            provider,
            "ignored",
            inbound_reason::WRONG_CONNECTION,
            &subscription.id,
            &connection.id,
        );
        return Ok(None);
    }

    Ok(Some((connection, subscription)))
}

/// One event, the bindings on its subject, and what routing already knows about
/// who wrote it.
///
/// Assembling the set is the caller's job because it is the one part of routing
/// that is not shared: a chat adapter spawns a task for a subject nothing is
/// bound to, the GitHub PR adapter spawns one only when a pull request opens.
/// Deciding who among them receives the event is not — that is
/// [`RoutingService::fan_out`].
pub struct Fanout<'a> {
    /// The adapter label the event is delivered and metered under: the value
    /// `task_messages.external_adapter` carries, and the key the adapter
    /// registry answers to for external-id aliases.
    pub adapter_label: &'a str,
    /// The provider key the subject's delivery receipts are stored under — the
    /// same key its bindings are. Equal to `adapter_label` for every adapter
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

/// Upsert a canonical task binding through the one port it needs — the
/// `task_bindings` DAO; every other parameter is binding-identity data. Callers
/// hold the port and pass it directly, so this names no `AppState`.
#[expect(
    clippy::too_many_arguments,
    reason = "nine binding-identity data parameters plus the one task_bindings \
              port; there is no AppState or port bundle to hoist, so the width is \
              the binding's own identity threaded to the DAO"
)]
pub async fn upsert_binding_into(
    task_bindings: &dyn TaskBindingDao,
    connection: &AdapterConnection,
    subscription: Option<&SubscriptionMatch>,
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
        adapter_kind: parse_adapter_kind(provider),
        subject_kind: subject_kind.to_string(),
        subject_key: subject_key.to_string(),
        connection_id: connection.id.clone(),
        loop_id: subscription.map(|r| r.id.clone()),
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
    Ok(task_bindings.upsert(&binding).await?)
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

    /// A `Vanished` names the row that is gone so an abandoned delivery says why,
    /// not just that it failed.
    #[test]
    fn a_vanished_row_names_what_is_missing() {
        let failure = super::AdapterDeliveryFailure::permanent(super::Vanished {
            what: "Loop",
            id: "tt_gone".into(),
        });
        let source = std::error::Error::source(&failure).expect("the cause is retained");
        let rendered = source.to_string();
        assert!(rendered.contains("Loop"), "{rendered}");
        assert!(rendered.contains("tt_gone"), "{rendered}");
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
        let err = AdapterError::TaskCreation(
            moatless_tasks::task::creation::TaskCreationError::Db(sqlx::Error::PoolClosed),
        );

        assert!(
            err.source()
                .and_then(|s| s.downcast_ref::<moatless_tasks::task::creation::TaskCreationError>())
                .is_some()
        );
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
        let err = AdapterError::MessageDelivery(moatless_tasks::message::error::MessageError::Db(
            sqlx::Error::PoolClosed,
        ));

        assert!(
            err.source()
                .and_then(|s| s.downcast_ref::<moatless_tasks::message::error::MessageError>())
                .is_some()
        );
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
