//! GitHub webhook request policy and durable admission.
//!
//! The HTTP handler passes only transport facts through [`WebhookRequest`].
//! This module owns everything from request verification through classification
//! of the durable admission outcome.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use tracing::{debug, warn};


use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::delivery::AdapterDeliveryRequest;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::ingress::{IngressError, Outcome};
use moatless_types::adapter::AdapterConnection;
use moatless_types::loops::SubscriptionMatch;
use moatless_adapter_model::loops::LoopReader;
use moatless_backend_core::identity::ProviderIdentityLink;
use moatless_loops::bindings::dao::TaskBindingDao;

use super::adapter::GithubPrAdapter;
use super::config::GithubPrRuleConfig;
use super::webhook::{NormalizeContext, PrimaryWebhook};

/// Transport facts extracted from one GitHub webhook request.
pub(crate) struct WebhookRequest<'a> {
    event_type: &'a str,
    delivery_id: &'a str,
    signature: &'a str,
    body: &'a [u8],
}

impl<'a> WebhookRequest<'a> {
    pub(crate) fn new(
        event_type: &'a str,
        delivery_id: &'a str,
        signature: &'a str,
        body: &'a [u8],
    ) -> Self {
        Self {
            event_type,
            delivery_id,
            signature,
            body,
        }
    }
}

/// Reject one ingress-path failure as worth another delivery.
///
/// GitHub does not retry a failed delivery on its own: a non-2xx marks it
/// failed in the delivery log, where it stays redeliverable by an operator for
/// 30 days. Everything this path rejects is therefore [`IngressError::Ingress`]
/// — a recorded, recoverable loss instead of a silent one.
fn ingress(e: AdapterError) -> IngressError {
    IngressError::Ingress(e)
}

