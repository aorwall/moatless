//! gRPC client wrapper for backend communication.
//!
//! A single gRPC connection to the backend's `BackendSandboxService` carries
//! event publishing and message polling.
//! Includes an auth interceptor that injects `authorization: Bearer <token>`
//! metadata on every outbound call.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use sandbox_proto::backend_sandbox_service_client::BackendSandboxServiceClient;
use sandbox_proto::{
    AcknowledgeMessagesRequest, DownloadTranscriptRequest, DownloadUploadRequest,
    DownloadUploadResponse, GetCredentialsRequest, GetCredentialsResponse,
    GetPendingMessagesRequest, GetSystemPromptRequest, GetWorkspaceObjectUrlsRequest,
    GetWorkspaceRepositoryAccessRequest, GetWorkspaceRepositoryAccessResponse,
    GetWorkspaceSyncUrlsRequest, GetWorkspaceSyncUrlsResponse, InterruptedTurn,
    PendingAgentDelivery, PublishEventRequest, RecordGitPushRequest, RecoverInterruptedRunRequest,
    ReportWorkspaceSyncRequest, TranscriptChunk, UpdateCredentialRequest,
};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{Request, Status};
use tracing::{debug, warn};

/// Errors that can occur when constructing a [`BackendGrpcClient`].
#[derive(Debug, thiserror::Error)]
pub enum BackendGrpcClientError {
    #[error("invalid gRPC endpoint URI: {0}")]
    InvalidUri(#[from] tonic::codegen::http::uri::InvalidUri),
}

/// Trait abstracting backend communication for testability.
///
/// The real implementation is [`BackendGrpcClient`]; tests can provide a mock.
#[async_trait::async_trait]
pub trait BackendClient: Send + Sync {
    /// Attempt a single publish of an event to the backend.
    ///
    /// One gRPC attempt with the channel's call timeout — no internal retry.
    /// Retry/backoff and ordering live in [`crate::outbox`], which owns the
    /// at-least-once guarantee; this method just reports the attempt's outcome.
    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), tonic::Status>;

