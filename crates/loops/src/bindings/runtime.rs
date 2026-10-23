//! Shared binding runtime backed by adapter connections, loops, subscriptions,
//! and task bindings.

use std::sync::Arc;

use moatless_types::adapter::MessageBufferConfig;
use tracing::{debug, info, warn};

use crate::bindings::buffer::dao::{BufferedBatch, MessageBufferStore};
use crate::bindings::buffer::policy::buffer_window;
use crate::bindings::buffer::{Part, combine};
use crate::bindings::operations::Bindings;
use crate::bindings::recipients::{EventProvenance, resolve_recipients};
use crate::loops::dao::LoopDao;
use crate::loops::firing::inbound;
use crate::loops::firing::receipts_dao::EventReceipts;
use moatless_adapter_model::binding::{BindScope, binding_context, hears_only_replies};
use moatless_adapter_model::connections::ConnectionRouting;
use moatless_adapter_model::delivery::AdapterDeliveryFailure;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::{delivery_only_subscription, subscription_context};
use moatless_adapter_model::metrics::{inbound_reason, record_inbound};
use moatless_adapter_model::registry::Adapters;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::MessageAcceptor;
use moatless_ports::task::TaskReader;
use moatless_ports::task::TranscriptAppend;
use moatless_ports::task::{EventSubject, InboundTurn, TaskOpenError, TaskOpener, TurnOrigin};
use moatless_tasks::message::wake::wake_for_message;
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_types::adapter::{Audience, InboundMessage};
use moatless_types::loops::RoutingMode;
use moatless_types::loops::SubscriptionMatch;
use moatless_types::task::TaskStatus;

pub use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN as ORIGIN_ADAPTER_EVENT;
pub use moatless_adapter_model::binding::CONNECTED_THREAD_BINDING_ORIGIN as ORIGIN_CONNECTED_THREAD;

/// Earlier thread messages quoted above a delivered turn, on top of the turn's
/// own events. The batch excludes itself from the fetch, so a limit that did not
/// grow with it would return nothing but the events already being delivered.
const THREAD_CONTEXT_MESSAGES: usize = 20;

/// The inbound-routing subgraph, owning the ports it reads so the routing
/// logic names none of them through `AppState`.
///
/// Held on `AppState` as `state.routing`; the durable inbound-delivery handler
/// and `subject_events` both route through one shared `Arc<RoutingService>`
/// instead of a field per port.
pub struct RoutingService {
    adapter_connections: Arc<ConnectionRouting>,
    loops: Arc<LoopDao>,
    task_bindings: Arc<Bindings>,
    messages: Arc<dyn TranscriptAppend>,
    tasks: Arc<dyn TaskReader>,
    task_creation: Arc<dyn TaskOpener>,
    message_send: Arc<dyn MessageAcceptor>,
    event_receipts: Arc<EventReceipts>,
    sandbox_waker: Arc<dyn SandboxWaker>,
    adapters: Option<Arc<Adapters>>,
    message_buffer: Arc<MessageBufferStore>,
    buffer_defaults: MessageBufferConfig,
}

/// Meter events a flush gave up on, so a batch lost after admission is visible
/// beside the ones that landed.
///
/// Counted per event, not per batch: admission already counted each buffered
/// event on its own, and this series is read against that one.
fn record_buffer_drop(batch: &BufferedBatch, reason: &'static str, connection_id: &str) {
    for _ in &batch.messages {
        record_inbound(
            &batch.adapter_label,
            "ignored",
            reason,
            &batch.loop_id,
            connection_id,
        );
    }
}

/// The connection or Loop a record was admitted against is gone.
///
/// A synthesized condition, not a wrapped failure: there is no underlying error
/// to retain, and no retry can bring the row back.
#[derive(Debug, thiserror::Error)]
#[error("the {what} (`{id}`) this delivery was admitted for is no longer available")]
struct Vanished {
    what: &'static str,
    id: String,
}

