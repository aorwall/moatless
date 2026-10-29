//! gRPC client wrapper for backend communication.
//!
//! A single gRPC connection to the backend's `BackendSandboxService` carries
//! event publishing and message polling. Includes an auth interceptor that
//! injects `authorization: Bearer <token>` metadata on every outbound call.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use sandbox_proto::backend_sandbox_service_client::BackendSandboxServiceClient;
use sandbox_proto::{
    AcknowledgeMessagesRequest, AppendTrajectoryRequest, AppendTrajectoryResponse,
    DownloadTranscriptRequest, DownloadUploadRequest, EnqueueSandboxMessageRequest,
    FetchPluginRequest, GetCredentialsRequest, GetCredentialsResponse, GetLastStopCauseRequest,
    GetPendingMessagesRequest, GetSystemPromptRequest, GetWorkspaceRepositoryAccessRequest,
    GetWorkspaceRepositoryAccessResponse, GetWorkspaceSyncUrlsRequest,
    GetWorkspaceSyncUrlsResponse, InterruptedTurn, PendingAgentDelivery, PublishEventRequest,
    ReadTrajectoryRequest, RecordGitPushRequest, RecordUnreportedCommandsRequest,
    RecoverInterruptedRunRequest, ReportWorkspaceSyncRequest, SessionHeader, StopCause,
    TakeUnreportedCommandsRequest, TranscriptChunk, UnreportedCommand, UpdateCredentialRequest,
};

use crate::commands::{CommandState, JournaledCommand};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{Request, Status};
use tracing::{debug, warn};

/// An uploaded file as [`BackendClient::download_upload`] reassembles it from
/// the `DownloadUploadChunked` stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadedUpload {
    pub data: Vec<u8>,
    pub filename: String,
    pub mime_type: String,
}

/// Errors that can occur when constructing a [`BackendGrpcClient`].
#[derive(Debug, thiserror::Error)]
pub enum BackendGrpcClientError {
    #[error("invalid gRPC endpoint URI: {0}")]
    InvalidUri(#[from] tonic::codegen::http::uri::InvalidUri),
}

/// A trajectory session as [`BackendClient::read_trajectory`] reads it.
#[derive(Debug, Clone, Default)]
pub struct TrajectoryRead {
    pub header: SessionHeader,
    pub lines: Vec<String>,
    /// Lines of the other harness a switch session renders after `lines`.
    pub transcode: Vec<sandbox_proto::TranscodeItem>,
}

/// Trait abstracting backend communication for testability.
///
/// The real implementation is [`BackendGrpcClient`], which overrides every
/// method. A read default answers as a backend with nothing stored, so a test
/// double implements only the calls its test exercises; a write default fails
/// as `unimplemented`, so a missed override never drops a write silently.
#[async_trait::async_trait]
pub trait BackendClient: Send + Sync {
    /// Attempt a single publish of an event to the backend.
    ///
    /// One gRPC attempt with the channel's call timeout, no internal retry;
    /// retry/backoff and ordering live in [`crate::outbox`], which owns the
    /// at-least-once guarantee.
    async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
        Err(tonic::Status::unimplemented("publish_event not supported"))
    }

