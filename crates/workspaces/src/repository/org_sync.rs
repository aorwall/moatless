//! Mirror configured GitHub organizations into the global registry.
//!
//! Every repository an organization owns is registered, which gives it its
//! default Workspace, and one GitHub reports archived, disabled, transferred
//! or deleted is soft-deleted.
//!
//! A repository is the organization's when its remote's host owner is the
//! organization, whoever registered it.

use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use tracing::info;

use moatless_core::config::WorkspaceSyncConfig;
use moatless_core::db::DbPool;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_types::access_control::{Actor, Scope, ScopeContext};
use moatless_types::git_host::GitHostAccount;
use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::repository::{RepositoryKind, RepositoryProvider};

use super::error::RepositoryError;
use super::models::{Repository, RepositoryPatch};
use super::operations::Repositories;
use super::remote::normalize_remote_url;
use super::schemas::CreateRepositoryRequest;
use crate::workspace::git_sync::GitSyncService;

/// GitHub's largest page.
const PAGE_SIZE: usize = 100;

/// One repository in a listing, or `GET /repos/{owner}/{name}`'s answer.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OrgRepo {
    name: String,
    html_url: String,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    disabled: bool,
    owner: Option<RepoOwner>,
    default_branch: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RepoOwner {
    login: String,
}

/// An account's repositories, and whether they are all of them.
struct Listing {
    repos: Vec<OrgRepo>,
    /// `false` for `/users/{account}/repos`, which lists public repositories
    /// only, so an unlisted repository says nothing about whether it exists.
    complete: bool,
}

/// What `GET /repos/{owner}/{name}` says about a repository a listing left out.
#[derive(Debug, PartialEq, Eq)]
enum Unlisted {
    Gone(&'static str),
    /// Still live under the organization, or GitHub's answer proves nothing.
    Kept(String),
}

impl OrgRepo {
    fn is_live(&self) -> bool {
        !self.archived && !self.disabled
    }
}

/// What one organization's listing asks of the registry.
#[derive(Default)]
pub(crate) struct OrgPlan {
    pub(crate) register: Vec<OrgRepo>,
    /// A known repository whose default branch or description GitHub reports
    /// differently.
    pub(crate) refresh: Vec<(String, RepositoryPatch)>,
    /// Known repositories the listing reports archived or disabled.
    pub(crate) remove: Vec<String>,
    /// Known repositories the listing leaves out, as `(id, owner, name)`. A
    /// listing can omit a live repository, so each is confirmed before it is
    /// removed.
    pub(crate) unlisted: Vec<(String, String, String)>,
}

/// What one organization's sync did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OrgSyncSummary {
    pub registered: usize,
    pub refreshed: usize,
    pub removed: usize,
    /// Unlisted repositories kept because GitHub did not confirm them gone,
    /// with why.
    pub kept: Vec<(String, String)>,
    /// Repositories a write failed for, with the error.
    pub failed: Vec<(String, String)>,
}

