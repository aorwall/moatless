//! Durable admission for the GitHub PR work that must not be lost.
//!
//! One GitHub webhook can loop several units of work that must not be lost:
//! routing an inbound PR message (which can create a task and provision a
//! sandbox), closing the tasks a merged PR created, telling the tasks that
//! outlive it that it merged, the post-merge CI notice, and the merge-conflict
//! check. None of them can run under GitHub's response deadline, and a deploy,
//! an OOM kill, or a panic must not lose work the sender was already told was
//! accepted.
//!
//! Each is admitted into [`moatless_loops::adapters::delivery`] inside the request
//! handler and executed by the supervised worker. The admission contract —
//! when the producer may be acknowledged, what is durable, how retries and
//! deduplication work, what happens at capacity, and how shutdown drains — is
//! stated once in that module and is not restated here. What this module owns
//! is the GitHub-specific half: the stable kinds and payloads.
//!
//! Ingress supplies the sender delivery identity. Dedup is per
//! `(adapter, kind, event_id)`, so one delivery that loops both a message and
//! a merge-conflict check is two records that retry and fail independently.
//!
//! # The only deduplication mechanism
//!
//! The durable unique index is it. There was once a 5-minute in-process cache
//! of delivery ids on [`GithubPrAdapter`](super::adapter::GithubPrAdapter) as
//! well; it is gone, along with the compensating eviction a failed admission
//! needed to undo it. What is left is authoritative for *effects* — whether
//! these units of work run twice — on any replica, after any restart, and
//! past any expiry, which is strictly more than the cache ever covered.
//!
//! What the cache did cover and the index does not is the work the ingress path
//! does *before* admitting: signature verification, parsing, and the
//! subscription / binding reads, all read-only; and the two best-effort status
//! projections scheduled by ingress that derive a value from the delivery
//! payload and overwrite it (PR external status, review decision). The same
//! payload derives the same value, so a redelivery re-runs them and lands on
//! what is already there. That costs a little work and changes nothing, which
//! is why this is a performance question rather than a correctness one.
//!
//! One consequence must be stated rather than assumed away: GitHub's manual
//! redelivery window is 30 days, which is longer than
//! [`AdapterDeliveryPolicy::succeeded_retention`](moatless_loops::adapters::delivery::AdapterDeliveryPolicy)
//! (7 days). A manual redelivery past that horizon is not a duplicate to the
//! index and the work runs again. That is acceptable because every effect
//! downstream is separately idempotent (per-binding event receipts keyed on
//! `external_event_id`, the adapter-event-task guard, per-run-id and
//! per-head-sha notice dedup) — not because it cannot happen. A *late*
//! redelivery of an older event can also overwrite newer PR state with the
//! state the old payload carries; that exposure is pre-existing and unrelated
//! to the cache, which expired in five minutes against a 30-day window.
//!
//! # Execution deadline
//!
//! [`AdapterDeliveryPolicy::execution_timeout`](moatless_loops::adapters::delivery::AdapterDeliveryPolicy)
//! is 2 minutes. [`kind::PR_MESSAGE`] is the one kind that can plausibly reach
//! it, because routing a `pull_request.opened` event creates a task and starts
//! its sandbox. If it does, the attempt is abandoned at the deadline and
//! retried, and the retry converges rather than duplicating: task creation is
//! guarded by `find_provider_event_created_for_subject` and delivery by the
//! event-receipt table. The one window that is not covered is a timeout landing
//! between task creation and the binding write that records it — a retry then
//! creates a second task. The spawned path lost the work outright in that same
//! window, so this is a narrower failure than the one it replaces, but it is
//! not zero.

use std::sync::Arc;

use moatless_types::access_control::ScopeContext;

use tracing::info;

use moatless_loops::adapters::delivery::register_typed_delivery;
use moatless_adapter_model::delivery::{AdapterDeliveryFailure, AdapterDeliveryHandlers, AdapterDeliveryRequest};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::models::{AdapterConnection, SubscriptionMatch};
use moatless_types::adapter::InboundMessage;

use super::router::GithubPrInboundRouter;
use super::webhook::PrCloseInfo;

/// Adapter name on every record this module admits. Matches the value used
/// for GitHub Adapter connections, trigger sources, and bindings.
pub(super) const ADAPTER: &str = "github";

