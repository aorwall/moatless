//! The [`crate::ports`] this crate's tests stub, implemented over the crates
//! that own the data. Each is the owner's call plus the translation into the
//! port's vocabulary.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_identity::credentials::CredentialService;
use moatless_ports::identity::{GitHostTokenIssuer, HarnessCredentials, SecretCatalog};
use moatless_tasks::task::relations::TaskRelations;
use moatless_types::access_control::{Actor, ScopeContext};
use moatless_types::store::StoreError;
use moatless_workspace_types::schemas::repository::BranchListResponse;
use moatless_workspace_types::workspace::ScriptConfig;
use moatless_workspaces::repository::branches::RepositoryBranchService;
use moatless_workspaces::repository::error::RepositoryError;
use moatless_workspaces::skill::service::SkillService;
use moatless_workspaces::workspace::Workspaces;
use moatless_workspaces::workspace::error::WorkspaceScriptsError;
use uuid::Uuid;

use crate::ports::{
    AgentCatalogSource, BranchListError, ChildTask, GithubIdentity, ParentTask,
    RepositoryBranchSource, SkillCatalog, SourceControlAuthSource, TaskRelationSource,
    WorkspaceScriptCommands, WorkspaceSkill,
};
use crate::scripts::ScriptsWriteError;

/// The agent catalog over the credentials a viewer has stored.
pub struct AgentCatalog {
    pub secrets: Arc<dyn SecretCatalog>,
    pub harness_credentials: Arc<dyn HarnessCredentials>,
}

#[async_trait]
impl AgentCatalogSource for AgentCatalog {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor> {
        moatless_tasks::agent::service::list_agents(
            self.secrets.as_ref(),
            self.harness_credentials.as_ref(),
            viewer,
        )
        .await
    }
}

#[async_trait]
impl WorkspaceScriptCommands for Workspaces {
    async fn replace_scripts(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
        scripts: &[ScriptConfig],
    ) -> Result<(), ScriptsWriteError> {
        moatless_workspaces::workspace::scripts::replace_scripts_preserving_env(
            self,
            viewer,
            workspace_id,
            scripts,
        )
        .await
        .map_err(|error| match error {
            WorkspaceScriptsError::NotFound => ScriptsWriteError::NotFound,
            WorkspaceScriptsError::GitSynced => ScriptsWriteError::GitSynced,
            WorkspaceScriptsError::Invalid(message) => ScriptsWriteError::Invalid(message),
            WorkspaceScriptsError::Database(source) => ScriptsWriteError::Database(source),
        })
    }
}

#[async_trait]
impl RepositoryBranchSource for RepositoryBranchService {
    async fn list_branches(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<BranchListResponse, BranchListError> {
        RepositoryBranchService::list_branches(self, viewer, repository_id, workspace_id)
            .await
            .map_err(branch_list_error)
    }
}

/// A repository read failure, as the branch picker is told it.
///
/// A repository the viewer cannot read answers `NotFound`, same as one that's
/// gone; a missing token or refusing git host is surfaced verbatim; a database
/// failure is kept apart so it's logged rather than sent.
fn branch_list_error(error: RepositoryError) -> BranchListError {
    match error {
        RepositoryError::NotFound(_) | RepositoryError::Forbidden(_) => BranchListError::None,
        RepositoryError::BadRequest(message)
        | RepositoryError::ProviderError(message)
        | RepositoryError::Conflict(message) => BranchListError::Unavailable(message),
        RepositoryError::Db(error) => {
            BranchListError::Failed(StoreError::with_source("list repository branches", error))
        }
        RepositoryError::Store(error) => BranchListError::Failed(error),
    }
}

/// The skills of a Workspace's primary repository.
pub struct WorkspaceSkills {
    pub workspaces: Arc<Workspaces>,
    pub skills: Arc<SkillService>,
}

#[async_trait]
impl SkillCatalog for WorkspaceSkills {
    async fn list_for_workspace(
        &self,
        viewer: &ScopeContext,
        caller: Uuid,
        workspace_id: &str,
    ) -> Vec<WorkspaceSkill> {
        // Unscoped, and safe to be: the placement only names a repository id,
        // and `list_for_repository` refuses both a workspace and a repository
        // this viewer cannot resolve.
        let primary = match self.workspaces.find_primary_repo(workspace_id).await {
            Ok(Some(placement)) => placement,
            Ok(None) => return Vec::new(),
            Err(error) => {
                tracing::warn!(%error, workspace_id, "T3 skill listing found no repository");
                return Vec::new();
            }
        };
        match self
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

/// The viewer's GitHub connection, from their provider-token row and the
/// token issuer.
pub struct GithubAuth {
    pub credentials: Arc<CredentialService>,
    pub git_host_tokens: Arc<dyn GitHostTokenIssuer>,
}

#[async_trait]
impl SourceControlAuthSource for GithubAuth {
    async fn github_identity(&self, viewer: &ScopeContext) -> Option<GithubIdentity> {
        let presence = self
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
        // Usability beyond this is the issuer's answer, not the row's: an App
        // installation row carries no token or login, so reading the row alone
        // would call every App-backed identity unauthenticated. A Workspace
        // pin is invisible here for the same reason.
        self.git_host_tokens
            .github_is_usable(&Actor::of(&viewer.user_id))
            .await
            .ok()?;
        Some(GithubIdentity {
            account: presence.and_then(|row| row.login),
        })
    }
}

#[async_trait]
impl TaskRelationSource for TaskRelations {
    async fn children(&self, parent_task_id: &str) -> Result<Vec<ChildTask>, StoreError> {
        // Every incoming edge, filtered by the caller: a narrower query here
        // would decide which relation types are children in two places.
        let relations = self
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

    async fn parent(&self, child_task_id: &str) -> Result<Option<ParentTask>, StoreError> {
        let relation = self
            .find_parent(child_task_id)
            .await
            .map_err(|cause| StoreError::with_source("find task parent", cause))?;
        Ok(relation.map(|relation| ParentTask {
            at_turn: relation
                .context
                .get("at_turn")
                .and_then(serde_json::Value::as_i64)
                .and_then(|turn| i32::try_from(turn).ok()),
            task_id: relation.to_task_id,
            relation: relation.relation_type,
        }))
    }
}