/// Decide what `listed` asks of `known`, every live repository the registry
/// holds.
///
/// Only a global, GitHub-hosted repository whose host owner is `org` is
/// compared, so a user's own registration of an organization repository is
/// never refreshed or removed.
pub(crate) fn plan(org: &str, listed: &[OrgRepo], known: &[Repository]) -> OrgPlan {
    let ours: Vec<&Repository> = known
        .iter()
        .filter(|r| r.scope == Scope::Global && r.provider == Some(RepositoryProvider::Github))
        .filter(|r| {
            r.remote
                .as_ref()
                .and_then(|remote| remote.host_repo.as_ref())
                .is_some_and(|host| host.owner().eq_ignore_ascii_case(org))
        })
        .collect();
    let url_key = |r: &Repository| {
        r.remote
            .as_ref()
            .map(|remote| normalize_remote_url(&remote.url))
    };

    let mut plan = OrgPlan::default();
    let mut listed_keys = HashSet::new();
    for repo in listed {
        let key = normalize_remote_url(&repo.html_url);
        listed_keys.insert(key.clone());
        let existing = ours
            .iter()
            .find(|r| url_key(r).as_deref() == Some(key.as_str()));
        if !repo.is_live() {
            plan.remove.extend(existing.map(|r| r.id.clone()));
            continue;
        }
        let Some(existing) = existing else {
            // A user's own registration keeps the repository private to them
            // rather than gaining a global twin.
            if !known
                .iter()
                .any(|r| url_key(r).as_deref() == Some(key.as_str()))
            {
                plan.register.push(repo.clone());
            }
            continue;
        };
        let changed = |stored: &Option<String>, reported: &Option<String>| {
            reported.as_ref().is_some_and(|r| !r.is_empty()) && stored != reported
        };
        let patch = RepositoryPatch {
            default_branch: changed(&existing.default_branch, &repo.default_branch)
                .then(|| repo.default_branch.clone()),
            description: changed(&existing.description, &repo.description)
                .then(|| repo.description.clone()),
            ..RepositoryPatch::default()
        };
        if patch.default_branch.is_some() || patch.description.is_some() {
            plan.refresh.push((existing.id.clone(), patch));
        }
    }
    plan.unlisted = ours
        .iter()
        .filter(|r| url_key(r).is_none_or(|key| !listed_keys.contains(&key)))
        .filter_map(|r| {
            let host = r.remote.as_ref()?.host_repo.as_ref()?;
            Some((
                r.id.clone(),
                host.owner().to_string(),
                host.name().to_string(),
            ))
        })
        .collect();
    plan
}

/// Judge `GET /repos/{owner}/{name}`'s answer for an unlisted repository.
///
/// A 404 keeps the row, deliberately: a token that lost access to a private
/// repository gets the same 404 as for a deleted one, and a removal is never
/// restored. GitHub redirects a rename, so another `name` means renamed.
fn judge_unlisted(
    org: &str,
    name: &str,
    status: reqwest::StatusCode,
    repo: Option<&OrgRepo>,
) -> Unlisted {
    match (status, repo) {
        (reqwest::StatusCode::NOT_FOUND, _) => {
            Unlisted::Kept("404: deleted, or hidden from the token".to_string())
        }
        (status, Some(repo)) if status.is_success() => {
            if !repo.is_live() {
                Unlisted::Gone("archived")
            } else if repo
                .owner
                .as_ref()
                .is_some_and(|owner| !owner.login.eq_ignore_ascii_case(org))
            {
                Unlisted::Gone("transferred")
            } else if !repo.name.eq_ignore_ascii_case(name) {
                Unlisted::Gone("renamed")
            } else {
                Unlisted::Kept("GitHub still serves it; the listing left it out".to_string())
            }
        }
        (status, _) => Unlisted::Kept(format!("GitHub answered {status}")),
    }
}

/// Syncs the configured organizations' repositories into the registry.
pub struct OrgSync {
    repositories: Arc<Repositories>,
    git_sync: Arc<GitSyncService>,
    db: DbPool,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    http: reqwest::Client,
    github_api_base: String,
    orgs: Vec<String>,
    /// Whose GitHub credential lists the organizations, and who is recorded as
    /// each registration's creator, so `DefaultBranchWatch` reads it with the
    /// same credential.
    acting_user_id: String,
}

impl OrgSync {
    /// `None` when `config` names no organization.
    pub(crate) fn new(
        repositories: Arc<Repositories>,
        git_sync: Arc<GitSyncService>,
        db: DbPool,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        timeouts: HttpTimeouts,
        github_api_base: String,
        config: &WorkspaceSyncConfig,
    ) -> Option<Self> {
        let acting_user_id = config.github_org_sync_user_id.clone()?;
        if config.github_orgs.is_empty() {
            return None;
        }
        Some(Self {
            repositories,
            git_sync,
            db,
            git_host_tokens,
            http: super::config_reader::client(timeouts),
            github_api_base,
            orgs: config.github_orgs.clone(),
            acting_user_id,
        })
    }