/// The delivery kinds this adapter admits. One kind per unit of work that can fail
/// and retry on its own — folding them behind a single kind would let a
/// permanent failure in one spend the retry budget of the others.
pub(super) mod kind {
    /// Route a parsed inbound PR event to its bound tasks, creating the review
    /// task when the PR was just opened.
    pub const PR_MESSAGE: &str = "github.pr_message";
    /// Auto-close the tasks a closed/merged PR's trigger created.
    pub const CLOSE: &str = "github.close";
    /// Tell the tasks that outlive a merged PR that it landed.
    pub const PR_MERGED: &str = "github.pr_merged";
    /// Announce a base-branch CI run that completed after a merge.
    pub const POST_MERGE: &str = "github.post_merge";
    /// Re-check a PR's mergeable state and announce a new conflict.
    pub const MERGE_CONFLICT: &str = "github.merge_conflict";
}

/// Register every GitHub delivery handler on the process-wide registry.
///
/// One call from the composition root; the worker then claims exactly these
/// kinds. A build without this call leaves GitHub records for a replica that
/// has it, rather than failing them.
pub fn register(handlers: &mut AdapterDeliveryHandlers, router: Arc<GithubPrInboundRouter>) {
    register_typed_delivery::<PrMessageDelivery, _, _>(handlers, kind::PR_MESSAGE, {
        let router = router.clone();
        move |delivery, payload| execute_pr_message(router.clone(), delivery, payload)
    });
    register_typed_delivery::<CloseDelivery, _, _>(handlers, kind::CLOSE, {
        let router = router.clone();
        move |delivery, payload| execute_close(router.clone(), delivery, payload)
    });
    register_typed_delivery::<PrMergedDelivery, _, _>(handlers, kind::PR_MERGED, {
        let router = router.clone();
        move |delivery, payload| execute_pr_merged(router.clone(), delivery, payload)
    });
    register_typed_delivery::<PostMergeDelivery, _, _>(handlers, kind::POST_MERGE, {
        let router = router.clone();
        move |delivery, payload| execute_post_merge(router.clone(), delivery, payload)
    });
    register_typed_delivery::<MergeConflictDelivery, _, _>(handlers, kind::MERGE_CONFLICT, {
        let router = router.clone();
        move |delivery, payload| execute_merge_conflict(router.clone(), delivery, payload)
    });
}

// =============================================================================
// Payloads
// =============================================================================

/// The part of every payload that says *which* connection and trigger the work
/// belongs to.
///
/// Ids, not snapshots. Re-resolving keeps webhook secrets and OAuth references
/// out of a table retained for weeks, and it means a record executed minutes
/// later sees the trigger as it is rather than as it was. A trigger deleted in
/// between resolves to `None`, which is exactly what a delivery for a repo with
/// no trigger has always meant: deliver to existing bindings, create nothing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct DeliveryContext {
    pub(super) connection_id: String,
    pub(super) loop_id: Option<String>,
    /// `owner/repo`, used to synthesize the delivery-only loop config when
    /// the repo has no subscription of its own.
    pub(super) repo_full_name: String,
}

impl DeliveryContext {
    /// Load the connection and (optional) trigger this record was admitted for.
    async fn resolve(
        &self,
        router: &GithubPrInboundRouter,
    ) -> Result<(AdapterConnection, Option<SubscriptionMatch>), AdapterDeliveryFailure> {
        // reason: a durable Adapter delivery runs in the worker with no user
        // session; the connection is named by an already-admitted record, not
        // by caller input.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let connection = router
            .adapter_connections
            .find_by_id(&self.connection_id, &sys_ctx)
            .await
            .map_err(AdapterDeliveryFailure::retryable)?
            .ok_or_else(|| {
                // The tenant's connection was deleted after the webhook was
                // accepted. No retry can bring it back.
                AdapterDeliveryFailure::permanent(GithubDeliveryError::ConnectionGone(
                    self.connection_id.clone(),
                ))
            })?;

        let trigger = match self.loop_id.as_deref() {
            Some(id) => router
                .loops
                .runnable_subscription(id)
                .await
                .map_err(AdapterDeliveryFailure::retryable)?,
            None => None,
        };
        Ok((connection, trigger))
    }

    /// The trigger as a config bag, falling back to the routing-owned
    /// delivery-only rule for a connection with no trigger.
    fn rule_for_parse(
        &self,
        connection: &AdapterConnection,
        trigger: Option<&SubscriptionMatch>,
    ) -> SubscriptionMatch {
        trigger
            .cloned()
            .unwrap_or_else(|| super::routing::delivery_only_rule(connection, &self.repo_full_name))
    }
}

