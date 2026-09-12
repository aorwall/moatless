//! The presigned URLs a sandbox syncs its workspace through.

use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

use sandbox_proto::{
    GetWorkspaceRepositoryAccessRequest, GetWorkspaceRepositoryAccessResponse,
    GetWorkspaceSyncUrlsRequest, GetWorkspaceSyncUrlsResponse, ReportWorkspaceSyncRequest,
    ReportWorkspaceSyncResponse,
};

use crate::sandbox::storage::{sandbox_key, workspace_repository_snapshots_prefix};
use crate::sandbox::workspace_repository::GrantError;
use moatless_s3 as file_s3;
use moatless_tasks::task_event::models::InsertTaskEvent;

use super::state::BackendSandboxServiceImpl;
use super::transcripts::fork_source_task_id;

/// How "this task holds a workspace in this format" is asked of S3.
///
/// The two variants cost the same one request. They differ because a `restic`
/// snapshot's id is its content hash, so no key that proves one exists can be
/// built from a task id.
#[derive(Clone, Copy)]
enum WorkspaceProbe {
    /// One object at a known key: `sandbox_key` for the tar.
    Object(fn(&str, &str) -> String),
    /// Any object under a prefix: `workspace_repository_snapshots_prefix` for
    /// the `restic` repository.
    AnyUnder(fn(&str, &str) -> String),
}

impl WorkspaceProbe {
    /// Whether `task_id` holds this format. `Err` when S3 could not answer,
    /// which every caller reads as "restore the task's own" rather than as `false`.
    async fn hit(
        self,
        s3_client: &moatless_s3::S3Storage,
        bucket: &str,
        prefix: &str,
        task_id: &str,
    ) -> Result<bool, moatless_s3::S3Error> {
        match self {
            Self::Object(key_for) => {
                let modified = file_s3::head_object_modified_at(
                    &s3_client.client,
                    bucket,
                    &key_for(prefix, task_id),
                )
                .await?;
                Ok(modified.is_some())
            }
            Self::AnyUnder(prefix_for) => {
                file_s3::prefix_has_any_object(
                    &s3_client.client,
                    bucket,
                    &prefix_for(prefix, task_id),
                )
                .await
            }
        }
    }
}

/// Lifetime of the presigned workspace-sync URLs. The sandbox re-mints them on
/// every sync tick, so this only needs to cover a single transfer. Kept short to
/// bound the blast radius of a leaked URL, but generous enough for a large tar
/// on a slow link.
const WORKSPACE_PRESIGN_TTL: std::time::Duration = std::time::Duration::from_mins(15);

impl BackendSandboxServiceImpl {
    /// The key a workspace restore should read for `task_id`.
    ///
    /// A fork that has not yet synced has no archive of its own, and the working
    /// tree its first turn expects is the parent's. Serving the parent's is also
    /// what keeps an isolated fork off the clone path: `restore_on_start` runs
    /// before `clone_repositories`, which skips a repo whose directory already
    /// exists, so a fork whose parent worked on an unpushed branch restores that
    /// branch instead of failing to clone it.
    ///
    /// Falls back only on a definitive "not there". `head_object_modified_at`
    /// reports "cannot tell" as `Err`, and reading that as absence would hand a
    /// fork that has already synced the parent's older tree.
    async fn workspace_restore_key(
        &self,
        s3_client: &moatless_s3::S3Storage,
        bucket: &str,
        task_id: &str,
        own_key: &str,
    ) -> String {
        let source = self
            .workspace_restore_task_id(
                s3_client,
                bucket,
                task_id,
                WorkspaceProbe::Object(sandbox_key),
            )
            .await;
        match source {
            Some(source_task_id) => sandbox_key(&self.workspace_s3_prefix, &source_task_id),
            None => own_key.to_string(),
        }
    }

    /// The task whose prefix a restore should read, or `None` for the task's own.
    ///
    /// `probe` decides it against the artifact the caller actually restores
    /// from, so the tar and the repository resolve independently: a fork whose
    /// parent wrote a repository but no tar must not be sent to the parent for a
    /// tar that is not there.
    ///
    /// One level only. `find_parent` is the fork's immediate parent, so a fork of
    /// a fork whose parent holds nothing in this format restores its own.
    async fn workspace_restore_task_id(
        &self,
        s3_client: &moatless_s3::S3Storage,
        bucket: &str,
        task_id: &str,
        probe: WorkspaceProbe,
    ) -> Option<String> {
        match probe
            .hit(s3_client, bucket, &self.workspace_s3_prefix, task_id)
            .await
        {
            Ok(false) => {}
            Ok(true) => return None,
            Err(e) => {
                warn!(error = %e, task_id = %task_id, "S3 probe for the task's workspace failed; restoring its own key");
                return None;
            }
        }

        let handlers = self.handlers.as_ref()?;
        let parent = match handlers.task_relations.find_parent(task_id).await {
            Ok(parent) => parent,
            Err(e) => {
                warn!(error = %e, task_id = %task_id, "Failed to read the fork relation; restoring the task's own key");
                return None;
            }
        };
        let source_task_id = fork_source_task_id(parent.as_ref())?.to_string();

        match probe
            .hit(
                s3_client,
                bucket,
                &self.workspace_s3_prefix,
                &source_task_id,
            )
            .await
        {
            Ok(true) => {
                info!(
                    task_id = %task_id,
                    source_task_id = %source_task_id,
                    "Restoring a fork from the source task's workspace; it has none of its own"
                );
                Some(source_task_id)
            }
            Ok(false) => None,
            Err(e) => {
                warn!(error = %e, task_id = %task_id, source_task_id = %source_task_id, "S3 probe for the source task's workspace failed");
                None
            }
        }
    }
}

