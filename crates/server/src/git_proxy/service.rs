//! Deciding which upstream a proxied git request reaches, and with whose
//! credential.

use std::sync::Arc;

use moatless_adapter_github_pr::own_event::{self, Made};
use moatless_loops::bindings::own_events::OwnEvents;
use moatless_ports::identity::{CommitIdentity, GitHostTokenError, GitHostTokenIssuer};
use moatless_sandbox_control::secrets::{resolve_task_actor, resolve_task_git_token};
use moatless_types::access_control::{Actor, ScopeContext, ScopedResource};
use moatless_types::git_host::GitHostAccount;
use moatless_types::http::HttpStreamTimeouts;
use moatless_workspace_ports::{RepositoryReader, WorkspaceResolver};
use moatless_workspace_types::repository::{
    GITLAB_HOST, GitHostRepo, Repository, RepositoryKind, RepositoryProvider,
};
use tracing::warn;

use crate::git_proxy::push_watch::{PushWatch, PushWatchConfig};
use crate::github_policy::GitHubPolicy;
use crate::github_policy::push::BranchMove;
use crate::proxy::caller::{CallerError, SandboxCallers, SandboxScope};

/// Where one proxied request goes, and what it presents on arrival.
pub struct Upstream {
    /// `https://{host}/{owner}/{name}.git`, rebuilt from the parsed remote.
    ///
    /// Honouring the stored `git_url`'s scheme would let one downgrade the
    /// connection its own credential travels over.
    pub base_url: String,
    /// What the proxy presents to `host`; see [`host_credential`].
    pub credential: Credential,
    /// The HTTP Basic username sent with the token. GitLab refuses an OAuth
    /// token under any username but `oauth2`; GitHub and Forgejo ignore it.
    pub username: &'static str,
    /// For the request log. The token is never logged.
    pub host: String,
    /// `{owner}/{name}` on `host`.
    pub path: String,
    /// The Moatless user whose `host` credential `token` is, unless the
    /// Workspace pins one.
    pub run_as_user_id: String,
    /// The Workspace of the Sandbox the task token names.
    pub workspace_id: String,
    /// The Sandbox the task token names, for the policy's log line.
    pub sandbox_id: String,
    /// The Task a push is claimed for; see [`SandboxScope::task`].
    pub task_id: Option<String>,
}

/// What a proxied request presents to its host.
#[derive(Debug, PartialEq, Eq)]
pub enum Credential {
    /// Sent as the HTTP Basic password, which is how git hosts take a token
    /// over smart-HTTP.
    Token(String),
    /// Nothing is sent. `because` is why the run-as identity has no token for
    /// the repository, quoted back to git when the host then answers 404.
    Anonymous { because: String },
}

impl Credential {
    /// The token to send, or `None` to fetch anonymously.
    pub fn token(&self) -> Option<&str> {
        match self {
            Self::Token(token) => Some(token),
            Self::Anonymous { .. } => None,
        }
    }
}

impl Upstream {
    fn new(
        repo: &GitHostRepo,
        credential: Credential,
        actor: &Actor,
        caller: SandboxScope,
    ) -> Self {
        let host = repo.host().to_string();
        Self {
            base_url: format!("https://{host}/{}/{}.git", repo.owner(), repo.name()),
            host,
            credential,
            username: match repo.provider() {
                RepositoryProvider::Gitlab => "oauth2",
                RepositoryProvider::Github | RepositoryProvider::Forgejo => "x-access-token",
            },
            path: format!("{}/{}", repo.owner(), repo.name()),
            run_as_user_id: actor.user_id().to_string(),
            workspace_id: caller.workspace,
            sandbox_id: caller.sandbox,
            task_id: caller.task,
        }
    }
}

/// What git shows, as `remote:` lines, when `upstream`'s host answers 404.
///
/// Safe to name the repository: the proxy only reaches the host for one the
/// caller may already use here, so this confirms no guessed id. `login` is the
/// Moatless login of [`Upstream::run_as_user_id`], not the host account.
fn refusal_message(upstream: &Upstream, login: Option<&str>) -> String {
    let user = match login {
        Some(login) => format!("{login} ({})", upstream.run_as_user_id),
        None => upstream.run_as_user_id.clone(),
    };
    if let Credential::Anonymous { because } = &upstream.credential {
        return format!(
            "{host} answered not found for {path}.\n\
             Moatless user {user}, this task's run-as identity, has no {host} token for it \
             ({because}), so the proxy fetched anonymously.\n\
             Either {path} does not exist on {host}, or it is private: give that user a \
             {host} credential that reaches it.\n",
            host = upstream.host,
            path = upstream.path,
        );
    }
    format!(
        "{host} answered not found for {path}.\n\
         The proxy presented the {host} credential of Moatless user {user}, this task's \
         run-as identity, or the one workspace {workspace} pins for {host}, which outranks it.\n\
         Either {path} does not exist on {host}, or the account behind that credential \
         cannot read it: grant that account read access on {host}.\n",
        host = upstream.host,
        path = upstream.path,
        workspace = upstream.workspace_id,
    )
}

