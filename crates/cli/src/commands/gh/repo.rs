//! The one repository every `moat gh` call addresses, read or written.

use std::borrow::Cow;

use moatless_workspace_types::workspace::proxy_clone_repository_id;

use crate::client::MoatClient;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct PullRequestRef {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) number: u64,
}

/// The repository a call addresses, and where that name came from.
pub(super) struct RepoTarget {
    pub(super) owner: String,
    pub(super) repo: String,
    /// `false` when the name came from the cwd's `origin` remote rather than
    /// from `-R`, which is what [`RepoTarget::failed`] has to say out loud.
    explicit: bool,
}

impl RepoTarget {
    pub(super) fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// Name the repository a failed call addressed, and where that name came
    /// from.
    ///
    /// GitHub answers a pull request in another repository with a 404, which on
    /// its own reads as a pull request that does not exist; the commonest cause
    /// is a working directory whose `origin` is not the pull request's repo.
    pub(super) fn failed(
        &self,
        target: &str,
        e: impl std::fmt::Display,
    ) -> Box<dyn std::error::Error> {
        let source = if self.explicit {
            "named by `-R`"
        } else {
            "inferred from the `origin` remote of the current directory"
        };
        format!(
            "{e}\nThis call addressed {}{target}, {source}. \
             Pass `-R owner/repo` when the target is in another repository.",
            self.slug()
        )
        .into()
    }
}

/// Resolve the target repository from an explicit `-R` value or the cwd git
/// repo.
pub(super) async fn resolve_repo(
    client: &MoatClient,
    flag: Option<&str>,
) -> Result<RepoTarget, Box<dyn std::error::Error>> {
    if let Some(value) = flag {
        let (owner, repo) = parse_repo_arg(value)
            .ok_or_else(|| format!("invalid repository `{value}` — expected owner/repo"))?;
        return Ok(RepoTarget {
            owner,
            repo,
            explicit: true,
        });
    }
    let (owner, repo) = origin_repo(client).await?;
    Ok(RepoTarget {
        owner,
        repo,
        explicit: false,
    })
}

/// Why `origin` named no GitHub repository.
///
/// The two callers differ on what a failure means: [`resolve_repo`] reports
/// either, while a passthrough falls back to `gh`'s own resolution and
/// announces only the variant that is not simply a checkout `moat gh` does not
/// serve.
#[derive(Debug, thiserror::Error)]
enum OriginError {
    /// `origin` is absent, or names a repository this command has no path to.
    #[error("{0}")]
    Unaddressable(String),
    /// A proxied origin whose repository row could not be read.
    #[error("{0}")]
    Unreadable(String),
}

/// What an `origin` URL names, before anything is read back from the backend.
enum OriginUrl<'a> {
    /// A GitHub remote, in any of the URL forms.
    Github(String, String),
    /// A `moat repos clone` remote, which names a repository id and no owner.
    Proxied(&'a str),
    /// Neither, so `moat gh` cannot address it.
    Foreign,
}

/// The `user:password@` span in an HTTP(S) URL, as `(start, end)` byte offsets
/// around the credential itself.
///
/// Deliberately HTTP(S) only: `ssh://git@github.com/o/r` carries a username and
/// no secret, which [`parse_github_remote`] needs in place.
fn userinfo_span(url: &str) -> Option<(usize, usize)> {
    let authority = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .map(|rest| url.len() - rest.len())?;
    let end = url[authority..]
        .find('/')
        .map_or(url.len(), |i| authority + i);
    let at = url[authority..end].rfind('@')? + authority;
    Some((authority, at + 1))
}

/// `url` with any credential dropped, for matching.
fn strip_userinfo(url: &str) -> Cow<'_, str> {
    match userinfo_span(url) {
        Some((start, end)) => Cow::Owned(format!("{}{}", &url[..start], &url[end..])),
        None => Cow::Borrowed(url),
    }
}

/// `url` with any credential masked, for printing.
///
/// Masked rather than dropped so the reader can tell the remote carries one —
/// which is often itself the reason a command behaved unexpectedly.
fn redact_userinfo(url: &str) -> Cow<'_, str> {
    match userinfo_span(url) {
        Some((start, end)) => Cow::Owned(format!("{}***@{}", &url[..start], &url[end..])),
        None => Cow::Borrowed(url),
    }
}

fn classify_origin<'a>(base_url: &str, url: &'a str) -> OriginUrl<'a> {
    if let Some((owner, name)) = parse_github_remote(url) {
        return OriginUrl::Github(owner, name);
    }
    match proxy_clone_repository_id(base_url, url).or_else(|| any_host_proxy_repository_id(url)) {
        Some(id) => OriginUrl::Proxied(id),
        None => OriginUrl::Foreign,
    }
}