    /// Fetch pending messages for a task.
    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
        Ok(vec![])
    }

    /// [`Self::get_pending_messages`], leaving `queue` messages pending when
    /// `hold_queued`: a fetch made mid-turn steers only what was sent to steer.
    async fn get_deliverable_messages(
        &self,
        task_id: &str,
        hold_queued: bool,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
        let _ = hold_queued;
        self.get_pending_messages(task_id).await
    }

    /// `run_id` identifies the agent run that consumed the messages, so the
    /// backend can put them on that run's turn.
    ///
    /// `answered_prompt` names the interactive call these messages answered,
    /// `None` on an ordinary ack (see `answered_prompt_tool_use_id`).
    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
        _answered_prompt: Option<&str>,
    ) -> Result<(), tonic::Status> {
        Err(tonic::Status::unimplemented(
            "acknowledge_messages not supported",
        ))
    }

    /// Fetch credentials for sandbox initialization.
    async fn get_credentials(
        &self,
        _task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        Ok(GetCredentialsResponse::default())
    }

    /// Push a credential rotated locally inside the sandbox back to the backend
    /// so subsequent `get_credentials` calls return the rotated value.
    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), tonic::Status> {
        Err(tonic::Status::unimplemented(
            "update_credential not supported",
        ))
    }

    /// Fetch the assembled append-system-prompt for a task. Called once per
    /// agent run; the result is cached locally as a fallback for transient gRPC
    /// failures.
    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
        Ok(String::new())
    }

    /// Fetch the bytes of an uploaded file by `file_id`, for the agent
    /// pre-fetch path that lands attached files on disk before invoking the
    /// agent. Backend-side scope-checks `file_id` against this task.
    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadedUpload, tonic::Status> {
        Err(tonic::Status::unimplemented(
            "download_upload not supported",
        ))
    }

    /// Returns `Some(turn)` when the most recent user-turn was acknowledged
    /// but never reached a terminal `agent` event; `None` when nothing needs
    /// recovery or the recovery cap is reached.
    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        Ok(None)
    }

    /// The turn [`Self::recover_interrupted_run`] would return, read without
    /// recording a recovery attempt.
    async fn probe_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        Ok(None)
    }

    /// Upload the task's transcript tar to S3 via the backend (the sandbox has
    /// no AWS credentials). The bytes are streamed in chunks. Best-effort at
    /// the call site — failures are logged, never fatal.
    async fn upload_transcript(
        &self,
        _task_id: &str,
        _tar_data: Vec<u8>,
    ) -> Result<(), tonic::Status> {
        Err(tonic::Status::unimplemented(
            "upload_transcript not supported",
        ))
    }

    /// Download the task's transcript tar from S3 via the backend, reassembling
    /// the streamed chunks. Returns an empty `Vec` when no transcript is stored
    /// yet.
    async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
        Ok(Vec::new())
    }

    /// Download one of the task's plugins as the gzip tar `FetchPlugin`
    /// describes, reassembled from its chunks, unless it is still at
    /// `known_version`.
    ///
    /// Default impl reports the RPC unsupported, so test doubles that start no
    /// plugins need not implement it.
    async fn fetch_plugin(
        &self,
        _task_id: &str,
        _plugin_name: &str,
        _known_version: Option<&str>,
    ) -> Result<PluginFetch, tonic::Status> {
        Err(tonic::Status::unimplemented("plugin fetch not supported"))
    }

    /// Mint short-lived presigned S3 PUT/GET URLs for this task's workspace
    /// archive, so the backend never sees the tar's bytes.
    ///
    /// Default impl reports the feature unsupported, so test doubles that
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

    /// Report a direct-to-S3 transfer's outcome for the backend's metrics and
    /// sandbox event (`format`: `ReportWorkspaceSyncRequest.format`).
    ///
    /// `error` is `None` on success, else the failure logged here: the only
    /// copy that outlives the pod. Fire-and-forget; default impl is a no-op.
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
    /// The error is returned, not logged: the caller leaves an unclaimed sha
    /// out of its claimed set so the next push refiles it.
    async fn record_git_push(
        &self,
        _task_id: &str,
        _owner: &str,
        _repo: &str,
        _sha: &str,
    ) -> Result<(), tonic::Status> {
        Err(tonic::Status::unimplemented("git push claim not supported"))
    }

    /// Mirror commands owed an outcome onto the Sandbox row so a replacement
    /// pod can name them after a lost snapshot.
    ///
    /// `revision` comes from [`crate::commands::CommandRegistry`]; fire-and-
    /// forget — the journal survives a backend outage.
    async fn record_unreported_commands(
        &self,
        _task_id: &str,
        _commands: &[JournaledCommand],
        _revision: i64,
    ) {
    }

    /// Read and clear what a prior pod recorded, so the loss is announced
    /// exactly once however many times the pod restarts.
    async fn take_unreported_commands(
        &self,
        _task_id: &str,
    ) -> Result<Vec<JournaledCommand>, tonic::Status> {
        Err(tonic::Status::unimplemented(
            "unreported commands not supported",
        ))
    }

    /// Why the task's previous container stopped; see the `GetLastStopCause`
    /// RPC for when it may be called.
    async fn get_last_stop_cause(
        &self,
        _task_id: &str,
    ) -> Result<Option<StopCause>, tonic::Status> {
        Ok(None)
    }

    /// Queue a user message this sandbox wrote for its own Task, on the same
    /// path an external send takes.
    ///
    /// Returns the queued uuid, or `None` when refused (already queued, or Task
    /// closed); pass `idempotency_key` so a same-text resend is not dropped.
    async fn enqueue_sandbox_message(
        &self,
        _task_id: &str,
        _text: &str,
        _idempotency_key: Option<&str>,
    ) -> Result<Option<String>, tonic::Status> {
        Err(tonic::Status::unimplemented(
            "sandbox message enqueue not supported",
        ))
    }

    /// Record harness session lines into the Task's trajectory.
    ///
    /// Default impl answers `Unimplemented`, which the recorder reads as
    /// recording off.
    async fn append_trajectory(
        &self,
        _req: AppendTrajectoryRequest,
    ) -> Result<AppendTrajectoryResponse, tonic::Status> {
        Err(tonic::Status::unimplemented("trajectory not supported"))
    }

    /// Read a trajectory session whole: its header, every line root-first,
    /// and the items a switch session renders after them. `NotFound`,
    /// `DataLoss` and `Unimplemented` send the caller to the transcript
    /// archive.
    async fn read_trajectory(
        &self,
        _req: ReadTrajectoryRequest,
    ) -> Result<TrajectoryRead, tonic::Status> {
        Err(tonic::Status::unimplemented("trajectory not supported"))
    }

    /// Replace the Bearer token injected on subsequent gRPC calls.
    ///
    /// Default no-op so test doubles need not implement it; see
    /// [`AuthInterceptor`] for the bootstrap-to-user-token swap this backs.
    fn update_auth_token(&self, _token: String) {}

    /// The Bearer token the next gRPC call would carry: the pod's task token
    /// once the first `GetCredentials` has swapped it in.
    ///
    /// Default `None` so test doubles need not implement it.
    fn auth_token(&self) -> Option<String> {
        None
    }
}

