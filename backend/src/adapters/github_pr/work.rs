//! Durable admission for the GitHub PR work that must not be lost.
//!
//! Four units of work used to be `tokio::spawn`ed from the webhook handler and
//! then acknowledged with a 200 before any of them had run: routing an inbound
//! PR message (which can create a task and provision a sandbox), closing the
//! tasks a merged PR created, the post-merge CI notice, and the merge-conflict
//! check. A deploy, an OOM kill, or a panic lost all four with no trace, and
//! GitHub had already been told the delivery was accepted.
//!
//! Each is now admitted into [`crate::adapters::work`] inside the request
//! handler and executed by the supervised worker. The admission contract —
//! when the producer may be acknowledged, what is durable, how retries and
//! deduplication work, what happens at capacity, and how shutdown drains — is
//! stated once in that module and is not restated here. What this module owns
//! is the GitHub-specific half: the kinds, the payloads, and the idempotency
//! key.
//!
//! # Idempotency key
//!
//! `event_id` is GitHub's own `X-GitHub-Delivery`, so a redelivery is a
//! duplicate rather than a second unit of work. When the header is absent —
//! GitHub always sends it, but the ingress path has always tolerated its
//! absence — the key falls back to `sha256:<hex>` over the request body, which
//! collapses an identical retry and keeps two genuinely different deliveries
//! apart. Keying an absent header as `""` would collapse every such delivery
//! into one record, which is the failure this fallback exists to avoid.
//!
//! Dedup is per `(provider, kind, event_id)`, so one delivery that triggers
//! both a message and a merge-conflict check is two records that retry and fail
//! independently.
//!
//! # Relationship to the in-process delivery cache
//!
//! [`GithubPrAdapter`](super::adapter::GithubPrAdapter) keeps a 5-minute
//! in-memory cache of delivery ids, and it stays. The two mechanisms are
//! authoritative for different things:
//!
//! - The **in-memory cache** is authoritative for *ingress*: it stops a
//!   redelivery arriving at the same replica within the TTL from being parsed
//!   again, from re-reading bindings, and from re-admitting. It covers the
//!   non-admitted work too (parsing, status persistence), which the durable
//!   index never sees.
//! - The **durable unique index** is authoritative for *effects*: whether these
//!   four units of work run twice. It covers what the cache cannot — a
//!   redelivery landing on another replica, after a restart, or after the TTL.
//!
//! Nothing the cache covered before is uncovered now; the durable index is
//! added underneath it. One consequence must be stated rather than assumed
//! away: GitHub's manual redelivery window is 30 days, which is longer than
//! [`AdapterWorkPolicy::succeeded_retention`](crate::adapters::work::AdapterWorkPolicy)
//! (7 days). A manual redelivery past that horizon is not a duplicate to the
//! index and the work runs again. That is acceptable because every effect
//! downstream is separately idempotent (per-binding event receipts keyed on
//! `external_event_id`, the provider-event-task guard, per-run-id and
//! per-head-sha notice dedup) — not because it cannot happen.
//!
//! # Execution deadline
//!
//! [`AdapterWorkPolicy::execution_timeout`](crate::adapters::work::AdapterWorkPolicy)
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

use moatless_backend_core::scope::ScopeContext;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, InboundMessage, SubscriptionMatch};
use crate::adapters::work::{
    AdapterWork, AdapterWorkFailure, AdapterWorkHandler, AdapterWorkHandlers, AdapterWorkRequest,
};
use crate::state::AppState;

use super::adapter::{PostMergeRun, PrCloseInfo};

/// Provider label on every record this module admits. Matches the value used
/// for GitHub adapter connections, trigger sources, and bindings.
pub const PROVIDER: &str = "github";