    /// The organizations this sync lists, in configured order.
    pub fn orgs(&self) -> &[String] {
        &self.orgs
    }

    /// List `org` and apply the [`plan`] it yields.
    ///
    /// A listing that fails changes nothing, and an incomplete one removes
    /// nothing it left out.
    pub async fn sync_org(&self, org: &str) -> Result<OrgSyncSummary, RepositoryError> {
        let token = self.token(org).await?;
        let listing = self.list(org, &token).await?;
        #[expect(clippy::disallowed_methods, reason = "the org sync acts for no caller")]
        let system = ScopeContext::system();
        let known = self.repositories.list_all(&system).await?;
        let mut plan = plan(org, &listing.repos, &known);

        let mut summary = OrgSyncSummary::default();
        for (id, owner, name) in std::mem::take(&mut plan.unlisted) {
            let verdict = if listing.complete {
                self.confirm_unlisted(org, &owner, &name, &token).await
            } else {
                Unlisted::Kept("the account's listing shows public repositories only".to_string())
            };
            match verdict {
                Unlisted::Gone(why) => {
                    info!(org, repository_id = %id, why, "An unlisted repository is gone");
                    plan.remove.push(id);
                }
                Unlisted::Kept(why) => summary.kept.push((id, why)),
            }
        }

        let admin = ScopeContext::admin(&self.acting_user_id);
        for repo in plan.register {
            match self.register(&admin, &repo).await {
                Ok(true) => summary.registered += 1,
                Ok(false) => {}
                Err(e) => summary.failed.push((repo.html_url, e.to_string())),
            }
        }
        for (id, patch) in plan.refresh {
            match self.repositories.store().update(&id, &patch, &system).await {
                Ok(_) => summary.refreshed += 1,
                Err(e) => summary.failed.push((id, e.to_string())),
            }
        }
        for id in plan.remove {
            let deleted = super::mutations::delete_repository(
                self.repositories.store(),
                &self.db,
                &system,
                id.clone(),
            )
            .await;
            match deleted {
                Ok(()) => {
                    info!(org, repository_id = %id, "Removed a repository the organization no longer has");
                    summary.removed += 1;
                }
                Err(e) => summary.failed.push((id, e.to_string())),
            }
        }
        Ok(summary)
    }

    async fn confirm_unlisted(&self, org: &str, owner: &str, name: &str, token: &str) -> Unlisted {
        let resp = match self.get(&format!("repos/{owner}/{name}"), token).await {
            Ok(resp) => resp,
            Err(e) => return Unlisted::Kept(e.to_string()),
        };
        let status = resp.status();
        let repo = if status.is_success() {
            resp.json::<OrgRepo>().await.ok()
        } else {
            None
        };
        judge_unlisted(org, name, status, repo.as_ref())
    }

    /// Register `repo` as a global repository, which syncs its Workspaces.
    ///
    /// Returns `false` without writing when the registry holds a soft-deleted
    /// global row for it: a repository an administrator removed, or one this
    /// sync removed when it was archived, stays removed.
    async fn register(
        &self,
        admin: &ScopeContext,
        repo: &OrgRepo,
    ) -> Result<bool, RepositoryError> {
        let key = normalize_remote_url(&repo.html_url);
        if self
            .repositories
            .store()
            .exists_by_normalized_remote_url_including_deleted(&key, Scope::Global, None)
            .await?
        {
            return Ok(false);
        }
        super::mutations::create_repository(
            self.repositories.store(),
            &self.git_sync,
            admin,
            CreateRepositoryRequest {
                name: repo.name.clone(),
                kind: RepositoryKind::Remote,
                remote_url: Some(repo.html_url.clone()),
                default_branch: repo.default_branch.clone(),
                description: repo.description.clone().filter(|d| !d.is_empty()),
                provider: Some(RepositoryProvider::Github),
                scope: Some(Scope::Global),
            },
        )
        .await?;
        info!(url = %repo.html_url, "Registered a GitHub organization repository");
        Ok(true)
    }

