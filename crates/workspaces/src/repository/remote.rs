//! Repository URL interpretation and matching.

use moatless_workspace_types::repository::{
    GITHUB_HOST, GitHostRepo, RepositoryProvider, RepositoryRemote,
};

/// Resolve the remote facts carried by a repository read.
pub(crate) fn resolve_remote(
    provider: Option<RepositoryProvider>,
    url: String,
) -> RepositoryRemote {
    RepositoryRemote {
        uses_ssh: is_ssh_url(&url),
        host_repo: provider.and_then(|provider| parse_host_repo(provider, &url)),
        url,
    }
}

/// Parse a supported remote for its declared git host.
fn parse_host_repo(provider: RepositoryProvider, remote_url: &str) -> Option<GitHostRepo> {
    match provider {
        RepositoryProvider::Github => parse_github(remote_url),
        RepositoryProvider::Gitlab | RepositoryProvider::Gitness | RepositoryProvider::Forgejo => {
            parse_hosted(provider, remote_url)
        }
    }
}

/// Forgejo's REST API v1 root for `repo`, on the host its remote named.
///
/// Every Forgejo call in this crate builds on this, so the scheme is decided
/// once — by [`moatless_identity::forgejo::instances::base_url`], which owns
/// why it is never the remote URL's.
pub(crate) fn forgejo_repo_api(repo: &GitHostRepo) -> String {
    format!(
        "{}/api/v1/repos/{}/{}",
        moatless_identity::forgejo::instances::base_url(repo.host()),
        repo.owner(),
        repo.name()
    )
}

/// Normalize a remote URL for identity matching without changing persisted spelling.
///
/// Two repositories are the same connection when this agrees on them, so the
/// uniqueness check, the git-sync cache key and any caller deciding whether a
/// host's repository is already registered must all go through this.
pub(crate) fn normalize_remote_url(remote_url: &str) -> String {
    remote_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Extra path segments, an empty owner and any host but `github.com` are
/// rejected.
fn parse_github(remote_url: &str) -> Option<GitHostRepo> {
    let trimmed = remote_url.trim().trim_end_matches('/');
    let path = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path.to_string()
    } else {
        let parsed = url::Url::parse(trimmed).ok()?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.host_str()?.eq_ignore_ascii_case(GITHUB_HOST)
        {
            return None;
        }
        parsed.path().trim_start_matches('/').to_string()
    };

    let path = path.trim_end_matches(".git").trim_end_matches('/');
    let mut segments = path.split('/');
    let owner = segments.next()?.trim();
    let name = segments.next()?.trim();
    if owner.is_empty() || name.is_empty() || segments.next().is_some() {
        return None;
    }
    Some(GitHostRepo::github(owner, name))
}

/// A self-hostable provider: any host, and a namespace that may nest, so
/// the last path segment is the repository and everything before it is the
/// owner.
fn parse_hosted(provider: RepositoryProvider, remote_url: &str) -> Option<GitHostRepo> {
    let trimmed = remote_url.trim().trim_end_matches('/');
    let (host, path) = if let Some(rest) = trimmed.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        (host.to_string(), path.to_string())
    } else {
        let parsed = url::Url::parse(trimmed).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        (
            parsed.host_str()?.to_string(),
            parsed.path().trim_start_matches('/').to_string(),
        )
    };

    let path = path.trim_end_matches(".git").trim_end_matches('/');
    let (owner, name) = path.rsplit_once('/')?;
    let owner = owner.trim();
    let name = name.trim();
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(GitHostRepo::new(provider, host, owner, name))
}
/// Whether a clone URL uses SSH transport.
///
/// Matches both `ssh://…` URLs and scp-like `[user@]host:path` syntax
/// (e.g. `git@github.com:owner/repo.git`), while excluding HTTP(S) URLs.
fn is_ssh_url(url: &str) -> bool {
    if url.starts_with("ssh://") || url.starts_with("git+ssh://") {
        return true;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return false;
    }
    // scp-like syntax: user@host:path — an '@' before a ':', and the ':' is not
    // part of a URL scheme.
    match (url.find('@'), url.find(':')) {
        (Some(at), Some(colon)) => colon > at,
        _ => false,
    }
}

