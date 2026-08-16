//! Presenting a task row as a `TaskResponse`.
//!
//! A `Task` row carries foreign keys, not the labels a client shows: a
//! repository id but not its name, a creator id but not a display name, no
//! tags, bindings, relations or live sandbox intent at all. Assembling the
//! response means eight independent lookups against eight ports, and both the
//! single-task path (`enrich_task`) and the list path (the batch resolvers)
//! draw on the same set. They share those ports, so they share one struct
//! rather than threading eight handles through every presenting handler.

use std::collections::HashMap;
use std::sync::Arc;

use moatless_backend_core::scope::ScopeContext;

use super::error::TaskError;
use super::models::Task;
use super::relations::{TaskRelation, TaskRelationDao};
use super::schemas::*;
use crate::tag::dao::TagDao;
use crate::task::binding_dao::TaskBindingDao;
use crate::task::binding_summary::task_binding_summary_from_binding;
use crate::task::dao::TaskDao;
use crate::task::loop_reader::LoopSubscriptionReader;
use crate::task::models::SandboxIntentView;
use moatless_api_schemas::bindings::TaskBindingSummary;
use moatless_identity::auth::users::UserDao;
use moatless_workspaces::repository::dao::RepositoryDao;
use moatless_workspaces::workspace::dao::WorkspaceDao;

/// The eight ports a task response is assembled from, held together because a
/// `TaskResponse` has eight independent sources and `enrich_task` reads all of
/// them for one row while the batch resolvers read subsets for a page of rows.
/// Assembled once at the composition root; all fields are cheap `Arc` clones.
pub struct TaskPresenter {
    pub(crate) tags: Arc<dyn TagDao>,
    pub(crate) repositories: Arc<dyn RepositoryDao>,
    pub(crate) workspaces: Arc<dyn WorkspaceDao>,
    pub(crate) users: Arc<dyn UserDao>,
    pub(crate) task_bindings: Arc<dyn TaskBindingDao>,
    pub(crate) loops: Arc<dyn LoopSubscriptionReader>,
    pub(crate) task_relations: Arc<dyn TaskRelationDao>,
    pub(crate) tasks: Arc<dyn TaskDao>,
}