/// What [`BackendClient::fetch_plugin`] answered.
#[derive(Debug)]
pub enum PluginFetch {
    /// The plugin is still at the version the caller named.
    Unchanged,
    Archive {
        version: String,
        data: Vec<u8>,
    },
}

/// Auth interceptor that injects a Bearer token on every outbound gRPC call.
///
/// The token lives behind a shared `RwLock` so it can be swapped at runtime:
/// the sandbox boots with a bootstrap token and swaps in the long-lived user
/// token after its first `GetCredentials`, without reconnecting.
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
/// Losing `GetWorkspaceSyncUrls` on a restore clones over the snapshot, which
/// is worth waiting past [`RPC_TIMEOUT`] for.
const WORKSPACE_RPC_TIMEOUT: Duration = Duration::from_mins(1);

/// Deadline for `FetchPlugin`, on [`BackendGrpcClient::plugins`]: past the
/// backend's 2-minute archive download, which a cold cache waits on before
/// the response starts.
const PLUGIN_RPC_TIMEOUT: Duration = Duration::from_mins(3);

/// Wrapper around `BackendSandboxServiceClient` with auth interceptor.
#[derive(Clone)]
pub struct BackendGrpcClient {
    client: BackendSandboxServiceClient<InterceptedService<Channel, AuthInterceptor>>,
    /// The same service over a channel carrying [`WORKSPACE_RPC_TIMEOUT`].
    ///
    /// A second channel, not a per-call deadline: `Endpoint::timeout` applies
    /// to every request on the channel and a longer `grpc-timeout` header
    /// cannot outlive it.
    workspace: BackendSandboxServiceClient<InterceptedService<Channel, AuthInterceptor>>,
    /// The same service over a channel carrying [`PLUGIN_RPC_TIMEOUT`].
    plugins: BackendSandboxServiceClient<InterceptedService<Channel, AuthInterceptor>>,
    /// Shared handle to the interceptor's token so it can be swapped at
    /// runtime.
    auth_token: Arc<RwLock<Option<String>>>,
}