/// Verify, normalize, plan, and durably admit one GitHub webhook.
pub(crate) async fn receive(
    adapter_connections: &dyn AdapterConnectionDao,
    loops: &dyn LoopReader,
    users: &dyn ProviderIdentityLink,
    task_bindings: &Arc<dyn TaskBindingDao>,
    adapter_delivery: &dyn DeliveryAdmission,
    github_provider: &GithubPrAdapter,
    request: WebhookRequest<'_>,
) -> Result<Outcome, IngressError> {
    let WebhookRequest {
        event_type,
        delivery_id,
        signature,
        body,
    } = request;

    if event_type.is_empty() {
        return Err(ingress(
            SharedAdapterError::BadRequest("Missing X-GitHub-Event header".into()).into(),
        ));
    }

    let payload: serde_json::Value = serde_json::from_slice(body).map_err(|e| {
        ingress(SharedAdapterError::BadRequest(format!("Invalid JSON: {e}")).into())
    })?;

    let repo_full_name = payload
        .pointer("/repository/full_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ingress(SharedAdapterError::BadRequest("Missing repository.full_name".into()).into())
        })?;

    debug!(
        event_type,
        repo = repo_full_name,
        delivery_id,
        "GitHub webhook received"
    );

    let owner_login = payload
        .pointer("/repository/owner/login")
        .and_then(serde_json::Value::as_str)
        .or_else(|| repo_full_name.split('/').next())
        .ok_or_else(|| {
            ingress(SharedAdapterError::BadRequest("Missing repository.owner.login".into()).into())
        })?;

    let resolved = resolve_provider_event(adapter_connections, loops, owner_login, repo_full_name)
        .await
        .map_err(ingress)?;
    let Some((connection, rule)) = resolved else {
        debug!(
            repo = repo_full_name,
            owner = owner_login,
            "GitHub webhook ignored: no provider_connection"
        );
        return Ok(Outcome::ignored(event_type, "no_connection"));
    };

    // An *inactive* trigger suppresses delivery. A *missing* trigger does not —
    // events still fan out to existing PR bindings (the binding is the opt-in);
    // only new-task creation is gated on a trigger, in `routing::route_event`.
    if rule.as_ref().is_some_and(|r| !r.active) {
        debug!(
            repo = repo_full_name,
            "GitHub webhook ignored: task trigger inactive"
        );
        return Ok(Outcome::ignored(event_type, "inactive"));
    }

    // Verify webhook signature against the org connection's secret.
    let secret = connection
        .webhook_secret
        .as_deref()
        .or(github_provider.global_webhook_secret());

    if let Some(secret) = secret {
        super::signature::verify_signature(secret, body, signature).map_err(ingress)?;
    } else if !signature.is_empty() {
        warn!(
            repo = repo_full_name,
            "GitHub webhook has signature but no secret configured on connection, refusing"
        );
        return Err(ingress(
            SharedAdapterError::BadRequest(
                "Webhook signature provided but no secret configured for connection".into(),
            )
            .into(),
        ));
    }

    // No delivery-level gate here on purpose. A GitHub redelivery reuses the
    // same `X-GitHub-Delivery` id and is collapsed by durable admission below —
    // on any replica, after any restart, and past any expiry. Everything this
    // function does before admitting is either read-only or derives a value
    // from the delivery payload and writes that same value again (PR status,
    // review decision), so re-running it on a redelivery costs work and changes
    // nothing.

    // Normalization reads the trigger as a config bag (event filter, source
    // label, sender→user fallback). When no trigger matches, synthesize a
    // delivery-only one from the connection so the event is still normalized.
    // Task-creation decisions use the real `rule` below, not this fallback.
    let rule_for_parse = match &rule {
        Some(rule) => rule.clone(),
        None => super::routing::delivery_only_rule(&connection, repo_full_name),
    };

    // Global trigger policy gates the complete delivery, including primary,
    // post-merge, and merge-conflict work. Normalization retains the same
    // value-only checks so it is complete when exercised directly, and is also
    // where the derived signals are filtered by their own names.
    let rule_config = GithubPrRuleConfig::from_rule(&rule_for_parse);
    let sender_login = payload
        .pointer("/sender/login")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    // A filtered raw event can still be the carrier for a derived signal the
    // loop named, so reject here only when nothing at all can come of it.
    // Normalization applies the same filter per effect.
    if rule_config.event_filtered_out(event_type)
        && rule_config.event_filtered_out(super::config::MERGE_CONFLICT)
        && rule_config.event_filtered_out(super::config::POST_MERGE)
        && rule_config.event_filtered_out(super::config::PR_MERGED)
    {
        debug!(
            event_type,
            "GitHub webhook ignored: event filtered by trigger"
        );
        return Ok(Outcome::ignored(event_type, "event_filtered"));
    }
    if rule_config.is_ignored_sender(sender_login) {
        debug!(
            sender_login,
            event_type, "GitHub webhook ignored: sender in ignored_senders"
        );
        return Ok(Outcome::ignored(event_type, "ignored_sender"));
    }

    let (owner, repo) = repo_full_name.split_once('/').ok_or_else(|| {
        ingress(SharedAdapterError::BadRequest("Missing repository.full_name".into()).into())
    })?;
    let normalized = super::webhook::normalize(
        event_type,
        &payload,
        &NormalizeContext {
            rule: &rule_for_parse,
            owner,
            repo,
            sender_login,
            frontend_url: github_provider.frontend_url(),
            users,
        },
    )
    .await;

    // Observations are scheduled before the primary result is propagated. This
    // pins the existing partial-payload ordering even though today's handlers
    // have no separate post-observation rejection for the documented Ticket 01
    // partial shape.
    for observation in normalized.status_observations {
        super::status::apply_webhook_observation(task_bindings.clone(), observation);
    }
    let primary = normalized.primary.map_err(ingress)?;

    let delivery_context = super::delivery::DeliveryContext {
        connection_id: connection.id.clone(),
        loop_id: rule.as_ref().map(|r| r.id.clone()),
        repo_full_name: repo_full_name.to_string(),
    };
    let event_id = delivery_event_id(delivery_id, body);
    let mut plan = Vec::with_capacity(4);

    match primary {
        PrimaryWebhook::Message(msg) => {
            plan.push(
                super::delivery::request(
                    super::delivery::kind::PR_MESSAGE,
                    event_id.clone(),
                    &super::delivery::PrMessageDelivery {
                        context: delivery_context.clone(),
                        message: *msg,
                    },
                )
                .map_err(ingress)?,
            );
        }
        PrimaryWebhook::CloseTask(info) => {
            // Auto-close only applies to trigger-created tasks; with no trigger
            // there is nothing to close. The merged-PR notice below is not
            // gated on a trigger, so this skips the close alone.
            if rule.is_none() {
                debug!(
                    repo = repo_full_name,
                    "PR closed but no trigger for this repo — nothing to auto-close"
                );
            } else {
                plan.push(
                    super::delivery::request(
                        super::delivery::kind::CLOSE,
                        event_id.clone(),
                        &super::delivery::CloseDelivery {
                            context: delivery_context.clone(),
                            close: info,
                        },
                    )
                    .map_err(ingress)?,
                );
            }
        }
        PrimaryWebhook::Ignore(reason) => {
            debug!(
                event_type,
                reason = reason.as_str(),
                "GitHub PR event ignored"
            );
        }
    }

    // Merged PR: a merge reaches this endpoint as `pull_request` `closed` with
    // `merged: true`, where the primary effect is the trigger's auto-close (or
    // nothing at all). Notifying the tasks that outlive the pull request is a
    // separate effect with its own retry budget, so it is admitted as its own
    // kind rather than folded into the close.
    if let Some(merged) = normalized.merged_candidate {
        plan.push(
            super::delivery::request(
                super::delivery::kind::PR_MERGED,
                event_id.clone(),
                &super::delivery::PrMergedDelivery {
                    context: delivery_context.clone(),
                    execution: super::merged::Execution {
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        merged,
                    },
                },
            )
            .map_err(ingress)?,
        );
    }

    // Post-merge CI: a base-branch `workflow_run` completing carries no PR
    // reference, so it never reaches the PR-keyed handlers above. Resolve the
    // merged PR from the commit sha and notify its still-connected tasks.
    // Admitted rather than run here because it makes several GitHub API calls;
    // deduped per run id downstream.
    if let Some(run) = normalized.post_merge_candidate {
        plan.push(
            super::delivery::request(
                super::delivery::kind::POST_MERGE,
                event_id.clone(),
                &super::delivery::PostMergeDelivery {
                    context: delivery_context.clone(),
                    execution: super::post_merge::Execution {
                        // Kept alongside `repo_full_name` because the ingress path
                        // historically split these facts before admission; the
                        // durable payload must retain the same stored values.
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        run,
                        sender_login: sender_login.to_string(),
                    },
                },
            )
            .map_err(ingress)?,
        );
    }

    // Merge-conflict check, independent of the primary event's outcome: any
    // event that names a PR re-checks its mergeable state, since a conflict can
    // appear out of band (e.g. the base branch moved) with no dedicated
    // webhook. Deduped per head sha downstream, and a no-op when no task is
    // bound to the PR.
    if let Some(candidate) = normalized.merge_conflict_candidate {
        plan.push(
            super::delivery::request(
                super::delivery::kind::MERGE_CONFLICT,
                event_id,
                &super::delivery::MergeConflictDelivery {
                    context: delivery_context,
                    execution: super::merge_conflict::Execution {
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        pr_number: candidate.pr_number,
                        sender_login: sender_login.to_string(),
                    },
                },
            )
            .map_err(ingress)?,
        );
    }

    admit_sequentially(adapter_delivery, plan).await?;

    Ok(Outcome::Processed {
        event_type: event_type.to_string(),
    })
}

