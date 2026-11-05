//! The repositories a Workspace writes to, and the capabilities its grant
//! adds.

use std::collections::{BTreeSet, HashMap, HashSet};

use moatless_core::workspace_identity_grants::{GitHubCapability, GitHubPolicyGrant};
use moatless_workspace_types::repository::{GITHUB_HOST, Repository};

/// Branches counted as a repository's default whatever its registered one,
/// since no REST lookup confirms the registered value is current.
const ALWAYS_DEFAULT: [&str; 2] = ["main", "master"];

#[derive(Debug, Default)]
pub(crate) struct PolicyScope {
    /// `host/owner/name`, lowercased, to the branches counted as its default.
    repos: HashMap<String, BTreeSet<String>>,
    capabilities: HashSet<GitHubCapability>,
}

/// The key [`PolicyScope::repos`] holds `owner/name` on `host` under.
pub(crate) fn repo_key(host: &str, owner: &str, name: &str) -> String {
    format!("{host}/{owner}/{name}").to_ascii_lowercase()
}

impl PolicyScope {
    /// `placed` are the Workspace's repositories; `grant` its `githubPolicy`.
    pub(crate) fn new<'a>(
        placed: impl IntoIterator<Item = &'a Repository>,
        grant: Option<&GitHubPolicyGrant>,
    ) -> Self {
        let mut scope = Self::default();
        for repository in placed {
            let Some(repo) = repository
                .remote
                .as_ref()
                .and_then(|r| r.host_repo.as_ref())
            else {
                continue;
            };
            let defaults = scope
                .repos
                .entry(repo_key(repo.host(), repo.owner(), repo.name()))
                .or_default();
            defaults.extend(repository.default_branch.iter().cloned());
        }
        if let Some(grant) = grant {
            for slug in &grant.extra_repos {
                if let Some((owner, name)) = slug.split_once('/') {
                    scope
                        .repos
                        .entry(repo_key(GITHUB_HOST, owner, name))
                        .or_default();
                }
            }
            scope
                .capabilities
                .extend(grant.capabilities.iter().copied());
        }
        scope
    }

    /// Whether `key` ([`repo_key`]) may be written to.
    pub(crate) fn allows(&self, key: &str) -> bool {
        self.repos.contains_key(&key.to_ascii_lowercase())
    }

    pub(crate) fn has(&self, capability: GitHubCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Whether `branch` counts as the default branch of `key`.
    pub(crate) fn is_default_branch(&self, key: &str, branch: &str) -> bool {
        ALWAYS_DEFAULT.contains(&branch)
            || self
                .repos
                .get(&key.to_ascii_lowercase())
                .is_some_and(|defaults| defaults.contains(branch))
    }
}

#[cfg(test)]
mod tests {
    use crate::github_policy::test_support::scope;

    #[test]
    fn a_placed_or_extra_repository_is_writable_and_others_are_not() {
        let scope = scope(&[]);
        assert!(scope.allows("github.com/soaplabs/moatless"));
        assert!(scope.allows("github.com/SoapLabs/Moatless"));
        assert!(scope.allows("github.com/soaplabs/extra"));
        assert!(!scope.allows("github.com/soaplabs/other"));
        assert!(!scope.allows("gitlab.com/soaplabs/moatless"));
    }

    #[test]
    fn main_master_and_the_registered_default_count_as_default() {
        let scope = scope(&[]);
        let key = "github.com/soaplabs/moatless";
        assert!(scope.is_default_branch(key, "develop"));
        assert!(scope.is_default_branch(key, "main"));
        assert!(scope.is_default_branch("github.com/soaplabs/extra", "master"));
        assert!(!scope.is_default_branch("github.com/soaplabs/extra", "develop"));
        assert!(!scope.is_default_branch(key, "feature/x"));
    }
}