impl TaskPresenter {
    /// Wire the presenter from the ports the backend composition root owns.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tags: Arc<dyn TagDao>,
        repositories: Arc<dyn RepositoryDao>,
        workspaces: Arc<dyn WorkspaceDao>,
        users: Arc<dyn UserDao>,
        task_bindings: Arc<dyn TaskBindingDao>,
        loops: Arc<dyn LoopSubscriptionReader>,
        task_relations: Arc<dyn TaskRelationDao>,
        tasks: Arc<dyn TaskDao>,
    ) -> Self {
        Self {
            tags,
            repositories,
            workspaces,
            users,
            task_bindings,
            loops,
            task_relations,
            tasks,
        }
    }

    /// Assemble the full `TaskResponse` for one task row, resolving every
    /// labelled foreign key, its tags, bindings, outgoing relations, live
    /// sandbox intent, and the requesting user's read/follow markers.
    pub async fn enrich_task(&self, row: &Task, user_id: &str) -> Result<TaskResponse, TaskError> {
        let tags = self.tags.get_tags_for_task(&row.task_id).await?;
        let tag_responses: Vec<TagResponse> = tags
            .iter()
            .map(|t| TagResponse {
                tag_id: t.tag_id.clone(),
                name: t.name.clone(),
            })
            .collect();

        // reason: display enrichment of a task the caller already read; the
        // task's configured repository name is part of that task's own data.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let repo_name = self
            .repositories
            .find_by_id(&row.repository_id, &sys_ctx)
            .await?
            .map(|r| r.name);

        // Same reasoning as `repo_name` above — labelling the workspace the caller
        // already has access to via the task.
        let workspace_name = match &row.workspace_id {
            Some(wid) => self
                .workspaces
                .find_names_by_ids(std::slice::from_ref(wid))
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, workspace_id = %wid, "Failed to resolve workspace name");
                    Vec::new()
                })
                .into_iter()
                .next()
                .map(|(_, name)| name),
            None => None,
        };

        let display_name = if let Some(ref uid) = row.created_by {
            self.resolve_single_user_display_name(uid).await
        } else {
            None
        };

        // Resolve external bindings for this task.
        let bindings = self.resolve_task_bindings(&row.task_id).await;

        // Resolve outgoing relations for this task (best-effort — empty on error).
        let relations = match self.task_relations.list_outgoing(&row.task_id).await {
            Ok(rows) => relations_to_dtos(rows),
            Err(e) => {
                tracing::warn!(error = %e, task_id = %row.task_id, "Failed to load task relations");
                Vec::new()
            }
        };

        // Durable Sandbox state comes from the `sandboxes` table; absence is the
        // lazy converged-Stopped default.
        let sandbox = match self
            .tasks
            .get_sandbox_intents(std::slice::from_ref(&row.task_id))
            .await
        {
            Ok(mut intents) => intents.remove(&row.task_id).unwrap_or_default(),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    task_id = %row.task_id,
                    "Failed to load Sandbox lifecycle intent for task response"
                );
                SandboxIntentView::default()
            }
        };
        let mut resp = build_task_response(
            row,
            &sandbox,
            tag_responses,
            ResolvedNames {
                creator: display_name,
                repository: repo_name,
                workspace: workspace_name,
            },
            bindings,
            relations,
        );

        // Resolve the requesting user's read marker (best-effort — leave None on
        // failure). Drives the unread indicator in clients.
        match self
            .tasks
            .get_read_receipts(user_id, std::slice::from_ref(&row.task_id))
            .await
        {
            Ok(receipts) => {
                resp.last_read_at = receipts.get(&row.task_id).map(chrono::DateTime::to_rfc3339);
            }
            Err(e) => {
                tracing::warn!(error = %e, task_id = %row.task_id, "Failed to load read receipt");
            }
        }

        // Resolve whether the requesting user follows this task (best-effort —
        // leave None on failure). Drives the follow/unfollow toggle in clients.
        match self
            .tasks
            .get_followed(user_id, std::slice::from_ref(&row.task_id))
            .await
        {
            Ok(followed) => {
                resp.is_following = Some(followed.contains(&row.task_id));
            }
            Err(e) => {
                tracing::warn!(error = %e, task_id = %row.task_id, "Failed to load follow state");
            }
        }

        Ok(resp)
    }

    /// Resolve display names for multiple user IDs in a single batch query.
    pub async fn resolve_user_display_names_batch(
        &self,
        user_ids: &[String],
    ) -> HashMap<String, String> {
        if user_ids.is_empty() {
            return HashMap::new();
        }
        let uuids: Vec<uuid::Uuid> = user_ids.iter().filter_map(|id| id.parse().ok()).collect();
        match self.users.get_users_by_ids(&uuids).await {
            Ok(users) => users
                .into_iter()
                .map(|u| {
                    let display_name = u.name.unwrap_or(u.login);
                    (u.id.to_string(), display_name)
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to batch resolve user display names");
                HashMap::new()
            }
        }
    }

    /// Resolve repository names for multiple repo IDs in a single batch query.
    pub async fn resolve_repo_names_batch(&self, repo_ids: &[String]) -> HashMap<String, String> {
        if repo_ids.is_empty() {
            return HashMap::new();
        }
        match self.repositories.find_names_by_ids(repo_ids).await {
            Ok(pairs) => pairs.into_iter().collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to batch resolve repository names");
                HashMap::new()
            }
        }
    }

    /// Resolve workspace names for multiple workspace IDs in a single batch query.
    pub async fn resolve_workspace_names_batch(
        &self,
        workspace_ids: &[String],
    ) -> HashMap<String, String> {
        if workspace_ids.is_empty() {
            return HashMap::new();
        }
        match self.workspaces.find_names_by_ids(workspace_ids).await {
            Ok(pairs) => pairs.into_iter().collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to batch resolve workspace names");
                HashMap::new()
            }
        }
    }

    /// Best-effort batch load of outgoing relations for many task ids, grouped
    /// into a `task_id -> Vec<TaskRelationDto>` map. Failures are logged and
    /// produce an empty map (callers fall back to empty `relations`).
    pub async fn load_outgoing_relations_batch(
        &self,
        task_ids: &[String],
    ) -> HashMap<String, Vec<TaskRelationDto>> {
        if task_ids.is_empty() {
            return HashMap::new();
        }
        let mut out: HashMap<String, Vec<TaskRelationDto>> = HashMap::new();
        for tid in task_ids {
            match self.task_relations.list_outgoing(tid).await {
                Ok(rows) => {
                    if !rows.is_empty() {
                        out.insert(tid.clone(), relations_to_dtos(rows));
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, task_id = %tid, "Failed to load relations for task");
                }
            }
        }
        out
    }

    /// Resolve a single user's display name.
    async fn resolve_single_user_display_name(&self, user_id: &str) -> Option<String> {
        let uid = user_id.parse::<uuid::Uuid>().ok()?;
        match self.users.get_user(&uid).await {
            Ok(Some(user)) => Some(user.name.unwrap_or(user.login)),
            _ => None,
        }
    }

    /// Resolve external bindings for a single task (used by `enrich_task`).
    async fn resolve_task_bindings(&self, task_id: &str) -> Vec<TaskBindingSummary> {
        let bindings = match self.task_bindings.find_by_task_id(task_id).await {
            Ok(bindings) => bindings,
            Err(e) => {
                tracing::warn!(error = %e, task_id = %task_id, "Failed to resolve task bindings");
                return vec![];
            }
        };

        let mut summaries = Vec::new();
        for binding in &bindings {
            let trigger = if let Some(ref loop_id) = binding.loop_id {
                self.loops
                    .find_subscription_by_id(loop_id)
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
            summaries.push(task_binding_summary_from_binding(binding, trigger.as_ref()));
        }
        summaries
    }
}