impl BackendGrpcClient {
    /// Create a new client with lazy connection to the backend gRPC server.
    ///
    /// `connect_lazy()` returns immediately — the connection is established on
    /// the first RPC call, letting the sandbox start before the backend gRPC
    /// server is ready. See [`AuthInterceptor`] for how `auth_token` is used.
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
        let limited = |client: BackendSandboxServiceClient<_>| {
            let limit = sandbox_proto::client::MAX_GRPC_MESSAGE_SIZE;
            client
                .max_decoding_message_size(limit)
                .max_encoding_message_size(limit)
        };
        let client = limited(BackendSandboxServiceClient::with_interceptor(
            endpoint(RPC_TIMEOUT)?,
            interceptor,
        ));
        let workspace = limited(BackendSandboxServiceClient::with_interceptor(
            endpoint(WORKSPACE_RPC_TIMEOUT)?,
            AuthInterceptor {
                token: Arc::clone(&auth_token),
            },
        ));

        let plugins = limited(BackendSandboxServiceClient::with_interceptor(
            endpoint(PLUGIN_RPC_TIMEOUT)?,
            AuthInterceptor {
                token: Arc::clone(&auth_token),
            },
        ));

        Ok(Self {
            client,
            workspace,
            plugins,
            auth_token,
        })
    }
}

