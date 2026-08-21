//! GitHub identity selection and credential materialization by use case.

use std::sync::Arc;

use tracing::debug;

use moatless_backend_core::identity::ProviderIdentityLink;
use moatless_types::access_control::Actor;
use moatless_identity::git_host_repo::GitHostRepo;
use moatless_identity::git_host_tokens::{GitHostTokenError, GitHostTokens};
use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::models::{SubscriptionMatch, TaskBinding};
use moatless_types::adapter::BindingContext;

use super::api::GithubPrApi;

pub(super) struct GithubPrAuth {
    /// The one way to a git-host token. This Adapter holds no credential of its
    /// own: every use case below names an identity and a repository.
    git_host_tokens: Arc<GitHostTokens>,
    /// REST root every client below is built against, when a test injected one.
    ///
    /// Unlike Slack and Telegram there is no single client to hand in: a token
    /// is selected per call, so the injection point is the base URL rather than
    /// the client. Test-only, because production reads
    /// [`super::api::api_base`] and has no second root to point at.
    #[cfg(test)]
    api_base: Option<String>,
}

pub(super) struct AuthenticatedGithub {
    pub(super) user_id: String,
    pub(super) api: GithubPrApi,
}

impl GithubPrAuth {
    pub(super) fn new(git_host_tokens: Arc<GitHostTokens>) -> Self {
        Self {
            git_host_tokens,
            #[cfg(test)]
            api_base: None,
        }
    }

    #[cfg(test)]
    pub(super) fn set_api_base(&mut self, base_url: impl Into<String>) {
        self.api_base = Some(base_url.into());
    }

    /// The one place a REST client is built, so the injected root reaches every
    /// use case without each of them knowing a test root exists.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "the injected base URL is the only state read here, and it \
                      exists only in test builds"
        )
    )]
    fn client(&self, token: &str) -> GithubPrApi {
        #[cfg(test)]
        if let Some(base_url) = &self.api_base {
            return GithubPrApi::with_base_url(token, base_url);
        }
        GithubPrApi::new(token)
    }

    pub(super) async fn client_for_durable_work(
        &self,
        github_login: &str,
        rule: &SubscriptionMatch,
        users: &dyn ProviderIdentityLink,
        repo: &GitHostRepo,
    ) -> Result<AuthenticatedGithub, AdapterError> {
        let user_id = resolve_sender_user(github_login, rule, users).await;
        let api = self.client_for(&Actor::of(&user_id), repo).await?;
        Ok(AuthenticatedGithub { user_id, api })
    }

    pub(super) async fn client_for_outbound_reply(
        &self,
        author_user_id: Option<&str>,
        binding: &TaskBinding,
        rule: Option<&SubscriptionMatch>,
        repo: &GitHostRepo,
    ) -> Result<GithubPrApi, AdapterError> {
        let actor = outbound_actor(author_user_id, binding, rule)?;
        self.client_for(&actor, repo).await
    }

    pub(super) async fn client_for_reaction(
        &self,
        binding: &BindingContext,
        repo: &GitHostRepo,
    ) -> Result<GithubPrApi, AdapterError> {
        let actor = reaction_actor(binding)?;
        self.client_for(&actor, repo).await
    }

    pub(super) async fn client_for_live_status(
        &self,
        acting_user: &str,
        repo: &GitHostRepo,
    ) -> Result<GithubPrApi, AdapterError> {
        self.client_for(&Actor::of(acting_user), repo).await
    }

    /// Mint a GitHub client from `actor`'s OWN row, scoped to `repo`.
    ///
    /// This is the only credential rule this Adapter has, for every one of its
    /// use cases. The identity that pushed the branch is the identity that
    /// opens the pull request, posts the comment and adds the reaction.
    ///
    /// It performs no scope walk: the repository's persisted scope, the org
    /// credential and the deployment's environment are never consulted. An
    /// identity with no row cannot reach GitHub here rather than silently
    /// acting as somebody else.
    ///
    /// Which repository is named therefore only scopes the token, never whether
    /// one is found: a pull request against a repo the task is not bound to
    /// resolves exactly like one against its own.
    pub(super) async fn client_for(
        &self,
        actor: &Actor,
        repo: &GitHostRepo,
    ) -> Result<GithubPrApi, AdapterError> {
        let token = self
            .git_host_tokens
            .token(actor, repo)
            .await
            .map_err(|source| adapter_error(actor, source))?;
        Ok(self.client(&token))
    }
}

