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
        host_repo: provider.and_then(|provider| parse_host_repo(provider, &url)),
        url,
    }
}

/// Parse a supported remote for its declared git host.
fn parse_host_repo(provider: RepositoryProvider, remote_url: &str) -> Option<GitHostRepo> {
    match provider {
        RepositoryProvider::Github => parse_github(remote_url),
        RepositoryProvider::Gitlab | RepositoryProvider::Forgejo => {
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
/// Reject a `remote_url` that is not `https://` or `http://`.
///
/// SSH is refused because no sandbox holds an SSH key: a clone authenticates
/// over HTTPS only. `ext::` is a remote helper git runs rather than clones.
/// Write-path only; migration 0213 converted the SSH rows stored before this.
pub(crate) fn ensure_remote_url_scheme_allowed(
    remote_url: &str,
) -> Result<(), super::error::RepositoryError> {
    if let Ok(parsed) = url::Url::parse(remote_url.trim()) {
        if matches!(parsed.scheme(), "http" | "https") {
            return Ok(());
        }
    }
    Err(super::error::RepositoryError::BadRequest(format!(
        "Remote URL '{remote_url}' is not allowed: only https and http remotes are \
         accepted; use the repository's HTTPS clone URL instead of SSH"
    )))
}

/// Reject a remote another provider addresses and the declared one does not.
///
/// A URL no provider reads as `owner/repo` resolves to no host however it is
/// declared, and is accepted: `sandbox::init::git` clones it anonymously.
pub(crate) fn ensure_no_provider_mismatch(
    provider: Option<RepositoryProvider>,
    url: &str,
) -> Result<(), super::error::RepositoryError> {
    let Some(provider) = provider else {
        return Ok(());
    };
    if parse_host_repo(provider, url).is_some() {
        return Ok(());
    }

    let parses_under: Vec<&str> = RepositoryProvider::ALL
        .iter()
        .copied()
        .filter(|candidate| *candidate != provider && parse_host_repo(*candidate, url).is_some())
        .map(RepositoryProvider::as_str)
        .collect();
    if parses_under.is_empty() {
        return Ok(());
    }
    Err(super::error::RepositoryError::BadRequest(format!(
        "Provider {provider} does not host {url}: it reads as owner/repo under {}",
        parses_under.join(", ")
    )))
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
        assert_eq!(
            resolved.host_repo,
            Some(GitHostRepo::github("Acme", "Repo"))
        );
        assert!(resolve_remote(None, url).host_repo.is_none());
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
    fn a_host_another_provider_serves_is_rejected_under_the_declared_one() {
        let message = match ensure_no_provider_mismatch(
            Some(RepositoryProvider::Github),
            "https://git.example.com/config-dev/config-default",
        ) {
            Err(super::super::error::RepositoryError::BadRequest(message)) => message,
            other => panic!("expected a BadRequest, got {other:?}"),
        };
        assert!(
            message.contains("Provider github does not host"),
            "{message}"
        );
        assert!(message.contains("gitlab, forgejo"), "{message}");
    }

    #[test]
    fn a_remote_no_provider_would_address_differently_is_accepted() {
        // A host the provider serves, no declared provider, and the two shapes
        // every parser rejects — a single path segment and a non-HTTP scheme.
        // The last three clone anonymously, so rejecting them would strand a
        // remote the CLI has no provider value to re-declare.
        for (provider, url) in [
            (
                Some(RepositoryProvider::Forgejo),
                "https://git.example.com/config-dev/config-default",
            ),
            (None, "https://git.example.com/acme/repo"),
            (
                Some(RepositoryProvider::Github),
                "https://git.example.com/repo.git",
            ),
            (
                Some(RepositoryProvider::Github),
                "git://git.example.com/acme/repo",
            ),
        ] {
            assert!(
                ensure_no_provider_mismatch(provider, url).is_ok(),
                "{provider:?} {url}"
            );
        }
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
            RepositoryProvider::Forgejo,
            "git@forgejo.internal:team/repo",
        )
        .expect("parses");
        assert_eq!(ssh.host(), "forgejo.internal");
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
    fn remote_url_scheme_accepts_https_and_http() {
        for url in [
            "https://github.com/acme/repo.git",
            "http://git.example.com/acme/repo",
        ] {
            assert!(
                ensure_remote_url_scheme_allowed(url).is_ok(),
                "expected {url} to be accepted"
            );
        }
    }

    #[test]
    fn remote_url_scheme_rejects_ssh_remotes() {
        for url in [
            "git@github.com:acme/repo.git",
            "ssh://deploy@git.internal:2222/team/repo.git",
            "git+ssh://git@github.com/acme/repo.git",
            "deploy@10.0.0.1:/srv/git/repo.git",
        ] {
            let err = ensure_remote_url_scheme_allowed(url).unwrap_err();
            assert!(
                matches!(err, super::super::error::RepositoryError::BadRequest(_)),
                "{url}"
            );
        }
    }

    #[test]
    fn remote_url_scheme_rejects_a_remote_helper_invocation() {
        let err = ensure_remote_url_scheme_allowed("ext::sh -c 'id'").unwrap_err();
        assert!(matches!(
            err,
            super::super::error::RepositoryError::BadRequest(_)
        ));
    }

    #[test]
    fn remote_url_scheme_rejects_file_and_other_unapproved_schemes() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/repo.git",
            "git://example.com/acme/repo",
        ] {
            let err = ensure_remote_url_scheme_allowed(url).unwrap_err();
            assert!(
                matches!(err, super::super::error::RepositoryError::BadRequest(_)),
                "{url}"
            );
        }
    }

    #[test]
    fn remote_url_scheme_rejects_a_local_path() {
        for url in ["/srv/repos/a@b:c", "./x@y:z"] {
            let err = ensure_remote_url_scheme_allowed(url).unwrap_err();
            assert!(
                matches!(err, super::super::error::RepositoryError::BadRequest(_)),
                "{url}"
            );
        }
    }

    #[test]
    fn remote_url_scheme_rejects_a_scheme_less_non_scp_string() {
        let err = ensure_remote_url_scheme_allowed("not-a-url-or-scp-path").unwrap_err();
        assert!(matches!(
            err,
            super::super::error::RepositoryError::BadRequest(_)
        ));
    }

    #[test]
    fn an_incomplete_path_is_rejected() {
        assert_eq!(
            parse(RepositoryProvider::Gitlab, "https://gitlab.com/acme"),
            None
        );
        assert_eq!(parse(RepositoryProvider::Forgejo, "not-a-url"), None);
    }
}