/// The resolved identity required by a git-host operation.
pub(crate) fn require_host_repo(
    remote: &RepositoryRemote,
) -> Result<&GitHostRepo, super::error::RepositoryError> {
    remote.host_repo.as_ref().ok_or_else(|| {
        super::error::RepositoryError::BadRequest(format!(
            "Repository has no resolved git host: {}",
            remote.url
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolving_a_remote_keeps_its_spelling_and_requires_a_declared_host() {
        let url = "  https://github.com/Acme/Repo.git/  ".to_string();
        let resolved = resolve_remote(Some(RepositoryProvider::Github), url.clone());
        assert_eq!(resolved.url, url);
        assert!(!resolved.uses_ssh);
        assert_eq!(
            resolved.host_repo,
            Some(GitHostRepo::github("Acme", "Repo"))
        );
        assert!(resolve_remote(None, url).host_repo.is_none());
    }

    #[test]
    fn ssh_transport_does_not_require_a_supported_host_identity() {
        let remote = resolve_remote(
            None,
            "ssh://deploy@git.internal:2222/team/repo.git".to_string(),
        );
        assert!(remote.uses_ssh);
        assert!(remote.host_repo.is_none());
    }

    #[test]
    fn a_mismatched_declared_host_keeps_the_remote_unresolved() {
        let remote = resolve_remote(
            Some(RepositoryProvider::Github),
            "https://gitlab.com/acme/repo".to_string(),
        );
        assert_eq!(remote.url, "https://gitlab.com/acme/repo");
        assert!(remote.host_repo.is_none());
    }

    #[test]
    fn normalize_remote_url_strips_and_lowercases() {
        assert_eq!(
            normalize_remote_url("https://github.com/Owner/Repo.git"),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("https://github.com/Owner/Repo/"),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("  https://GitHub.com/o/r.git/  "),
            "https://github.com/o/r"
        );
        // Two URLs differing only by trailing `.git`/slash/case normalize equal.
        assert_eq!(
            normalize_remote_url("https://github.com/a/b"),
            normalize_remote_url("https://github.com/A/B.git/")
        );
    }

    fn parse(provider: RepositoryProvider, url: &str) -> Option<(String, String)> {
        parse_host_repo(provider, url).map(|r| (r.owner().to_string(), r.name().to_string()))
    }

    #[test]
    fn github_parser_accepts_https_and_ssh_and_rejects_other_hosts() {
        let expected = Some(("acme".to_string(), "repo".to_string()));
        assert_eq!(
            parse(
                RepositoryProvider::Github,
                "https://github.com/acme/repo.git"
            ),
            expected
        );
        assert_eq!(
            parse(RepositoryProvider::Github, "git@github.com:acme/repo.git"),
            expected
        );
        assert_eq!(
            parse(RepositoryProvider::Github, "https://gitlab.com/acme/repo"),
            None
        );
        assert_eq!(parse(RepositoryProvider::Github, "not-a-url"), None);
    }

    #[test]
    fn github_carries_the_one_host_it_has() {
        let repo = parse_host_repo(RepositoryProvider::Github, "https://github.com/acme/repo")
            .expect("parses");
        assert_eq!(repo.host(), GITHUB_HOST);
        assert_eq!(repo.provider(), RepositoryProvider::Github);
        assert_eq!(repo.to_string(), "acme/repo");
        assert_eq!(GitHostRepo::github("acme", "repo").host(), GITHUB_HOST);
    }

    #[test]
    fn a_self_hosted_provider_keeps_its_own_host() {
        let repo = parse_host_repo(
            RepositoryProvider::Gitlab,
            "https://gitlab.example.com/acme/repo.git",
        )
        .expect("parses");
        assert_eq!(repo.host(), "gitlab.example.com");
        assert_eq!(repo.owner(), "acme");
        assert_eq!(repo.name(), "repo");

        let ssh = parse_host_repo(
            RepositoryProvider::Gitness,
            "git@gitness.internal:team/repo",
        )
        .expect("parses");
        assert_eq!(ssh.host(), "gitness.internal");
        assert_eq!(ssh.owner(), "team");
    }

    #[test]
    fn a_nested_namespace_keeps_every_segment_but_the_last_as_the_owner() {
        let repo = parse_host_repo(
            RepositoryProvider::Gitlab,
            "https://gitlab.com/group/subgroup/repo.git",
        )
        .expect("parses");
        assert_eq!(repo.owner(), "group/subgroup");
        assert_eq!(repo.name(), "repo");
    }

    #[test]
    fn detects_ssh_urls() {
        assert!(is_ssh_url("git@github.com:owner/repo.git"));
        assert!(is_ssh_url("git@gitlab.example.com:group/sub/repo.git"));
        assert!(is_ssh_url("ssh://git@example.com/owner/repo.git"));
        assert!(is_ssh_url("deploy@10.0.0.1:/srv/git/repo.git"));
    }

    #[test]
    fn rejects_non_ssh_urls() {
        assert!(!is_ssh_url("https://github.com/owner/repo.git"));
        assert!(!is_ssh_url("http://gitlab.example.com/owner/repo.git"));
        assert!(!is_ssh_url("https://user:tok@github.com/owner/repo.git"));
    }

    #[test]
    fn an_incomplete_path_is_rejected() {
        assert_eq!(
            parse(RepositoryProvider::Gitlab, "https://gitlab.com/acme"),
            None
        );
        assert_eq!(parse(RepositoryProvider::Gitness, "not-a-url"), None);
    }
}
