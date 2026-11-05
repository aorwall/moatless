//! GitHub identity selection and credential materialization by use case.

use std::sync::Arc;

use tracing::debug;

use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_ports::identity::ProviderIdentityLink;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_types::access_control::Actor;
use moatless_types::adapter::TaskBinding;
use moatless_types::http::HttpTimeouts;
use moatless_types::loops::SubscriptionMatch;
use moatless_workspace_types::repository::GitHostRepo;

use super::adapter::GithubPrAdapter;
use super::api::GithubPrApi;

pub(super) struct GithubPrAuth {
    /// The one way to a git-host token. This Adapter holds no credential of its
    /// own: every use case below names an identity and a repository.
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// REST root every client below is built against. A token is selected per
    /// call, so the root is held rather than a shared client.
    api_base: String,
    http: reqwest::Client,
}

/// A client that writes to one repository, and the driver to credit when the
/// App it authenticates as, rather than the driver, authors what it posts.
pub(super) struct RepoWriter {
    pub(super) api: GithubPrApi,
    /// GitHub login of the driver the App writes for; `None` when the client
    /// writes as the identity itself, which GitHub already shows.
    pub(super) driver_login: Option<String>,
}

pub(super) struct AuthenticatedGithub {
    pub(super) user_id: String,
    pub(super) api: GithubPrApi,
}

impl GithubPrAuth {
    pub(super) fn new(
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        api_base: String,
        github: HttpTimeouts,
    ) -> Self {
        Self {
            git_host_tokens,
            api_base,
            http: super::api::http_client(github),
        }
    }

    #[cfg(test)]
    pub(super) fn set_api_base(&mut self, base_url: impl Into<String>) {
        self.api_base = base_url.into();
    }

    /// The one place a REST client is built, so the root reaches every use case.
    fn client(&self, token: &str) -> GithubPrApi {
        GithubPrApi::new(self.http.clone(), token, &self.api_base)
    }

    /// Mint a client for durable work `github_login` triggered.
    ///
    /// No pull request author to stand in: a merge-conflict check names a PR
    /// whose author it has not read, and a post-merge workflow run happens on
    /// the default branch, where there is no pull request at all.
    pub(super) async fn client_for_durable_work(
        &self,
        github_login: &str,
        rule: &SubscriptionMatch,
        users: &dyn ProviderIdentityLink,
        repo: &GitHostRepo,
    ) -> Result<AuthenticatedGithub, AdapterError> {
        let user_id = resolve_sender_user(github_login, None, rule, users).await;
        let api = self.client_for(&Actor::of(&user_id), repo).await?;
        Ok(AuthenticatedGithub { user_id, api })
    }

    pub(super) async fn client_for_outbound_reply(
        &self,
        author_user_id: Option<&str>,
        binding: &TaskBinding,
        rule: Option<&SubscriptionMatch>,
        repo: &GitHostRepo,
    ) -> Result<RepoWriter, AdapterError> {
        let actor = outbound_actor(author_user_id, binding, rule)?;
        self.writer_for(&actor, repo).await
    }

    pub(super) async fn client_for_reaction(
        &self,
        binding: &BindingContext,
        acting_user_id: &str,
        repo: &GitHostRepo,
    ) -> Result<GithubPrApi, AdapterError> {
        self.client_for(&reaction_actor(binding, acting_user_id), repo)
            .await
    }

    pub(super) async fn client_for_live_status(
        &self,
        acting_user: &str,
        repo: &GitHostRepo,
    ) -> Result<GithubPrApi, AdapterError> {
        self.client_for(&Actor::of(acting_user), repo).await
    }

    /// A client that writes to `repo` as the App `actor` signed in through,
    /// else as `actor`, per [`GitHostTokenIssuer::github_app_authored`].
    pub(super) async fn writer_for(
        &self,
        actor: &Actor,
        repo: &GitHostRepo,
    ) -> Result<RepoWriter, AdapterError> {
        let authored = self
            .git_host_tokens
            .github_app_authored(actor, repo.owner(), repo.name())
            .await
            .map_err(|source| adapter_error(actor, source))?;
        match authored {
            Some(authored) => Ok(RepoWriter {
                api: self.client(&authored.token),
                driver_login: Some(authored.driver_login),
            }),
            None => Ok(RepoWriter {
                api: self.client_for(actor, repo).await?,
                driver_login: None,
            }),
        }
    }