/// Route one parsed inbound PR event.
///
/// The parsed [`InboundMessage`] is carried rather than re-derived: parsing
/// happened on the request path (it is cheap and needs the raw body), and a
/// payload that could not reproduce it would not be self-contained.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PrMessageDelivery {
    #[serde(flatten)]
    pub(super) context: DeliveryContext,
    pub(super) message: InboundMessage,
}

/// Close the tasks a PR's trigger created.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CloseDelivery {
    #[serde(flatten)]
    pub(super) context: DeliveryContext,
    pub(super) close: PrCloseInfo,
}

/// Notify the tasks that outlive a merged PR.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PrMergedDelivery {
    #[serde(flatten)]
    pub(super) context: DeliveryContext,
    #[serde(flatten)]
    pub(super) execution: super::merged::Execution,
}

/// Announce a completed base-branch CI run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PostMergeDelivery {
    #[serde(flatten)]
    pub(super) context: DeliveryContext,
    #[serde(flatten)]
    pub(super) execution: super::post_merge::Execution,
}

/// Re-check a PR's mergeable state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct MergeConflictDelivery {
    #[serde(flatten)]
    pub(super) context: DeliveryContext,
    #[serde(flatten)]
    pub(super) execution: super::merge_conflict::Execution,
}

/// Failures this module raises on its own behalf, as opposed to those it
/// forwards from the routing module it calls.
#[derive(Debug, thiserror::Error)]
enum GithubDeliveryError {
    #[error("adapter connection {0} no longer exists")]
    ConnectionGone(String),
}

/// Build the admission request for one GitHub delivery.
///
/// Fails only if the payload cannot be serialized, which for these types means
/// a `serde_json::Value` containing a non-string map key — impossible for
/// values parsed from a webhook body, but not worth an `unwrap`.
pub(super) fn request(
    kind: &'static str,
    event_id: String,
    payload: &impl serde::Serialize,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    let payload = serde_json::to_value(payload).map_err(|e| {
        SharedAdapterError::Internal(format!("failed to serialize {kind} work payload: {e}"))
    })?;
    Ok(AdapterDeliveryRequest {
        adapter: ADAPTER,
        kind,
        event_id,
        payload,
    })
}

async fn execute_pr_message(
    router: Arc<GithubPrInboundRouter>,
    _delivery: moatless_adapter_model::delivery::AdapterDelivery,
    payload: PrMessageDelivery,
) -> Result<(), AdapterDeliveryFailure> {
    let (connection, trigger) = payload.context.resolve(&router).await?;
    router
        .route_event(&connection, trigger.as_ref(), payload.message)
        .await
        .map_err(AdapterDeliveryFailure::from)
}

async fn execute_close(
    router: Arc<GithubPrInboundRouter>,
    delivery: moatless_adapter_model::delivery::AdapterDelivery,
    payload: CloseDelivery,
) -> Result<(), AdapterDeliveryFailure> {
    let (_connection, trigger) = payload.context.resolve(&router).await?;
    // Auto-close only ever applied to trigger-created tasks. A trigger
    // deleted between admission and execution leaves nothing to close.
    let Some(trigger) = trigger else {
        info!(
            id = %delivery.id,
            pr_number = payload.close.pr_number,
            "GitHub PR close skipped: its trigger no longer exists"
        );
        return Ok(());
    };
    router
        .close_subject(&trigger, &payload.close)
        .await
        .map_err(AdapterDeliveryFailure::from)
}

async fn execute_pr_merged(
    router: Arc<GithubPrInboundRouter>,
    _delivery: moatless_adapter_model::delivery::AdapterDelivery,
    payload: PrMergedDelivery,
) -> Result<(), AdapterDeliveryFailure> {
    let (connection, trigger) = payload.context.resolve(&router).await?;
    // A trigger deleted between admission and execution leaves the bindings on
    // the pull request intact, so the notice is still owed to them; the
    // delivery-only rule carries the labelling they route under.
    let rule = payload.context.rule_for_parse(&connection, trigger.as_ref());
    router
        .run_pr_merged(&connection, &rule, &payload.execution)
        .await
        .map_err(AdapterDeliveryFailure::from)
}

