//! GitHub identity selection and credential materialization by use case.

use std::sync::Arc;

use tracing::debug;

use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_loops::adapters::models::{BindingContext, SubscriptionMatch, TaskBinding};
use moatless_identity::auth::credentials::resolve_user_github_token;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_identity::auth::github::service::GitHubAppService;
use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
use moatless_identity::auth::provider_tokens::ProviderTokenAuthDao;
use moatless_identity::secret::service::SecretService;
use moatless_core::db::DbPool;

use super::api::GithubPrApi;

pub(super) struct GithubPrAuth {
    global_bot_token: Option<String>,
    provider_tokens: Arc<dyn ProviderTokenDao>,
    provider_token_auth: Option<Arc<dyn ProviderTokenAuthDao>>,
    github_app_service: Option<Arc<GitHubAppService>>,
    github_apps: Arc<GitHubAppRegistry>,
    secrets: Option<Arc<SecretService>>,
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
    pub(super) fn new(
        global_bot_token: Option<String>,
        provider_tokens: Arc<dyn ProviderTokenDao>,
        provider_token_auth: Option<Arc<dyn ProviderTokenAuthDao>>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Self {
        Self {
            global_bot_token,
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            secrets: None,
            #[cfg(test)]
            api_base: None,
        }
    }

    pub(super) fn set_secrets(&mut self, secrets: Arc<SecretService>) {
        self.secrets = Some(secrets);
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
        db: &DbPool,
        repository_owner: &str,
    ) -> Result<AuthenticatedGithub, AdapterError> {
        let user_id = resolve_sender_user(github_login, rule, db).await;
        let api = self.client_for_user(&user_id, repository_owner).await?;
        Ok(AuthenticatedGithub { user_id, api })
    }

    pub(super) async fn client_for_outbound_reply(
        &self,
        author_user_id: Option<&str>,
        binding: &TaskBinding,
        rule: Option<&SubscriptionMatch>,
        repository_owner: &str,
    ) -> Result<GithubPrApi, AdapterError> {
        let user_id = outbound_user_id(author_user_id, binding, rule).ok_or_else(|| {
            SharedAdapterError::AdapterApi(
                "Cannot resolve GitHub token: no task owner, binding filter_user_id, \
                 or task trigger attached"
                    .into(),
            )
        })?;
        self.client_for_user(&user_id, repository_owner).await
    }

    pub(super) async fn client_for_reaction(
        &self,
        binding: &BindingContext,
        repository_owner: &str,
    ) -> Result<GithubPrApi, AdapterError> {
        let token = self.reaction_token(binding, repository_owner).await?;
        Ok(self.client(&token))
    }

    async fn reaction_token(
        &self,
        binding: &BindingContext,
        repository_owner: &str,
    ) -> Result<String, AdapterError> {
        let user_id = binding_filter_user_id(&binding.destination_config, &binding.metadata);
        if let Some(user_id) = user_id {
            return self
                .resolve_token_for_user(user_id, Some(repository_owner))
                .await;
        }

        self.global_token().map(str::to_string).ok_or_else(|| {
            SharedAdapterError::AdapterApi(
                "No GitHub token available for reactions: binding has no filter_user_id and \
                 GITHUB_CHANNEL_TOKEN env is not set"
                    .into(),
            )
            .into()
        })
    }

    pub(super) async fn client_for_live_status(
        &self,
        acting_user: &str,
        repository_owner: &str,
    ) -> Result<GithubPrApi, AdapterError> {
        self.client_for_user(acting_user, repository_owner).await
    }

    async fn client_for_user(
        &self,
        user_id: &str,
        repository_owner: &str,
    ) -> Result<GithubPrApi, AdapterError> {
        let token = self
            .resolve_token_for_user(user_id, Some(repository_owner))
            .await?;
        Ok(self.client(&token))
    }

    /// Mint a GitHub client from `user_id`'s OWN GitHub row, scoped to
    /// `owner`/`repo` — the `moat gh` typed-API path (SP-2457), where the
    /// identity that pushed the branch is the identity that opens the PR or
    /// posts the comment. Wraps
    /// [`moatless_identity::auth::credentials::resolve_run_as_user_github_token`], which
    /// reads only the user's own row keyed on `owner`+`repo`.
    ///
    /// What it deliberately does NOT do, unlike [`Self::client_for_user`]:
    /// - no scope walk — the repository's persisted scope and the global row are
    ///   never consulted;
    /// - no `GITHUB_CHANNEL_TOKEN` bot-token fallback;
    /// - it fails closed — a user with no matching row cannot reach GitHub here
    ///   rather than silently acting as the bot.
    ///
    /// Which repository is named therefore only scopes the token, never whether
    /// one is found: a PR against a repo the task isn't bound to resolves exactly
    /// like one against its own.
    ///
    /// A third GitHub credential policy exists and deliberately does NOT live on
    /// this struct: the repository-scoped one, keyed on the task's repository row
    /// rather than on any user and deferring to `RepositoryTokenResolver` — see
    /// `shell_reconcile::resolve_task_repository_github_token`. It stays with the
    /// repository ports because it selects no identity; this method and
    /// `client_for_user` are the two that answer "which identity acts", so a
    /// reader meeting both here should know the identity-independent one is over
    /// there.
    pub(super) async fn client_for_run_as_user(
        &self,
        user_id: &str,
        owner: &str,
        repo: &str,
    ) -> Result<GithubPrApi, AdapterError> {
        let token = moatless_identity::auth::credentials::resolve_run_as_user_github_token(
            self.provider_tokens.as_ref(),
            self.provider_token_auth.as_deref(),
            self.github_app_service.as_deref(),
            self.github_apps.as_ref(),
            self.secrets.as_deref(),
            user_id,
            owner,
            repo,
        )
        .await
        .map_err(|source| AdapterError::UserCredential {
            user_id: user_id.to_string(),
            source,
        })?
        .ok_or_else(|| {
            SharedAdapterError::AdapterApi(format!(
                "User {user_id} has no GitHub credential — connect GitHub to act on {owner}/{repo}"
            ))
        })?;
        Ok(self.client(&token))
    }

    async fn resolve_token_for_user(
        &self,
        user_id: &str,
        repository_owner: Option<&str>,
    ) -> Result<String, AdapterError> {
        match resolve_user_github_token(
            self.provider_tokens.as_ref(),
            self.provider_token_auth.as_deref(),
            self.github_app_service.as_deref(),
            self.github_apps.as_ref(),
            self.secrets.as_deref(),
            user_id,
            repository_owner,
        )
        .await
        {
            Ok(Some(token)) if !token.is_empty() => return Ok(token),
            Ok(_) => {}
            Err(source) => {
                return Err(AdapterError::UserCredential {
                    user_id: user_id.to_string(),
                    source,
                });
            }
        }

        if let Some(token) = self.global_token() {
            return Ok(token.to_string());
        }

        Err(SharedAdapterError::AdapterApi(format!(
            "No GitHub token available: user {user_id} has no GitHub provider token and GITHUB_CHANNEL_TOKEN env is not set"
        )).into())
    }

    fn global_token(&self) -> Option<&str> {
        self.global_bot_token
            .as_deref()
            .filter(|token| !token.is_empty())
    }
}

pub(super) async fn resolve_sender_user(
    github_login: &str,
    rule: &SubscriptionMatch,
    db: &DbPool,
) -> String {
    if let Ok(Some(user_id)) = mapped_user(github_login, db).await {
        return user_id;
    }

    debug!(
        github_login,
        fallback = %rule.effective_user_id(),
        "GitHub user not mapped, using rule owner"
    );
    rule.effective_user_id().to_string()
}

async fn mapped_user(identity: &str, db: &DbPool) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM provider_tokens WHERE provider = 'github' AND provider_login = $1 LIMIT 1",
    )
    .bind(identity)
    .fetch_optional(db)
    .await
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
    use super::*;
    use moatless_identity::auth::provider_token_dao::ProviderTokenDao;
    use moatless_identity::auth::test_support::{StubProviderTokens, github_row};

    fn fake_github_apps() -> Arc<GitHubAppRegistry> {
        Arc::new(GitHubAppRegistry::default())
    }

    fn auth_with_tokens(
        global_token: Option<&str>,
        provider_tokens: Arc<dyn ProviderTokenDao>,
    ) -> GithubPrAuth {
        GithubPrAuth::new(
            global_token.map(str::to_string),
            provider_tokens,
            None,
            None,
            fake_github_apps(),
        )
    }

    fn binding_with_filter(filter_user_id: Option<&str>) -> TaskBinding {
        let now = chrono::Utc::now();
        let filter = serde_json::json!({"filter_user_id": filter_user_id});
        TaskBinding {
            id: "binding".into(),
            task_id: "task".into(),
            adapter_kind: "github_pr".into(),
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
        use moatless_loops::adapters::models::RoutingMode;
        use chrono::Utc;
        use moatless_backend_core::scope::Scope;

        SubscriptionMatch {
            id: "rule".into(),
            adapter_kind: "github".into(),
            connection_id: "connection".into(),
            source_matcher: "soaplabs/repo".into(),
            source_name: None,
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repository".into(),
            prompt_template: None,
            task_name: Some("Review".into()),
            tag_ids: vec![],
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

    #[tokio::test]
    async fn reaction_identity_prefers_binding_filter_then_non_empty_global_token() {
        let auth = auth_with_tokens(
            Some("global-token"),
            Arc::new(StubProviderTokens::with_user_row_for(
                "filter-user",
                github_row("manual", "filter-token"),
            )),
        );
        let binding = BindingContext {
            binding_id: "binding".into(),
            loop_id: None,
            task_id: "task".into(),
            subject_kind: "github_pr".into(),
            subject_key: "soaplabs/repo#42".into(),
            destination_config: serde_json::json!({"filter_user_id": "filter-user"}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            last_message_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
        };

        assert_eq!(
            auth.reaction_token(&binding, "soaplabs").await.unwrap(),
            "filter-token"
        );

        let mut no_filter = binding.clone();
        no_filter.destination_config = serde_json::json!({});
        assert_eq!(
            auth.reaction_token(&no_filter, "soaplabs").await.unwrap(),
            "global-token"
        );

        let empty_global = auth_with_tokens(Some(""), Arc::new(StubProviderTokens::default()));
        assert!(
            empty_global
                .reaction_token(&no_filter, "soaplabs")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn user_credentials_keep_non_empty_global_fallback() {
        let auth = auth_with_tokens(
            Some("global-token"),
            Arc::new(StubProviderTokens::default()),
        );
        assert_eq!(
            auth.resolve_token_for_user("unmapped-user", Some("soaplabs"))
                .await
                .unwrap(),
            "global-token"
        );

        let empty_global = auth_with_tokens(Some(""), Arc::new(StubProviderTokens::default()));
        assert!(
            empty_global
                .resolve_token_for_user("unmapped-user", Some("soaplabs"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn github_app_installation_resolution_keeps_repository_owner_context() {
        let mut row = github_row(
            moatless_identity::auth::provider_tokens::AUTH_METHOD_APP_INSTALLATION,
            moatless_identity::auth::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER,
        );
        row.provider_user_id = Some("missing-test-app".to_string());
        let auth = auth_with_tokens(
            None,
            Arc::new(StubProviderTokens::with_user_row_for("bot-user", row)),
        );

        let with_owner = auth
            .resolve_token_for_user("bot-user", Some("soaplabs"))
            .await
            .expect_err("the test registry has no App");
        assert!(
            with_owner.to_string().contains("unknown GitHub App key"),
            "the resolver got past its mandatory repository-owner check: {with_owner}"
        );

        let without_owner = auth
            .resolve_token_for_user("bot-user", None)
            .await
            .expect_err("installation auth requires repository owner context");
        assert!(
            without_owner
                .to_string()
                .contains("no repo owner in context"),
            "{without_owner}"
        );
    }
}
