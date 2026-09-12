//! What a sandbox reports once it has pushed.

use tonic::{Request, Response, Status};
use tracing::{debug, warn};

use sandbox_proto::{RecordGitPushRequest, RecordGitPushResponse};

use super::state::BackendSandboxServiceImpl;

/// Claim a sha the task pushed, so GitHub's `synchronize` webhook for it is
/// passed over rather than delivered back to its author.
///
/// Ahead of the webhook by construction: the sandbox calls this as the push
/// completes, and the delivery that reads the claim runs in the worker off a
/// durable record. A failure is returned rather than swallowed, so the watcher
/// leaves the sha unclaimed and refiles it on the next push to that repository.
pub(super) async fn record_git_push(
    service: &BackendSandboxServiceImpl,
    request: Request<RecordGitPushRequest>,
) -> Result<Response<RecordGitPushResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    if req.owner.is_empty() || req.repo.is_empty() || req.sha.is_empty() {
        return Err(Status::invalid_argument(
            "owner, repo and sha are all required",
        ));
    }

    let transcript = service
        .handlers
        .as_ref()
        .map(|h| h.transcript.clone())
        .ok_or_else(|| Status::failed_precondition("Database not configured"))?;

    moatless_adapter_github_pr::push_claim::record(
        transcript.as_ref(),
        &req.task_id,
        &req.owner,
        &req.repo,
        &req.sha,
    )
    .await
    .map_err(|e| {
        warn!(
            task_id = %req.task_id,
            repo = %format!("{}/{}", req.owner, req.repo),
            sha = %req.sha,
            error = %e,
            "Failed to record git push claim"
        );
        Status::internal("Failed to record git push claim")
    })?;

    debug!(
        task_id = %req.task_id,
        repo = %format!("{}/{}", req.owner, req.repo),
        sha = %req.sha,
        "Recorded git push claim"
    );
    Ok(Response::new(RecordGitPushResponse {}))
}