/// Display names resolved for the foreign keys on a task row, so the response
/// can label them without the client re-querying. Each is `None` when the
/// referenced row is absent or its lookup failed; clients fall back to the id.
#[derive(Default)]
pub(crate) struct ResolvedNames {
    pub(crate) creator: Option<String>,
    pub(crate) repository: Option<String>,
    pub(crate) workspace: Option<String>,
}

/// Build a `TaskResponse` from a row plus pre-resolved tags, names, bindings,
/// and outgoing relations. `parent_task_id` is derived from the first
/// `CreatedBy` relation for backward compatibility with clients that pre-date
/// the relations field.
pub(crate) fn build_task_response(
    row: &Task,
    sandbox: &SandboxIntentView,
    tags: Vec<TagResponse>,
    names: ResolvedNames,
    bindings: Vec<TaskBindingSummary>,
    relations: Vec<TaskRelationDto>,
) -> TaskResponse {
    let title = derive_title(row);

    let parent_task_id = relations
        .iter()
        .find(|r| {
            matches!(
                r.relation_type,
                moatless_api_schemas::enums::TaskRelationType::CreatedBy
            )
        })
        .map(|r| r.to_task_id.clone());

    TaskResponse {
        task_id: row.task_id.clone(),
        short_id: row.short_id.clone(),
        title,
        name: row.name.clone(),
        description: row.description.clone(),
        status: task_status_to_wire(row.status),
        agent_type: agent_type_to_wire(&row.agent_type),
        repository_id: row.repository_id.clone(),
        repository_name: names.repository,
        workspace_id: row.workspace_id.clone(),
        workspace_name: names.workspace,
        sandbox_id: row.sandbox_id.clone(),
        checkout_mode: row.checkout_mode.map(checkout_mode_to_wire),
        branch: row.branch.clone(),
        model: row.model.clone(),
        created_at: row.created_at.clone(),
        last_message_at: row.last_message_at.clone(),
        total_cost_usd: row.total_cost_usd,
        site_url: row.site_url.clone(),
        template_id: row.template_id.clone(),
        vibe: row.vibe,
        created_by: row.created_by.clone(),
        created_by_display_name: names.creator,
        visibility: visibility_to_wire(row.visibility),
        scope: scope_to_wire(row.scope),
        owner_user_id: row.owner_user_id.clone(),
        parent_task_id,
        relations,
        append_system_prompt: row.append_system_prompt.clone(),
        tags,
        bindings,
        ttl_minutes: row.ttl_minutes,
        inactive_ttl_days: row.inactive_ttl_days,
        sandbox_started_at: sandbox.started_at.map(|t| t.to_rfc3339()),
        sandbox_desired_state: sandbox_desired_state_to_wire(sandbox.desired_state),
        sandbox_stop_reason: sandbox.stop_reason.map(sandbox_stop_reason_to_wire),
        sandbox_desired_state_updated_at: sandbox.desired_state_updated_at.to_rfc3339(),
        // Populated per-request by callers that know the requesting user.
        last_read_at: None,
        is_following: None,
    }
}

/// Translate domain `TaskRelation` rows into wire DTOs.
fn relations_to_dtos(rows: Vec<TaskRelation>) -> Vec<TaskRelationDto> {
    rows.into_iter()
        .map(|r| TaskRelationDto {
            id: r.id,
            from_task_id: r.from_task_id,
            to_task_id: r.to_task_id,
            relation_type: relation_type_to_wire(r.relation_type),
            context: r.context,
            created_at: r.created_at.to_rfc3339(),
        })
        .collect()
}

/// Derive task title: name > description first line > agent_type > "Task".
fn derive_title(row: &Task) -> String {
    if let Some(ref name) = row.name {
        if !name.is_empty() {
            return name.clone();
        }
    }
    if let Some(ref desc) = row.description {
        if !desc.is_empty() {
            return desc.lines().next().unwrap_or("Task").to_string();
        }
    }
    row.agent_type.to_string()
}