/// The work kinds this adapter admits. One kind per unit of work that can fail
/// and retry on its own — folding them behind a single kind would let a
/// permanent failure in one spend the retry budget of the others.
pub mod kind {
    /// Route a parsed inbound PR event to its bound tasks, creating the review
    /// task when the PR was just opened.
    pub const PR_MESSAGE: &str = "github.pr_message";
    /// Auto-close the tasks a closed/merged PR's trigger created.
    pub const CLOSE: &str = "github.close";
    /// Announce a base-branch CI run that completed after a merge.
    pub const POST_MERGE: &str = "github.post_merge";
    /// Re-check a PR's mergeable state and announce a new conflict.
    pub const MERGE_CONFLICT: &str = "github.merge_conflict";
}

/// Register every GitHub work handler on the process-wide registry.
///
/// One call from the composition root; the worker then claims exactly these
/// kinds. A build without this call leaves GitHub records for a replica that
/// has it, rather than failing them.
pub fn register(handlers: &mut AdapterWorkHandlers, state: &AppState) {
    handlers.register(kind::PR_MESSAGE, Arc::new(PrMessageHandler(state.clone())));
    handlers.register(kind::CLOSE, Arc::new(CloseHandler(state.clone())));
    handlers.register(kind::POST_MERGE, Arc::new(PostMergeHandler(state.clone())));
    handlers.register(
        kind::MERGE_CONFLICT,
        Arc::new(MergeConflictHandler(state.clone())),
    );
}

/// The provider's idempotency key for one delivery.
///
/// `X-GitHub-Delivery` when GitHub sent one. The body digest otherwise, so an
/// absent header cannot collapse unrelated deliveries onto a single record.
pub(super) fn delivery_event_id(delivery_id: &str, body: &[u8]) -> String {
    if !delivery_id.is_empty() {
        return delivery_id.to_string();
    }
    format!("sha256:{}", hex::encode(Sha256::digest(body)))
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
pub(super) struct WorkContext {
    pub(super) connection_id: String,
    pub(super) trigger_id: Option<String>,
    /// `owner/repo`, used to synthesize the delivery-only trigger config when
    /// the repo has no trigger of its own.
    pub(super) repo_full_name: String,
}

impl WorkContext {
    /// Load the connection and (optional) trigger this record was admitted for.
    async fn resolve(
        &self,
        state: &AppState,
    ) -> Result<(AdapterConnection, Option<SubscriptionMatch>), AdapterWorkFailure> {
        // reason: durable adapter work runs in the worker with no user
        // session; the connection is named by an already-admitted record, not
        // by caller input.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let connection = PgAdapterConnectionRepository(state.db.clone())
            .find_by_id(&self.connection_id, &sys_ctx)
            .await
            .map_err(|e| AdapterWorkFailure::retryable(AdapterError::Db(e)))?
            .ok_or_else(|| {
                // The tenant's connection was deleted after the webhook was
                // accepted. No retry can bring it back.
                AdapterWorkFailure::permanent(GithubWorkError::ConnectionGone(
                    self.connection_id.clone(),
                ))
            })?;

        let trigger = match self.trigger_id.as_deref() {
            Some(id) => state
                .loops
                .find_subscription_by_id(id)
                .await
                .map_err(|e| AdapterWorkFailure::retryable(AdapterError::Db(e)))?,
            None => None,
        };
        Ok((connection, trigger))
    }

    /// The trigger as a config bag, falling back to the delivery-only rule the
    /// ingress path synthesizes for a connection with no trigger.
    fn rule_for_parse(
        &self,
        connection: &AdapterConnection,
        trigger: Option<&SubscriptionMatch>,
    ) -> SubscriptionMatch {
        trigger.cloned().unwrap_or_else(|| {
            super::runtime::connection_only_rule(connection, &self.repo_full_name)
        })
    }
}

/// Route one parsed inbound PR event.
///
/// The parsed [`InboundMessage`] is carried rather than re-derived: parsing
/// happened on the request path (it is cheap and needs the raw body), and a
/// payload that could not reproduce it would not be self-contained.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PrMessageWork {
    #[serde(flatten)]
    pub(super) context: WorkContext,
    pub(super) message: InboundMessage,
}