/// Mint presigned PUT + GET URLs for the task's workspace archive.
///
/// The `s3` storage backend uses this instead of the transcript-style gRPC
/// proxy: the workspace tar can be hundreds of MB, so rather than stream it
/// through the backend, we hand the sandbox short-lived URLs scoped to
/// exactly `<prefix>/<task_id>/workspace.tar` and it transfers directly to
/// S3. The sandbox stays AWS-credential-less; the URL is the only capability
/// it holds and it expires. Auth: task-scoped JWT via gRPC metadata.
pub(super) async fn get_workspace_sync_urls(
    service: &BackendSandboxServiceImpl,
    request: Request<GetWorkspaceSyncUrlsRequest>,
) -> Result<Response<GetWorkspaceSyncUrlsResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    let Some(ref s3_client) = service.s3 else {
        return Err(Status::failed_precondition("S3 not configured"));
    };
    let Some(ref bucket) = service.s3_bucket else {
        return Err(Status::failed_precondition("S3 bucket not configured"));
    };

    let key = sandbox_key(&service.workspace_s3_prefix, &req.task_id);

    // The PUT is always the task's own key, so a fork's first sync lands on
    // its own archive and every later restore reads that rather than the
    // parent's. Only the GET falls back.
    let put_url = file_s3::presign_put(s3_client, bucket, &key, WORKSPACE_PRESIGN_TTL)
        .await
        .map_err(|e| {
            warn!(error = %e, task_id = %req.task_id, "presign PUT for workspace failed");
            Status::internal("Failed to presign workspace upload URL")
        })?;

    let get_key = service
        .workspace_restore_key(s3_client, bucket, &req.task_id, &key)
        .await;
    let get_url = file_s3::presign_get(s3_client, bucket, &get_key, WORKSPACE_PRESIGN_TTL)
        .await
        .map_err(|e| {
            warn!(error = %e, task_id = %req.task_id, "presign GET for workspace failed");
            Status::internal("Failed to presign workspace download URL")
        })?;

    debug!(task_id = %req.task_id, key = %key, get_key = %get_key, "Minted workspace sync URLs");
    Ok(Response::new(GetWorkspaceSyncUrlsResponse {
        expires_in_secs: put_url.expires_in.min(get_url.expires_in).as_secs() as i64,
        put_url: put_url.url,
        get_url: get_url.url,
    }))
}

/// Hand the sandbox direct S3 access to its own workspace `restic`
/// repository, plus the password that opens it.
///
/// Two things a presigned URL cannot carry. `rustic_core` lists the
/// repository by prefix on every open, and a signature covers one key; and
/// the repository is encrypted, so access without the password reads
/// nothing. Only this task's prefix is writable, and the password is derived
/// from the task id.
///
/// A fork that holds no repository of its own is granted its source's
/// prefix as well, read-only — the same fallback
/// [`BackendSandboxServiceImpl::workspace_restore_task_id`] resolves for the tar, against the
/// object each restores from.
///
/// Auth: task-scoped JWT via gRPC metadata.
pub(super) async fn get_workspace_repository_access(
    service: &BackendSandboxServiceImpl,
    request: Request<GetWorkspaceRepositoryAccessRequest>,
) -> Result<Response<GetWorkspaceRepositoryAccessResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;

    let Some(access) = service.workspace_repository.as_deref() else {
        return Err(Status::failed_precondition(
            "Workspace repositories are not configured",
        ));
    };

    // The repository lives in the workspace bucket, so a deployment with no
    // S3 client configured has no fork question to ask — the grant is then
    // this task's own prefix, which is what it was before forks resolved.
    let source_task_id = match (service.s3.as_ref(), service.s3_bucket.as_ref()) {
        (Some(s3_client), Some(bucket)) => {
            service
                .workspace_restore_task_id(
                    s3_client,
                    bucket,
                    &task_id,
                    WorkspaceProbe::AnyUnder(workspace_repository_snapshots_prefix),
                )
                .await
        }
        _ => None,
    };

    let grant = access
        .grant(&task_id, source_task_id.as_deref())
        .await
        .map_err(|error| match error {
            GrantError::Sts(_) => Status::internal("Failed to grant workspace repository access"),
        })?;

    info!(
        task_id = %task_id,
        expires_at = %grant.expires_at,
        source_task_id = ?source_task_id,
        "Granted workspace repository access"
    );
    let source = grant.source;
    Ok(Response::new(GetWorkspaceRepositoryAccessResponse {
        bucket: grant.bucket,
        prefix: grant.prefix,
        region: grant.region,
        access_key_id: grant.access_key_id,
        secret_access_key: grant.secret_access_key,
        session_token: grant.session_token,
        expires_at: grant
            .expires_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        password: grant.password,
        source_prefix: source.as_ref().map(|source| source.prefix.clone()),
        source_password: source.map(|source| source.password),
    }))
}