#[derive(Debug, thiserror::Error)]
pub enum GitProxyError {
    /// Every rejection of the caller's own credential. The message is returned
    /// to git, which shows it to the person watching the clone.
    #[error("{0}")]
    Unauthorized(String),
    /// No repository this token may reach answers to that id. Deliberately not
    /// distinguished from "no such repository": the id is guessable, and a
    /// distinct 403 would confirm one that exists.
    #[error("Repository not found")]
    NotFound,
    /// The repository resolved but its credential did not.
    #[error("{0}")]
    Upstream(String),
}

/// Resolves a proxied git request to an upstream and a credential.
pub struct GitProxy {
    callers: Arc<SandboxCallers>,
    workspaces: Arc<dyn WorkspaceResolver>,
    repositories: Arc<dyn RepositoryReader>,
    users: Arc<dyn CommitIdentity>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// Decides which refs a push may update.
    pub(crate) policy: Arc<GitHubPolicy>,
    /// Where a pushed commit is claimed for its Task; `None` claims nothing.
    own_events: Option<Arc<OwnEvents>>,
    /// Where a GitHub push is checked after it lands; `None` checks nothing.
    push_watch: Option<PushWatchConfig>,
    pub(crate) http: reqwest::Client,
}

/// Idle, not total: a pack streams for as long as the repository is big.
///
/// Panics when the TLS backend cannot be loaded, as `reqwest::Client::new`
/// does.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
fn http_client(timeouts: HttpStreamTimeouts) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .read_timeout(timeouts.idle)
        .user_agent(moatless_core::USER_AGENT)
        .build()
        .expect("the TLS backend initialises")
}

impl GitProxy {
    pub(crate) fn new(
        callers: Arc<SandboxCallers>,
        workspaces: Arc<dyn WorkspaceResolver>,
        repositories: Arc<dyn RepositoryReader>,
        users: Arc<dyn CommitIdentity>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        policy: Arc<GitHubPolicy>,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        Self {
            callers,
            workspaces,
            repositories,
            users,
            git_host_tokens,
            policy,
            own_events: None,
            push_watch: None,
            // `FORWARDED_REQUEST_HEADERS` copies git's own `User-Agent` over
            // this default, so it covers only a client that sends none. See
            // `moatless_core::USER_AGENT`.
            http: http_client(timeouts),
        }
    }

    /// Record pushes through `own_events`; see [`Self::claim_pushed_commits`].
    pub(crate) fn with_own_events(mut self, own_events: Arc<OwnEvents>) -> Self {
        self.own_events = Some(own_events);
        self
    }

    /// Check each GitHub push through `api_base` once it lands; see
    /// [`crate::git_proxy::push_watch`].
    pub(crate) fn with_push_watch(mut self, api_base: &str) -> Self {
        self.push_watch = Some(PushWatchConfig {
            api_base: api_base.to_string(),
            timing: crate::git_proxy::push_watch::Timing::default(),
        });
        self
    }

