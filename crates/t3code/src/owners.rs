//! The [`crate::ports`] this crate's tests stub, implemented over the crates
//! that own the data. Each is the owner's call plus the translation into the
//! port's vocabulary.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_identity::credentials::CredentialService;
use moatless_ports::identity::{GitHostTokenIssuer, HarnessCredentials, SecretCatalog};
use moatless_ports::task::TaskReader;
use moatless_tasks::artifact::error::ArtifactError;
use moatless_tasks::artifact::operations::Artifacts;
use moatless_tasks::secret_request::error::SecretRequestError;
use moatless_tasks::secret_request::service::{Answer, SecretRequests};
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::relations::TaskRelations;
use moatless_types::access_control::{Actor, ScopeContext};
use moatless_types::store::StoreError;
use moatless_types::task::Visibility;
use moatless_workspace_types::schemas::repository::BranchListResponse;
use moatless_workspace_types::workspace::ScriptConfig;
use moatless_workspaces::repository::branches::RepositoryBranchService;
use moatless_workspaces::repository::error::RepositoryError;
use moatless_workspaces::skill::service::SkillService;
use moatless_workspaces::workspace::Workspaces;
use uuid::Uuid;

use crate::hub_models::HubModels;
use crate::ports::{
    AgentCatalogSource, ArtifactVisibilityError, ArtifactVisibilitySetter, BranchListError,
    ChildTask, GithubIdentity, ParentTask, RepositoryBranchSource, SecretRequestAnswerError,
    SecretRequestAnswerer, SkillCatalog, SourceControlAuthSource, TaskRelationSource,
    WorkspaceScriptSource, WorkspaceSkill,
};
use crate::providers;

/// The agent catalog over the credentials a viewer has stored.
pub struct AgentCatalog {
    pub hub_available: bool,
    /// Replaces Pi's static models with the hub's own list when it answers.
    pub hub_models: Option<Arc<HubModels>>,
    pub secrets: Arc<dyn SecretCatalog>,
    pub harness_credentials: Arc<dyn HarnessCredentials>,
}

#[async_trait]
impl AgentCatalogSource for AgentCatalog {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor> {
        let mut agents = moatless_tasks::agent::service::list_agents(
            self.secrets.as_ref(),
            self.harness_credentials.as_ref(),
            viewer,
            self.hub_available,
        )
        .await;
        if let Some(hub_models) = &self.hub_models {
            for agent in agents
                .iter_mut()
                .filter(|agent| providers::kind_of(agent) == providers::PI_INSTANCE)
            {
                let discovered = hub_models.models(&agent.models).await;
                if !discovered.is_empty() {
                    agent.models = discovered;
                }
            }
        }
        agents
    }
}

/// The Task read gates the Workspace read: a viewer who may read a Task runs
/// its Workspace's Scripts whether or not they may read that Workspace.
pub struct TaskWorkspaceScripts {
    pub tasks: Arc<dyn TaskReader>,
    pub workspaces: Arc<Workspaces>,
}

#[async_trait]
impl WorkspaceScriptSource for TaskWorkspaceScripts {
    async fn scripts_for_task(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Vec<ScriptConfig>>, StoreError> {
        let Some(workspace_id) = self
            .tasks
            .describe(task_id, viewer)
            .await?
            .and_then(|task| task.workspace_id)
        else {
            return Ok(None);
        };
        self.workspaces
            .scripts_unscoped(&workspace_id)
            .await
            .map_err(|e| StoreError::with_source(format!("read {workspace_id}'s scripts"), e))
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

/// The skills of a Workspace's primary repository, or its plugin skills alone
/// when it places none.
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
        let listed = match self.workspaces.find_primary_repo(workspace_id).await {
            Ok(Some(primary)) => {
                self.skills
                    .list_for_repository(&primary.repository_id, Some(workspace_id), caller, viewer)
                    .await
            }
            Ok(None) => {
                self.skills
                    .list_for_workspace_without_repository(workspace_id, caller, viewer)
                    .await
            }
            Err(error) => {
                tracing::warn!(%error, workspace_id, "T3 skill listing found no repository");
                return Vec::new();
            }
        };
        match listed {
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
                side_chat: relation.is_side_chat(),
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
            side_chat: relation.is_side_chat(),
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

/// Artifact visibility, set through the Task band's own write check.
pub struct TaskArtifacts {
    pub tasks: Arc<TaskCommands>,
    pub artifacts: Arc<Artifacts>,
}

#[async_trait]
impl ArtifactVisibilitySetter for TaskArtifacts {
    async fn set_visibility(
        &self,
        viewer: &ScopeContext,
        artifact_id: &str,
        visibility: Visibility,
    ) -> Result<(), ArtifactVisibilityError> {
        moatless_tasks::artifact::service::set_visibility(
            &self.tasks,
            &self.artifacts,
            viewer,
            artifact_id,
            visibility,
        )
        .await
        .map(|_| ())
        .map_err(|error| match error {
            ArtifactError::NotFound(_) | ArtifactError::TaskNotFound(_) => {
                ArtifactVisibilityError::NotFound
            }
            ArtifactError::Forbidden(_) => ArtifactVisibilityError::Forbidden,
            other => ArtifactVisibilityError::Failed(moatless_core::error::causal_chain(&other)),
        })
    }
}

/// Secret request answers, through the Task band's own access checks.
pub struct TaskSecretRequests(pub Arc<SecretRequests>);

#[async_trait]
impl SecretRequestAnswerer for TaskSecretRequests {
    async fn answer(
        &self,
        viewer: &ScopeContext,
        login: &str,
        task_id: &str,
        activity_id: Uuid,
        answer: Answer,
    ) -> Result<(), SecretRequestAnswerError> {
        self.0
            .answer(viewer, login, task_id, activity_id, answer)
            .await
            .map_err(|error| match error {
                SecretRequestError::TaskNotFound(_) | SecretRequestError::NotFound(_) => {
                    SecretRequestAnswerError::NotFound
                }
                SecretRequestError::AlreadyAnswered => SecretRequestAnswerError::AlreadyAnswered,
                SecretRequestError::Forbidden(message)
                | SecretRequestError::BadRequest(message) => {
                    SecretRequestAnswerError::Refused(message)
                }
                other @ (SecretRequestError::Store(_) | SecretRequestError::Db(_)) => {
                    SecretRequestAnswerError::Failed(moatless_core::error::causal_chain(&other))
                }
            })
    }
}
