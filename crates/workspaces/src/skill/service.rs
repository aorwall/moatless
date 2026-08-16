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
use crate::plugin::dao::PluginDao;
use crate::plugin::models::Plugin;
use crate::plugin::sync::PluginSyncService;
use crate::repository::dao::RepositoryDao;
use crate::repository::models::{Repository, RepositoryKind};
use moatless_identity::auth::actor::Actor;
use moatless_identity::extractors::AuthUser;
use moatless_identity::git_host_repo::GitHostRepo;
use moatless_identity::git_host_tokens::{GitHostTokenError, GitHostTokens};

/// Orchestrates skill discovery across workspace + plugins.
pub struct SkillService {
    repositories: Arc<dyn RepositoryDao>,
    plugins: Arc<dyn PluginDao>,
    git_host_tokens: Arc<GitHostTokens>,
    fetcher: Arc<dyn SkillFetcher>,
    plugin_sync: Arc<PluginSyncService>,
    cache: Cache<String, Arc<Vec<Skill>>>,
}

impl SkillService {
    pub fn new(
        repositories: Arc<dyn RepositoryDao>,
        plugins: Arc<dyn PluginDao>,
        git_host_tokens: Arc<GitHostTokens>,
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
            git_host_tokens,
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
        let actor = Actor::caller(&ctx);

        let repo = self
            .repositories
            .find_by_id(repository_id, &ctx)
            .await?
            .ok_or_else(|| SkillError::RepositoryNotFound(repository_id.to_string()))?;

        let workspace = self.workspace_skills(&repo, auth).await?;

        // Whatever this caller's activation records give them, in stable order
        // from the DAO.
        let resolved =
            crate::plugin::activation::resolve_for_user(self.plugins.as_ref(), Some(auth.user_id))
                .await?;
        let mut from_plugins: Vec<(Vec<Skill>, Vec<String>)> = Vec::with_capacity(resolved.len());
        for (plugin, picked) in resolved {
            from_plugins.push((self.plugin_skills(&actor, &plugin).await?, picked));
        }

        let merged = merge_skills(workspace.as_ref().clone(), from_plugins);
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

        // Keyed by caller as well as repository: the fetch runs with their own
        // credential, so two people can legitimately see different skills in
        // the same repository.
        let actor = Actor::caller(&auth.scope_context());
        let cache_key = format!("workspace:{}:{actor}", repo.id);
        if let Some(cached) = self.cache.get(&cache_key).await {
            debug!(repository_id = %repo.id, "Skill cache hit (workspace)");
            return Ok(cached);
        }

        let token = self.workspace_token(&actor, repo, remote_url).await?;

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
    async fn plugin_skills(
        &self,
        actor: &Actor,
        plugin: &Plugin,
    ) -> Result<Vec<Skill>, SkillError> {
        let stored = self.plugin_sync.skills(actor, plugin).await?;
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

    /// The caller's own token for a workspace repository, on whichever host it
    /// lives on.
    ///
    /// Not connecting the host is not an error — a public repository still
    /// fetches unauthenticated. A credential that exists and cannot be used is.
    async fn workspace_token(
        &self,
        actor: &Actor,
        repo: &Repository,
        remote_url: &str,
    ) -> Result<Option<String>, SkillError> {
        let Some(provider) = repo.provider else {
            return Ok(None);
        };
        let Some(host_repo) = GitHostRepo::parse(provider.into(), remote_url) else {
            return Ok(None);
        };
        match self.git_host_tokens.token(actor, &host_repo).await {
            Ok(token) => Ok(Some(token)),
            Err(GitHostTokenError::NoCredential { .. }) => Ok(None),
            Err(GitHostTokenError::Database(e)) => Err(SkillError::Db(e)),
            Err(e) => Err(SkillError::RepositoryToken(
                crate::repository::access::git_host_token_error(e),
            )),
        }
    }
}

/// Merge the workspace's own skills with the plugin skills a caller picked,
/// name-deduplicated and sorted.
///
/// `plugins` is one entry per plugin, in DAO order: everything the plugin
/// stores, paired with the names that plugin's activation records resolved to.
/// A stored skill the caller did not pick is dropped here — this is the same
/// filter the sandbox applies when it prunes the cloned plugin directory, so
/// what this endpoint lists is what the agent will actually have.
///
/// Workspace skills shadow plugin skills with the same name; among plugins the
/// first occurrence wins, which makes DAO order the tie-break.
fn merge_skills(workspace: Vec<Skill>, plugins: Vec<(Vec<Skill>, Vec<String>)>) -> Vec<Skill> {
    let mut merged: Vec<Skill> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for skill in workspace {
        if seen.insert(skill.name.clone()) {
            merged.push(skill);
        }
    }

    for (stored, picked) in plugins {
        for skill in stored {
            if !picked.contains(&skill.name) {
                continue;
            }
            if seen.insert(skill.name.clone()) {
                merged.push(skill);
            }
        }
    }

    merged.sort_by(|a, b| a.name.cmp(&b.name));
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_skill(name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("{name} description"),
            source: SkillSource::Workspace,
            plugin_id: None,
            plugin_name: None,
        }
    }

    fn plugin_skill(plugin: &str, name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("{name} description"),
            source: SkillSource::Plugin,
            plugin_id: Some(format!("{plugin}-id")),
            plugin_name: Some(plugin.to_string()),
        }
    }