/// A failed resolution keeps the seam's own message: it already distinguishes
/// "has not connected" from "cannot reach this repository" from "broken", and
/// those have three different remedies.
fn adapter_error(actor: &Actor, source: GitHostTokenError) -> AdapterError {
    match source {
        GitHostTokenError::Database(e) => AdapterError::from(SharedAdapterError::AdapterApi(
            format!("GitHub token lookup failed for {actor}: {e}"),
        )),
        other => SharedAdapterError::AdapterApi(other.to_string()).into(),
    }
}

/// The identity an outbound reply acts as: the task's own author, else the
/// binding's filter user, else the subscription's run-as user.
fn outbound_actor(
    author_user_id: Option<&str>,
    binding: &TaskBinding,
    rule: Option<&SubscriptionMatch>,
) -> Result<Actor, AdapterError> {
    outbound_user_id(author_user_id, binding, rule)
        .map(Actor::of)
        .ok_or_else(|| {
            SharedAdapterError::AdapterApi(
                "Cannot resolve GitHub token: no task owner, binding filter_user_id, \
                 or task trigger attached"
                    .into(),
            )
            .into()
        })
}

/// The identity a reaction acts as.
///
/// A `BindingContext` carries no author and no subscription, so the precedence
/// starts at the binding's filter user. There is no deployment-wide fallback
/// below it: a reaction is posted by somebody, and nobody is not an answer.
fn reaction_actor(binding: &BindingContext) -> Result<Actor, AdapterError> {
    binding_filter_user_id(&binding.destination_config, &binding.metadata)
        .map(Actor::of)
        .ok_or_else(|| {
            SharedAdapterError::AdapterApi(
                "Cannot resolve GitHub token for a reaction: the binding names no \
                 filter_user_id"
                    .into(),
            )
            .into()
        })
}

pub(super) async fn resolve_sender_user(
    github_login: &str,
    rule: &SubscriptionMatch,
    users: &dyn ProviderIdentityLink,
) -> String {
    if let Ok(Some(user_id)) = users.user_with_login(super::delivery::ADAPTER, github_login).await {
        return user_id;
    }

    debug!(
        github_login,
        fallback = %rule.effective_user_id(),
        "GitHub user not mapped, using rule owner"
    );
    rule.effective_user_id().to_string()
}

