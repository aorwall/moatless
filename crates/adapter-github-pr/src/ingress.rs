//! GitHub webhook request policy and durable admission.
//!
//! The HTTP handler passes only transport facts through [`WebhookRequest`].
//! This module owns everything from request verification through classification
//! of the durable admission outcome.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::delivery::AdapterDeliveryRequest;
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::ingress::{IngressError, Outcome};
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::identity::ProviderIdentityLink;
use moatless_types::adapter::AdapterConnection;
use moatless_types::loops::SubscriptionMatch;

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

/// Wrap one ingress-path failure as [`IngressError::Ingress`].
///
/// `handlers.rs` logs and meters this variant, then acknowledges the
/// delivery with `200` — GitHub never redelivers it.
fn ingress(e: AdapterError) -> IngressError {
    IngressError::Ingress(e)
}

/// Verify, normalize, plan, and durably admit one GitHub webhook.
pub(crate) async fn receive(
    adapter_connections: &dyn ConnectionLookup,
    loops: &dyn LoopReader,
    users: &dyn ProviderIdentityLink,
    task_bindings: &Arc<Bindings>,
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

    // No delivery-level gate here on purpose: a GitHub redelivery reuses the
    // same `X-GitHub-Delivery` id, collapsed by durable admission below on
    // any replica, after any restart, past any expiry. Everything above is
    // read-only or rewrites the same derived value (PR status, review
    // decision), so a redelivery costs work but changes nothing.

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
        && rule_config.event_filtered_out(super::config::CHECK_RUN_TRIGGER)
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
    // `catch_unwind` turns a panic mid-normalization into an
    // `IngressError::Ingress`, which `handlers.rs` logs and meters, rather
    // than an unwound request task with nothing recorded.
    let normalized = AssertUnwindSafe(super::webhook::normalize(
        event_type,
        &payload,
        &NormalizeContext {
            rule: &rule_for_parse,
            owner,
            repo,
            sender_login,
            pr_author_login: super::webhook::pr_author_login(&payload),
            frontend_url: github_provider.frontend_url(),
            users,
        },
    ))
    .catch_unwind()
    .await
    .map_err(|panic| {
        let reason = panic_message(panic.as_ref());
        warn!(
            event_type,
            repo = repo_full_name,
            delivery_id,
            reason,
            "GitHub webhook normalization panicked — recording as a recoverable ingress failure"
        );
        ingress(
            SharedAdapterError::Internal(format!("webhook normalization panicked: {reason}"))
                .into(),
        )
    })?;

    // Observations apply before `primary`'s error is surfaced, so a partial
    // failure still updates PR status.
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
        PrimaryWebhook::CheckRun(execution) => {
            plan.push(
                super::delivery::request(
                    super::delivery::kind::CHECK_RUN,
                    event_id.clone(),
                    &super::delivery::CheckRunDelivery {
                        context: delivery_context.clone(),
                        execution: *execution,
                    },
                )
                .map_err(ingress)?,
            );
        }
        PrimaryWebhook::CheckRunTrigger(execution) => {
            plan.push(
                super::delivery::request(
                    super::delivery::kind::CHECK_RUN_TRIGGER,
                    event_id.clone(),
                    &super::delivery::CheckRunTriggerDelivery {
                        context: delivery_context.clone(),
                        execution: *execution,
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
                        // splits these facts before admission; the durable
                        // payload must retain the same stored values.
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
/// A connection is required for signature and tenant policy; a Subscription is
/// optional since existing PR bindings stay opted in even with no matching
/// trigger.
async fn resolve_provider_event(
    adapter_connections: &dyn ConnectionLookup,
    loops: &dyn LoopReader,
    org: &str,
    repo_full_name: &str,
) -> Result<Option<(AdapterConnection, Option<SubscriptionMatch>)>, AdapterError> {
    let connection = adapter_connections
        .by_scope_value(super::delivery::ADAPTER, org)
        .await?;
    let Some(connection) = connection else {
        return Ok(None);
    };

    let same_connection = |rule: &SubscriptionMatch| {
        let same = rule.connection_id == connection.id;
        if !same {
            debug!(
                source_matcher = %rule.source_matcher,
                "Task trigger belongs to a different connection — ignoring trigger; \
                 delivering to existing bindings only"
            );
        }
        same
    };
    let mut rule = loops
        .subscription_for_source(super::delivery::ADAPTER, repo_full_name)
        .await?
        .filter(same_connection);
    // An exact loop wins. The org's `<owner>/*` loop is only ever a check-run
    // trigger, re-checked here so a row written past sync's check
    // cannot turn every repository's PR openings into Tasks.
    if rule.is_none() {
        rule = loops
            .subscription_for_source(super::delivery::ADAPTER, &super::routing::org_wildcard(org))
            .await?
            .filter(same_connection)
            .filter(|rule| {
                GithubPrRuleConfig::from_rule(rule).create_on == super::config::CreateOn::CheckRun
            })
            .map(|rule| super::routing::for_repository(rule, repo_full_name));
    }

    Ok(Some((connection, rule)))
}

/// The human-readable half of a panic payload, when there is one.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "unknown panic payload"
    }
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
/// Each successful admission commits independently, so a redelivery sees
/// earlier kinds as duplicates and resumes at the first missing kind.
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
    use super::*;

    use std::net::TcpListener;
    use std::time::Duration;

    use async_trait::async_trait;
    use sqlx::postgres::PgPoolOptions;

    use moatless_adapter_model::test_support::{
        FakeAdmission, FakeAdmissionAnswer, InMemoryConnections, connection,
    };
    use moatless_ports::identity::{MockGitHostTokenIssuer, MockProviderIdentityLink};
    use moatless_types::adapter::AdapterKind;
    use moatless_types::store::StoreError;

    /// A [`LoopReader`] with no Subscriptions at all: every event falls back
    /// to the delivery-only rule `receive` synthesizes from the connection.
    struct NoSubscriptions;

    #[async_trait]
    impl LoopReader for NoSubscriptions {
        async fn subscription(
            &self,
            _loop_id: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(None)
        }

        async fn runnable_subscription(
            &self,
            _loop_id: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(None)
        }

        async fn subscription_for_source(
            &self,
            _adapter_kind: &str,
            _source_matcher: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(None)
        }

        async fn subscriptions_for_source(
            &self,
            _adapter_kind: &str,
            _source_matcher: &str,
        ) -> Result<Vec<SubscriptionMatch>, StoreError> {
            Ok(vec![])
        }

        async fn wildcard_subscription(
            &self,
            _adapter_kind: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(None)
        }
    }

    /// An `Arc<Bindings>` over a pool that refuses to connect. `receive`'s only
    /// DB write on this path is `status::apply_webhook_observation`'s, fired
    /// off as a detached task, and the `pull_request` `opened` event this test
    /// sends produces no status observation anyway — so nothing here needs a
    /// live Postgres.
    fn unconnected_bindings() -> std::sync::Arc<moatless_loops::bindings::operations::Bindings> {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("can bind an ephemeral loopback port")
            .local_addr()
            .expect("a bound listener has a local address")
            .port();
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy(&format!(
                "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
            ))
            .expect("connect_lazy only parses the URL; it never dials out");
        moatless_loops::compose::loop_graph(pool).bindings()
    }

    fn opened_pr_payload() -> serde_json::Value {
        serde_json::json!({
            "action": "opened",
            "repository": {
                "full_name": "acme/widgets",
                "owner": { "login": "acme" },
            },
            "sender": { "login": "octocat" },
            "pull_request": {
                "number": 42,
                "title": "Add a feature",
                "html_url": "https://github.com/acme/widgets/pull/42",
                "head": { "ref": "feature" },
                "base": { "ref": "main" },
            },
        })
    }

    /// A panic inside normalization (here, a user-lookup port that panics
    /// instead of answering) must come out of `receive` as a logged, metered
    /// [`IngressError::Ingress`] — never as a propagated panic that unwinds
    /// the request task with nothing logged.
    #[tokio::test]
    async fn a_panic_during_normalization_is_recorded_not_propagated() {
        let connections =
            InMemoryConnections::new(vec![connection("conn-1", AdapterKind::Github, "acme")]);
        let loops = NoSubscriptions;
        let mut users = MockProviderIdentityLink::new();
        users
            .expect_user_with_login()
            .returning(|_, _| panic!("boom: user lookup should never panic in production"));
        let task_bindings = unconnected_bindings();
        let admission = FakeAdmission::new(FakeAdmissionAnswer::Admitted);
        let github_provider = GithubPrAdapter::new(
            None,
            "https://app.example".to_string(),
            "https://api.github.com".to_string(),
            std::sync::Arc::new(MockGitHostTokenIssuer::new()),
            moatless_types::http::HttpTimeouts {
                connect: Duration::from_secs(5),
                total: Duration::from_secs(30),
            },
        );

        let body = serde_json::to_vec(&opened_pr_payload()).expect("serializes");
        let request = WebhookRequest::new("pull_request", "delivery-1", "", &body);

        let result = receive(
            &connections as &dyn ConnectionLookup,
            &loops as &dyn LoopReader,
            &users,
            &task_bindings,
            &admission,
            &github_provider,
            request,
        )
        .await;

        match result {
            Err(IngressError::Ingress(_)) => {}
            other => panic!("expected a recorded Ingress failure, got {other:?}"),
        }
        assert!(
            admission.seen().is_empty(),
            "normalization panicked before anything was planned for admission"
        );
    }

    fn ci_payload(event_type: &str, head_sha: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "completed",
            "repository": {
                "full_name": "acme/widgets",
                "owner": { "login": "acme" },
            },
            "sender": { "login": "github-actions[bot]" },
            event_type: {
                "id": 7,
                "head_sha": head_sha,
                "conclusion": "success",
                "pull_requests": [{ "number": 42 }],
            },
        })
    }

    /// One CI push fans out into many `check_run` webhooks, and each
    /// merge-conflict check on a bound PR reads the PR through one user's
    /// quota, so only a suite's completion schedules one.
    #[tokio::test]
    async fn only_completed_check_suites_admit_a_merge_conflict_check() {
        let connections =
            InMemoryConnections::new(vec![connection("conn-1", AdapterKind::Github, "acme")]);
        let mut users = MockProviderIdentityLink::new();
        users.expect_user_with_login().returning(|_, _| Ok(None));
        let task_bindings = unconnected_bindings();
        let admission = FakeAdmission::new(FakeAdmissionAnswer::Admitted);
        let github_provider = GithubPrAdapter::new(
            None,
            "https://app.example".to_string(),
            "https://api.github.com".to_string(),
            std::sync::Arc::new(MockGitHostTokenIssuer::new()),
            moatless_types::http::HttpTimeouts {
                connect: Duration::from_secs(5),
                total: Duration::from_secs(30),
            },
        );

        let deliveries = [
            ("check_run", "run-1", "abc"),
            ("check_run", "run-2", "abc"),
            ("check_suite", "suite-1", "abc"),
            ("check_suite", "suite-2", "abc"),
            ("check_suite", "suite-3", "def"),
        ];
        for (event_type, delivery_id, head_sha) in deliveries {
            let body = serde_json::to_vec(&ci_payload(event_type, head_sha)).expect("serializes");
            receive(
                &connections as &dyn ConnectionLookup,
                &NoSubscriptions as &dyn LoopReader,
                &users,
                &task_bindings,
                &admission,
                &github_provider,
                WebhookRequest::new(event_type, delivery_id, "", &body),
            )
            .await
            .expect("a CI webhook is accepted");
        }

        let checks: Vec<String> = admission
            .seen()
            .into_iter()
            .filter(|request| request.kind == super::super::delivery::kind::MERGE_CONFLICT)
            .map(|request| request.event_id)
            .collect();
        assert_eq!(
            checks,
            ["suite-1", "suite-2", "suite-3"],
            "no check per run; each suite rechecks, so the last one records the settled state"
        );
    }

    /// Subscriptions keyed by their exact source matcher, as
    /// `find_subscription_by_source_matcher` reads them.
    struct ByMatcher(Vec<SubscriptionMatch>);

    #[async_trait]
    impl LoopReader for ByMatcher {
        async fn subscription(&self, id: &str) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(self.0.iter().find(|s| s.id == id).cloned())
        }

        async fn runnable_subscription(
            &self,
            id: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            self.subscription(id).await
        }

        async fn subscription_for_source(
            &self,
            _adapter_kind: &str,
            source_matcher: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(self
                .0
                .iter()
                .find(|s| s.source_matcher == source_matcher)
                .cloned())
        }

        async fn subscriptions_for_source(
            &self,
            _adapter_kind: &str,
            _source_matcher: &str,
        ) -> Result<Vec<SubscriptionMatch>, StoreError> {
            Ok(vec![])
        }

        async fn wildcard_subscription(
            &self,
            _adapter_kind: &str,
        ) -> Result<Option<SubscriptionMatch>, StoreError> {
            Ok(None)
        }
    }

    fn loop_for(
        id: &str,
        source_matcher: &str,
        adapter_config: serde_json::Value,
    ) -> SubscriptionMatch {
        let conn = connection("conn-1", AdapterKind::Github, "acme");
        SubscriptionMatch {
            id: id.to_string(),
            source_matcher: source_matcher.to_string(),
            adapter_config: Some(adapter_config),
            ..super::super::routing::delivery_only_rule(&conn, source_matcher)
        }
    }

    fn trigger_config() -> serde_json::Value {
        serde_json::json!({
            "create_on": "check_run",
            "check_run": {"names": ["soap-review-request"], "app_ids": [7]},
        })
    }

    async fn resolved_loop(loops: &ByMatcher) -> Option<SubscriptionMatch> {
        let connections =
            InMemoryConnections::new(vec![connection("conn-1", AdapterKind::Github, "acme")]);
        resolve_provider_event(&connections, loops, "acme", "acme/widgets")
            .await
            .expect("resolves")
            .expect("has a connection")
            .1
    }

    #[tokio::test]
    async fn a_repository_without_a_loop_falls_back_to_the_org_trigger_loop() {
        let rule = resolved_loop(&ByMatcher(vec![loop_for(
            "tr_org",
            "acme/*",
            trigger_config(),
        )]))
        .await
        .expect("the org loop");
        assert_eq!(rule.id, "tr_org");
        assert_eq!(rule.source_matcher, "acme/widgets");
    }

    #[tokio::test]
    async fn an_exact_loop_wins_over_the_org_trigger_loop() {
        let rule = resolved_loop(&ByMatcher(vec![
            loop_for("tr_org", "acme/*", trigger_config()),
            loop_for("tr_exact", "acme/widgets", serde_json::json!({})),
        ]))
        .await
        .expect("the exact loop");
        assert_eq!(rule.id, "tr_exact");
    }

    #[tokio::test]
    async fn an_org_wildcard_without_create_on_is_never_matched() {
        let rule = resolved_loop(&ByMatcher(vec![loop_for(
            "tr_org",
            "acme/*",
            serde_json::json!({}),
        )]))
        .await;
        assert!(rule.is_none());
    }

    /// The whole ingress path: a gate's check run on a repository only the org
    /// loop covers is admitted as a check-run trigger under that loop.
    #[tokio::test]
    async fn a_check_run_trigger_is_admitted_under_the_org_loop() {
        let connections =
            InMemoryConnections::new(vec![connection("conn-1", AdapterKind::Github, "acme")]);
        let loops = ByMatcher(vec![loop_for("tr_org", "acme/*", trigger_config())]);
        let mut users = MockProviderIdentityLink::new();
        users.expect_user_with_login().returning(|_, _| Ok(None));
        let admission = FakeAdmission::new(FakeAdmissionAnswer::Admitted);
        let github_provider = GithubPrAdapter::new(
            None,
            "https://app.example".to_string(),
            "https://api.github.com".to_string(),
            std::sync::Arc::new(MockGitHostTokenIssuer::new()),
            moatless_types::http::HttpTimeouts {
                connect: Duration::from_secs(5),
                total: Duration::from_secs(30),
            },
        );
        let marker = format!(
            "{}{} -->",
            crate::check_run_trigger::MARKER_PREFIX,
            serde_json::json!({"head": "abc", "skills": ["pr-review"], "matched": []})
        );
        let body = serde_json::to_vec(&serde_json::json!({
            "action": "completed",
            "repository": {"full_name": "acme/widgets", "owner": {"login": "acme"}},
            "sender": {"login": "gate[bot]"},
            "check_run": {
                "id": 1,
                "name": "soap-review-request",
                "head_sha": "abc",
                "conclusion": "neutral",
                "app": {"id": 7},
                "output": {"text": marker},
                "pull_requests": [{"number": 42}],
            },
        }))
        .expect("serializes");

        receive(
            &connections as &dyn ConnectionLookup,
            &loops as &dyn LoopReader,
            &users,
            &unconnected_bindings(),
            &admission,
            &github_provider,
            WebhookRequest::new("check_run", "delivery-1", "", &body),
        )
        .await
        .expect("accepted");

        let requests: Vec<_> = admission
            .seen()
            .into_iter()
            .filter(|request| request.kind == super::super::delivery::kind::CHECK_RUN_TRIGGER)
            .collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].payload["loop_id"], "tr_org");
        assert_eq!(requests[0].payload["pr_number"], 42);
    }

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