    fn names(skills: &[Skill]) -> Vec<&str> {
        skills.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn picked_plugin_skills_are_listed() {
        let merged = merge_skills(
            Vec::new(),
            vec![(vec![plugin_skill("docs", "spec")], vec!["spec".to_string()])],
        );

        assert_eq!(names(&merged), vec!["spec"]);
        assert_eq!(merged[0].source, SkillSource::Plugin);
        assert_eq!(merged[0].plugin_name.as_deref(), Some("docs"));
    }

    #[test]
    fn unpicked_sibling_skills_are_dropped() {
        let merged = merge_skills(
            Vec::new(),
            vec![(
                vec![
                    plugin_skill("docs", "spec"),
                    plugin_skill("docs", "reviewing-specs"),
                ],
                vec!["spec".to_string()],
            )],
        );

        assert_eq!(names(&merged), vec!["spec"]);
    }

    #[test]
    fn a_plugin_with_no_records_contributes_nothing() {
        let merged = merge_skills(
            vec![workspace_skill("local")],
            vec![(vec![plugin_skill("docs", "spec")], Vec::new())],
        );

        assert_eq!(names(&merged), vec!["local"]);
    }

    #[test]
    fn workspace_shadows_a_plugin_skill_of_the_same_name() {
        let merged = merge_skills(
            vec![workspace_skill("spec")],
            vec![(vec![plugin_skill("docs", "spec")], vec!["spec".to_string()])],
        );

        assert_eq!(names(&merged), vec!["spec"]);
        assert_eq!(merged[0].source, SkillSource::Workspace);
    }

    #[test]
    fn first_plugin_in_dao_order_wins_a_name_clash() {
        let merged = merge_skills(
            Vec::new(),
            vec![
                (
                    vec![plugin_skill("first", "shared")],
                    vec!["shared".to_string()],
                ),
                (
                    vec![plugin_skill("second", "shared")],
                    vec!["shared".to_string()],
                ),
            ],
        );

        assert_eq!(names(&merged), vec!["shared"]);
        assert_eq!(merged[0].plugin_name.as_deref(), Some("first"));
    }

    #[test]
    fn the_merged_list_is_sorted_by_name() {
        let merged = merge_skills(
            vec![workspace_skill("zebra"), workspace_skill("alpha")],
            vec![(
                vec![plugin_skill("docs", "middle")],
                vec!["middle".to_string()],
            )],
        );

        assert_eq!(names(&merged), vec!["alpha", "middle", "zebra"]);
    }
}