    /// The watch for a push to `upstream` moving `moves`, or `None` when the
    /// push is not to github.com, moves no existing branch, or presents no
    /// token to check it with.
    pub(super) fn push_watch(
        &self,
        upstream: &Upstream,
        moves: Vec<BranchMove>,
    ) -> Option<PushWatch> {
        let config = self.push_watch.clone()?;
        let (owner, repo) = upstream.path.split_once('/')?;
        let token = upstream.credential.token()?;
        if upstream.host != "github.com" || moves.is_empty() {
            return None;
        }
        Some(PushWatch {
            http: self.http.clone(),
            config,
            git_url: upstream.base_url.clone(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            username: upstream.username,
            token: token.to_string(),
            moves,
            claim: self.own_events.clone().zip(upstream.task_id.clone()),
        })
    }

    /// Record each commit a push moves a GitHub branch to as the Task's own,
    /// so its `synchronize` webhook is not delivered back to the Task.
    ///
    /// Called before the push is forwarded: the webhook can beat the response.
    pub(crate) async fn claim_pushed_commits(&self, upstream: &Upstream, shas: &[String]) {
        let (Some(own_events), Some(task_id), Some((owner, repo))) = (
            self.own_events.as_ref(),
            upstream.task_id.as_deref(),
            upstream.path.split_once('/'),
        ) else {
            return;
        };
        if upstream.host != "github.com" {
            return;
        }
        for sha in shas {
            own_events
                .record_best_effort(
                    task_id,
                    own_event::event(owner, repo, &Made::Push(sha.clone())),
                )
                .await;
        }
    }

    /// Resolve `repository_id` for the sandbox `task_token` was minted for;
    /// `push` is a `receive-pack`, sent per [`app_push_token`].
    ///
    /// The credential is the *run-as* identity's, not the token's `userId`.
    /// Authorization checks the Workspace's placements first, then every
    /// repository the token's own identity may use.
    pub async fn resolve(
        &self,
        repository_id: &str,
        task_token: &str,
        push: bool,
    ) -> Result<Upstream, GitProxyError> {
        let caller = self.authenticate(task_token).await?;
        let placed = self
            .workspaces
            .placements(&caller.workspace)
            .await
            .map_err(|e| {
                warn!(error = %e, workspace_id = %caller.workspace, "Failed to read the workspace placements");
                GitProxyError::Upstream("Failed to read the workspace".to_string())
            })?
            .into_iter()
            .find(|placement| placement.repository_id == repository_id)
            .and_then(|placement| placement.repository);

        let repository = match placed {
            Some(repository) => repository,
            None => usable_repository(
                &self.repositories,
                &self.users,
                &caller.token_user,
                repository_id,
            )
            .await
            .ok_or(GitProxyError::NotFound)?,
        };

        let repo = git_host_repo(&repository)
            .ok_or(GitProxyError::NotFound)?
            .clone();
        let actor = self.actor(&caller).await;

        // Through the resolver the pre-provision preflight uses, so a clone this
        // proxy refuses is one `preflight_validate_git_credential` would have
        // refused before any pod existed.
        if let Some(token) = app_push_token(&self.git_host_tokens, &actor, &repo, push).await? {
            return Ok(Upstream::new(
                &repo,
                Credential::Token(token),
                &actor,
                caller,
            ));
        }
        let resolved =
            resolve_task_git_token(Some(&self.git_host_tokens), &actor, Some(&repository)).await;
        let credential = registered_credential(resolved, repository.kind, push, repo.host())?;
        Ok(Upstream::new(&repo, credential, &actor, caller))
    }

    /// Resolve `{owner}/{name}` on github.com, registered or not, for the
    /// sandbox `task_token` was minted for; `push` is a `receive-pack`, sent
    /// per [`app_push_token`].
    ///
    /// No placement or repository row is consulted: the run-as identity's own
    /// GitHub credential is the whole authorization ([`host_credential`]).
    pub async fn resolve_github(
        &self,
        owner: &str,
        name: &str,
        task_token: &str,
        push: bool,
    ) -> Result<Upstream, GitProxyError> {
        let repo = github_repo(owner, name).ok_or(GitProxyError::NotFound)?;
        let caller = self.authenticate(task_token).await?;
        let actor = self.actor(&caller).await;
        if let Some(token) = app_push_token(&self.git_host_tokens, &actor, &repo, push).await? {
            return Ok(Upstream::new(
                &repo,
                Credential::Token(token),
                &actor,
                caller,
            ));
        }
        let resolved = self
            .git_host_tokens
            .token(&actor, &repo.credential_account())
            .await;
        let credential = host_credential(resolved, push, repo.host())?;
        Ok(Upstream::new(&repo, credential, &actor, caller))
    }

    /// Resolve `path` (`{group…}/{name}`) on the GitLab instance `host`,
    /// registered or not; `push` is a `receive-pack`.
    ///
    /// `host` must be `gitlab.com` or the host of a GitLab repository the
    /// Workspace places: the run-as identity's GitLab token goes to it, and a
    /// pod-chosen host would receive that token.
    pub async fn resolve_gitlab(
        &self,
        host: &str,
        path: &str,
        task_token: &str,
        push: bool,
    ) -> Result<Upstream, GitProxyError> {
        let repo = gitlab_repo(host, path).ok_or(GitProxyError::NotFound)?;
        let caller = self.authenticate(task_token).await?;
        if !gitlab_host_allowed(&self.workspaces, &caller.workspace, repo.host()).await? {
            warn!(host, workspace_id = %caller.workspace, "Refused a git request to a GitLab host the workspace does not place");
            return Err(GitProxyError::NotFound);
        }
        let actor = self.actor(&caller).await;
        let resolved = self
            .git_host_tokens
            .token(&actor, &GitHostAccount::Gitlab)
            .await;
        let credential = host_credential(resolved, push, repo.host())?;
        Ok(Upstream::new(&repo, credential, &actor, caller))
    }

    /// The Sandbox and Workspace a task token names.
    async fn authenticate(&self, task_token: &str) -> Result<SandboxScope, GitProxyError> {
        self.callers
            .authenticate(Some(task_token))
            .await
            .map_err(|e| match e {
                CallerError::Unauthorized(message) => GitProxyError::Unauthorized(message),
                CallerError::SandboxGone => GitProxyError::NotFound,
                CallerError::Upstream(message) => GitProxyError::Upstream(message),
            })
    }

    /// The identity whose credential a request from `caller` presents, on
    /// behalf of the Sandbox's driver.
    async fn actor(&self, caller: &SandboxScope) -> Actor {
        resolve_task_actor(
            Some(&self.workspaces),
            Some(&caller.workspace),
            caller.run_as.as_deref(),
            &caller.user,
        )
        .await
    }

    /// The body answering a 404 from `upstream`'s host; see [`refusal_message`].
    ///
    /// Reads the login here rather than in [`Self::resolve`], which every
    /// proxied request pays for.
    pub async fn refusal(&self, upstream: &Upstream) -> String {
        let login = match self.users.git_identity(&upstream.run_as_user_id).await {
            Ok(identity) => identity.map(|identity| identity.login),
            Err(e) => {
                warn!(error = %e, user_id = %upstream.run_as_user_id, "Failed to read the run-as login for a git refusal");
                None
            }
        };
        refusal_message(upstream, login.as_deref())
    }
}

/// The repository `user_id` may fetch and push by id, or `None` if not.
///
/// `user_id` is the pod's own identity (`created_by`), not the run-as actor,
/// checked via `ScopeContext::can_use_repository` against the row's owner.
/// Returns `None` for a hidden, unauthorized, or unreadable repository.
async fn usable_repository(
    repositories: &Arc<dyn RepositoryReader>,
    users: &Arc<dyn CommitIdentity>,
    user_id: &str,
    repository_id: &str,
) -> Option<Repository> {
    let ctx = ScopeContext::new(
        user_id.to_string(),
        matches!(users.is_platform_admin(user_id).await, Ok(true)),
    );
    let repository = repositories
        .describe(&ctx, repository_id)
        .await
        .map_err(|e| {
            warn!(error = %e, repository_id, "Failed to read a repository for a proxied git request");
        })
        .ok()??;
    let owner = ScopedResource::borrowed(
        repository.scope,
        repository
            .owner_user_id
            .as_deref()
            .or(repository.created_by.as_deref()),
    );
    ctx.can_use_repository(&owner).then_some(repository)
}

/// The remote a proxied request forwards to, or `None` when the repository
/// names none this proxy can address.
///
/// A row naming no provider, or a remote URL its provider's parser rejects, has
/// no host to forward to.
fn git_host_repo(repository: &Repository) -> Option<&GitHostRepo> {
    repository.remote.as_ref()?.host_repo.as_ref()
}

/// `owner/name` on github.com as a git URL path names it, or `None` for one
/// GitHub could not hold.
///
/// Checked here rather than left to GitHub: both halves are interpolated into
/// the upstream URL, so a `..` or a `/` would address some other path on it.
fn github_repo(owner: &str, name: &str) -> Option<GitHostRepo> {
    let name = name.strip_suffix(".git").unwrap_or(name);
    let owner_ok = !owner.is_empty()
        && owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    let name_ok = !matches!(name, "" | "." | "..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (owner_ok && name_ok).then(|| GitHostRepo::github(owner, name))
}

/// `{group…}/{name}` on GitLab `host` as a git URL path names it, or `None`
/// for a host or path GitLab could not hold.
///
/// Checked as [`github_repo`] is: each piece is interpolated into the
/// upstream URL. A host carries no port or userinfo, so it names one HTTPS
/// origin.
fn gitlab_repo(host: &str, path: &str) -> Option<GitHostRepo> {
    let host = host.to_ascii_lowercase();
    let host_ok = host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.rsplit_once('/')?;
    let segment_ok = |segment: &str| {
        !matches!(segment, "" | "." | "..")
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (host_ok && segment_ok(name) && owner.split('/').all(segment_ok))
        .then(|| GitHostRepo::new(RepositoryProvider::Gitlab, host, owner, name))
}

/// Whether `host` is `gitlab.com` or a GitLab host `workspace_id` places.
async fn gitlab_host_allowed(
    workspaces: &Arc<dyn WorkspaceResolver>,
    workspace_id: &str,
    host: &str,
) -> Result<bool, GitProxyError> {
    if host == GITLAB_HOST {
        return Ok(true);
    }
    let placements = workspaces.placements(workspace_id).await.map_err(|e| {
        warn!(error = %e, workspace_id, "Failed to read the workspace placements");
        GitProxyError::Upstream("Failed to read the workspace".to_string())
    })?;
    Ok(placements
        .iter()
        .filter_map(|p| git_host_repo(p.repository.as_ref()?))
        .any(|repo| {
            repo.provider() == RepositoryProvider::Gitlab && repo.host().eq_ignore_ascii_case(host)
        }))
}

/// The App's installation token when `push` goes to GitHub `repo`, per
/// [`GitHostTokenIssuer::github_app_write`]; `None` sends the run-as token.
async fn app_push_token(
    issuer: &Arc<dyn GitHostTokenIssuer>,
    actor: &Actor,
    repo: &GitHostRepo,
    push: bool,
) -> Result<Option<String>, GitProxyError> {
    if !push || repo.provider() != RepositoryProvider::Github {
        return Ok(None);
    }
    let app = issuer
        .github_app_write(actor, repo.owner(), repo.name())
        .await
        .map_err(|error| credential_error(&error, repo.host()))?;
    Ok(app.map(|app| app.token))
}

/// The identity's token, or anonymous for a fetch it has no token to reach:
/// `NoCredential`, or `Forbidden` as for an App not installed on the owner.
///
/// Deliberately anonymous rather than refused: public installs and submodules
/// come here, and anonymous reaches only public repos. A push, or a `Broken`
/// credential, is refused rather than hidden.
fn host_credential(
    resolved: Result<String, GitHostTokenError>,
    push: bool,
    host: &str,
) -> Result<Credential, GitProxyError> {
    match resolved {
        Ok(token) => Ok(Credential::Token(token)),
        Err(e @ (GitHostTokenError::NoCredential { .. } | GitHostTokenError::Forbidden { .. }))
            if !push =>
        {
            Ok(Credential::Anonymous {
                because: e.to_string(),
            })
        }
        Err(e) => Err(credential_error(&e, host)),
    }
}

/// The credential for a registered repository: the identity's token, else
/// [`host_credential`]'s anonymous fetch for a `Template` only. Preflight starts
/// a `Template` with no credential (`requires_credential_preflight`), so
/// refusing its clone here fails a sandbox that already started.
fn registered_credential(
    resolved: Option<Result<String, GitHostTokenError>>,
    kind: RepositoryKind,
    push: bool,
    host: &str,
) -> Result<Credential, GitProxyError> {
    match resolved {
        Some(Ok(token)) => Ok(Credential::Token(token)),
        Some(Err(e)) if kind == RepositoryKind::Template => host_credential(Err(e), push, host),
        Some(Err(e)) => Err(credential_error(&e, host)),
        // Unreachable: the caller's `git_host_repo` answered for this same
        // repository, and this is the condition it maps to `NotFound`.
        None => Err(GitProxyError::NotFound),
    }
}

fn credential_error(error: &GitHostTokenError, host: &str) -> GitProxyError {
    warn!(%error, host, "No credential for a proxied git request");
    match error {
        GitHostTokenError::NoCredential { .. } | GitHostTokenError::Forbidden { .. } => {
            GitProxyError::Unauthorized(error.to_string())
        }
        GitHostTokenError::Broken { .. } | GitHostTokenError::Store(_) => {
            GitProxyError::Upstream(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_core::sandbox_jwt::{StaticTokenGenerations, generate_user_auth_token};
    use moatless_ports::identity::{AppWriteToken, MockCommitIdentity, MockGitHostTokenIssuer};
    use moatless_ports::task::MockTranscriptAppend;
    use moatless_types::access_control::Scope;
    use moatless_types::store::StoreError;
    use moatless_workspace_ports::{MockRepositoryReader, MockWorkspaceResolver};
    use moatless_workspace_types::repository::RepositoryProvider;

    const POD_USER: &str = "user-1";
    const SECRET: &str = "git-proxy-test-secret";

    fn repository(scope: Scope, owner_user_id: Option<&str>) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "config-default".to_string(),
            kind: RepositoryKind::Git,
            remote: Some(moatless_workspace_types::repository::RepositoryRemote {
                url: "https://git.example.com/owner/config-default.git".to_string(),
                host_repo: Some(moatless_workspace_types::repository::GitHostRepo::new(
                    RepositoryProvider::Forgejo,
                    "git.example.com",
                    "owner",
                    "config-default",
                )),
            }),
            default_branch: None,
            description: None,
            provider: Some(RepositoryProvider::Forgejo),
            scope,
            owner_user_id: owner_user_id.map(str::to_string),
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    /// A reader answering `shown` for every id, and an identity that is
    /// nobody's admin.
    fn ports(
        shown: Result<Option<Repository>, String>,
    ) -> (Arc<dyn RepositoryReader>, Arc<dyn CommitIdentity>) {
        let mut repositories = MockRepositoryReader::new();
        repositories
            .expect_describe()
            .returning(move |_, _| shown.clone().map_err(StoreError::new));
        let mut users = MockCommitIdentity::new();
        users.expect_is_platform_admin().returning(|_| Ok(false));
        (Arc::new(repositories), Arc::new(users))
    }

    #[tokio::test]
    async fn a_global_repository_outside_the_workspace_is_usable() {
        let (repositories, users) = ports(Ok(Some(repository(Scope::Global, None))));

        let found = usable_repository(&repositories, &users, POD_USER, "repo-1").await;

        assert_eq!(found.map(|repo| repo.id), Some("repo-1".to_string()));
    }

    #[tokio::test]
    async fn a_repository_the_reader_hides_is_not_usable() {
        let (repositories, users) = ports(Ok(None));

        assert!(
            usable_repository(&repositories, &users, POD_USER, "repo-1")
                .await
                .is_none()
        );
    }

    /// The scoped read already filters this one. The gate runs anyway, so a
    /// reader that widens later does not widen the proxy with it.
    #[tokio::test]
    async fn a_repository_owned_by_somebody_else_is_not_usable() {
        let (repositories, users) = ports(Ok(Some(repository(Scope::User, Some("user-2")))));

        assert!(
            usable_repository(&repositories, &users, POD_USER, "repo-1")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_failed_read_is_not_usable() {
        let (repositories, users) = ports(Err("database is down".to_string()));

        assert!(
            usable_repository(&repositories, &users, POD_USER, "repo-1")
                .await
                .is_none()
        );
    }

    fn upstream() -> Upstream {
        Upstream {
            base_url: "https://github.com/soaplabs/moatless.git".to_string(),
            credential: Credential::Token("secret-token".to_string()),
            username: "x-access-token",
            host: "github.com".to_string(),
            path: "soaplabs/moatless".to_string(),
            run_as_user_id: POD_USER.to_string(),
            workspace_id: "ws_1".to_string(),
            sandbox_id: "sandbox-1".to_string(),
            task_id: None,
        }
    }

    #[test]
    fn a_refusal_names_the_repository_the_host_and_the_moatless_run_as_user() {
        let refusal = refusal_message(&upstream(), Some("SoapBot"));

        assert!(refusal.starts_with("github.com answered not found for soaplabs/moatless.\n"));
        assert!(refusal.contains("Moatless user SoapBot (user-1)"));
        assert!(refusal.contains("workspace ws_1"));
        assert!(refusal.contains("does not exist on github.com"));
        assert!(!refusal.contains("secret-token"));
    }

    #[test]
    fn an_identity_with_no_github_connection_fetches_anonymously_but_never_pushes() {
        let missing = || {
            Err(GitHostTokenError::NoCredential {
                actor: Actor::of(POD_USER),
                provider: RepositoryProvider::Github,
                host: None,
            })
        };
        assert!(matches!(
            host_credential(missing(), false, "github.com").unwrap(),
            Credential::Anonymous { .. }
        ));
        assert!(matches!(
            host_credential(missing(), true, "github.com"),
            Err(GitProxyError::Unauthorized(_))
        ));
        assert_eq!(
            host_credential(Ok("ghs".to_string()), true, "github.com").unwrap(),
            Credential::Token("ghs".to_string())
        );
        let broken = Err(GitHostTokenError::Store(StoreError::new("down")));
        assert!(matches!(
            host_credential(broken, false, "github.com"),
            Err(GitProxyError::Upstream(_))
        ));
    }

    #[test]
    fn only_a_template_with_no_credential_is_fetched_anonymously() {
        let missing = || {
            Some(Err(GitHostTokenError::NoCredential {
                actor: Actor::of(POD_USER),
                provider: RepositoryProvider::Github,
                host: None,
            }))
        };
        assert!(matches!(
            registered_credential(missing(), RepositoryKind::Template, false, "github.com"),
            Ok(Credential::Anonymous { .. })
        ));
        assert!(matches!(
            registered_credential(missing(), RepositoryKind::Template, true, "github.com"),
            Err(GitProxyError::Unauthorized(_))
        ));
        assert!(matches!(
            registered_credential(missing(), RepositoryKind::Git, false, "github.com"),
            Err(GitProxyError::Unauthorized(_))
        ));
    }

    /// The bot whose App is not installed on `openai` fetching the public
    /// `openai/plugins`: a 502 there failed the clone and paged as an error.
    #[test]
    fn a_credential_that_cannot_reach_the_owner_fetches_anonymously_but_never_pushes() {
        let out_of_reach = || {
            Err(GitHostTokenError::Forbidden {
                actor: Actor::of(POD_USER),
                account: Box::new(moatless_types::git_host::GitHostAccount::Github {
                    owner: "openai".to_string(),
                    repository: Some("plugins".to_string()),
                }),
                source: "GitHub App reviewer is not installed on owner openai".into(),
            })
        };
        let Credential::Anonymous { because } =
            host_credential(out_of_reach(), false, "github.com").unwrap()
        else {
            panic!("a fetch out of the credential's reach goes anonymous");
        };
        assert!(because.contains("cannot access"), "{because}");
        assert!(matches!(
            host_credential(out_of_reach(), true, "github.com"),
            Err(GitProxyError::Unauthorized(_))
        ));
        let broken = Err(GitHostTokenError::Broken {
            actor: Actor::of(POD_USER),
            provider: RepositoryProvider::Github,
            source: "bad_refresh_token".into(),
        });
        assert!(matches!(
            host_credential(broken, false, "github.com"),
            Err(GitProxyError::Upstream(_))
        ));
    }

    #[test]
    fn an_anonymous_refusal_says_no_credential_was_presented() {
        let anonymous = Upstream {
            credential: Credential::Anonymous {
                because: "user-1 has not connected github".to_string(),
            },
            ..upstream()
        };
        let refusal = refusal_message(&anonymous, None);
        assert!(
            refusal.contains("has no github.com token for it (user-1 has not connected github)"),
            "{refusal}"
        );
        assert!(refusal.contains("fetched anonymously"), "{refusal}");
    }

    #[test]
    fn a_refusal_without_a_login_names_the_user_id() {
        let refusal = refusal_message(&upstream(), None);

        assert!(refusal.contains("Moatless user user-1, this task's run-as identity"));
    }

    #[test]
    fn every_provider_the_row_names_resolves_to_its_own_host() {
        let forgejo = repository(Scope::User, None);
        assert_eq!(git_host_repo(&forgejo).unwrap().host(), "git.example.com");

        let mut github = repository(Scope::User, None);
        github.provider = Some(RepositoryProvider::Github);
        github.remote = Some(moatless_workspace_types::repository::RepositoryRemote {
            url: "https://github.com/owner/repo.git".to_string(),
            host_repo: Some(GitHostRepo::github("owner", "repo")),
        });
        let repo = git_host_repo(&github).expect("a GitHub row resolves");
        assert_eq!(repo.host(), "github.com");
        assert_eq!(repo.owner(), "owner");
    }

    #[test]
    fn a_github_path_resolves_to_the_repository_it_names() {
        let repo = github_repo("soaplabs", "moatless.git").expect("a plain path resolves");
        assert_eq!((repo.owner(), repo.name()), ("soaplabs", "moatless"));
        assert_eq!(
            github_repo("soaplabs", ".github").unwrap().name(),
            ".github"
        );
        // An Enterprise Managed User login.
        assert_eq!(
            github_repo("alice_acme", "repo").unwrap().owner(),
            "alice_acme"
        );
    }

    #[test]
    fn a_github_path_that_would_leave_the_repository_resolves_to_nothing() {
        for (owner, name) in [
            ("..", "repo"),
            ("owner", ".."),
            ("owner", ".git"),
            ("owner", "a%2Fb"),
            ("", "repo"),
            ("own er", "repo"),
            ("owner", "repo?x"),
        ] {
            assert!(github_repo(owner, name).is_none(), "{owner}/{name}");
        }
    }

    #[test]
    fn a_gitlab_path_keeps_its_nested_groups_and_refuses_one_that_would_leave_the_host() {
        let repo = gitlab_repo("GitLab.com", "group/sub/project.git").unwrap();
        assert_eq!(
            (repo.host(), repo.owner(), repo.name()),
            ("gitlab.com", "group/sub", "project")
        );
        let upstream = Upstream::new(
            &repo,
            Credential::Token("t".to_string()),
            &Actor::of("user-1"),
            SandboxScope {
                attribution: Default::default(),
                sandbox: "sandbox-1".to_string(),
                token_user: "user-1".to_string(),
                user: "user-1".to_string(),
                workspace: "ws_1".to_string(),
                run_as: None,
                service_account: None,
                task: None,
            },
        );
        assert_eq!(upstream.username, "oauth2");
        assert_eq!(
            upstream.base_url,
            "https://gitlab.com/group/sub/project.git"
        );
        for (host, path) in [
            ("gitlab.com", "project"),
            ("gitlab.com", "group/../project"),
            ("gitlab.com", "group/.git"),
            ("gitlab.com", "group/a%2Fb"),
            ("gitlab.com:8443", "group/project"),
            ("user@gitlab.com", "group/project"),
            ("localhost", "group/project"),
            ("gitlab..com", "group/project"),
        ] {
            assert!(gitlab_repo(host, path).is_none(), "{host} {path}");
        }
    }

    #[tokio::test]
    async fn a_gitlab_host_is_reachable_only_when_it_is_gitlab_com_or_placed() {
        let mut placed = repository(Scope::Global, None);
        placed.provider = Some(RepositoryProvider::Gitlab);
        placed.remote = Some(moatless_workspace_types::repository::RepositoryRemote {
            url: "https://gitlab.example.com/group/project.git".to_string(),
            host_repo: Some(GitHostRepo::new(
                RepositoryProvider::Gitlab,
                "gitlab.example.com",
                "group",
                "project",
            )),
        });
        let placements = vec![
            moatless_workspace_ports::RepoPlacement {
                repository_id: "repo-1".to_string(),
                branch: None,
                mount_name: None,
                is_primary: true,
                position: 0,
                repository: Some(placed),
            },
            moatless_workspace_ports::RepoPlacement {
                repository_id: "repo-2".to_string(),
                branch: None,
                mount_name: None,
                is_primary: false,
                position: 1,
                repository: Some(repository(Scope::Global, None)),
            },
        ];
        let mut workspaces = MockWorkspaceResolver::new();
        workspaces
            .expect_placements()
            .returning(move |_| Ok(placements.clone()));
        let workspaces: Arc<dyn WorkspaceResolver> = Arc::new(workspaces);
        for (host, allowed) in [
            ("gitlab.com", true),
            ("gitlab.example.com", true),
            ("GitLab.Example.com", true),
            ("git.example.com", false),
            ("attacker.example.com", false),
        ] {
            assert_eq!(
                gitlab_host_allowed(&workspaces, "ws_1", host)
                    .await
                    .unwrap(),
                allowed,
                "{host}"
            );
        }
    }

    #[test]
    fn a_row_with_no_provider_or_an_unparsable_remote_resolves_to_nothing() {
        let mut untagged = repository(Scope::User, None);
        untagged.provider = None;
        untagged.remote.as_mut().unwrap().host_repo = None;
        assert!(git_host_repo(&untagged).is_none());

        let mut unparsable = repository(Scope::User, None);
        unparsable.remote = Some(moatless_workspace_types::repository::RepositoryRemote {
            url: "https://git.example.com/repo.git".to_string(),
            host_repo: None,
        });
        assert!(git_host_repo(&unparsable).is_none());
    }

    /// A proxy whose sandbox row is unreadable, so a token that passes every
    /// check fails on the read with `Upstream`.
    fn proxy(generation: i64) -> GitProxy {
        proxy_claiming(generation, MockTranscriptAppend::new())
    }

    fn proxy_claiming(generation: i64, transcript: MockTranscriptAppend) -> GitProxy {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        GitProxy::new(
            Arc::new(SandboxCallers::new(
                moatless_sandbox_control::compose::sandbox_graph(pool).sandbox_ingress(),
                Arc::new(StaticTokenGenerations::one(
                    "sandbox-1",
                    generation,
                    POD_USER,
                )),
                Some(SECRET.to_string()),
            )),
            Arc::new(MockWorkspaceResolver::new()),
            Arc::new(MockRepositoryReader::new()),
            Arc::new(MockCommitIdentity::new()),
            Arc::new(MockGitHostTokenIssuer::new()),
            crate::github_policy::test_support::policy(
                moatless_core::proxy_rules::ProxyPolicyMode::Log,
                Arc::new(MockWorkspaceResolver::new()),
                None,
            ),
            moatless_core::config::HttpConfig::default().git_proxy,
        )
        .with_own_events(Arc::new(OwnEvents::new(Arc::new(transcript), None)))
    }

    #[tokio::test]
    async fn a_token_from_an_older_generation_is_refused() {
        let proxy = proxy(2);
        let superseded =
            generate_user_auth_token("user-1", "user", "sandbox-1", 1, SECRET, 600).unwrap();
        let refused = proxy
            .resolve("repo-1", &superseded, false)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(refused, GitProxyError::Unauthorized(_)),
            "{refused}"
        );
        let refused = proxy
            .resolve_github("acme", "widgets", &superseded, false)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(refused, GitProxyError::Unauthorized(_)),
            "{refused}"
        );

        let current =
            generate_user_auth_token("user-1", "user", "sandbox-1", 2, SECRET, 600).unwrap();
        let past_the_check = proxy
            .resolve("repo-1", &current, false)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(past_the_check, GitProxyError::Upstream(_)),
            "{past_the_check}"
        );
    }

    fn issuer(
        answer: fn() -> Result<Option<AppWriteToken>, GitHostTokenError>,
    ) -> Arc<dyn GitHostTokenIssuer> {
        let mut issuer = MockGitHostTokenIssuer::new();
        issuer
            .expect_github_app_write()
            .returning(move |_, _, _| answer());
        Arc::new(issuer)
    }

    #[tokio::test]
    async fn a_github_push_presents_the_app_token() {
        let issuer = issuer(|| {
            Ok(Some(AppWriteToken {
                token: "ghs_app".to_string(),
            }))
        });
        let repo = GitHostRepo::github("acme", "widgets");
        let token = app_push_token(&issuer, &Actor::of("user-1"), &repo, true).await;
        assert_eq!(token.unwrap().as_deref(), Some("ghs_app"));
    }

    #[tokio::test]
    async fn a_push_without_an_app_token_falls_back_to_the_run_as_token() {
        let repo = GitHostRepo::github("acme", "widgets");
        let not_app = issuer(|| Ok(None));
        assert_eq!(
            app_push_token(&not_app, &Actor::of("user-1"), &repo, true)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_push_by_a_driver_below_write_access_is_refused() {
        let repo = GitHostRepo::github("acme", "widgets");
        let below_write = issuer(|| {
            Err(GitHostTokenError::Forbidden {
                actor: Actor::of("user-1"),
                account: Box::new(GitHostAccount::Github {
                    owner: "acme".to_string(),
                    repository: Some("widgets".to_string()),
                }),
                source: "read only".into(),
            })
        });
        let refused = app_push_token(&below_write, &Actor::of("user-1"), &repo, true)
            .await
            .unwrap_err();
        assert!(
            matches!(refused, GitProxyError::Unauthorized(_)),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn a_fetch_never_asks_for_an_app_token() {
        let issuer: Arc<dyn GitHostTokenIssuer> = Arc::new(MockGitHostTokenIssuer::new());
        let repo = GitHostRepo::github("acme", "widgets");
        assert_eq!(
            app_push_token(&issuer, &Actor::of("user-1"), &repo, false)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_gitlab_push_never_asks_for_an_app_token() {
        let issuer: Arc<dyn GitHostTokenIssuer> = Arc::new(MockGitHostTokenIssuer::new());
        let repo = GitHostRepo::new(RepositoryProvider::Gitlab, "gitlab.com", "acme", "widgets");
        assert_eq!(
            app_push_token(&issuer, &Actor::of("user-1"), &repo, true)
                .await
                .unwrap(),
            None
        );
    }

    /// A push through the proxy records each branch commit as the token's
    /// Task's own, under the id a `synchronize` webhook looks its maker up by;
    /// a push with no Task, or to another host, records nothing.
    #[tokio::test]
    async fn a_proxied_github_push_claims_its_commits_for_the_task() {
        let mut transcript = MockTranscriptAppend::new();
        transcript
            .expect_record_external_event()
            .withf(|task_id, event| {
                task_id == "task-1"
                    && event.external_id == "github:push:soaplabs/repo:abc123"
                    && event.source_name.as_deref() == Some("soaplabs/repo")
            })
            .times(1)
            .returning(|_, _| Ok(()));
        let proxy = proxy_claiming(1, transcript);
        let upstream = |host: &str, task: Option<&str>| Upstream {
            base_url: String::new(),
            credential: Credential::Token("t".to_string()),
            username: "x-access-token",
            host: host.to_string(),
            path: "soaplabs/repo".to_string(),
            run_as_user_id: "user-1".to_string(),
            workspace_id: "ws_1".to_string(),
            sandbox_id: "sandbox-1".to_string(),
            task_id: task.map(str::to_string),
        };
        let shas = ["abc123".to_string()];

        proxy
            .claim_pushed_commits(&upstream("github.com", Some("task-1")), &shas)
            .await;
        proxy
            .claim_pushed_commits(&upstream("github.com", None), &shas)
            .await;
        proxy
            .claim_pushed_commits(&upstream("gitlab.com", Some("task-1")), &shas)
            .await;
    }
}
