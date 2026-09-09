use super::error::TaskError;
use super::models::TaskPatch;
use super::models::TaskStatus;
use super::operations::TaskCommands;
use moatless_core::events::EventPublisher;
use moatless_ports::sandbox::{PreflightPort, SandboxLifecycle};
use moatless_ports::workspace::RepositoryReader;
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use moatless_workspaces::workspace::git_sync::GitSyncService;

/// Collaborators needed to validate and start a task.
pub struct StartTaskPorts<'a> {
    pub tasks: &'a TaskCommands,
    pub repositories: &'a dyn RepositoryReader,
    pub preflight: &'a dyn PreflightPort,
    pub events: Option<&'a dyn EventPublisher>,
    pub sandbox: &'a dyn SandboxLifecycle,
    pub git_sync: &'a GitSyncService,
}

/// Validate task access and preflight before synchronizing config and starting resources.
pub async fn start_task(
    ports: StartTaskPorts<'_>,
    scope_ctx: &ScopeContext,
    task_id: String,
) -> Result<(), TaskError> {
    let StartTaskPorts {
        tasks,
        repositories,
        preflight,
        events,
        sandbox,
        git_sync,
    } = ports;

    let row = tasks
        .find_by_id(&task_id, scope_ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
    if !scope_ctx.can_control(&row.scoped_resource(), row.visibility) {
        return Err(TaskError::NotFound(task_id.clone())); // existence hiding
    }

    if row.status != TaskStatus::Open && row.status != TaskStatus::Error {
        return Err(TaskError::BadRequest(format!(
            "Cannot start task in '{}' status",
            row.status
        )));
    }

    tracing::info!(task_id = %task_id, user_id = %scope_ctx.user_id, "Starting task");

    // Pre-flight: validate the GitHub token + repo access before any
    // sandbox resources are created. On failure: mark Error, publish a
    // sandbox.error event, and return 400.
    let repo = repositories
        .describe(scope_ctx, &row.repository_id)
        .await?
        .ok_or_else(|| TaskError::RepoNotFound(row.repository_id.clone()))?;

    if let Err(e) = preflight.validate(&row, &repo).await {
        tracing::warn!(task_id = %task_id, error = %e, "Start preflight failed");
        if let Err(db_err) = tasks
            .update_task(
                &task_id,
                &TaskPatch {
                    status: Some(TaskStatus::Error),
                    ..Default::default()
                },
                scope_ctx,
            )
            .await
        {
            tracing::error!(
                task_id = %task_id,
                error = %db_err,
                "Failed to mark task Error after preflight failure"
            );
        }
        if let Some(events) = events {
            events
                .publish_sandbox_event_with_payload(
                    &task_id,
                    SandboxStatus::Error,
                    &SandboxEventPayload {
                        message: e.user_message.clone(),
                        error: Some(e.detail.clone()),
                        lifecycle_event: Some("preflight_failed".to_string()),
                        ..Default::default()
                    },
                )
                .await;
        }
        return Err(TaskError::BadRequest(e.user_message));
    }

    // Best-effort: if the repo ships a `.moatless/` directory, reconcile its
    // workspace config before provisioning. Loops are never synchronized as a
    // side effect of task start.
    git_sync
        .sync_repo_config_on_start_for(scope_ctx, &repo)
        .await;

    sandbox.start(&task_id).await?;

    tracing::info!(task_id = %task_id, "Sandbox start initiated (initializing)");
    Ok(())
}
