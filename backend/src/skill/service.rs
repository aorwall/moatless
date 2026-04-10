//! Orchestrator for discovering skills in the workspace repository and its
//! linked plugin repositories.
//!
//! Caches the parsed skill lists in memory with a TTL. Cache keys:
//!   - `workspace:{repository_id}` — skills from the workspace repo's clone
//!   - `plugin:{plugin_repo_id}` — skills from a plugin repo clone (shared
//!     across all workspace repos that reference it)

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tracing::debug;

use super::error::SkillError;
use super::git_cache::GitFetcher;
use super::models::{Skill, SkillSource};
use super::scanner;
use super::schemas::SkillResponse;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::extractors::AuthUser;
use crate::plugin_repo::dao::PluginRepoRepository;
use crate::plugin_repo::models::PluginRepo;
use crate::repository::dao::RepoRepository;
use crate::repository::models::{Repository, RepositoryKind, RepositoryProvider};

/// Orchestrates skill discovery across workspace + plugin repositories.
pub struct SkillService {
    repositories: Arc<dyn RepoRepository>,
    plugin_repos: Arc<dyn PluginRepoRepository>,
    provider_tokens: Arc<dyn ProviderTokenRepository>,
    fetcher: Arc<dyn GitFetcher>,
    cache: Cache<String, Arc<Vec<Skill>>>,
}

impl SkillService {
    pub fn new(
        repositories: Arc<dyn RepoRepository>,
        plugin_repos: Arc<dyn PluginRepoRepository>,
        provider_tokens: Arc<dyn ProviderTokenRepository>,
        fetcher: Arc<dyn GitFetcher>,
        ttl: Duration,
    ) -> Self {
        let cache = Cache::builder().time_to_live(ttl).max_capacity(1024).build();
        Self {
            repositories,
            plugin_repos,
            provider_tokens,
            fetcher,
            cache,
        }
    }

    /// Return the merged, deduplicated skill list for a repository.
    ///
    /// Workspace skills shadow plugin skills with the same name; among
    /// plugins, the first occurrence in DAO order wins.
    pub async fn list_for_repository(
        &self,
        repository_id: &str,
        auth: &AuthUser,
    ) -> Result<Vec<SkillResponse>, SkillError> {
        let ctx = auth.scope_context();

        let repo = self
            .repositories
            .find_by_id(repository_id, &ctx)
            .await?
            .ok_or_else(|| SkillError::RepositoryNotFound(repository_id.to_string()))?;

        let mut merged: Vec<Skill> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // ---- Workspace skills ----
        let workspace = self.workspace_skills(&repo, auth).await?;
        for skill in workspace.iter().cloned() {
            if seen.insert(skill.name.clone()) {
                merged.push(skill);
            }
        }

        // ---- Plugin skills (stable order from DAO) ----
        let plugins = self
            .plugin_repos
            .find_by_repository_id(repository_id)
            .await?;
        for plugin in plugins {
            let plugin_skills = self.plugin_skills(&plugin, auth).await?;
            for skill in plugin_skills.iter().cloned() {
                if seen.insert(skill.name.clone()) {
                    merged.push(skill);
                }
            }
        }

        Ok(merged.into_iter().map(SkillResponse::from).collect())
    }

    async fn workspace_skills(
        &self,
        repo: &Repository,
        auth: &AuthUser,
    ) -> Result<Arc<Vec<Skill>>, SkillError> {
        // Templates don't have a remote URL we can clone — skip silently.
        if repo.kind == RepositoryKind::Template {
            return Ok(Arc::new(Vec::new()));
        }
        let Some(remote_url) = repo.remote_url.as_deref() else {
            return Ok(Arc::new(Vec::new()));
        };

        let cache_key = format!("workspace:{}", repo.id);
        if let Some(cached) = self.cache.get(&cache_key).await {
            debug!(repository_id = %repo.id, "Skill cache hit (workspace)");
            return Ok(cached);
        }

        let token = self
            .resolve_token_for_provider(auth, repo.provider.as_ref())
            .await?;

        let clone_path = self
            .fetcher
            .fetch_or_update(remote_url, token.as_deref())
            .await?;

        let skills = scanner::scan_dir(&clone_path, SkillSource::Workspace, None, None).await?;
        let skills = Arc::new(skills);
        self.cache.insert(cache_key, skills.clone()).await;
        Ok(skills)
    }

