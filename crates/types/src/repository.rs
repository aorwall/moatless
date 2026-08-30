//! Repository: a connection to a git repo, and the host it lives on.
//!
//! Run configuration belongs to the Workspace, not here. A Task references a
//! Workspace, never a Repository directly.

use chrono::{DateTime, Utc};

use crate::access_control::Scope;

crate::string_enum!(
    /// Kind of repository source.
    RepositoryKind {
        Git => "git",
        Remote => "remote",
        Template => "template",
    }
);

crate::string_enum!(
    /// Git hosting provider.
    RepositoryProvider {
        Github => "github",
        Gitlab => "gitlab",
        Gitness => "gitness",
    }
);

/// A connection to a git repo — its name, remote URL, default branch, and host.
#[derive(Debug, Clone)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
    /// The git host this repository lives on. `None` when it was never set or
    /// the persisted value is one this build does not know.
    pub provider: Option<RepositoryProvider>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub deleted: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A repository named on a git host: which host, whose account, and what it is
/// called.
///
/// Parsed once from a remote URL so no call site re-derives an owner and a name
/// from a string. The provider travels with them, so a consumer that resolves
/// access does not have to ask which host the repository lives on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHostRepo {
    provider: RepositoryProvider,
    owner: String,
    name: String,
    host: Option<String>,
}

impl GitHostRepo {
    /// Parse a supported remote URL for `provider`.
    ///
    /// Both HTTPS/HTTP and the common SCP-style SSH form are accepted. GitHub
    /// has exactly one host, so a URL on any other host is rejected and `host`
    /// stays `None`. GitLab and Gitness are self-hostable, so any host is
    /// accepted and carried.
    pub fn parse(provider: RepositoryProvider, remote_url: &str) -> Option<Self> {
        match provider {
            RepositoryProvider::Github => Self::parse_github(remote_url),
            RepositoryProvider::Gitlab | RepositoryProvider::Gitness => {
                Self::parse_hosted(provider, remote_url)
            }
        }
    }

    /// A GitHub repository whose owner and name a caller already holds — a
    /// webhook subject, say, where there is no remote URL to parse.
    pub fn github(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            provider: RepositoryProvider::Github,
            owner: owner.into(),
            name: name.into(),
            host: None,
        }
    }

    pub fn provider(&self) -> RepositoryProvider {
        self.provider
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The host, for a provider that does not have exactly one.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Extra path segments, an empty owner and any host but `github.com` are
    /// rejected.
    fn parse_github(remote_url: &str) -> Option<Self> {
        let trimmed = remote_url.trim().trim_end_matches('/');
        let path = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
            path.to_string()
        } else {
            let parsed = url::Url::parse(trimmed).ok()?;
            if !matches!(parsed.scheme(), "http" | "https")
                || !parsed.host_str()?.eq_ignore_ascii_case("github.com")
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
        Some(Self {
            provider: RepositoryProvider::Github,
            owner: owner.to_string(),
            name: name.to_string(),
            host: None,
        })
    }

    /// A self-hostable provider: any host, and a namespace that may nest, so
    /// the last path segment is the repository and everything before it is the
    /// owner.
    fn parse_hosted(provider: RepositoryProvider, remote_url: &str) -> Option<Self> {
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
        Some(Self {
            provider,
            owner: owner.to_string(),
            name: name.to_string(),
            host: Some(host),
        })
    }
}

impl std::fmt::Display for GitHostRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(provider: RepositoryProvider, url: &str) -> Option<(String, String)> {
        GitHostRepo::parse(provider, url).map(|r| (r.owner, r.name))
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
    fn github_carries_no_host_because_it_has_only_one() {
        let repo = GitHostRepo::parse(RepositoryProvider::Github, "https://github.com/acme/repo")
            .expect("parses");
        assert_eq!(repo.host(), None);
        assert_eq!(repo.provider(), RepositoryProvider::Github);
        assert_eq!(repo.to_string(), "acme/repo");
    }

    #[test]
    fn a_self_hosted_provider_keeps_its_own_host() {
        let repo = GitHostRepo::parse(
            RepositoryProvider::Gitlab,
            "https://gitlab.example.com/acme/repo.git",
        )
        .expect("parses");
        assert_eq!(repo.host(), Some("gitlab.example.com"));
        assert_eq!(repo.owner(), "acme");
        assert_eq!(repo.name(), "repo");

        let ssh = GitHostRepo::parse(
            RepositoryProvider::Gitness,
            "git@gitness.internal:team/repo",
        )
        .expect("parses");
        assert_eq!(ssh.host(), Some("gitness.internal"));
        assert_eq!(ssh.owner(), "team");
    }

    #[test]
    fn a_nested_namespace_keeps_every_segment_but_the_last_as_the_owner() {
        let repo = GitHostRepo::parse(
            RepositoryProvider::Gitlab,
            "https://gitlab.com/group/subgroup/repo.git",
        )
        .expect("parses");
        assert_eq!(repo.owner(), "group/subgroup");
        assert_eq!(repo.name(), "repo");
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
