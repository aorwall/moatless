//! The commands a sandbox reports running, and the ones it never reported.

use tonic::{Request, Response, Status};
use tracing::warn;

use sandbox_proto::{
    RecordUnreportedCommandsRequest, RecordUnreportedCommandsResponse,
    TakeUnreportedCommandsRequest, TakeUnreportedCommandsResponse, UnreportedCommand,
};

use crate::sandbox::lifecycle::OutstandingCommand;

use super::state::BackendSandboxServiceImpl;

fn outstanding_command(entry: UnreportedCommand) -> OutstandingCommand {
    OutstandingCommand {
        id: entry.id,
        command: entry.command,
        label: entry.label,
        started_at_unix_ms: entry.started_at_unix_ms,
        state: entry.state,
        exit_code: entry.exit_code,
    }
}

fn unreported_command_entry(command: OutstandingCommand) -> UnreportedCommand {
    UnreportedCommand {
        id: command.id,
        command: command.command,
        label: command.label,
        started_at_unix_ms: command.started_at_unix_ms,
        state: command.state,
        exit_code: command.exit_code,
    }
}

impl BackendSandboxServiceImpl {
    /// Mirror the pod's running-command count onto the Sandbox row, where the
    /// Task listing can read it.
    ///
    /// Logged rather than propagated: the count is absolute, so the next
    /// transition corrects any failure here.
    pub(super) async fn record_running_commands(&self, task_id: &str, running: i32) {
        let Some(sandboxes) = self.handlers.as_ref().map(|h| &h.sandboxes) else {
            return;
        };
        if let Err(error) = sandboxes.record_running_commands(task_id, running).await {
            warn!(
                task_id,
                running,
                %error,
                "failed to record running commands; the Task listing keeps the previous count"
            );
        }
    }
}

/// Mirror the `moat cmd` commands that still owe the agent an outcome onto the
/// Sandbox row.
///
/// The pod's own journal rides the S3 snapshot, so a failed restore loses it
/// silently; this row is the copy no restore touches.
pub(super) async fn record_unreported_commands(
    service: &BackendSandboxServiceImpl,
    request: Request<RecordUnreportedCommandsRequest>,
) -> Result<Response<RecordUnreportedCommandsResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    let sandboxes = service
        .handlers
        .as_ref()
        .map(|h| h.sandboxes.clone())
        .ok_or_else(|| Status::failed_precondition("Database not configured"))?;

    let commands: Vec<OutstandingCommand> =
        req.commands.into_iter().map(outstanding_command).collect();
    sandboxes
        .record_unreported_commands(&task_id, &commands, req.revision)
        .await
        .map_err(|error| {
            warn!(task_id, %error, "failed to record the unreported commands");
            Status::internal("Failed to record the unreported commands")
        })?;

    Ok(Response::new(RecordUnreportedCommandsResponse {}))
}

/// Read and clear what a prior pod recorded.
///
/// Clearing here makes the loss announced exactly once, however many times the
/// pod restarts.
pub(super) async fn take_unreported_commands(
    service: &BackendSandboxServiceImpl,
    request: Request<TakeUnreportedCommandsRequest>,
) -> Result<Response<TakeUnreportedCommandsResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;

    let sandboxes = service
        .handlers
        .as_ref()
        .map(|h| h.sandboxes.clone())
        .ok_or_else(|| Status::failed_precondition("Database not configured"))?;

    let commands = sandboxes
        .take_unreported_commands(&task_id)
        .await
        .map_err(|e| {
            warn!(task_id, error = %e, "failed to take the unreported commands");
            Status::internal("Failed to take the unreported commands")
        })?;

    Ok(Response::new(TakeUnreportedCommandsResponse {
        commands: commands.into_iter().map(unreported_command_entry).collect(),
    }))
}