    /// Mint a GitHub client from `actor`'s OWN row, scoped to `repo`.
    ///
    /// No scope walk: an identity with no row fails rather than silently acting
    /// as somebody else, and `repo` only scopes the token, never whether one is
    /// found.
    pub(super) async fn client_for(
        &self,
        actor: &Actor,
        repo: &GitHostRepo,
    ) -> Result<GithubPrApi, AdapterError> {
        let token = self
            .git_host_tokens
            .token(actor, &repo.credential_account())
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
        GitHostTokenError::Store(e) => AdapterError::from(SharedAdapterError::AdapterApi(format!(
            "GitHub token lookup failed for {actor}: {e}"
        ))),
        // `Forbidden` displays without its source, which carries what to do.
        forbidden @ GitHostTokenError::Forbidden { .. } => {
            let message = match std::error::Error::source(&forbidden) {
                Some(reason) => format!("{forbidden}: {reason}"),
                None => forbidden.to_string(),
            };
            SharedAdapterError::AdapterApi(message).into()
        }
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

/// The identity a reaction acts as: the binding's filter user, else the
/// already-authorized caller.
///
/// `BindingContext` carries no author or subscription, so the caller stands
/// in rather than failing: authorization to react already happened.
fn reaction_actor(binding: &BindingContext, acting_user_id: &str) -> Actor {
    binding_filter_user_id(&binding.destination_config, &binding.metadata)
        .map_or_else(|| Actor::of(acting_user_id), Actor::of)
}

/// The moatless user an inbound event is attributed to, from the first login
/// that has connected GitHub.
///
/// The pull request's author stands in for a sender that has not: nobody can
/// link `github-actions[bot]`, and `sent_by` would otherwise credit the rule's
/// run-as user, who had nothing to do with the event.
pub(super) async fn resolve_sender_user(
    github_login: &str,
    pr_author_login: Option<&str>,
    rule: &SubscriptionMatch,
    users: &dyn ProviderIdentityLink,
) -> String {
    for login in [Some(github_login), pr_author_login]
        .into_iter()
        .flatten()
        .filter(|login| !login.is_empty())
    {
        if let Ok(Some(user_id)) = users.user_with_login(super::delivery::ADAPTER, login).await {
            return user_id;
        }
    }

    debug!(
        github_login,
        pr_author_login,
        fallback = %rule.effective_user_id(),
        "Neither the sender nor the PR author is mapped, using rule owner"
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

/// The identity a task posts as on this binding. Nothing filters on it despite
/// the key's name: routing asks provenance, never a sender identity.
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

/// Resolve a GitHub API client for `user_id` acting on `workspace_id`, minted
/// against `owner/repo_name`.
///
/// `user_id` is the caller, the sandbox's driver for an `X-User-Token`; a write
/// a Task makes goes through [`resolve_task_github_api`] instead. The
/// credential rule itself lives on `GithubPrAuth::client_for`.
pub(super) async fn resolve_github_api(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo_name: &str,
) -> Result<GithubPrApi, AdapterError> {
    resolve_task_github_api(
        adapter,
        &Actor::of(user_id).on_workspace(workspace_id),
        owner,
        repo_name,
    )
    .await
}

/// Resolve a GitHub API client for `actor`, minted against `owner/repo_name`.
pub(super) async fn resolve_task_github_api(
    adapter: &GithubPrAdapter,
    actor: &Actor,
    owner: &str,
    repo_name: &str,
) -> Result<GithubPrApi, AdapterError> {
    adapter
        .auth()
        .client_for(actor, &GitHostRepo::github(owner, repo_name))
        .await
}

/// Resolve the writer for `actor` posting to `owner/repo_name`, per
/// [`GithubPrAuth::writer_for`].
pub(super) async fn resolve_task_github_writer(
    adapter: &GithubPrAdapter,
    actor: &Actor,
    owner: &str,
    repo_name: &str,
) -> Result<RepoWriter, AdapterError> {
    adapter
        .auth()
        .writer_for(actor, &GitHostRepo::github(owner, repo_name))
        .await
}

/// The identity a Task's GitHub write acts as: the Task's own run-as on behalf
/// of `caller`, else `caller`, matching the sandbox's `GITHUB_TOKEN`.
///
/// The pod authenticates as its driver, so `caller` alone would open a bot
/// run-as Task's pull request under the driver's own account.
pub(super) fn task_write_actor(
    caller: &str,
    task_run_as: Option<&str>,
    workspace_id: Option<&str>,
) -> Actor {
    match task_run_as.filter(|id| !id.is_empty()) {
        Some(run_as) => Actor::of(run_as)
            .on_workspace(workspace_id)
            .on_behalf_of(Some(caller)),
        None => Actor::of(caller).on_workspace(workspace_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_identity::auth::test_support::{StubProviderTokens, github_row};
    use moatless_types::adapter::AdapterKind;

    #[test]
    fn a_refusal_keeps_what_to_do_about_it() {
        let error = adapter_error(
            &Actor::of("ada"),
            GitHostTokenError::Forbidden {
                actor: Actor::of("ada"),
                account: Box::new(GitHostRepo::github("acme", "repo").credential_account()),
                source: "ada needs write access to acme/repo".into(),
            },
        );
        assert!(
            error
                .to_string()
                .ends_with(": ada needs write access to acme/repo"),
            "{error}"
        );
    }

    #[test]
    fn a_task_run_as_writes_on_behalf_of_the_caller() {
        let actor = task_write_actor("ada", Some("bot"), Some("ws-1"));
        assert_eq!(actor.user_id(), "bot");
        assert_eq!(actor.person(), Some("ada"));
        assert_eq!(actor.workspace_id(), Some("ws-1"));
    }

    #[test]
    fn without_a_task_run_as_the_caller_writes_as_themselves() {
        for run_as in [None, Some(""), Some("ada")] {
            let actor = task_write_actor("ada", run_as, None);
            assert_eq!(actor.user_id(), "ada", "run_as {run_as:?}");
            assert_eq!(actor.person(), None, "run_as {run_as:?}");
        }
    }

    fn auth_with_tokens(provider_tokens: Arc<StubProviderTokens>) -> GithubPrAuth {
        GithubPrAuth::new(
            moatless_identity::git_host_tokens::seam_over(provider_tokens),
            moatless_core::config::DEFAULT_GITHUB_API_BASE.to_string(),
            moatless_core::config::HttpConfig::default().github,
        )
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
            watching: true,
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
            workspace_id: "ws-1".into(),
            prompt_template: None,
            task_name: Some("Review".into()),
            tag_ids: vec![],
            memory_preload: Vec::new(),
            agent_properties: Default::default(),
            skills: Vec::new(),
            run_as_user_id: Some(user_id.into()),
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            model: None,
            close_after_settled_days: None,
            settle_after_hours: None,
            node_selector: None,
            stop_when_done: None,
            room: None,
            message_buffer_seconds: None,
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

    /// The GitHub logins that have connected, keyed to a user id by prefix.
    /// Every other port method is scenery for the cases below.
    struct LinkedLogins(&'static [&'static str]);

    #[async_trait::async_trait]
    impl moatless_ports::identity::ProviderIdentityLink for LinkedLogins {
        async fn user_with_login(
            &self,
            _provider: &str,
            login: &str,
        ) -> Result<Option<String>, moatless_types::store::StoreError> {
            Ok(self.0.contains(&login).then(|| format!("user:{login}")))
        }
    }

    #[tokio::test]
    async fn the_pull_request_author_stands_in_for_an_unmapped_sender() {
        let rule = rule_with_run_as("run-as-user");
        let users = LinkedLogins(&["aorwall", "jacobrask"]);
        let bot = "github-actions[bot]";

        assert_eq!(
            resolve_sender_user(bot, Some("aorwall"), &rule, &users).await,
            "user:aorwall"
        );
        assert_eq!(
            resolve_sender_user("jacobrask", Some("aorwall"), &rule, &users).await,
            "user:jacobrask",
            "a sender that has connected outranks the author"
        );
        assert_eq!(
            resolve_sender_user(bot, None, &rule, &users).await,
            "run-as-user",
            "a check run carries no pull request author"
        );
        assert_eq!(
            resolve_sender_user(bot, Some("dependabot[bot]"), &rule, &users).await,
            "run-as-user",
            "an author that has not connected is no better than the sender"
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
    fn reaction_identity_prefers_the_binding_filter_user_over_the_caller() {
        assert_eq!(
            reaction_actor(&binding_context(Some("filter-user")), "caller").user_id(),
            "filter-user"
        );
        assert_eq!(
            reaction_actor(&binding_context(None), "caller").user_id(),
            "caller",
            "a binding naming nobody reacts as whoever asked for the reaction"
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
                "caller",
                &GitHostRepo::github("soaplabs", "repo"),
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn a_binding_without_a_filter_user_reacts_as_the_caller() {
        let auth = auth_with_tokens(Arc::new(StubProviderTokens::with_user_row_for(
            "caller",
            github_row("manual", "caller-token"),
        )));
        assert!(
            auth.client_for_reaction(
                &binding_context(None),
                "caller",
                &GitHostRepo::github("soaplabs", "repo"),
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn an_identity_with_no_row_cannot_reach_github() {
        // No fallback token path: no env token, no org row, no other identity.
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

        // The repository is mandatory in the signature, so the only way to
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