    /// Fetch pending messages for a task.
    async fn get_pending_messages(
        &self,
        task_id: &str,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status>;

    /// Acknowledge that messages have been delivered to a session.
    /// `run_id` identifies the agent run that consumed the messages; the
    /// backend uses it to put them on that run's turn.
    ///
    /// `answered_prompt` names the interactive call these messages answered,
    /// and is `None` on an ordinary ack. See
    /// `AcknowledgeMessagesRequest.answered_prompt_tool_use_id`.
    async fn acknowledge_messages(
        &self,
        task_id: &str,
        message_uuids: Vec<String>,
        session_id: &str,
        run_id: &str,
        answered_prompt: Option<&str>,
    ) -> Result<(), tonic::Status>;

    /// Fetch credentials for sandbox initialization.
    async fn get_credentials(&self, task_id: &str)
    -> Result<GetCredentialsResponse, tonic::Status>;

    /// Push a credential rotated locally inside the sandbox back to the backend
    /// so subsequent `get_credentials` calls return the rotated value.
    async fn update_credential(
        &self,
        task_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), tonic::Status>;

    /// Fetch the assembled append-system-prompt for a task. Called once per
    /// agent run; the result is cached locally as a fallback for transient
    /// gRPC failures.
    async fn get_system_prompt(&self, task_id: &str) -> Result<String, tonic::Status>;

    /// Fetch the bytes of an uploaded file by `file_id`. Used by the agent
    /// pre-fetch path to land attached files on disk before invoking the
    /// agent. Backend-side scope-checks the `file_id` against this task.
    ///
    /// The bytes arrive as a chunk stream and are reassembled here, so the
    /// upload's size is bounded by the sandbox's memory rather than by any
    /// per-message limit on the wire.
    async fn download_upload(
        &self,
        task_id: &str,
        file_id: &str,
    ) -> Result<DownloadUploadResponse, tonic::Status>;

    /// Ask the backend whether the previous agent run for this task was
    /// interrupted. Returns `Some(turn)` when the most recent user-turn was
    /// acknowledged but never reached a terminal `agent` event, or `None`
    /// when nothing needs recovery (or the recovery cap has been reached).
    async fn recover_interrupted_run(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status>;

    /// Upload the task's transcript tar to S3 via the backend (the sandbox has no
    /// AWS credentials). The bytes are streamed in chunks. Best-effort at the call
    /// site — failures are logged, never fatal.
    async fn upload_transcript(
        &self,
        task_id: &str,
        tar_data: Vec<u8>,
    ) -> Result<(), tonic::Status>;

    /// Download the task's transcript tar from S3 via the backend, reassembling the
    /// streamed chunks. Returns an empty `Vec` when no transcript is stored yet.
    async fn download_transcript(&self, task_id: &str) -> Result<Vec<u8>, tonic::Status>;

    /// Mint short-lived presigned S3 PUT/GET URLs for this task's workspace
    /// archive (the `s3` storage backend). The sandbox transfers the workspace
    /// tar directly to/from S3 with these URLs — the backend never sees the
    /// bytes. Default impl reports the feature unsupported so test doubles that
    /// don't exercise workspace sync need not implement it.
    async fn get_workspace_sync_urls(
        &self,
        _task_id: &str,
    ) -> Result<GetWorkspaceSyncUrlsResponse, tonic::Status> {
        Err(tonic::Status::unimplemented("workspace sync not supported"))
    }

    /// Fetch direct S3 access to the task's workspace `restic` repository, plus
    /// the password that opens it.
    ///
    /// Default impl reports the feature unsupported, which is also what a
    /// backend without a repository role answers.
    async fn get_workspace_repository_access(
        &self,
        _task_id: &str,
    ) -> Result<GetWorkspaceRepositoryAccessResponse, tonic::Status> {
        Err(tonic::Status::unimplemented(
            "workspace repositories not supported",
        ))
    }

    /// Mint one presigned S3 URL per workspace-relative path for the file
    /// mirror, in the requested order. `operation` is `"put"`, `"get"` or
    /// `"delete"`; `restore_source` sends a fork's reads to the source task's
    /// mirror when it has none of its own, and is ignored for the other two.
    ///
    /// Default impl reports the feature unsupported, which is also what a
    /// backend predating the mirror answers — [`crate::init::workspace_mirror`]
    /// reads that as "write a tar instead".
    async fn get_workspace_object_urls(
        &self,
        _task_id: &str,
        _paths: &[String],
        _operation: &str,
        _restore_source: bool,
    ) -> Result<Vec<String>, tonic::Status> {
        Err(tonic::Status::unimplemented(
            "workspace file mirror not supported",
        ))
    }

    /// Report the outcome of a direct-to-S3 workspace transfer so the backend
    /// can record comparison metrics and publish the sandbox event. The legal
    /// `format` values are on `ReportWorkspaceSyncRequest.format` in
    /// `sandbox.proto`. `error` is `None` on success, and otherwise the
    /// failure this sandbox logged — the event is the only copy that outlives
    /// the pod, so a caller that has a reason must pass it.
    /// Fire-and-forget; the default impl is a no-op.
    async fn report_workspace_sync(
        &self,
        _task_id: &str,
        _direction: &str,
        _format: &str,
        _bytes: u64,
        _duration_ms: u64,
        _error: Option<&str>,
    ) {
    }

    /// Claim a sha this task pushed to `owner/repo`, so GitHub's
    /// `pull_request.synchronize` webhook for it is not delivered back here.
    ///
    /// The error is returned rather than logged: the caller leaves an unclaimed
    /// sha out of its claimed set so the next push refiles it.
    async fn record_git_push(
        &self,
        _task_id: &str,
        _owner: &str,
        _repo: &str,
        _sha: &str,
    ) -> Result<(), tonic::Status> {
        Err(tonic::Status::unimplemented("git push claim not supported"))
    }

    /// Replace the Bearer token injected on subsequent gRPC calls.
    ///
    /// The sandbox starts with a short-lived *bootstrap* token (enough to
    /// authenticate the initial `GetCredentials` call) and swaps to the
    /// long-lived user token delivered in that response. Default no-op so test
    /// doubles need not implement it.
    fn update_auth_token(&self, _token: String) {}
}

/// Auth interceptor that injects a Bearer token on every outbound gRPC call.
///
/// The token lives behind a shared `RwLock` so it can be swapped at runtime:
/// the sandbox boots with a bootstrap token and, after its first
/// `GetCredentials`, swaps in the long-lived user token without reconnecting.
#[derive(Clone)]
pub struct AuthInterceptor {
    token: Arc<RwLock<Option<String>>>,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        // Read-lock is brief and never held across an await (this is a sync
        // interceptor), so a std RwLock is fine here.
        let token = self
            .token
            .read()
            .map_err(|_| Status::internal("Auth token lock poisoned"))?
            .clone();
        if let Some(token) = token {
            let header_value = format!("Bearer {token}")
                .parse()
                .map_err(|_| Status::internal("Invalid auth token format"))?;
            request.metadata_mut().insert("authorization", header_value);
        }
        Ok(request)
    }
}

/// Deadline for an RPC other than a workspace-sync one.
///
/// Sized for a call the sandbox can afford to lose: `publish_event` shares the
/// channel, and one in-order drainer stalls behind whatever it waits on.
const RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadline for the workspace-sync RPCs, on [`BackendGrpcClient::workspace`].
///
/// `GetWorkspaceObjectUrls` presigns a batch of up to 500 keys, so it is the one
/// call whose server-side work scales with the request. Losing it on a restore
/// clones over the snapshot, which is worth waiting past [`RPC_TIMEOUT`] for.
const WORKSPACE_RPC_TIMEOUT: Duration = Duration::from_mins(1);

/// Wrapper around `BackendSandboxServiceClient` with auth interceptor.
#[derive(Clone)]
pub struct BackendGrpcClient {
    client: BackendSandboxServiceClient<InterceptedService<Channel, AuthInterceptor>>,
    /// The same service over a channel carrying [`WORKSPACE_RPC_TIMEOUT`]. A
    /// second channel rather than a per-call deadline: `Endpoint::timeout`
    /// applies to every request on the channel and a longer `grpc-timeout`
    /// header cannot outlive it.
    workspace: BackendSandboxServiceClient<InterceptedService<Channel, AuthInterceptor>>,
    /// Shared handle to the interceptor's token so it can be swapped at runtime.
    auth_token: Arc<RwLock<Option<String>>>,
}

impl BackendGrpcClient {
    /// Create a new client with lazy connection to the backend gRPC server.
    ///
    /// `connect_lazy()` returns immediately — the actual TCP/HTTP2 connection
    /// is established on the first RPC call. This lets the sandbox start
    /// before the backend gRPC server is ready.
    ///
    /// The `auth_token` is injected as a Bearer token on every outbound call and
    /// can later be replaced via [`BackendClient::update_auth_token`].
    pub fn connect(url: &str, auth_token: Option<String>) -> Result<Self, BackendGrpcClientError> {
        let endpoint = |timeout| {
            Channel::from_shared(url.to_string()).map(|endpoint| {
                endpoint
                    .connect_timeout(Duration::from_secs(5))
                    .timeout(timeout)
                    .http2_keep_alive_interval(Duration::from_secs(30))
                    .keep_alive_timeout(Duration::from_secs(10))
                    .connect_lazy()
            })
        };

        let auth_token = Arc::new(RwLock::new(auth_token));
        let interceptor = AuthInterceptor {
            token: Arc::clone(&auth_token),
        };
        let client =
            BackendSandboxServiceClient::with_interceptor(endpoint(RPC_TIMEOUT)?, interceptor);
        let workspace = BackendSandboxServiceClient::with_interceptor(
            endpoint(WORKSPACE_RPC_TIMEOUT)?,
            AuthInterceptor {
                token: Arc::clone(&auth_token),
            },
        );

        Ok(Self {
            client,
            workspace,
            auth_token,
        })
    }
}

#[async_trait::async_trait]
impl BackendClient for BackendGrpcClient {
    fn update_auth_token(&self, token: String) {
        if let Ok(mut guard) = self.auth_token.write() {
            *guard = Some(token);
            debug!("Swapped backend gRPC auth token");
        } else {
            warn!("Auth token lock poisoned; keeping previous token");
        }
    }

    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let resp = client.publish_event(req).await?;
        debug!(
            event_id = %resp.into_inner().event_id,
            "Event published via gRPC"
        );
        Ok(())
    }