/// Record a workspace-sync outcome reported by the sandbox as Prometheus
/// metrics. The `s3` backend transfers directly to S3, so the backend never
/// sees the bytes and must be told the size/duration/outcome to build the
/// EBS-vs-S3 comparison. Best-effort: it only touches metrics, never fails
/// the sandbox's sync. Auth: task-scoped JWT via gRPC metadata.
pub(super) async fn report_workspace_sync(
    service: &BackendSandboxServiceImpl,
    request: Request<ReportWorkspaceSyncRequest>,
) -> Result<Response<ReportWorkspaceSyncResponse>, Status> {
    let task_id = request.get_ref().task_id.clone();
    service.authorizer.authorize(&request, &task_id).await?;
    let req = request.into_inner();

    crate::sandbox::metrics::record_workspace_sync(
        &req.direction,
        &req.format,
        req.bytes.max(0) as u64,
        std::time::Duration::from_millis(req.duration_ms.max(0) as u64),
        req.ok,
    );

    // Every direction is recorded, and a failure as well as a success. The
    // sandbox only warns into its own container log, so without a durable
    // event a workspace that has stopped reaching S3 is invisible to `moat
    // events`, `moat tasks diagnose` and the UI — and the agent goes on
    // being told its working tree survives a restart when it does not. A
    // failed download is the more consequential of the two: that pod clears
    // the tree and clones, and its log dies with it.
    if let Some(task_events) = service.ingress.as_ref().map(|i| &i.task_events) {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut payload = serde_json::json!({
            "direction": req.direction,
            "format": if req.format.is_empty() { "tar" } else { &req.format },
            "bytes": req.bytes.max(0),
            "durationMs": req.duration_ms.max(0),
            "ok": req.ok,
        });
        // `error` is what every reader of this event renders, and the
        // container log holding the same reason dies with the pod. A
        // sandbox predating the field sends none, so the key is absent
        // rather than empty.
        if !req.error.is_empty() {
            payload["error"] = serde_json::Value::String(req.error.clone());
        }
        let event = InsertTaskEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            task_id: req.task_id.clone(),
            event_type: "sandbox".to_string(),
            sub_type: if req.ok {
                "workspace_synced".to_string()
            } else {
                "workspace_sync_failed".to_string()
            },
            is_error: !req.ok,
            agent_type: None,
            turn_number: None,
            payload,
            timestamp: now,
        };
        if let Err(e) = task_events.insert_event(&event).await {
            warn!(
                task_id = %req.task_id,
                direction = %req.direction,
                ok = req.ok,
                error = %e,
                "Failed to persist workspace sync marker"
            );
        }
    }

    debug!(
        task_id = %req.task_id,
        direction = %req.direction,
        bytes = req.bytes,
        duration_ms = req.duration_ms,
        ok = req.ok,
        "Recorded workspace sync report"
    );
    Ok(Response::new(ReportWorkspaceSyncResponse {}))
}

#[cfg(test)]
mod tests {
    use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;

    use super::super::test_support::service_with;
    use super::super::test_support::task_fixture;
    use super::super::test_support::tasks_returning;
    use super::super::test_support::users_regular;
    use super::*;

    use moatless_types::access_control::Scope;

    // GetWorkspaceRepositoryAccess — what an unconfigured deployment answers

    fn repository_request(task_id: &str) -> Request<GetWorkspaceRepositoryAccessRequest> {
        Request::new(GetWorkspaceRepositoryAccessRequest {
            task_id: task_id.to_string(),
        })
    }

    /// An unconfigured deployment answers `FailedPrecondition` and not
    /// `Internal`: the sandbox retries an internal error and gives up on this
    /// one, which is the right behaviour for a deployment that simply has no
    /// repository role. Which pieces are missing is
    /// `WorkspaceRepositoryAccess::new`'s question, tested where it lives.
    #[tokio::test]
    async fn a_deployment_without_a_repository_refuses_the_grant() {
        let svc = service_with(
            tasks_returning(Some(task_fixture(Scope::User, Some("owner")))),
            users_regular(),
        );

        let err = svc
            .get_workspace_repository_access(repository_request("task-1"))
            .await
            .expect_err("no repository access handle, no grant");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }
}