/// `(owner, repo)` for the cwd's `origin` remote — the one repository every
/// `moat gh` call addresses, read or write.
///
/// Deliberately `origin` and not `gh`'s own pick, which prefers a remote named
/// `upstream`: in a fork clone holding both, a read would answer about the
/// upstream PR sharing the number while a write went to the fork.
async fn origin_repo(client: &MoatClient) -> Result<(String, String), OriginError> {
    let url = git_stdout(&["remote", "get-url", "origin"]).map_err(|e| {
        OriginError::Unaddressable(format!(
            "could not infer the GitHub repository ({e}) — pass `-R owner/repo`"
        ))
    })?;
    let bare = strip_userinfo(url.trim());
    match classify_origin(client.base_url(), &bare) {
        OriginUrl::Github(owner, name) => Ok((owner, name)),
        OriginUrl::Proxied(id) => proxied_repo(client, id).await,
        OriginUrl::Foreign => Err(OriginError::Unaddressable(format!(
            "origin remote `{}` is not a GitHub repository — pass `-R owner/repo`",
            redact_userinfo(url.trim())
        ))),
    }
}

/// The repository id in a `/git/{id}` remote served by any host.
///
/// The sandbox writes this remote from `BACKEND_API_URL` while the CLI matches
/// it against whatever the login profile resolved to, so a deployment naming
/// one backend two ways would otherwise reject its own proxied remote. Only the
/// id comes from the URL; the row is still read from our own backend.
fn any_host_proxy_repository_id(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let (_, path) = rest.split_once("/git/")?;
    let id = path.trim_end_matches('/');
    let id = id.strip_suffix(".git").unwrap_or(id);
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// The `(owner, repo)` the repository row behind a proxied origin names.
async fn proxied_repo(
    client: &MoatClient,
    repository_id: &str,
) -> Result<(String, String), OriginError> {
    let row = client
        .api()
        .get_repository(repository_id)
        .await
        .map_err(|e| {
            OriginError::Unreadable(format!(
                "could not read the proxied origin's repository ({e})"
            ))
        })?;
    let remote = row.remote.ok_or_else(|| {
        OriginError::Unaddressable(format!("repository {repository_id} stores no remote URL"))
    })?;
    let host = remote.host_repo.ok_or_else(|| {
        OriginError::Unaddressable(format!(
            "repository {repository_id} has no resolved git host to address"
        ))
    })?;
    if host.provider != moatless_workspace_types::repository::RepositoryProvider::Github {
        return Err(OriginError::Unaddressable(format!(
            "repository {repository_id} is hosted on {}, and `moat gh` serves GitHub only — \
             there is no pull-request path for it from a sandbox",
            host.provider
        )));
    }
    Ok((host.owner, host.name))
}

/// The `owner/repo` to hand a spawned `gh` as `GH_REPO`, so a passthrough reads
/// the repository [`origin_repo`] writes to.
pub(super) async fn origin_gh_repo(client: &MoatClient) -> Option<String> {
    if std::env::var_os("GH_REPO").is_some() {
        return None;
    }
    match origin_repo(client).await {
        Ok((owner, name)) => Some(format!("{owner}/{name}")),
        // A checkout `moat gh` does not serve, a Forgejo one above all, hits
        // this on every invocation, so it stays quiet and leaves `gh` to itself.
        Err(OriginError::Unaddressable(_)) => None,
        Err(e @ OriginError::Unreadable(_)) => {
            eprintln!("moat: {e}");
            None
        }
    }
}

/// Parse a `-R` value: `owner/repo` shorthand or any GitHub remote URL form.
pub(super) fn parse_repo_arg(value: &str) -> Option<(String, String)> {
    if let Some(parsed) = parse_github_remote(value) {
        return Some(parsed);
    }
    let mut parts = value.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((owner, repo))
}

/// Decompose a GitHub remote URL into `(owner, repo)`. Supports HTTPS, SSH
/// shorthand (`git@github.com:owner/repo`), and the explicit `ssh://` form.
fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

pub(super) fn parse_pull_request_url(url: &str) -> Option<PullRequestRef> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if parts.next()? != "pull" {
        return None;
    }
    let number = parts.next()?.parse().ok()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(PullRequestRef {
        owner,
        repo,
        number,
    })
}

pub(super) fn current_git_branch() -> Result<String, Box<dyn std::error::Error>> {
    let branch = git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"])
        .map_err(|e| format!("could not infer the head branch ({e}) — pass `--head <branch>`"))?;
    let branch = branch.trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Err("detached HEAD — pass `--head <branch>`".into());
    }
    Ok(branch)
}

fn git_stdout(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests;