    async fn token(&self, org: &str) -> Result<String, RepositoryError> {
        self.git_host_tokens
            .token(
                &Actor::of(&self.acting_user_id),
                &GitHostAccount::Github {
                    owner: org.to_string(),
                    repository: None,
                },
            )
            .await
            .map_err(|e| RepositoryError::ProviderError(format!("no GitHub token for {org}: {e}")))
    }

    /// Every repository `org` owns.
    ///
    /// Reads `/orgs/{org}/repos`, and for a user account `/user/repos` when
    /// the token is that user's, else the public `/users/{org}/repos`.
    async fn list(&self, org: &str, token: &str) -> Result<Listing, RepositoryError> {
        if let Some(repos) = self
            .list_pages(&format!("orgs/{org}/repos?type=all"), token)
            .await?
        {
            return Ok(Listing {
                repos,
                complete: true,
            });
        }
        let login = match self.get("user", token).await {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<RepoOwner>().await.ok().map(|owner| owner.login)
            }
            _ => None,
        };
        if login.is_some_and(|login| login.eq_ignore_ascii_case(org)) {
            let repos = self
                .list_pages("user/repos?affiliation=owner&visibility=all", token)
                .await?
                .unwrap_or_default()
                .into_iter()
                .filter(|r| {
                    r.owner
                        .as_ref()
                        .is_some_and(|o| o.login.eq_ignore_ascii_case(org))
                })
                .collect();
            return Ok(Listing {
                repos,
                complete: true,
            });
        }
        let repos = self
            .list_pages(&format!("users/{org}/repos?type=owner"), token)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(format!("GitHub account {org}")))?;
        Ok(Listing {
            repos,
            complete: false,
        })
    }

    async fn get(&self, path: &str, token: &str) -> Result<reqwest::Response, RepositoryError> {
        self.http
            .get(format!("{}/{path}", self.github_api_base))
            .header("User-Agent", moatless_core::USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| RepositoryError::ProviderError(format!("GitHub request failed: {e}")))
    }

    /// Read every page of `path`, stopping on an empty one. `None` when the
    /// first page answers 404.
    async fn list_pages(
        &self,
        path: &str,
        token: &str,
    ) -> Result<Option<Vec<OrgRepo>>, RepositoryError> {
        let mut repos = Vec::new();
        for page in 1.. {
            let resp = self
                .get(&format!("{path}&per_page={PAGE_SIZE}&page={page}"), token)
                .await?;
            let status = resp.status();
            if status == reqwest::StatusCode::NOT_FOUND && page == 1 {
                return Ok(None);
            }
            if !status.is_success() {
                return Err(RepositoryError::ProviderError(format!(
                    "GitHub returned {status} listing {path}"
                )));
            }
            let batch: Vec<OrgRepo> = resp.json().await.map_err(|e| {
                RepositoryError::ProviderError(format!("unreadable GitHub listing of {path}: {e}"))
            })?;
            if batch.is_empty() {
                break;
            }
            repos.extend(batch);
        }
        Ok(Some(repos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::remote::resolve_remote;

    fn listed(name: &str) -> OrgRepo {
        OrgRepo {
            name: name.to_string(),
            html_url: format!("https://github.com/acme/{name}"),
            archived: false,
            disabled: false,
            owner: Some(RepoOwner {
                login: "acme".to_string(),
            }),
            default_branch: Some("main".to_string()),
            description: None,
        }
    }

    fn known(id: &str, url: &str, scope: Scope) -> Repository {
        Repository {
            id: id.to_string(),
            name: id.to_string(),
            kind: RepositoryKind::Remote,
            remote: Some(resolve_remote(
                Some(RepositoryProvider::Github),
                url.to_string(),
            )),
            default_branch: Some("main".to_string()),
            description: None,
            provider: Some(RepositoryProvider::Github),
            scope,
            owner_user_id: None,
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn an_unknown_repository_is_registered_and_a_known_one_is_not() {
        let plan = plan(
            "acme",
            &[listed("api"), listed("web")],
            &[known(
                "r1",
                "https://github.com/acme/api.git",
                Scope::Global,
            )],
        );

        let registered: Vec<_> = plan.register.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(registered, ["web"]);
        assert!(plan.refresh.is_empty());
        assert!(plan.remove.is_empty());
    }

    #[test]
    fn an_archived_repository_is_removed_and_an_unlisted_one_is_confirmed() {
        let mut archived = listed("old");
        archived.archived = true;
        let plan = plan(
            "ACME",
            &[listed("api"), archived],
            &[
                known("api", "https://github.com/acme/api", Scope::Global),
                known("old", "https://github.com/acme/old", Scope::Global),
                known("gone", "https://github.com/acme/gone", Scope::Global),
            ],
        );

        assert!(plan.register.is_empty());
        assert_eq!(plan.remove, ["old"]);
        assert_eq!(
            plan.unlisted,
            [("gone".to_string(), "acme".to_string(), "gone".to_string())]
        );
    }

    /// A listing can omit a live repository, so only GitHub's answer for the
    /// repository itself removes it.
    #[test]
    fn an_unlisted_repository_is_gone_only_when_github_says_so() {
        use reqwest::StatusCode;
        let live = listed("api");
        let mut archived = listed("api");
        archived.archived = true;
        let mut moved = listed("api");
        moved.owner = Some(RepoOwner {
            login: "other".to_string(),
        });
        let renamed = listed("api-v2");
        let verdict = |status, repo| {
            matches!(
                judge_unlisted("acme", "api", status, repo),
                Unlisted::Gone(_)
            )
        };

        assert!(!verdict(StatusCode::NOT_FOUND, None));
        assert!(!verdict(StatusCode::FORBIDDEN, None));
        assert!(!verdict(StatusCode::OK, Some(&live)));
        assert!(verdict(StatusCode::OK, Some(&archived)));
        assert!(verdict(StatusCode::OK, Some(&moved)));
        assert!(verdict(StatusCode::OK, Some(&renamed)));
    }

    /// The org sync owns the global registrations of its organization only, and
    /// adds none beside a user's registration.
    #[test]
    fn another_owner_or_a_user_registration_is_left_alone() {
        let plan = plan(
            "acme",
            &[listed("api")],
            &[
                known("other", "https://github.com/other/api", Scope::Global),
                known("mine", "https://github.com/acme/api", Scope::User),
            ],
        );

        assert!(plan.register.is_empty());
        assert!(plan.remove.is_empty());
        assert!(plan.unlisted.is_empty());
    }

    #[test]
    fn a_changed_default_branch_or_description_is_refreshed() {
        let mut moved = listed("api");
        moved.default_branch = Some("trunk".to_string());
        moved.description = Some("The API".to_string());
        let mut blank = listed("web");
        blank.description = Some(String::new());

        let plan = plan(
            "acme",
            &[moved, blank],
            &[
                known("api", "https://github.com/acme/api", Scope::Global),
                known("web", "https://github.com/acme/web", Scope::Global),
            ],
        );

        assert_eq!(plan.refresh.len(), 1);
        let (id, patch) = &plan.refresh[0];
        assert_eq!(id, "api");
        assert_eq!(patch.default_branch, Some(Some("trunk".to_string())));
        assert_eq!(patch.description, Some(Some("The API".to_string())));
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "org_sync_db_tests.rs"]
mod db_tests;