    async fn plugin_skills(
        &self,
        plugin: &PluginRepo,
        auth: &AuthUser,
    ) -> Result<Arc<Vec<Skill>>, SkillError> {
        let cache_key = format!("plugin:{}", plugin.id);
        if let Some(cached) = self.cache.get(&cache_key).await {
            debug!(plugin_repo_id = %plugin.id, "Skill cache hit (plugin)");
            return Ok(cached);
        }

        let provider = derive_provider_from_url(&plugin.git_url);
        let token = self.resolve_token_for_provider(auth, provider.as_ref()).await?;

        let clone_path = self
            .fetcher
            .fetch_or_update(&plugin.git_url, token.as_deref())
            .await?;

        let skills = scanner::scan_dir(
            &clone_path,
            SkillSource::Plugin,
            Some(&plugin.id),
            Some(&plugin.name),
        )
        .await?;
        let skills = Arc::new(skills);
        self.cache.insert(cache_key, skills.clone()).await;
        Ok(skills)
    }

    async fn resolve_token_for_provider(
        &self,
        auth: &AuthUser,
        provider: Option<&RepositoryProvider>,
    ) -> Result<Option<String>, SkillError> {
        let Some(provider) = provider else {
            return Ok(None);
        };
        let provider_str = provider_to_token_key(*provider);
        let token = self
            .provider_tokens
            .find_provider_token(&auth.user_id, provider_str)
            .await?;
        Ok(token.map(|t| t.access_token))
    }
}

/// Map a `RepositoryProvider` to the string used as the `provider` column in
/// the `provider_tokens` table. The variants already serialise as lowercase
/// so this is effectively a static-dispatch slug.
fn provider_to_token_key(provider: RepositoryProvider) -> &'static str {
    match provider {
        RepositoryProvider::Github => "github",
        RepositoryProvider::Gitlab => "gitlab",
        RepositoryProvider::Gitness => "gitness",
    }
}

/// Derive a `RepositoryProvider` from a git URL's host. Returns `None` for
/// hosts we don't recognise — the service will attempt an unauthenticated
/// clone for those.
fn derive_provider_from_url(url: &str) -> Option<RepositoryProvider> {
    let lower = url.to_ascii_lowercase();
    if lower.contains("github.com") {
        Some(RepositoryProvider::Github)
    } else if lower.contains("gitlab.com") {
        Some(RepositoryProvider::Gitlab)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_provider_github() {
        assert_eq!(
            derive_provider_from_url("https://github.com/foo/bar.git"),
            Some(RepositoryProvider::Github)
        );
        assert_eq!(
            derive_provider_from_url("git@github.com:foo/bar.git"),
            Some(RepositoryProvider::Github)
        );
    }

    #[test]
    fn derive_provider_gitlab() {
        assert_eq!(
            derive_provider_from_url("https://gitlab.com/foo/bar.git"),
            Some(RepositoryProvider::Gitlab)
        );
    }

    #[test]
    fn derive_provider_unknown_returns_none() {
        assert_eq!(
            derive_provider_from_url("https://self-hosted.example.com/foo/bar.git"),
            None
        );
    }

    #[test]
    fn provider_to_token_key_matches_column_values() {
        assert_eq!(provider_to_token_key(RepositoryProvider::Github), "github");
        assert_eq!(provider_to_token_key(RepositoryProvider::Gitlab), "gitlab");
        assert_eq!(provider_to_token_key(RepositoryProvider::Gitness), "gitness");
    }
}