fn outbound_user_id(
    author_user_id: Option<&str>,
    binding: &TaskBinding,
    rule: Option<&SubscriptionMatch>,
) -> Option<String> {
    author_user_id
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            binding_filter_user_id(&binding.destination_config, &binding.metadata)
                .map(str::to_string)
        })
        .or_else(|| {
            rule.map(SubscriptionMatch::effective_user_id)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn binding_filter_user_id<'a>(
    destination_config: &'a serde_json::Value,
    metadata: &'a serde_json::Value,
) -> Option<&'a str> {
    destination_config
        .get("filter_user_id")
        .or_else(|| metadata.get("filter_user_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use moatless_types::adapter::AdapterKind;
    use super::*;
    use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
    use moatless_identity::auth::test_support::{StubProviderTokens, github_row};

    fn auth_with_tokens(provider_tokens: Arc<dyn ProviderTokenDao>) -> GithubPrAuth {
        GithubPrAuth::new(moatless_identity::git_host_tokens::seam_over(
            provider_tokens,
        ))
    }

    fn binding_with_filter(filter_user_id: Option<&str>) -> TaskBinding {
        let now = chrono::Utc::now();
        let filter = serde_json::json!({"filter_user_id": filter_user_id});
        TaskBinding {
            id: "binding".into(),
            task_id: "task".into(),
            adapter_kind: AdapterKind::GithubPr,
            subject_kind: "github_pr".into(),
            subject_key: "soaplabs/repo#42".into(),
            connection_id: "connection".into(),
            loop_id: None,
            destination_config: filter.clone(),
            metadata: filter,
            last_external_event_id: None,
            origin: "connect".into(),
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
        }
    }

    fn rule_with_run_as(user_id: &str) -> SubscriptionMatch {
        use chrono::Utc;
        use moatless_types::access_control::Scope;
        use moatless_types::loops::RoutingMode;

        SubscriptionMatch {
            name: "test-loop".to_string(),
            id: "rule".into(),
            adapter_kind: AdapterKind::Github,
            connection_id: "connection".into(),
            source_matcher: "soaplabs/repo".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repository".into(),
            prompt_template: None,
            task_name: Some("Review".into()),
            tag_ids: vec![],
            skills: Vec::new(),
            run_as_user_id: Some(user_id.into()),
            agent_type: "claude-code".into(),
            inactive_ttl_days: None,
            allow_direct_send: false,
            created_by: "creator".into(),
            adapter_config: None,
            ongoing_task_id: None,
            close_task_on_close: false,
            notify_on_success: false,
            active: true,
            scope: Scope::User,
            owner_user_id: Some("creator".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn outbound_identity_precedence_is_task_then_binding_then_subscription() {
        let binding = binding_with_filter(Some("filter-user"));
        let rule = rule_with_run_as("run-as-user");
        assert_eq!(
            outbound_user_id(Some("task-owner"), &binding, Some(&rule)).as_deref(),
            Some("task-owner")
        );
        assert_eq!(
            outbound_user_id(None, &binding, Some(&rule)).as_deref(),
            Some("filter-user")
        );
        assert_eq!(
            outbound_user_id(None, &binding_with_filter(None), Some(&rule)).as_deref(),
            Some("run-as-user")
        );
        assert_eq!(
            outbound_user_id(None, &binding_with_filter(None), None),
            None
        );
    }

    fn binding_context(filter_user_id: Option<&str>) -> BindingContext {
        BindingContext {
            binding_id: "binding".into(),
            loop_id: None,
            task_id: "task".into(),
            subject_kind: "github_pr".into(),
            subject_key: "soaplabs/repo#42".into(),
            destination_config: match filter_user_id {
                Some(id) => serde_json::json!({ "filter_user_id": id }),
                None => serde_json::json!({}),
            },
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            last_message_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn reaction_identity_is_the_binding_filter_user_and_nothing_below_it() {
        assert_eq!(
            reaction_actor(&binding_context(Some("filter-user")))
                .expect("a named filter user is an identity")
                .user_id(),
            "filter-user"
        );
        assert!(
            reaction_actor(&binding_context(None)).is_err(),
            "a binding naming nobody has no deployment-wide fallback to reach"
        );
    }

    #[tokio::test]
    async fn a_reaction_acts_as_the_binding_filter_user() {
        let auth = auth_with_tokens(Arc::new(StubProviderTokens::with_user_row_for(
            "filter-user",
            github_row("manual", "filter-token"),
        )));
        assert!(
            auth.client_for_reaction(
                &binding_context(Some("filter-user")),
                &GitHostRepo::github("soaplabs", "repo"),
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn an_identity_with_no_row_cannot_reach_github() {
        // The deleted fallback, asserted as absent: no env token, no org row,
        // no other identity.
        let auth = auth_with_tokens(Arc::new(StubProviderTokens::default()));
        let error = auth
            .client_for(
                &Actor::of("unmapped-user"),
                &GitHostRepo::github("soaplabs", "repo"),
            )
            .await
            .err()
            .expect("an identity holding no row has no token");
        assert!(
            error.to_string().contains("has not connected"),
            "got {error}"
        );
    }

    #[tokio::test]
    async fn an_installation_row_resolves_against_the_named_repository() {
        let mut row = github_row(
            moatless_identity::auth::provider_tokens::AUTH_METHOD_APP_INSTALLATION,
            moatless_identity::auth::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER,
        );
        row.provider_user_id = Some("missing-test-app".to_string());
        let auth = auth_with_tokens(Arc::new(StubProviderTokens::with_user_row_for(
            "bot-user", row,
        )));

        // The repository is now mandatory in the signature, so the only way to
        // fail is the registry lookup — which is where an unregistered App key
        // must surface rather than being masked by an owner-wide mint.
        let error = auth
            .client_for(
                &Actor::of("bot-user"),
                &GitHostRepo::github("soaplabs", "repo"),
            )
            .await
            .err()
            .expect("the test registry has no App");
        assert!(
            error.to_string().contains("unknown GitHub App key"),
            "got {error}"
        );
    }
}