    async fn get_pending_messages(
        &self,
        task_id: &str,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .get_pending_messages(GetPendingMessagesRequest {
                task_id: task_id.to_string(),
                auth_token: String::new(), // kept for proto compat, auth via interceptor
            })
            .await?;

        Ok(resp.into_inner().deliveries)
    }

    async fn acknowledge_messages(
        &self,
        task_id: &str,
        message_uuids: Vec<String>,
        session_id: &str,
        run_id: &str,
        answered_prompt: Option<&str>,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        client
            .acknowledge_messages(AcknowledgeMessagesRequest {
                task_id: task_id.to_string(),
                message_uuids,
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                answered_prompt_tool_use_id: answered_prompt.unwrap_or_default().to_string(),
            })
            .await?;
        Ok(())
    }

    async fn get_credentials(
        &self,
        task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .get_credentials(GetCredentialsRequest {
                task_id: task_id.to_string(),
            })
            .await?;

        Ok(resp.into_inner())
    }

    async fn update_credential(
        &self,
        task_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        client
            .update_credential(UpdateCredentialRequest {
                task_id: task_id.to_string(),
                key: key.to_string(),
                value: value.to_string(),
            })
            .await?;
        Ok(())
    }

    async fn get_system_prompt(&self, task_id: &str) -> Result<String, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .get_system_prompt(GetSystemPromptRequest {
                task_id: task_id.to_string(),
            })
            .await?;
        Ok(resp.into_inner().system_prompt)
    }

    async fn download_upload(
        &self,
        task_id: &str,
        file_id: &str,
    ) -> Result<DownloadUploadResponse, tonic::Status> {
        use futures::StreamExt;

        let mut client = self.client.clone();
        let resp = client
            .download_upload_chunked(DownloadUploadRequest {
                task_id: task_id.to_string(),
                file_id: file_id.to_string(),
            })
            .await?;

        // The channel's call timeout covers this await — tonic applies it to the
        // response future, which resolves on the response headers — but not to
        // the chunk stream below, which is bounded only by the HTTP/2 keep-alive.
        let mut stream = resp.into_inner();
        let mut reassembled: Option<DownloadUploadResponse> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            match reassembled {
                // The first chunk names the file; the rest carry bytes only.
                None => {
                    reassembled = Some(DownloadUploadResponse {
                        data: chunk.data,
                        filename: chunk.filename,
                        mime_type: chunk.mime_type,
                    });
                }
                Some(ref mut acc) => acc.data.extend_from_slice(&chunk.data),
            }
        }

        // An empty upload is still one chunk, so no chunks at all means the
        // server never sent the metadata — treat that as a failed download
        // rather than as an empty file with no name.
        reassembled.ok_or_else(|| {
            tonic::Status::internal("DownloadUploadChunked returned an empty stream")
        })
    }

    async fn recover_interrupted_run(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .recover_interrupted_run(RecoverInterruptedRunRequest {
                task_id: task_id.to_string(),
            })
            .await?;
        Ok(resp.into_inner().turn)
    }

    async fn upload_transcript(
        &self,
        task_id: &str,
        tar_data: Vec<u8>,
    ) -> Result<(), tonic::Status> {
        // 1 MiB chunks keep each message well under tonic's default cap. The
        // first chunk carries the task_id; the backend reads it once and
        // ignores it on later chunks.
        const CHUNK_SIZE: usize = 1024 * 1024;
        let task_id = task_id.to_string();
        let chunks: Vec<TranscriptChunk> = if tar_data.is_empty() {
            vec![TranscriptChunk {
                task_id,
                data: Vec::new(),
            }]
        } else {
            tar_data
                .chunks(CHUNK_SIZE)
                .enumerate()
                .map(|(i, c)| TranscriptChunk {
                    task_id: if i == 0 {
                        task_id.clone()
                    } else {
                        String::new()
                    },
                    data: c.to_vec(),
                })
                .collect()
        };

        let mut client = self.client.clone();
        client
            .upload_transcript(futures::stream::iter(chunks))
            .await?;
        Ok(())
    }

    async fn download_transcript(&self, task_id: &str) -> Result<Vec<u8>, tonic::Status> {
        use futures::StreamExt;

        let mut client = self.client.clone();
        let resp = client
            .download_transcript(DownloadTranscriptRequest {
                task_id: task_id.to_string(),
            })
            .await?;

        let mut stream = resp.into_inner();
        let mut data = Vec::new();
        while let Some(chunk) = stream.next().await {
            data.extend_from_slice(&chunk?.data);
        }
        Ok(data)
    }

    async fn get_workspace_sync_urls(
        &self,
        task_id: &str,
    ) -> Result<GetWorkspaceSyncUrlsResponse, tonic::Status> {
        let mut client = self.workspace.clone();
        let resp = client
            .get_workspace_sync_urls(GetWorkspaceSyncUrlsRequest {
                task_id: task_id.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    async fn get_workspace_repository_access(
        &self,
        task_id: &str,
    ) -> Result<GetWorkspaceRepositoryAccessResponse, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .get_workspace_repository_access(GetWorkspaceRepositoryAccessRequest {
                task_id: task_id.to_string(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    async fn get_workspace_object_urls(
        &self,
        task_id: &str,
        paths: &[String],
        operation: &str,
        restore_source: bool,
    ) -> Result<Vec<String>, tonic::Status> {
        let mut client = self.workspace.clone();
        let resp = client
            .get_workspace_object_urls(GetWorkspaceObjectUrlsRequest {
                task_id: task_id.to_string(),
                paths: paths.to_vec(),
                operation: operation.to_string(),
                restore_source,
            })
            .await?;
        let urls = resp.into_inner().urls;
        if urls.len() != paths.len() {
            return Err(tonic::Status::internal(format!(
                "backend returned {} URLs for {} paths",
                urls.len(),
                paths.len()
            )));
        }
        Ok(urls)
    }

    async fn report_workspace_sync(
        &self,
        task_id: &str,
        direction: &str,
        format: &str,
        bytes: u64,
        duration_ms: u64,
        error: Option<&str>,
    ) {
        let mut client = self.client.clone();
        if let Err(e) = client
            .report_workspace_sync(ReportWorkspaceSyncRequest {
                task_id: task_id.to_string(),
                direction: direction.to_string(),
                format: format.to_string(),
                bytes: bytes as i64,
                duration_ms: duration_ms as i64,
                ok: error.is_none(),
                error: error.unwrap_or_default().to_string(),
            })
            .await
        {
            debug!(task_id, error = %e, "Failed to report workspace sync (non-fatal)");
        }
    }

    async fn record_git_push(
        &self,
        task_id: &str,
        owner: &str,
        repo: &str,
        sha: &str,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        client
            .record_git_push(RecordGitPushRequest {
                task_id: task_id.to_string(),
                owner: owner.to_string(),
                repo: repo.to_string(),
                sha: sha.to_string(),
            })
            .await?;
        Ok(())
    }
}