/// Close the tasks a PR's trigger created.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CloseWork {
    #[serde(flatten)]
    pub(super) context: WorkContext,
    pub(super) close: PrCloseInfo,
}

/// Announce a completed base-branch CI run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PostMergeWork {
    #[serde(flatten)]
    pub(super) context: WorkContext,
    /// Kept alongside `repo_full_name` because the ingress path falls back to
    /// `(owner_login, "")` when the full name has no `/`; re-splitting here
    /// would silently change which repo is queried.
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) run: PostMergeRun,
    pub(super) sender_login: String,
}

/// Re-check a PR's mergeable state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct MergeConflictWork {
    #[serde(flatten)]
    pub(super) context: WorkContext,
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) pr_number: u64,
    pub(super) sender_login: String,
}

/// Failures this module raises on its own behalf, as opposed to those it
/// forwards from the runtime it calls.
#[derive(Debug, thiserror::Error)]
enum GithubWorkError {
    #[error("adapter connection {0} no longer exists")]
    ConnectionGone(String),
    #[error("adapter work payload does not match its kind")]
    Payload(#[source] serde_json::Error),
}

/// Build the admission request for one unit of work.
///
/// Fails only if the payload cannot be serialized, which for these types means
/// a `serde_json::Value` containing a non-string map key — impossible for
/// values parsed from a webhook body, but not worth an `unwrap`.
pub(super) fn request(
    kind: &'static str,
    event_id: String,
    payload: &impl serde::Serialize,
) -> Result<AdapterWorkRequest, AdapterError> {
    let payload = serde_json::to_value(payload).map_err(|e| {
        AdapterError::Internal(format!("failed to serialize {kind} work payload: {e}"))
    })?;
    Ok(AdapterWorkRequest {
        provider: PROVIDER,
        kind,
        event_id,
        payload,
    })
}

fn parse<T: serde::de::DeserializeOwned>(work: &AdapterWork) -> Result<T, AdapterWorkFailure> {
    serde_json::from_value(work.payload.clone())
        // A payload that does not deserialize was written by a build that is
        // gone or is corrupt; no retry can fix either.
        .map_err(|e| AdapterWorkFailure::permanent(GithubWorkError::Payload(e)))
}

// =============================================================================
// Handlers
// =============================================================================

struct PrMessageHandler(AppState);

#[async_trait::async_trait]
impl AdapterWorkHandler for PrMessageHandler {
    async fn execute(&self, work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
        let payload: PrMessageWork = parse(work)?;
        let (connection, trigger) = payload.context.resolve(&self.0).await?;
        super::runtime::route_inbound_message(
            &self.0,
            &connection,
            trigger.as_ref(),
            payload.message,
        )
        .await
        .map_err(AdapterWorkFailure::from)
    }
}

struct CloseHandler(AppState);

#[async_trait::async_trait]
impl AdapterWorkHandler for CloseHandler {
    async fn execute(&self, work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
        let payload: CloseWork = parse(work)?;
        let (_connection, trigger) = payload.context.resolve(&self.0).await?;
        // Auto-close only ever applied to trigger-created tasks. A trigger
        // deleted between admission and execution leaves nothing to close.
        let Some(trigger) = trigger else {
            info!(
                id = %work.id,
                pr_number = payload.close.pr_number,
                "GitHub PR close skipped: its trigger no longer exists"
            );
            return Ok(());
        };
        super::runtime::close_tasks_for_pr(&self.0, &trigger, &payload.close)
            .await
            .map_err(AdapterWorkFailure::from)
    }
}

struct PostMergeHandler(AppState);

#[async_trait::async_trait]
impl AdapterWorkHandler for PostMergeHandler {
    async fn execute(&self, work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
        let payload: PostMergeWork = parse(work)?;
        let (connection, trigger) = payload.context.resolve(&self.0).await?;
        let rule = payload
            .context
            .rule_for_parse(&connection, trigger.as_ref());
        super::adapter::check_post_merge_ci(
            &self.0,
            &connection,
            &rule,
            &payload.owner,
            &payload.repo,
            &payload.run,
            &payload.sender_login,
        )
        .await
        .map_err(AdapterWorkFailure::from)
    }
}

struct MergeConflictHandler(AppState);

#[async_trait::async_trait]
impl AdapterWorkHandler for MergeConflictHandler {
    async fn execute(&self, work: &AdapterWork) -> Result<(), AdapterWorkFailure> {
        let payload: MergeConflictWork = parse(work)?;
        let (connection, trigger) = payload.context.resolve(&self.0).await?;
        let rule = payload
            .context
            .rule_for_parse(&connection, trigger.as_ref());
        super::adapter::check_pr_merge_conflict(
            &self.0,
            &connection,
            &rule,
            &payload.owner,
            &payload.repo,
            payload.pr_number,
            &payload.sender_login,
        )
        .await
        .map_err(AdapterWorkFailure::from)
    }
}

/// Log line shared by the four admission sites, so an operator can follow one
/// delivery from ingress into the queue.
pub(super) fn log_admission(
    kind: &'static str,
    event_id: &str,
    admission: crate::adapters::work::Admission,
) {
    match admission {
        crate::adapters::work::Admission::Admitted => {
            debug!(kind, event_id, "GitHub work admitted");
        }
        crate::adapters::work::Admission::Duplicate => {
            warn!(
                kind,
                event_id, "GitHub work already admitted for this delivery; nothing re-run"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::adapters::models::Audience;
    use crate::adapters::AdapterKind;

    use super::*;

    fn context() -> WorkContext {
        WorkContext {
            connection_id: "conn-1".to_string(),
            trigger_id: Some("trigger-1".to_string()),
            repo_full_name: "acme/widgets".to_string(),
        }
    }

    fn message() -> InboundMessage {
        InboundMessage {
            provider: AdapterKind::GithubPr,
            source_id: "acme/widgets".to_string(),
            thread_id: Some("42".to_string()),
            message_thread_id: "42".to_string(),
            user_id: "user-1".to_string(),
            text: "hello".to_string(),
            file_ids: vec![],
            thread_properties: serde_json::json!({
                "owner": "acme", "repo": "widgets", "pr_number": 42
            }),
            provider_context: serde_json::json!({}),
            external_event_id: Some("github:pr:opened:42".to_string()),
            source_task_id: None,
            audience: Audience::Everyone,
        }
    }

    /// The payload is the whole of what a handler gets after a restart, so
    /// every field has to survive the round trip through Postgres JSON.
    #[test]
    fn a_pr_message_payload_survives_the_round_trip() {
        let work = PrMessageWork {
            context: context(),
            message: message(),
        };
        let value = serde_json::to_value(&work).expect("serialize");
        let back: PrMessageWork = serde_json::from_value(value).expect("deserialize");

        assert_eq!(back.context.connection_id, "conn-1");
        assert_eq!(back.context.trigger_id.as_deref(), Some("trigger-1"));
        assert_eq!(back.context.repo_full_name, "acme/widgets");
        assert_eq!(back.message.text, "hello");
        assert_eq!(back.message.user_id, "user-1");
        assert_eq!(
            back.message.external_event_id.as_deref(),
            Some("github:pr:opened:42")
        );
        assert_eq!(back.message.provider, AdapterKind::GithubPr);
        assert_eq!(
            back.message.thread_properties,
            work.message.thread_properties
        );
    }

    /// `CloseWork` and `PostMergeWork` wrap types owned by `adapter.rs`, whose
    /// fields this module cannot name. Pinning them as literal JSON is the only
    /// way to assert the stored shape from here — and it is the shape the
    /// crash-recovery test hardcodes, so a rename or a lost `flatten` has to
    /// fail here rather than in a test that needs a database to run.
    #[test]
    fn the_close_payload_is_stored_flat_with_a_nested_close() {
        let stored = serde_json::json!({
            "connection_id": "conn-1",
            "trigger_id": "trigger-1",
            "repo_full_name": "acme/widgets",
            "close": {
                "pr_number": 42,
                "merged": true,
                "title": "Add the widget",
                "html_url": "https://github.com/acme/widgets/pull/42",
                "sender_login": "octocat",
            },
        });
        let work: CloseWork = serde_json::from_value(stored.clone()).expect("deserialize");

        assert_eq!(work.context.connection_id, "conn-1");
        assert_eq!(work.close.pr_number, 42);
        assert!(work.close.merged);
        assert_eq!(serde_json::to_value(&work).expect("serialize"), stored);
    }

    #[test]
    fn the_post_merge_payload_is_stored_flat_with_a_nested_run() {
        let stored = serde_json::json!({
            "connection_id": "conn-1",
            "trigger_id": serde_json::Value::Null,
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
        let work: PostMergeWork = serde_json::from_value(stored.clone()).expect("deserialize");

        assert!(work.context.trigger_id.is_none());
        assert_eq!(work.owner, "acme");
        // A run id is a `u64` behind a `flatten`ed sibling, which routes through
        // serde's buffering representation; assert it comes back unrounded.
        assert_eq!(serde_json::to_value(&work).expect("serialize"), stored);
    }

    #[test]
    fn a_merge_conflict_payload_survives_the_round_trip() {
        let work = MergeConflictWork {
            context: WorkContext {
                trigger_id: None,
                ..context()
            },
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            pr_number: 42,
            sender_login: "octocat".to_string(),
        };
        let back: MergeConflictWork =
            serde_json::from_value(serde_json::to_value(&work).expect("serialize"))
                .expect("deserialize");
        assert!(back.context.trigger_id.is_none());
        assert_eq!(back.pr_number, 42);
        assert_eq!(back.sender_login, "octocat");
    }

    #[test]
    fn the_delivery_header_is_the_idempotency_key() {
        assert_eq!(delivery_event_id("d-1", b"{}"), "d-1");
    }

    /// The header has always been optional on this path. Keying its absence as
    /// `""` would make every such delivery the same record; the digest keeps
    /// distinct bodies distinct and still collapses an identical retry.
    #[test]
    fn a_missing_delivery_header_falls_back_to_the_body_digest() {
        let a = delivery_event_id("", br#"{"action":"opened"}"#);
        let b = delivery_event_id("", br#"{"action":"closed"}"#);
        let a_again = delivery_event_id("", br#"{"action":"opened"}"#);

        assert!(a.starts_with("sha256:"), "{a}");
        assert_ne!(a, b, "different deliveries must not share a record");
        assert_eq!(a, a_again, "an identical retry must collapse");
    }

    /// A record's kind names its handler; the request must carry the kind and
    /// the provider's own key, not a generated one.
    #[test]
    fn a_request_carries_the_kind_and_the_provider_key() {
        let request = request(
            kind::PR_MESSAGE,
            "delivery-9".to_string(),
            &PrMessageWork {
                context: context(),
                message: message(),
            },
        )
        .expect("serialize");

        assert_eq!(request.provider, PROVIDER);
        assert_eq!(request.kind, "github.pr_message");
        assert_eq!(request.event_id, "delivery-9");
        assert_eq!(request.payload["connection_id"], "conn-1");
    }

    /// Every kind is distinct, so one delivery that triggers several of them
    /// produces several records under the same `event_id` rather than one.
    #[test]
    fn the_kinds_are_distinct_and_namespaced() {
        let kinds = [
            kind::PR_MESSAGE,
            kind::CLOSE,
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
