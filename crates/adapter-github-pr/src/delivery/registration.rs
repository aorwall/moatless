//! Durable admission for the GitHub PR work that must not be lost.
//!
//! One webhook loops several units of work, each admitted separately so they
//! retry and fail independently.
//!
//! The durable unique index dedups only these effects, not ingress's
//! read-only work or status projections. GitHub's redelivery window outlives
//! `succeeded_retention`, so a late redelivery can overwrite newer PR state,
//! and [`kind::PR_MESSAGE`] can duplicate a task if its 2-minute timeout
//! lands between task creation and the binding write.

use super::kind;
use std::sync::Arc;

use tracing::info;

use moatless_adapter_model::delivery::{
    AdapterDeliveryFailure, AdapterDeliveryHandlers, AdapterDeliveryRequest,
};
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::delivery::register_typed_delivery;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::InboundMessage;
use moatless_types::loops::SubscriptionMatch;

use crate::router::GithubPrInboundRouter;
use crate::webhook::PrCloseInfo;

/// Adapter name on every record this module admits. Matches the value used for
/// GitHub Adapter connections, trigger sources, and bindings.
pub(crate) const ADAPTER: &str = moatless_types::adapter::AdapterKind::Github.as_str();

/// Register every GitHub delivery handler on the process-wide registry.
///
/// One call from the composition root; a build without this call leaves GitHub
/// records for a replica that has it, rather than failing them.
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
    register_typed_delivery::<CheckRunDelivery, _, _>(handlers, kind::CHECK_RUN, {
        let router = router.clone();
        move |delivery, payload| execute_check_run(router.clone(), delivery, payload)
    });
    register_typed_delivery::<CheckRunTriggerDelivery, _, _>(handlers, kind::CHECK_RUN_TRIGGER, {
        let router = router.clone();
        move |delivery, payload| execute_check_run_trigger(router.clone(), delivery, payload)
    });
}

// =============================================================================
// Payloads
// =============================================================================

/// The part of every payload that says *which* connection and trigger the work
/// belongs to.
///
/// Ids, not snapshots: re-resolving keeps secrets out of a table retained for
/// weeks, and a trigger deleted in between resolves to `None`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct DeliveryContext {
    pub(crate) connection_id: String,
    pub(crate) loop_id: Option<String>,
    /// `owner/repo`, used to synthesize the delivery-only loop config when the
    /// repo has no subscription of its own.
    pub(crate) repo_full_name: String,
}

impl DeliveryContext {
    /// Load the connection and (optional) trigger this record was admitted for.
    async fn resolve(
        &self,
        router: &GithubPrInboundRouter,
    ) -> Result<(AdapterConnection, Option<SubscriptionMatch>), AdapterDeliveryFailure> {
        let connection = router
            .adapter_connections
            .by_id_trusted(&self.connection_id)
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
        let trigger =
            trigger.map(|rule| crate::routing::for_repository(rule, &self.repo_full_name));
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
            .unwrap_or_else(|| crate::routing::delivery_only_rule(connection, &self.repo_full_name))
    }
}

/// Route one parsed inbound PR event.
///
/// The parsed [`InboundMessage`] is carried rather than re-derived, since
/// parsing needs the raw body from the request path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PrMessageDelivery {
    #[serde(flatten)]
    pub(crate) context: DeliveryContext,
    pub(crate) message: InboundMessage,
}

/// Close the tasks a PR's trigger created and those whose Binding opted in.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CloseDelivery {
    #[serde(flatten)]
    pub(crate) context: DeliveryContext,
    pub(crate) close: PrCloseInfo,
    /// Whether the trigger's `close_task_on_close` applies. Defaults to `true`
    /// because a stored delivery without the field was admitted for it alone.
    #[serde(default = "admitted_for_the_trigger")]
    pub(crate) trigger_closes: bool,
}

fn admitted_for_the_trigger() -> bool {
    true
}

