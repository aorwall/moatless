//! Fixtures for the policy's tests.

use std::sync::Arc;

use moatless_core::proxy_rules::ProxyPolicyMode;
use moatless_core::workspace_identity_grants::WorkspaceIdentityGrant;
use moatless_workspace_ports::WorkspaceResolver;

use super::policy::GitHubPolicy;
use crate::proxy::grants::GrantReader;

struct FixedGrant(Option<WorkspaceIdentityGrant>);

#[async_trait::async_trait]
impl GrantReader for FixedGrant {
    async fn grant_for(&self, _: &str) -> Result<Option<WorkspaceIdentityGrant>, String> {
        Ok(self.0.clone())
    }
}

/// A policy over `workspaces`' placements, with `grant` for every Workspace.
pub(crate) fn policy(
    mode: ProxyPolicyMode,
    workspaces: Arc<dyn WorkspaceResolver>,
    grant: Option<WorkspaceIdentityGrant>,
) -> Arc<GitHubPolicy> {
    Arc::new(GitHubPolicy::new(
        mode,
        workspaces,
        Arc::new(FixedGrant(grant)),
    ))
}

use moatless_core::workspace_identity_grants::{GitHubCapability, GitHubPolicyGrant};
use moatless_workspace_types::repository::{
    GitHostRepo, Repository, RepositoryKind, RepositoryProvider, RepositoryRemote,
};

use super::scope::PolicyScope;

/// A github.com repository row with `default_branch`.
pub(crate) fn github_repository(slug: &str, default_branch: Option<&str>) -> Repository {
    let (owner, name) = slug.split_once('/').unwrap_or((slug, ""));
    Repository {
        id: slug.to_string(),
        name: name.to_string(),
        kind: RepositoryKind::Git,
        remote: Some(RepositoryRemote {
            url: format!("https://github.com/{slug}.git"),
            host_repo: Some(GitHostRepo::github(owner, name)),
        }),
        default_branch: default_branch.map(str::to_string),
        description: None,
        provider: Some(RepositoryProvider::Github),
        scope: moatless_types::access_control::Scope::Global,
        owner_user_id: None,
        created_by: None,
        deleted: false,
        created_at: None,
        updated_at: None,
    }
}

/// `soaplabs/moatless` placed with default branch `develop`, plus
/// `capabilities` and `soaplabs/extra` as an extra repository.
pub(crate) fn scope(capabilities: &[GitHubCapability]) -> PolicyScope {
    let placed = github_repository("soaplabs/moatless", Some("develop"));
    PolicyScope::new(
        [&placed],
        Some(&GitHubPolicyGrant {
            capabilities: capabilities.to_vec(),
            extra_repos: vec!["soaplabs/extra".to_string()],
        }),
    )
}
