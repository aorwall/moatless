//! Orchestrator for discovering skills in the workspace repository and its
//! linked plugins.
//!
//! Workspace skills are read from git on request and cached in memory with a
//! TTL, keyed `workspace:{repository_id}` — they belong to a repository the
//! caller already has access to, and there is nothing durable to attach them
//! to. Plugin skills are stored records; see [`crate::plugin::sync`].

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tracing::debug;

use super::error::SkillError;
use super::fetcher::{SkillFetcher, SkillLayout};
use super::models::{Skill, SkillSource};
use super::scanner;
use super::schemas::SkillResponse;
use crate::auth::credentials::resolve_user_github_token;
use crate::auth::error::AuthError;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::extractors::AuthUser;
use crate::plugin::dao::PluginRepository;
use crate::plugin::models::Plugin;
use crate::plugin::sync::PluginSyncService;
use crate::repository::dao::RepoRepository;
use crate::repository::models::{Repository, RepositoryKind, RepositoryProvider};
use crate::secret::service::SecretService;

/// Orchestrates skill discovery across workspace + plugins.
pub struct SkillService {
    repositories: Arc<dyn RepoRepository>,
    plugins: Arc<dyn PluginRepository>,
    provider_tokens: Arc<dyn ProviderTokenRepository>,
    provider_token_auth: Arc<dyn ProviderTokenAuthRepository>,
    github_app_service: Option<Arc<GitHubAppService>>,
    github_apps: Arc<GitHubAppRegistry>,
    secrets: Arc<SecretService>,
    fetcher: Arc<dyn SkillFetcher>,
    plugin_sync: Arc<PluginSyncService>,
    cache: Cache<String, Arc<Vec<Skill>>>,
}

impl SkillService {
    #[allow(clippy::too_many_arguments)]
    // reason: this service fans out to four DAOs plus the GitHub App service,
    // and the constructor is called once from AppState::new.
    pub fn new(
        repositories: Arc<dyn RepoRepository>,
        plugins: Arc<dyn PluginRepository>,
        provider_tokens: Arc<dyn ProviderTokenRepository>,
        provider_token_auth: Arc<dyn ProviderTokenAuthRepository>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
        secrets: Arc<SecretService>,
        fetcher: Arc<dyn SkillFetcher>,
        plugin_sync: Arc<PluginSyncService>,
        ttl: Duration,
    ) -> Self {
        let cache = Cache::builder()
            .time_to_live(ttl)
            .max_capacity(1024)
            .build();
        Self {
            repositories,
            plugins,
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            secrets,
            fetcher,
            plugin_sync,
            cache,
        }
    }

    /// Return the merged, deduplicated skill list a caller gets in a
    /// repository: the repository's own skills plus the plugin skills their
    /// activation records resolve to.
    ///
    /// Workspace skills shadow plugin skills with the same name; among
    /// plugins, the first occurrence in DAO order wins.
    ///
    /// The plugin half is per-caller, not per-repository — two people looking
    /// at the same repository can legitimately see different skills, because
    /// that is what will arrive in their respective sandboxes.
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

        // ---- Plugin skills (whatever this caller's activation records give
        // them, in stable order from the DAO) ----
        let resolved =
            crate::plugin::activation::resolve_for_user(self.plugins.as_ref(), Some(auth.user_id))
                .await?;
        for (plugin, picked) in resolved {
            for skill in self.plugin_skills(&plugin).await? {
                if !picked.contains(&skill.name) {
                    continue;
                }
                if seen.insert(skill.name.clone()) {
                    merged.push(skill);
                }
            }
        }

        merged.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(merged.into_iter().map(SkillResponse::from).collect())
    }

    async fn workspace_skills(
        &self,
        repo: &Repository,
        auth: &AuthUser,
    ) -> Result<Arc<Vec<Skill>>, SkillError> {
        // Templates don't have a remote URL we can fetch — skip silently.
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

        let token = if repo.provider == Some(RepositoryProvider::Github) {
            crate::repository::access::resolve_repository_github_token_with(
                self.provider_tokens.as_ref(),
                Some(self.provider_token_auth.as_ref()),
                self.github_app_service.as_deref(),
                self.github_apps.as_ref(),
                Some(self.secrets.as_ref()),
                repo,
                remote_url,
            )
            .await
            .map_err(SkillError::RepositoryToken)?
        } else {
            self.resolve_token_for_provider(auth, repo.provider.as_ref())
                .await?
        };

        let files = self
            .fetcher
            .fetch_skill_files(remote_url, SkillLayout::Workspace, token.as_deref())
            .await?;

        let mut skills: Vec<Skill> = files
            .into_iter()
            .filter_map(|f| {
                scanner::parse_skill(&f.content, &f.dir_name, SkillSource::Workspace, None, None)
            })
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));

        let skills = Arc::new(skills);
        self.cache.insert(cache_key, skills.clone()).await;
        Ok(skills)
    }

    /// Plugin skills come from stored records, not from git on request. The
    /// sync service decides whether the source has moved on and is the only
    /// thing that fetches.
    async fn plugin_skills(&self, plugin: &Plugin) -> Result<Vec<Skill>, SkillError> {
        let stored = self.plugin_sync.skills(plugin).await?;
        Ok(stored
            .into_iter()
            .map(|s| Skill {
                name: s.name,
                description: s.description,
                source: SkillSource::Plugin,
                plugin_id: Some(plugin.id.clone()),
                plugin_name: Some(plugin.name.clone()),
            })
            .collect())
    }

    async fn resolve_token_for_provider(
        &self,
        auth: &AuthUser,
        provider: Option<&RepositoryProvider>,
    ) -> Result<Option<String>, SkillError> {
        let Some(provider) = provider else {
            return Ok(None);
        };

        if *provider == RepositoryProvider::Github {
            return match resolve_user_github_token(
                self.provider_tokens.as_ref(),
                Some(self.provider_token_auth.as_ref()),
                self.github_app_service.as_deref(),
                self.github_apps.as_ref(),
                Some(self.secrets.as_ref()),
                &auth.user_id.to_string(),
                None,
            )
            .await
            {
                Ok(token) => Ok(token),
                // No token configured — fall through to unauthenticated fetch.
                Err(AuthError::NotFound(_)) => Ok(None),
                Err(AuthError::Database(e)) => Err(SkillError::Db(e)),
                Err(e) => Err(SkillError::UserToken(e)),
            };
        }

        let provider_str = provider_to_token_key(*provider);
        let token = self
            .provider_tokens
            .find_provider_token(&auth.user_id.to_string(), provider_str)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_to_token_key_matches_column_values() {
        assert_eq!(provider_to_token_key(RepositoryProvider::Github), "github");
        assert_eq!(provider_to_token_key(RepositoryProvider::Gitlab), "gitlab");
        assert_eq!(
            provider_to_token_key(RepositoryProvider::Gitness),
            "gitness"
        );
    }
}