/// Notify the tasks that outlive a merged PR.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PrMergedDelivery {
    #[serde(flatten)]
    pub(crate) context: DeliveryContext,
    #[serde(flatten)]
    pub(crate) execution: crate::merged::Execution,
}

/// Announce a completed base-branch CI run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PostMergeDelivery {
    #[serde(flatten)]
    pub(crate) context: DeliveryContext,
    #[serde(flatten)]
    pub(crate) execution: crate::post_merge::Execution,
}

/// Re-check a PR's mergeable state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct MergeConflictDelivery {
    #[serde(flatten)]
    pub(crate) context: DeliveryContext,
    #[serde(flatten)]
    pub(crate) execution: crate::merge_conflict::Execution,
}

/// Announce a failed check run, once its annotations have been read.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CheckRunDelivery {
    #[serde(flatten)]
    pub(crate) context: DeliveryContext,
    #[serde(flatten)]
    pub(crate) execution: crate::check_run::Execution,
}

/// Open or grow the Task a check-run trigger asks for.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CheckRunTriggerDelivery {
    #[serde(flatten)]
    pub(crate) context: DeliveryContext,
    #[serde(flatten)]
    pub(crate) execution: crate::check_run_trigger::Execution,
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
/// Fails only on a non-string JSON map key, impossible for values parsed from a
/// webhook body but not worth an `unwrap`.
pub(crate) fn request(
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
    let (connection, trigger) = payload.context.resolve(&router).await?;
    // A trigger deleted between admission and execution closes none of the
    // tasks it created; opted-in Bindings still close.
    if payload.trigger_closes && trigger.is_none() {
        info!(
            id = %delivery.id,
            pr_number = payload.close.pr_number,
            "GitHub PR auto-close skipped: its trigger no longer exists"
        );
    }
    let trigger_closes = payload.trigger_closes && trigger.is_some();
    let rule = payload
        .context
        .rule_for_parse(&connection, trigger.as_ref());
    router
        .close_subject(&rule, &payload.close, trigger_closes)
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
    let rule = payload
        .context
        .rule_for_parse(&connection, trigger.as_ref());
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

async fn execute_check_run(
    router: Arc<GithubPrInboundRouter>,
    _delivery: moatless_adapter_model::delivery::AdapterDelivery,
    payload: CheckRunDelivery,
) -> Result<(), AdapterDeliveryFailure> {
    let (connection, trigger) = payload.context.resolve(&router).await?;
    router
        .run_check_run(&connection, trigger.as_ref(), &payload.execution)
        .await
        .map_err(AdapterDeliveryFailure::from)
}

async fn execute_check_run_trigger(
    router: Arc<GithubPrInboundRouter>,
    _delivery: moatless_adapter_model::delivery::AdapterDelivery,
    payload: CheckRunTriggerDelivery,
) -> Result<(), AdapterDeliveryFailure> {
    let (connection, trigger) = payload.context.resolve(&router).await?;
    router
        .run_check_run_trigger(&connection, trigger.as_ref(), &payload.execution)
        .await
        .map_err(AdapterDeliveryFailure::from)
}

/// Log line shared by the admission sites, so an operator can follow one
/// delivery from ingress into the queue.
pub(crate) fn log_admission(
    kind: &'static str,
    event_id: &str,
    admission: moatless_adapter_model::delivery::Admission,
) {
    moatless_adapter_model::ingress::log_admission(ADAPTER, kind, event_id, admission);
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
            requester: None,
            text: "hello".to_string(),
            file_ids: vec![],
            attachments: Vec::new(),
            thread_properties: serde_json::json!({
                "owner": "acme", "repo": "widgets", "pr_number": 42
            }),
            adapter_context: serde_json::json!({}),
            external_event_id: Some("github:pr:opened:42".to_string()),
            source_task_id: None,
            audience: Audience::Everyone,
            replies_to: Vec::new(),
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

    /// `CloseDelivery` and `PostMergeDelivery` wrap types owned by
    /// `adapter.rs`, whose fields this module cannot name.
    ///
    /// Pinning them as literal JSON is the only way to assert the stored shape
    /// here, so a rename or lost `flatten` fails in this test rather than one
    /// that needs a database.
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
                "pr_author_login": "hubot",
            },
            "trigger_closes": false,
        });
        let delivery: CloseDelivery = serde_json::from_value(stored.clone()).expect("deserialize");

        assert_eq!(delivery.context.connection_id, "conn-1");
        assert_eq!(delivery.close.pr_number, 42);
        assert!(delivery.close.merged);
        assert!(!delivery.trigger_closes);
        assert_eq!(serde_json::to_value(&delivery).expect("serialize"), stored);
    }

    /// A delivery stored without `trigger_closes` was admitted only for the
    /// trigger's auto-close, so it still closes the trigger's tasks.
    #[test]
    fn a_close_stored_without_the_trigger_flag_is_the_triggers() {
        let mut stored = serde_json::json!({
            "connection_id": "conn-1",
            "loop_id": "trigger-1",
            "repo_full_name": "acme/widgets",
            "close": {
                "pr_number": 42,
                "merged": false,
                "title": "",
                "html_url": "",
                "sender_login": "octocat",
            },
        });
        let delivery: CloseDelivery = serde_json::from_value(stored.clone()).expect("deserialize");
        assert!(delivery.trigger_closes);
        stored["trigger_closes"] = true.into();
        stored["close"]["pr_author_login"] = serde_json::Value::Null;
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
                "pr_author_login": "hubot",
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
            execution: crate::merge_conflict::Execution {
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

    #[test]
    fn a_check_run_payload_survives_the_round_trip() {
        let delivery = CheckRunDelivery {
            context: context(),
            execution: crate::check_run::Execution {
                owner: "acme".to_string(),
                repo: "widgets".to_string(),
                check_run_id: 103_925_595_227,
                annotations_count: 4,
                message: message(),
            },
        };
        let stored = serde_json::to_value(&delivery).expect("serialize");
        let back: CheckRunDelivery = serde_json::from_value(stored.clone()).expect("deserialize");

        assert_eq!(back.context.connection_id, "conn-1");
        // A check-run id is a `u64` behind a `flatten`ed sibling, which routes
        // through serde's buffering representation; assert it comes back
        // unrounded.
        assert_eq!(back.execution.check_run_id, 103_925_595_227);
        assert_eq!(back.execution.annotations_count, 4);
        assert_eq!(serde_json::to_value(&back).expect("serialize"), stored);
    }

    #[test]
    fn a_check_run_trigger_payload_survives_the_round_trip() {
        let delivery = CheckRunTriggerDelivery {
            context: context(),
            execution: crate::check_run_trigger::Execution {
                owner: "acme".to_string(),
                repo: "widgets".to_string(),
                pr_number: 42,
                head_sha: "deadbeef".to_string(),
                check_name: "soap-review-request".to_string(),
                conclusion: "neutral".to_string(),
                output_title: "Review requested".to_string(),
                output_summary: String::new(),
                skills: vec!["pr-review".to_string()],
                matched: vec![crate::check_run_trigger::MatchedRule {
                    id: "infra".to_string(),
                    skills: vec!["pr-review".to_string()],
                }],
                message: Some("Focus on the migration.".to_string()),
                default_branch: Some("main".to_string()),
                description: None,
            },
        };
        let stored = serde_json::to_value(&delivery).expect("serialize");
        let back: CheckRunTriggerDelivery =
            serde_json::from_value(stored.clone()).expect("deserialize");

        assert_eq!(back.execution.pr_number, 42);
        assert_eq!(back.execution.matched[0].id, "infra");
        assert_eq!(serde_json::to_value(&back).expect("serialize"), stored);
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
            kind::CHECK_RUN,
            kind::CHECK_RUN_TRIGGER,
        ];
        let mut unique = kinds.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len());
        assert!(kinds.iter().all(|k| k.starts_with("github.")));
    }
}