impl RoutingService {
    #[expect(
        clippy::too_many_arguments,
        reason = "twelve independent inputs the inbound-routing subgraph reads; they \
                  are exactly the AppState fields this service absorbs, wired in \
                  from the composition root, and there is no narrower grouping \
                  that is not itself an AppState-shaped bag"
    )]
    pub(crate) fn new(
        adapter_connections: Arc<ConnectionRouting>,
        loops: Arc<LoopDao>,
        task_bindings: Arc<Bindings>,
        messages: Arc<dyn TranscriptAppend>,
        tasks: Arc<dyn TaskReader>,
        task_creation: Arc<dyn TaskOpener>,
        message_send: Arc<dyn MessageAcceptor>,
        event_receipts: Arc<EventReceipts>,
        sandbox_waker: Arc<dyn SandboxWaker>,
        adapters: Option<Arc<Adapters>>,
        message_buffer: Arc<MessageBufferStore>,
        buffer_defaults: MessageBufferConfig,
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
            message_buffer,
            buffer_defaults,
        }
    }

    /// Re-read the connection and Loop a durable inbound delivery was admitted
    /// against.
    ///
    /// Read at execution time rather than captured at admission so the work
    /// runs against the current configuration, and a deleted Loop ends the
    /// record instead of routing through a stale copy of itself.
    pub async fn resolve_connection_and_subscription(
        &self,
        connection_id: &str,
        loop_id: &str,
    ) -> Result<(AdapterConnection, SubscriptionMatch), AdapterDeliveryFailure> {
        let connection = self
            .adapter_connections
            .by_id_trusted(connection_id)
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
    ) -> Result<Option<String>, AdapterError> {
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
        // Provenance: the task an adapter recovered from the event body, else
        // the task an already-recorded event identifier names — never an
        // identity. The identifier is asked about only on this subject: a
        // provider numbering events per subject (not globally) reuses one id
        // across subjects, so a task holding it elsewhere didn't write this.
        let origin_task_id = match msg.source_task_id.clone() {
            Some(task_id) => Some(task_id),
            None => match msg.external_event_id.as_deref() {
                Some(ext_id) => {
                    let candidates = self.external_id_aliases(adapter_label, ext_id);
                    self.messages
                        .find_author_on_subject(
                            &candidates,
                            EventSubject {
                                adapter: adapter_label,
                                source_name: &msg.source_id,
                                subject_key,
                            },
                        )
                        .await?
                }
                None => None,
            },
        };

        let mut bindings = binding_repo
            .for_subject(adapter_label, subject_kind, subject_key)
            .await?;

        // Skip bindings whose task has closed — a closed task is terminal and must
        // not be resurrected by a new inbound message. Fresh bindings for the
        // originating / ongoing task are re-added active below.
        bindings.retain(|b| b.active);

        if let Some(origin_task_id) = origin_task_id.as_deref() {
            if !bindings.iter().any(|s| s.task_id == origin_task_id) {
                let sub = self
                    .task_bindings
                    .bind(
                        connection,
                        Some(subscription),
                        origin_task_id,
                        adapter_label,
                        subject_kind,
                        subject_key,
                        destination_config.clone(),
                        ORIGIN_CONNECTED_THREAD,
                        msg.external_event_id.clone(),
                        &BindScope::default(),
                    )
                    .await?;
                bindings.push(sub);
            }
        }

        if subscription.routing_mode == RoutingMode::Ongoing {
            if let Some(task_id) = self.open_ongoing_task_id(subscription).await {
                if !bindings.iter().any(|s| s.task_id == task_id) {
                    // None here: a newly attached binding starts with no
                    // cursor. `deliver_to_binding` fetches the full prior
                    // thread as context, and `touch_cursor` advances it
                    // after delivery. An existing binding's COALESCE in
                    // the upsert keeps its current cursor.
                    let sub = self
                        .task_bindings
                        .bind(
                            connection,
                            Some(subscription),
                            &task_id,
                            adapter_label,
                            subject_kind,
                            subject_key,
                            destination_config.clone(),
                            ORIGIN_ADAPTER_EVENT,
                            None,
                            &BindScope::default(),
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
            self.claim_task_creation(
                connection,
                subscription,
                subject_kind,
                subject_key,
                adapter_label,
                msg.external_event_id.as_deref(),
            )
            .await?;

            // The earliest binding for this subject, if any, belongs to the
            // task that originated it — typically a `connected_thread`
            // binding from an earlier task that produced the resource. Link
            // the new task to it as `related_to` (a peer sharing the
            // artifact), not as a parent.
            let related = bindings.first().map(|b| moatless_types::task::RelatedTask {
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
            let presentation = self
                .adapters
                .as_ref()
                .and_then(|providers| providers.inbound_presentation_by_name(&config.adapter_kind));
            let reply_instructions = match presentation {
                Some(p) => p.reply_instructions().await,
                None => None,
            };
            // The creation service records the `related_to` relation and provisions
            // the sandbox as part of the owned sequence.
            match inbound::create_task_for_external_message(
                self.task_creation.as_ref(),
                &config,
                &msg,
                related,
                reply_instructions,
                None,
            )
            .await
            {
                Ok(outcome) => {
                    let task_id = outcome.task_id;
                    if subscription.routing_mode == RoutingMode::Ongoing {
                        self.loops
                            .set_ongoing_task(&subscription.id, &task_id)
                            .await?;
                    }
                    // Seed the cursor as None so `fetch_context` retrieves the full
                    // prior thread on the first delivery; `touch_cursor` advances it
                    // afterward.
                    let sub = self
                        .task_bindings
                        .bind(
                            connection,
                            Some(subscription),
                            &task_id,
                            adapter_label,
                            subject_kind,
                            subject_key,
                            destination_config.clone(),
                            ORIGIN_ADAPTER_EVENT,
                            None,
                            &BindScope::default(),
                        )
                        .await?;
                    new_task_binding_id = Some(sub.id.clone());
                    bindings.push(sub);
                    self.record_created_task(&subscription.id, subject_kind, subject_key, &task_id)
                        .await?;
                    info!(task_id = %task_id, subscription_id = %subscription.id, provider = adapter_label, "Created task from provider event");
                }
                // Permanent failure (e.g. bad GitHub token): already
                // persisted as errored, with a `SandboxStatus::Error`
                // event published. Skip the new binding and its delivery,
                // but fall through to existing bindings and return `Ok` —
                // a 5xx would make it redeliver and duplicate the error.
                Err(TaskOpenError::Refused(reason)) => {
                    self.release_task_creation(&subscription.id, subject_kind, subject_key)
                        .await?;
                    warn!(subscription_id = %subscription.id, provider = adapter_label, reason = %reason, "Skipping task creation from provider event: preflight failed");
                }
                // Transient failure (Db / ShortId / RepoNotFound) — propagate
                // so the source retries.
                Err(e) => {
                    // Transient, so the sender retries — but the claim must not
                    // outlive the attempt that took it, or the retry loses to
                    // itself.
                    self.release_task_creation(&subscription.id, subject_kind, subject_key)
                        .await?;
                    return Err(AdapterError::task_handoff(
                        "open a task from a provider event",
                        e,
                    ));
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
    /// The single delivery decision point, via [`resolve_recipients`]. Fails
    /// loudly when a subject with bindings reaches no task; returns the last
    /// task reached, or `None` when every binding only commented and this
    /// answers none.
    pub(crate) async fn fan_out(
        &self,
        connection: &AdapterConnection,
        subscription: &SubscriptionMatch,
        msg: &InboundMessage,
        subject: Fanout<'_>,
    ) -> Result<Option<String>, AdapterError> {
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
                replies_to: &msg.replies_to,
            },
        )
        .into_iter()
        .map(|binding| binding.id.clone())
        .collect();

        // Bindings the subject carried when the walk began, and why each
        // one got nothing. A subject with no bindings expects nothing of
        // this event; one whose every binding was passed over expected
        // something and got no task.
        let active_bindings = bindings.len();
        let mut passed_over: Vec<(String, &'static str)> = Vec::new();
        let mut delivered_task_id = None;
        let receipts = self.event_receipts.clone();
        for binding in bindings {
            // Provenance is the only thing asked. Do not also match the sender
            // against the binding's `filter_user_id`: that field names the
            // identity the task *posts as*, which on GitHub is the owner's own
            // credential, so matching it mutes the owner's reviews on the very
            // pull requests their agent opened.
            if author_task_id == Some(binding.task_id.as_str()) {
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

            // Named someone else: deliberately not marked delivered, since
            // nothing holds it on this binding. `delivered_task_id` is now
            // the only empty-result signal (the GitHub PR router's ledger
            // is gone); marking here would make a review reaching no task
            // falsely report success — the exact fault this signal catches.
            if !recipient_ids.contains(&binding.id) {
                let reason = if hears_only_replies(&binding) {
                    inbound_reason::NOT_A_REPLY
                } else {
                    inbound_reason::NOT_IN_AUDIENCE
                };
                debug!(
                    task_id = %binding.task_id,
                    subject_key,
                    provider = adapter_label,
                    reason,
                    "Skipping binding: not in this event's audience"
                );
                record_inbound(
                    adapter_label,
                    "ignored",
                    reason,
                    &subscription.id,
                    &connection.id,
                );
                passed_over.push((binding.task_id, reason));
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

        if delivered_task_id.is_some() {
            return Ok(delivered_task_id);
        }

        // Nothing was bound to the subject, so nothing was expected of this
        // event. The caller decides whether that is a failure of its own.
        if passed_over.is_empty() {
            return Err(SharedAdapterError::ConfigNotFound(subscription.id.clone()).into());
        }

        // Every task on the subject only commented and this answers none of
        // them: nothing was owed, so this is no lost event.
        if passed_over
            .iter()
            .all(|(_, reason)| *reason == inbound_reason::NOT_A_REPLY)
        {
            return Ok(None);
        }

        // Something *was* expected and no task got it. Report it as an error: at
        // debug level a real review reaching nobody looks like a quiet day.
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
        // Permanent by classification (`adapters/delivery`): the same event
        // walking the same bindings is passed over the same way, so
        // retrying only burns budget. The record lands in `failed` with
        // this message — the event isn't lost quietly.
        Err(SharedAdapterError::NotFound(format!(
            "no task binding on {subject_key} could receive this {adapter_label} event \
             ({active_bindings} passed over: {reasons})"
        ))
        .into())
    }

    /// The task that authored the parent event an [`Audience::ParentAuthor`]
    /// names, or `None` otherwise and when no task authored it.
    ///
    /// Sits above [`resolve_recipients`] (no I/O) but asks across every
    /// subject, not just this one, since the parent id is allocated once
    /// by the provider and names the comment unaided.
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
        Ok(self.messages.find_comment_author(&candidates).await?)
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

    /// Claim the right to open `subject_key`'s Task, taking over a claim
    /// whose Task has closed. Two events can both read no Binding (seen: Slack
    /// duplicates ~300ms apart); a loser gets `CreationClaimed` and retries
    /// onto the winner's Binding. End a won claim with
    /// [`Self::record_created_task`] or [`Self::release_task_creation`].
    pub async fn claim_task_creation(
        &self,
        connection: &AdapterConnection,
        subscription: &SubscriptionMatch,
        subject_kind: &str,
        subject_key: &str,
        adapter_label: &str,
        external_event_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let claimed = self
            .event_receipts
            .claim_task_creation(
                &subscription.id,
                subject_kind,
                subject_key,
                adapter_label,
                external_event_id,
            )
            .await?
            || self
                .take_over_closed_claim(
                    &subscription.id,
                    subject_kind,
                    subject_key,
                    adapter_label,
                    external_event_id,
                )
                .await?;
        if claimed {
            return Ok(());
        }
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
        Err(AdapterError::CreationClaimed {
            subject_kind: subject_kind.to_string(),
            subject_key: subject_key.to_string(),
        })
    }

    /// Point a held claim at the Task it produced, so a later delivery that
    /// loses the claim can be told what won it.
    pub async fn record_created_task(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        task_id: &str,
    ) -> Result<(), AdapterError> {
        Ok(self
            .event_receipts
            .record_claimed_task(loop_id, subject_kind, subject_key, task_id)
            .await?)
    }

    /// Hand back a held claim no Task came of, or every later event for the
    /// subject loses a claim held by nothing and retries until its budget runs
    /// out.
    pub async fn release_task_creation(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<(), AdapterError> {
        Ok(self
            .event_receipts
            .release_claim(loop_id, subject_kind, subject_key)
            .await?)
    }

    /// Take over the subject's claim when the task it produced is no longer open.
    ///
    /// A claim outlives its task, so an event after a close would otherwise
    /// retry forever. A claim with no task recorded yet is never taken.
    async fn take_over_closed_claim(
        &self,
        loop_id: &str,
        subject_kind: &str,
        subject_key: &str,
        adapter_label: &str,
        external_event_id: Option<&str>,
    ) -> Result<bool, AdapterError> {
        let Some(task_id) = self
            .event_receipts
            .claimed_task(loop_id, subject_kind, subject_key)
            .await?
        else {
            return Ok(false);
        };
        #[allow(clippy::disallowed_methods)]
        // reason: event routing is a background path with no caller identity
        let sys = ScopeContext::system();
        match self.tasks.describe(&task_id, &sys).await {
            Ok(Some(task)) if task.status == TaskStatus::Open => return Ok(false),
            Ok(_) => {}
            // Unknown is not closed: keep the claim, and let the retry ask again.
            Err(_) => return Ok(false),
        }
        Ok(self
            .event_receipts
            .take_over_claim(
                loop_id,
                subject_kind,
                subject_key,
                &task_id,
                adapter_label,
                external_event_id,
            )
            .await?)
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

    /// Hold this event for its Binding's window, or deliver it on its own.
    ///
    /// The seam every Adapter's inbound events pass through, so buffering is
    /// configured here rather than per provider. A brand-new Task is never
    /// buffered: it is created with no first turn, so holding its only event
    /// would leave a fresh sandbox idle for the length of the window.
    async fn deliver_to_binding(
        &self,
        subscription: &SubscriptionMatch,
        binding: &TaskBinding,
        msg: &InboundMessage,
        adapter_label: &str,
        is_new_task: bool,
    ) -> Result<(), AdapterError> {
        let window = buffer_window(subscription, msg.adapter_kind, &self.buffer_defaults);
        if window.is_zero() || is_new_task {
            return self
                .deliver_now(
                    subscription,
                    binding,
                    std::slice::from_ref(msg),
                    adapter_label,
                    is_new_task,
                )
                .await;
        }

        // Always a Loop id: `buffer_window` gives a delivery-only Subscription
        // no window, and its id resolves to no row a flush could read back.
        self.message_buffer
            .push(&binding.id, &subscription.id, adapter_label, msg, window)
            .await?;
        debug!(
            task_id = %binding.task_id,
            provider = adapter_label,
            window_secs = window.as_secs(),
            "Buffered an inbound event for its binding's window"
        );
        Ok(())
    }

    /// Deliver `msg` to `binding` at once, activating `skills` in place of
    /// `subscription`'s when the binding serves it (see `serves_loop`).
    ///
    /// Bypasses the buffer deliberately: a flush re-reads the Loop and would
    /// send its skills rather than these.
    pub async fn deliver_with_skills(
        &self,
        subscription: &SubscriptionMatch,
        binding: &TaskBinding,
        msg: &InboundMessage,
        adapter_label: &str,
        skills: Vec<String>,
    ) -> Result<(), AdapterError> {
        let subscription = SubscriptionMatch {
            skills,
            ..subscription.clone()
        };
        self.deliver_now(
            &subscription,
            binding,
            std::slice::from_ref(msg),
            adapter_label,
            false,
        )
        .await
    }

    /// Deliver one Binding's buffered events as a single turn.
    ///
    /// Reads the Binding and its route back rather than trusting what was
    /// buffered, so a flush runs against the current configuration; a Binding
    /// that has since closed drops its events instead of resurrecting its Task.
    pub(crate) async fn flush_buffered(&self, batch: &BufferedBatch) -> Result<(), AdapterError> {
        let Some(binding) = self.task_bindings.get(&batch.binding_id).await? else {
            info!(binding_id = %batch.binding_id, "Dropping buffered events: the binding is gone");
            record_buffer_drop(batch, inbound_reason::BINDING_GONE, "");
            return Ok(());
        };
        if !binding.active {
            info!(task_id = %binding.task_id, "Dropping buffered events: the binding is no longer active");
            record_buffer_drop(batch, inbound_reason::BINDING_GONE, &binding.connection_id);
            return Ok(());
        }
        let Some(first) = batch.messages.first() else {
            return Ok(());
        };
        let Some(subscription) = self.flush_route(&binding, &batch.loop_id, first).await? else {
            warn!(
                task_id = %binding.task_id,
                connection_id = %binding.connection_id,
                events = batch.messages.len(),
                "Dropping buffered events: the binding's connection is gone"
            );
            record_buffer_drop(batch, inbound_reason::NO_CONNECTION, &binding.connection_id);
            return Ok(());
        };

        self.deliver_now(
            &subscription,
            &binding,
            &batch.messages,
            &batch.adapter_label,
            false,
        )
        .await
    }

    /// The Subscription a batch delivers under: `loop_id`'s Loop where that is
    /// still runnable, else one derived from the Binding's connection.
    ///
    /// The connection deliberately, not a stopped Loop's own config: an
    /// unbuffered event routes that way too, losing the Loop's `skills`. `None`
    /// only when the connection is gone.
    async fn flush_route(
        &self,
        binding: &TaskBinding,
        loop_id: &str,
        msg: &InboundMessage,
    ) -> Result<Option<SubscriptionMatch>, AdapterError> {
        if let Some(subscription) = self.loops.find_runnable_subscription_by_id(loop_id).await? {
            return Ok(Some(subscription));
        }
        let Some(connection) = self
            .adapter_connections
            .by_id_trusted(&binding.connection_id)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(delivery_only_subscription(
            &connection,
            msg.adapter_kind,
            &msg.source_id,
        )))
    }

    /// Compose `msgs` into one turn and hand it to the Task.
    ///
    /// Thread context is fetched once for the whole batch and excludes every
    /// buffered event, so the newest message is not preceded by a quotation of
    /// the ones rendered beneath it.
    async fn deliver_now(
        &self,
        subscription: &SubscriptionMatch,
        binding: &TaskBinding,
        msgs: &[InboundMessage],
        adapter_label: &str,
        is_new_task: bool,
    ) -> Result<(), AdapterError> {
        let Some(last) = msgs.last() else {
            return Ok(());
        };
        let config = subscription_context(subscription);
        let thread = binding_context(binding);
        let presentation = self
            .adapters
            .as_ref()
            .and_then(|providers| providers.inbound_presentation_by_name(adapter_label));

        // Any part of the batch, not just the newest: a review mixes comments
        // that carry a `thread_id` with a body that does not, and the ones that
        // do still want their thread quoted.
        let on_a_thread = msgs.iter().any(|msg| msg.thread_id.is_some());
        let thread_context = match (&presentation, on_a_thread) {
            (Some(presentation), true) => {
                let mut exclude_ids = self
                    .messages
                    .external_event_ids(&binding.task_id)
                    .await
                    .unwrap_or_default();
                exclude_ids.extend(msgs.iter().filter_map(|msg| msg.external_event_id.clone()));
                presentation
                    .fetch_context(
                        &thread,
                        &config,
                        THREAD_CONTEXT_MESSAGES + msgs.len(),
                        &exclude_ids,
                    )
                    .await
                    .unwrap_or_else(|e| {
                        warn!(provider = adapter_label, error = %e, "Failed to fetch thread context");
                        None
                    })
            }
            _ => None,
        };

        let parts: Vec<Part> = msgs
            .iter()
            .map(|msg| {
                let mut text = msg.text.clone();
                if let Some(prefix) = presentation
                    .as_ref()
                    .and_then(|presentation| presentation.inbound_message_prefix(msg))
                {
                    text = format!("{prefix}\n\n{text}");
                }
                if msg.thread_id.is_some() {
                    if let Some(hint) = presentation
                        .as_ref()
                        .and_then(|presentation| presentation.thread_reply_hint(msg))
                    {
                        text = format!("{hint}\n\n{text}");
                    }
                }
                Part {
                    author: msg.author.clone(),
                    text,
                }
            })
            .collect();
        let mut message_text = combine(parts);

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
            .accept(InboundTurn {
                task_id: binding.task_id.clone(),
                text: message_text,
                file_ids: msgs.iter().flat_map(|msg| msg.file_ids.clone()).collect(),
                resume: true,
                // The newest event in the batch, since that is the one a reply
                // is most likely meant for. `combine` names every other event's
                // author in the body, because a turn carries one header.
                origin: TurnOrigin {
                    event_id: last.external_event_id.clone(),
                    adapter: Some(adapter_label.to_string()),
                    subject_key: Some(binding.subject_key.clone()),
                    // The source's own name where the subscription carries one, else the
                    // matcher that identified it — a repository full name, a channel id.
                    // A label a reader recognises beats no label at all.
                    source_name: Some(
                        config
                            .source_name
                            .clone()
                            .unwrap_or_else(|| config.source_id.clone()),
                    ),
                    author: last.author.clone(),
                    sender_id: last.sender_external_id.clone(),
                },
                sent_by: Some(last.user_id.clone()),
                // A `routing_mode: ongoing` Loop fires into an already-open
                // task, so the task's own skills are held back during
                // resume; the Loop still means them, so send them here.
                skills: if serves_loop(binding, &subscription.id) {
                    config.skills.clone()
                } else {
                    Vec::new()
                },
            })
            .await
            .map_err(|e| AdapterError::task_handoff("deliver a provider turn to a task", e))?;

        self.task_bindings
            .record_inbound_event(binding, last.external_event_id.as_deref())
            .await?;

        if !is_new_task {
            if let Some(e) = wake_for_message(self.sandbox_waker.as_ref(), &binding.task_id).await {
                warn!(task_id = %binding.task_id, error = %e, "Failed to ensure sandbox for external message");
            }
        }

        Ok(())
    }
}

/// Whether `binding`'s task does the work of Loop `loop_id`, and so gets that
/// Loop's skills with its events: the Loop spawned it, or it bound under the
/// Loop for more than a comment. A task that only commented on a reviewed pull
/// request is not its reviewer.
fn serves_loop(binding: &TaskBinding, loop_id: &str) -> bool {
    binding.origin == ORIGIN_ADAPTER_EVENT
        || (binding.loop_id.as_deref() == Some(loop_id) && !hears_only_replies(binding))
}

/// One event, the bindings on its subject, and what routing already knows
/// about who wrote it.
///
/// Assembling the set is the caller's job (spawning varies per adapter: a
/// chat adapter spawns for any unbound subject, GitHub PR only when a PR
/// opens); deciding who receives it lives in [`RoutingService::fan_out`].
pub struct Fanout<'a> {
    /// The adapter label the event is delivered and metered under: the value
    /// `task_inbox.external_adapter` carries, and the key the adapter
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

#[cfg(test)]
mod tests {
    use super::{ORIGIN_ADAPTER_EVENT, ORIGIN_CONNECTED_THREAD, serves_loop};
    use moatless_types::adapter::TaskBinding;

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

    /// `filter_user_id` names the identity a task *posts as*, so matching an
    /// inbound sender against it mutes that person's own events. Routing asks
    /// provenance only.
    #[test]
    fn routing_never_matches_an_inbound_sender_against_a_binding_identity() {
        // Comment lines are dropped: the trap comment in `fan_out` names the
        // field, and `concat!` keeps this needle out of its own source.
        let code = include_str!("runtime.rs")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains(concat!("filter_", "user_id")),
            "fan_out must ask provenance only, never a sender identity"
        );
    }

    fn bound_under(origin: &str, loop_id: Option<&str>, reason: Option<&str>) -> TaskBinding {
        let now = chrono::Utc::now();
        TaskBinding {
            id: "ts_1".into(),
            task_id: "t_1".into(),
            adapter_kind: moatless_types::adapter::AdapterKind::Github,
            subject_kind: "pull_request".into(),
            subject_key: "acme/widgets#1".into(),
            connection_id: "pc_1".into(),
            loop_id: loop_id.map(str::to_string),
            destination_config: serde_json::json!({}),
            metadata: reason.map_or_else(
                || serde_json::json!({}),
                |reason| serde_json::json!({ "bind_reason": reason }),
            ),
            last_external_event_id: None,
            origin: origin.into(),
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
            watching: true,
        }
    }

    /// The review Loop's skills go to the task it spawned and to a task that
    /// bound under it for more than a comment, never to one that only
    /// commented on a pull request the Loop reviews.
    #[test]
    fn loop_skills_reach_only_the_tasks_that_serve_the_loop() {
        let cases = [
            (ORIGIN_ADAPTER_EVENT, Some("lp_review"), None, true),
            (ORIGIN_ADAPTER_EVENT, None, None, true),
            (
                ORIGIN_CONNECTED_THREAD,
                Some("lp_review"),
                Some("creator"),
                true,
            ),
            (ORIGIN_CONNECTED_THREAD, Some("lp_review"), None, true),
            (
                ORIGIN_CONNECTED_THREAD,
                Some("lp_review"),
                Some("commenter"),
                false,
            ),
            (
                ORIGIN_CONNECTED_THREAD,
                Some("lp_other"),
                Some("creator"),
                false,
            ),
            (ORIGIN_CONNECTED_THREAD, None, Some("creator"), false),
        ];
        for (origin, loop_id, reason, expected) in cases {
            assert_eq!(
                serves_loop(&bound_under(origin, loop_id, reason), "lp_review"),
                expected,
                "{origin} under {loop_id:?} for {reason:?}"
            );
        }
    }

    /// A `Vanished` names the row that is gone so an abandoned delivery says
    /// why, not just that it failed.
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

    use moatless_adapter_model::error::AdapterError;
    use moatless_adapter_model::test_support::{response_snapshot, source_chain};

    /// These assert *reachability* of the causal error, not the depth of the
    /// chain. `TaskHandoff` boxes its source, so what has to keep working is
    /// reaching the cause through `Error::source` while the HTTP body stays
    /// redacted.
    #[tokio::test]
    async fn a_task_handoff_failure_keeps_its_cause_and_redacts() {
        let err = AdapterError::task_handoff(
            "open a task from a provider event",
            sqlx::Error::PoolClosed,
        );

        assert!(
            err.source()
                .and_then(|s| s.downcast_ref::<sqlx::Error>())
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

#[cfg(all(test, feature = "db-tests"))]
#[path = "runtime_claim_db_tests.rs"]
mod claim_db_tests;
