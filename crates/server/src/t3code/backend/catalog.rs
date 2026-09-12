//! The read ports and the one-call write ports: an agent list, a skill list,
//! a branch list, a workspace's scripts, a GitHub identity and a thread's
//! children. Each is one lookup `AppState` already answers for the Moatless UI.

use crate::state::AppState;
use async_trait::async_trait;
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_t3code::ports::{
    AgentCatalogSource, BranchListError, ChildTask, GithubIdentity, RepositoryBranchSource,
    SkillCatalog, SourceControlAuthSource, TaskRelationSource, WorkspaceSkill,
};
use moatless_types::access_control::Actor;
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use moatless_workspaces::repository::error::RepositoryError;
use std::sync::Arc;

pub struct AppStateAgents(pub AppState);

#[async_trait]
impl AgentCatalogSource for AppStateAgents {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor> {
        moatless_tasks::agent::service::list_agents(
            self.0.secrets.as_ref(),
            self.0.harness_credentials.as_ref(),
            viewer,
        )
        .await
    }
}

pub struct AppStateBranches(pub AppState);

/// Adapt T3 script commands to the Workspace operation owner.
pub struct WorkspaceScripts(pub Arc<moatless_workspaces::workspace::Workspaces>);

#[async_trait]
impl moatless_t3code::ports::WorkspaceScriptCommands for WorkspaceScripts {
    async fn replace_scripts(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
        scripts: &[moatless_types::workspace::ScriptConfig],
    ) -> Result<(), moatless_t3code::scripts::ScriptsWriteError> {
        use moatless_t3code::scripts::ScriptsWriteError as Target;
        use moatless_workspaces::workspace::error::WorkspaceScriptsError as Source;
        moatless_workspaces::workspace::scripts::replace_scripts_preserving_env(
            &self.0,
            viewer,
            workspace_id,
            scripts,
        )
        .await
        .map_err(|error| match error {
            Source::NotFound => Target::NotFound,
            Source::GitSynced => Target::GitSynced,
            Source::Invalid(message) => Target::Invalid(message),
            Source::Database(source) => Target::Database(source),
        })
    }
}

#[async_trait]
impl RepositoryBranchSource for AppStateBranches {
    async fn list_branches(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<moatless_api_schemas::repository::BranchListResponse, BranchListError> {
        self.0
            .repository_branches
            .list_branches(viewer, repository_id, workspace_id)
            .await
            .map_err(branch_list_error)
    }
}

/// A repository read failure, as the branch picker is told it.
///
/// A repository the viewer cannot read answers `NotFound`, the same as one that
/// is gone — neither is a branch list to show. A missing token or a git host
/// that refused is surfaced verbatim; a database failure is kept apart so it is
/// logged rather than sent.
fn branch_list_error(error: RepositoryError) -> BranchListError {
    match error {
        RepositoryError::NotFound(_) | RepositoryError::Forbidden(_) => BranchListError::None,
        RepositoryError::BadRequest(message)
        | RepositoryError::ProviderError(message)
        | RepositoryError::Conflict(message) => BranchListError::Unavailable(message),
        RepositoryError::Db(error) => BranchListError::Failed(
            moatless_types::store::StoreError::with_source("list repository branches", error),
        ),
        RepositoryError::Store(error) => BranchListError::Failed(error),
    }
}

pub struct AppStateSkills(pub AppState);

#[async_trait]
impl SkillCatalog for AppStateSkills {
    async fn list_for_workspace(
        &self,
        viewer: &ScopeContext,
        caller: uuid::Uuid,
        workspace_id: &str,
    ) -> Vec<WorkspaceSkill> {
        // Unscoped, and safe to be: the placement only names a repository id,
        // and `list_for_repository` refuses both a workspace and a repository
        // this viewer cannot resolve.
        let primary = match self.0.workspaces.find_primary_repo(workspace_id).await {
            Ok(Some(placement)) => placement,
            Ok(None) => return Vec::new(),
            Err(error) => {
                tracing::warn!(%error, workspace_id, "T3 skill listing found no repository");
                return Vec::new();
            }
        };
        match self
            .0
            .skills
            .list_for_repository(&primary.repository_id, Some(workspace_id), caller, viewer)
            .await
        {
            Ok(skills) => skills
                .into_iter()
                .map(|skill| WorkspaceSkill {
                    name: skill.name,
                    description: skill.description,
                    plugin_name: skill.plugin_name,
                })
                .collect(),
            Err(error) => {
                tracing::warn!(%error, workspace_id, "T3 skill listing failed");
                Vec::new()
            }
        }
    }
}

pub struct AppStateSourceControlAuth(pub AppState);

#[async_trait]
impl SourceControlAuthSource for AppStateSourceControlAuth {
    async fn github_identity(&self, viewer: &ScopeContext) -> Option<GithubIdentity> {
        let presence = self
            .0
            .credentials
            .github_presence(&viewer.user_id)
            .await
            .ok()
            .flatten();
        // A row the refresher marked failed is a connection the viewer must
        // renew, so it is not a usable credential the panel should call
        // authenticated. Asked here rather than left to the issuer, which
        // reports an OAuth row usable on the stored token when a refresh fails.
        if presence.as_ref().is_some_and(|row| row.refresh_failure) {
            return None;
        }
        // Whether anything else is usable is the issuer's answer, not this
        // row's: an App installation row carries no token and no login, so a
        // caller reading the row alone reports every App-backed identity
        // unauthenticated — telling someone to connect an account while their
        // tasks are already pushing as the App. A Workspace pin is invisible
        // here for the same reason.
        self.0
            .git_host_tokens
            .github_is_usable(&Actor::of(&viewer.user_id))
            .await
            .ok()?;
        Some(GithubIdentity {
            account: presence.and_then(|row| row.login),
        })
    }
}

pub struct AppStateTaskRelations(pub AppState);

#[async_trait]
impl TaskRelationSource for AppStateTaskRelations {
    async fn children(&self, parent_task_id: &str) -> Result<Vec<ChildTask>, StoreError> {
        // Every incoming edge, filtered by the compatibility surface: a
        // narrower query here would decide which relation types are children
        // in two places.
        let relations = self
            .0
            .task_relations
            .list_incoming(parent_task_id, None)
            .await
            .map_err(|cause| StoreError::with_source("list incoming task relations", cause))?;
        Ok(relations
            .into_iter()
            .map(|relation| ChildTask {
                task_id: relation.from_task_id,
                relation: relation.relation_type,
            })
            .collect())
    }
}