async fn execute_post_merge(
    router: Arc<GithubPrInboundRouter>,
    _delivery: moatless_adapter_model::delivery::AdapterDelivery,
    payload: PostMergeDelivery,
) -> Result<(), AdapterDeliveryFailure> {
    let (connection, trigger) = payload.context.resolve(&router).await?;
    let rule = payload
        .context
        .rule_for_parse(&connection, trigger.as_ref());
    router
        .run_post_merge(&connection, &rule, &payload.execution)
        .await
        .map_err(AdapterDeliveryFailure::from)
}

async fn execute_merge_conflict(
    router: Arc<GithubPrInboundRouter>,
    _delivery: moatless_adapter_model::delivery::AdapterDelivery,
    payload: MergeConflictDelivery,
) -> Result<(), AdapterDeliveryFailure> {
    let (connection, trigger) = payload.context.resolve(&router).await?;
    let rule = payload
        .context
        .rule_for_parse(&connection, trigger.as_ref());
    router
        .run_merge_conflict(&connection, &rule, &payload.execution)
        .await
        .map_err(AdapterDeliveryFailure::from)
}

/// Log line shared by the four admission sites, so an operator can follow one
/// delivery from ingress into the queue.
pub(super) fn log_admission(
    kind: &'static str,
    event_id: &str,
    admission: moatless_adapter_model::delivery::Admission,
) {
    moatless_loops::adapters::ingress::log_admission(ADAPTER, kind, event_id, admission);
}

#[cfg(test)]
mod tests {
    use moatless_types::adapter::AdapterKind;
    use moatless_types::adapter::Audience;

    use super::*;

    fn context() -> DeliveryContext {
        DeliveryContext {
            connection_id: "conn-1".to_string(),
            loop_id: Some("trigger-1".to_string()),
            repo_full_name: "acme/widgets".to_string(),
        }
    }