/// Look up the connection and optional matching Subscription for one event.
///
/// A connection is required for signature and tenant policy. A Subscription is
/// optional because existing PR bindings remain opted in even when no trigger
/// matches; only trigger-created Task behavior requires the real Subscription.
async fn resolve_provider_event(
    adapter_connections: &dyn AdapterConnectionDao,
    loops: &dyn LoopReader,
    org: &str,
    repo_full_name: &str,
) -> Result<Option<(AdapterConnection, Option<SubscriptionMatch>)>, AdapterError> {
    let connection = adapter_connections
        .find_by_scope_value(super::delivery::ADAPTER, org)
        .await?;
    let Some(connection) = connection else {
        return Ok(None);
    };

    let rule = loops
        .subscription_for_source(super::delivery::ADAPTER, repo_full_name)
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

/// Use GitHub's delivery id, or a body digest when that header is absent.
fn delivery_event_id(delivery_id: &str, body: &[u8]) -> String {
    if !delivery_id.is_empty() {
        return delivery_id.to_string();
    }
    format!("sha256:{}", hex::encode(Sha256::digest(body)))
}

/// Admit every planned kind in order, stopping at the first failure.
///
/// Each successful admission commits independently. A redelivery therefore
/// sees earlier kinds as duplicates and resumes at the first missing kind.
async fn admit_sequentially(
    adapter_delivery: &dyn DeliveryAdmission,
    plan: Vec<AdapterDeliveryRequest>,
) -> Result<(), IngressError> {
    for request in plan {
        let kind = request.kind;
        let event_id = request.event_id.clone();
        let admission =
            moatless_adapter_model::ingress::admit_delivery(adapter_delivery, request).await?;
        super::delivery::log_admission(kind, &event_id, admission);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::delivery_event_id;

    #[test]
    fn the_delivery_header_is_the_idempotency_key() {
        assert_eq!(delivery_event_id("d-1", b"{}"), "d-1");
    }

    #[test]
    fn a_missing_delivery_header_falls_back_to_the_body_digest() {
        let a = delivery_event_id("", br#"{"action":"opened"}"#);
        let b = delivery_event_id("", br#"{"action":"closed"}"#);
        let a_again = delivery_event_id("", br#"{"action":"opened"}"#);

        assert!(a.starts_with("sha256:"), "{a}");
        assert_ne!(a, b, "different deliveries must not share a record");
        assert_eq!(a, a_again, "an identical retry must collapse");
    }
}