#[async_trait::async_trait]
impl BackendClient for BackendGrpcClient {
    fn auth_token(&self) -> Option<String> {
        self.auth_token.read().ok()?.clone()
    }

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
        self.get_deliverable_messages(task_id, false).await
    }

    async fn get_deliverable_messages(
        &self,
        task_id: &str,
        hold_queued: bool,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .get_pending_messages(GetPendingMessagesRequest {
                task_id: task_id.to_string(),
                auth_token: String::new(), // kept for proto compat, auth via interceptor
                hold_queued,
            })
            .await?
            .into_inner();
        // The turn's new driver: the token this client holds is refused from now.
        if !resp.user_auth_token.is_empty() {
            self.update_auth_token(resp.user_auth_token);
        }
        Ok(resp.deliveries)
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
    ) -> Result<DownloadedUpload, tonic::Status> {
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
        let mut reassembled: Option<DownloadedUpload> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            match reassembled {
                // The first chunk names the file; the rest carry bytes only.
                None => {
                    reassembled = Some(DownloadedUpload {
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
                probe_only: false,
            })
            .await?;
        Ok(resp.into_inner().turn)
    }

    async fn probe_interrupted_run(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .recover_interrupted_run(RecoverInterruptedRunRequest {
                task_id: task_id.to_string(),
                probe_only: true,
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

    async fn fetch_plugin(
        &self,
        task_id: &str,
        plugin_name: &str,
        known_version: Option<&str>,
    ) -> Result<PluginFetch, tonic::Status> {
        use futures::StreamExt;

        let mut client = self.plugins.clone();
        let resp = client
            .fetch_plugin(FetchPluginRequest {
                task_id: task_id.to_string(),
                plugin_name: plugin_name.to_string(),
                known_version: known_version.unwrap_or_default().to_string(),
            })
            .await?;

        let mut stream = resp.into_inner();
        let (mut version, mut data) = (String::new(), Vec::new());
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if chunk.unchanged {
                return Ok(PluginFetch::Unchanged);
            }
            if version.is_empty() {
                version = chunk.version;
            }
            data.extend_from_slice(&chunk.data);
        }
        Ok(PluginFetch::Archive { version, data })
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

    async fn append_trajectory(
        &self,
        req: AppendTrajectoryRequest,
    ) -> Result<AppendTrajectoryResponse, tonic::Status> {
        let mut client = self.client.clone();
        Ok(client.append_trajectory(req).await?.into_inner())
    }

    async fn read_trajectory(
        &self,
        req: ReadTrajectoryRequest,
    ) -> Result<TrajectoryRead, tonic::Status> {
        use futures::StreamExt;
        use sandbox_proto::read_trajectory_chunk::Body;

        let mut client = self.client.clone();
        let mut stream = client.read_trajectory(req).await?.into_inner();
        let mut header = None;
        let mut lines = Vec::new();
        let mut transcode = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk?.body {
                Some(Body::Header(h)) => header = Some(h),
                Some(Body::Line(line)) => lines.push(line),
                Some(Body::Transcode(item)) => transcode.push(item),
                None => {}
            }
        }
        let header = header.ok_or_else(|| Status::data_loss("trajectory stream had no header"))?;
        Ok(TrajectoryRead {
            header,
            lines,
            transcode,
        })
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

    async fn record_unreported_commands(
        &self,
        task_id: &str,
        commands: &[JournaledCommand],
        revision: i64,
    ) {
        let mut client = self.client.clone();
        if let Err(e) = client
            .record_unreported_commands(RecordUnreportedCommandsRequest {
                task_id: task_id.to_string(),
                commands: commands.iter().map(unreported_entry).collect(),
                revision,
            })
            .await
        {
            debug!(task_id, error = %e, "Failed to record the unreported commands (non-fatal)");
        }
    }

    async fn get_last_stop_cause(&self, task_id: &str) -> Result<Option<StopCause>, Status> {
        let mut client = self.client.clone();
        let response = client
            .get_last_stop_cause(GetLastStopCauseRequest {
                task_id: task_id.to_string(),
            })
            .await?;
        Ok(response.into_inner().cause)
    }

    async fn take_unreported_commands(
        &self,
        task_id: &str,
    ) -> Result<Vec<JournaledCommand>, Status> {
        let mut client = self.client.clone();
        let response = client
            .take_unreported_commands(TakeUnreportedCommandsRequest {
                task_id: task_id.to_string(),
            })
            .await?;
        Ok(response
            .into_inner()
            .commands
            .into_iter()
            .map(journaled_command)
            .collect())
    }

    async fn enqueue_sandbox_message(
        &self,
        task_id: &str,
        text: &str,
        idempotency_key: Option<&str>,
    ) -> Result<Option<String>, Status> {
        let mut client = self.client.clone();
        let uuid = client
            .enqueue_sandbox_message(EnqueueSandboxMessageRequest {
                task_id: task_id.to_string(),
                text: text.to_string(),
                idempotency_key: idempotency_key.unwrap_or_default().to_string(),
            })
            .await?
            .into_inner()
            .message_uuid;
        Ok((!uuid.is_empty()).then_some(uuid))
    }
}

fn unreported_entry(command: &JournaledCommand) -> UnreportedCommand {
    UnreportedCommand {
        id: command.id.clone(),
        command: command.command.clone(),
        label: command.label.clone(),
        started_at_unix_ms: command.started_at_unix_ms,
        state: command.state.as_str().to_string(),
        exit_code: command.exit_code,
    }
}

fn journaled_command(entry: UnreportedCommand) -> JournaledCommand {
    JournaledCommand {
        id: entry.id,
        command: entry.command,
        label: entry.label,
        started_at_unix_ms: entry.started_at_unix_ms,
        state: CommandState::from_wire(&entry.state),
        exit_code: entry.exit_code,
        // The row mirror carries no log: it exists to survive a restore that
        // skips the workspace, and log text does not belong on a backend row.
        // `merge_lost_commands` prefers the journal, which does carry the tail.
        log_tail: None,
    }
}