    fn message() -> InboundMessage {
        InboundMessage {
            adapter_kind: AdapterKind::GithubPr,
            source_id: "acme/widgets".to_string(),
            thread_id: Some("42".to_string()),
            message_thread_id: "42".to_string(),
            user_id: "user-1".to_string(),
            author: Some("octocat".to_string()),
            sender_external_id: None,
            text: "hello".to_string(),
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": "acme", "repo": "widgets", "pr_number": 42
            }),
            adapter_context: serde_json::json!({}),
            external_event_id: Some("github:pr:opened:42".to_string()),
            source_task_id: None,
            audience: Audience::Everyone,
        }
    }

    /// The payload is the whole of what a handler gets after a restart, so
    /// every field has to survive the round trip through Postgres JSON.
    #[test]
    fn a_pr_message_payload_survives_the_round_trip() {
        let delivery = PrMessageDelivery {
            context: context(),
            message: message(),
        };
        let value = serde_json::to_value(&delivery).expect("serialize");
        let back: PrMessageDelivery = serde_json::from_value(value).expect("deserialize");

        assert_eq!(back.context.connection_id, "conn-1");
        assert_eq!(back.context.loop_id.as_deref(), Some("trigger-1"));
        assert_eq!(back.context.repo_full_name, "acme/widgets");
        assert_eq!(back.message.text, "hello");
        assert_eq!(back.message.user_id, "user-1");
        assert_eq!(
            back.message.external_event_id.as_deref(),
            Some("github:pr:opened:42")
        );
        assert_eq!(back.message.adapter_kind, AdapterKind::GithubPr);
        assert_eq!(
            back.message.thread_properties,
            delivery.message.thread_properties
        );
    }

    /// `CloseDelivery` and `PostMergeDelivery` wrap types owned by `adapter.rs`, whose
    /// fields this module cannot name. Pinning them as literal JSON is the only
    /// way to assert the stored shape from here — and it is the shape the
    /// crash-recovery test hardcodes, so a rename or a lost `flatten` has to
    /// fail here rather than in a test that needs a database to run.
    #[test]
    fn the_close_payload_is_stored_flat_with_a_nested_close() {
        let stored = serde_json::json!({
            "connection_id": "conn-1",
            "loop_id": "trigger-1",
            "repo_full_name": "acme/widgets",
            "close": {
                "pr_number": 42,
                "merged": true,
                "title": "Add the widget",
                "html_url": "https://github.com/acme/widgets/pull/42",
                "sender_login": "octocat",
            },
        });
        let delivery: CloseDelivery = serde_json::from_value(stored.clone()).expect("deserialize");

        assert_eq!(delivery.context.connection_id, "conn-1");
        assert_eq!(delivery.close.pr_number, 42);
        assert!(delivery.close.merged);
        assert_eq!(serde_json::to_value(&delivery).expect("serialize"), stored);
    }

    #[test]
    fn the_pr_merged_payload_is_stored_flat_with_a_nested_merge() {
        let stored = serde_json::json!({
            "connection_id": "conn-1",
            "loop_id": "trigger-1",
            "repo_full_name": "acme/widgets",
            "owner": "acme",
            "repo": "widgets",
            "merged": {
                "pr_number": 42,
                "title": "Add the widget",
                "html_url": "https://github.com/acme/widgets/pull/42",
                "merged_by": "octocat",
                "base_ref": "main",
                "head_ref": "feature",
                "merge_commit_sha": "abcdef0123456789",
            },
        });
        let delivery: PrMergedDelivery =
            serde_json::from_value(stored.clone()).expect("deserialize");

        assert_eq!(delivery.context.connection_id, "conn-1");
        assert_eq!(delivery.execution.owner, "acme");
        assert_eq!(delivery.execution.merged.pr_number, 42);
        assert_eq!(delivery.execution.merged.merged_by, "octocat");
        assert_eq!(serde_json::to_value(&delivery).expect("serialize"), stored);
    }

    #[test]
    fn the_post_merge_payload_is_stored_flat_with_a_nested_run() {
        let stored = serde_json::json!({
            "connection_id": "conn-1",
            "loop_id": serde_json::Value::Null,
            "repo_full_name": "acme/widgets",
            "owner": "acme",
            "repo": "widgets",
            "sender_login": "octocat",
            "run": {
                "run_id": 12_345_678_901_u64,
                "name": "CI",
                "conclusion": "success",
                "head_sha": "deadbeef",
                "head_branch": "main",
                "html_url": "https://github.com/acme/widgets/actions/runs/12345678901",
            },
        });
        let delivery: PostMergeDelivery =
            serde_json::from_value(stored.clone()).expect("deserialize");

        assert!(delivery.context.loop_id.is_none());
        assert_eq!(delivery.execution.owner, "acme");
        // A run id is a `u64` behind a `flatten`ed sibling, which routes through
        // serde's buffering representation; assert it comes back unrounded.
        assert_eq!(serde_json::to_value(&delivery).expect("serialize"), stored);
    }

    #[test]
    fn a_merge_conflict_payload_survives_the_round_trip() {
        let delivery = MergeConflictDelivery {
            context: DeliveryContext {
                loop_id: None,
                ..context()
            },
            execution: super::super::merge_conflict::Execution {
                owner: "acme".to_string(),
                repo: "widgets".to_string(),
                pr_number: 42,
                sender_login: "octocat".to_string(),
            },
        };
        let back: MergeConflictDelivery =
            serde_json::from_value(serde_json::to_value(&delivery).expect("serialize"))
                .expect("deserialize");
        assert!(back.context.loop_id.is_none());
        assert_eq!(back.execution.pr_number, 42);
        assert_eq!(back.execution.sender_login, "octocat");
    }

    /// A record's kind names its handler; the request must carry the kind and
    /// the sender's own key, not a generated one.
    #[test]
    fn a_request_carries_the_kind_and_the_senders_key() {
        let request = request(
            kind::PR_MESSAGE,
            "delivery-9".to_string(),
            &PrMessageDelivery {
                context: context(),
                message: message(),
            },
        )
        .expect("serialize");

        assert_eq!(request.adapter, ADAPTER);
        assert_eq!(request.kind, "github.pr_message");
        assert_eq!(request.event_id, "delivery-9");
        assert_eq!(request.payload["connection_id"], "conn-1");
    }

    /// Every kind is distinct, so one delivery that loops several of them
    /// produces several records under the same `event_id` rather than one.
    #[test]
    fn the_kinds_are_distinct_and_namespaced() {
        let kinds = [
            kind::PR_MESSAGE,
            kind::CLOSE,
            kind::PR_MERGED,
            kind::POST_MERGE,
            kind::MERGE_CONFLICT,
        ];
        let mut unique = kinds.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len());
        assert!(kinds.iter().all(|k| k.starts_with("github.")));
    }
}
