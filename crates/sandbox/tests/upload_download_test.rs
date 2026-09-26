//! `download_upload` over a real gRPC connection.
//!
//! The bug this guards is a size ceiling on the wire, so the test has to put
//! bytes through an actual tonic server and client rather than call the
//! reassembly logic directly: a unary response larger than tonic's 4 MiB
//! default decoding cap is rejected by the transport, not by any code here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::pin::Pin;

use futures::Stream;
use moatless_sandbox::backend_client::{BackendClient, BackendGrpcClient};
use sandbox_proto::backend_sandbox_service_server::{
    BackendSandboxService, BackendSandboxServiceServer,
};
use sandbox_proto::{
    AcknowledgeMessagesRequest, AcknowledgeMessagesResponse, DownloadTranscriptRequest,
    DownloadUploadRequest, DownloadUploadResponse, EnqueueSandboxMessageRequest,
    EnqueueSandboxMessageResponse, GetCredentialsRequest, GetCredentialsResponse,
    GetPendingMessagesRequest, GetPendingMessagesResponse, GetSystemPromptRequest,
    GetSystemPromptResponse, GetWorkspaceRepositoryAccessRequest,
    GetWorkspaceRepositoryAccessResponse, GetWorkspaceSyncUrlsRequest,
    GetWorkspaceSyncUrlsResponse, PublishEventRequest, PublishEventResponse, RecordGitPushRequest,
    RecordGitPushResponse, RecordUnreportedCommandsRequest, RecordUnreportedCommandsResponse,
    RecoverInterruptedRunRequest, RecoverInterruptedRunResponse, ReportWorkspaceSyncRequest,
    ReportWorkspaceSyncResponse, TakeUnreportedCommandsRequest, TakeUnreportedCommandsResponse,
    TranscriptChunk, UpdateCredentialRequest, UpdateCredentialResponse, UploadChunk,
    UploadTranscriptResponse,
};
use tonic::{Request, Response, Status, Streaming};

/// 1 MiB — the chunk size the backend serves with.
const CHUNK: usize = 1024 * 1024;

/// A backend that serves one canned upload over `DownloadUploadChunked` and
/// nothing else.
struct UploadOnlyBackend {
    data: Vec<u8>,
    filename: String,
    /// Chunks to emit, overriding the split of `data`. Set to an empty vec to
    /// serve the empty stream a well-behaved server never sends.
    chunks_override: Option<Vec<UploadChunk>>,
}

impl UploadOnlyBackend {
    fn serving(data: Vec<u8>) -> Self {
        Self {
            data,
            filename: "report.pdf".to_string(),
            chunks_override: None,
        }
    }

    fn serving_chunks(chunks: Vec<UploadChunk>) -> Self {
        Self {
            data: Vec::new(),
            filename: String::new(),
            chunks_override: Some(chunks),
        }
    }
}

#[tonic::async_trait]
impl BackendSandboxService for UploadOnlyBackend {
    type DownloadUploadChunkedStream =
        Pin<Box<dyn Stream<Item = Result<UploadChunk, Status>> + Send>>;

    async fn download_upload_chunked(
        &self,
        _request: Request<DownloadUploadRequest>,
    ) -> Result<Response<Self::DownloadUploadChunkedStream>, Status> {
        let chunks = self.chunks_override.clone().unwrap_or_else(|| {
            let mut chunks: Vec<UploadChunk> = self
                .data
                .chunks(CHUNK)
                .map(|c| UploadChunk {
                    data: c.to_vec(),
                    filename: String::new(),
                    mime_type: String::new(),
                })
                .collect();
            if chunks.is_empty() {
                chunks.push(UploadChunk {
                    data: Vec::new(),
                    filename: String::new(),
                    mime_type: String::new(),
                });
            }
            if let Some(first) = chunks.first_mut() {
                first.filename.clone_from(&self.filename);
                first.mime_type = "application/pdf".to_string();
            }
            chunks
        });

        let stream = futures::stream::iter(chunks.into_iter().map(Ok::<UploadChunk, Status>));
        Ok(Response::new(Box::pin(stream)))
    }

    // --- Everything else on the service is out of this test's scope. ---

    type DownloadTranscriptStream =
        Pin<Box<dyn Stream<Item = Result<TranscriptChunk, Status>> + Send>>;

    async fn download_transcript(
        &self,
        _request: Request<DownloadTranscriptRequest>,
    ) -> Result<Response<Self::DownloadTranscriptStream>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn download_upload(
        &self,
        _request: Request<DownloadUploadRequest>,
    ) -> Result<Response<DownloadUploadResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn publish_event(
        &self,
        _request: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn get_pending_messages(
        &self,
        _request: Request<GetPendingMessagesRequest>,
    ) -> Result<Response<GetPendingMessagesResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn acknowledge_messages(
        &self,
        _request: Request<AcknowledgeMessagesRequest>,
    ) -> Result<Response<AcknowledgeMessagesResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn get_credentials(
        &self,
        _request: Request<GetCredentialsRequest>,
    ) -> Result<Response<GetCredentialsResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn get_system_prompt(
        &self,
        _request: Request<GetSystemPromptRequest>,
    ) -> Result<Response<GetSystemPromptResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn update_credential(
        &self,
        _request: Request<UpdateCredentialRequest>,
    ) -> Result<Response<UpdateCredentialResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn recover_interrupted_run(
        &self,
        _request: Request<RecoverInterruptedRunRequest>,
    ) -> Result<Response<RecoverInterruptedRunResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn upload_transcript(
        &self,
        _request: Request<Streaming<TranscriptChunk>>,
    ) -> Result<Response<UploadTranscriptResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn get_workspace_sync_urls(
        &self,
        _request: Request<GetWorkspaceSyncUrlsRequest>,
    ) -> Result<Response<GetWorkspaceSyncUrlsResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn get_workspace_repository_access(
        &self,
        _request: Request<GetWorkspaceRepositoryAccessRequest>,
    ) -> Result<Response<GetWorkspaceRepositoryAccessResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn report_workspace_sync(
        &self,
        _request: Request<ReportWorkspaceSyncRequest>,
    ) -> Result<Response<ReportWorkspaceSyncResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn record_git_push(
        &self,
        _request: Request<RecordGitPushRequest>,
    ) -> Result<Response<RecordGitPushResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn record_unreported_commands(
        &self,
        _request: Request<RecordUnreportedCommandsRequest>,
    ) -> Result<Response<RecordUnreportedCommandsResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn take_unreported_commands(
        &self,
        _request: Request<TakeUnreportedCommandsRequest>,
    ) -> Result<Response<TakeUnreportedCommandsResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn enqueue_sandbox_message(
        &self,
        _request: Request<EnqueueSandboxMessageRequest>,
    ) -> Result<Response<EnqueueSandboxMessageResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn append_trajectory(
        &self,
        _request: Request<sandbox_proto::AppendTrajectoryRequest>,
    ) -> Result<Response<sandbox_proto::AppendTrajectoryResponse>, Status> {
        Err(Status::unimplemented("not used"))
    }

    type ReadTrajectoryStream =
        Pin<Box<dyn Stream<Item = Result<sandbox_proto::ReadTrajectoryChunk, Status>> + Send>>;

    async fn read_trajectory(
        &self,
        _request: Request<sandbox_proto::ReadTrajectoryRequest>,
    ) -> Result<Response<Self::ReadTrajectoryStream>, Status> {
        Err(Status::unimplemented("not used"))
    }
}

/// Serve `backend` on an ephemeral loopback port and return a client wired to
/// it, plus the shutdown trigger.
///
/// The client is the production one, so the test exercises the real channel
/// configuration, decoding cap and call timeout included.
async fn serve(
    backend: UploadOnlyBackend,
) -> (BackendGrpcClient, tokio::sync::oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(BackendSandboxServiceServer::new(backend))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await;
    });

    let client = BackendGrpcClient::connect(&format!("http://{addr}"), None).expect("client");
    (client, shutdown_tx)
}

/// A position-dependent pattern, so a reassembly that drops, duplicates or
/// reorders a chunk fails on content rather than only on length.
fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).unwrap_or(0))
        .collect()
}

#[tokio::test]
async fn an_upload_past_the_unary_message_cap_round_trips_whole() {
    // 6 MiB: comfortably past tonic's 4 MiB default decoding cap, which is
    // where a single-message response is refused with `resource_exhausted`.
    let data = pattern(6 * CHUNK);
    let (client, _shutdown) = serve(UploadOnlyBackend::serving(data.clone())).await;

    let resp = client
        .download_upload("task-1", "file-1")
        .await
        .expect("a 6 MiB upload must round-trip");

    assert_eq!(resp.data.len(), data.len());
    assert_eq!(resp.data, data, "bytes must survive chunking in order");
    assert_eq!(resp.filename, "report.pdf");
    assert_eq!(resp.mime_type, "application/pdf");
}

#[tokio::test]
async fn a_zero_byte_upload_arrives_as_an_empty_file_with_its_name() {
    let (client, _shutdown) = serve(UploadOnlyBackend::serving(Vec::new())).await;

    let resp = client
        .download_upload("task-1", "file-1")
        .await
        .expect("an empty upload is a successful download");

    assert!(resp.data.is_empty());
    assert_eq!(
        resp.filename, "report.pdf",
        "the name must survive even with no bytes to carry it"
    );
}

#[tokio::test]
async fn an_upload_that_lands_on_a_chunk_boundary_round_trips_whole() {
    // Exactly two chunks, so the last chunk is full and no short one follows.
    let data = pattern(2 * CHUNK);
    let (client, _shutdown) = serve(UploadOnlyBackend::serving(data.clone())).await;

    let resp = client
        .download_upload("task-1", "file-1")
        .await
        .expect("a boundary-aligned upload must round-trip");

    assert_eq!(resp.data, data);
    assert_eq!(resp.filename, "report.pdf");
}

#[tokio::test]
async fn the_per_message_cap_the_chunking_exists_to_dodge_is_real() {
    // Guards the premise of every test above: serve the same 6 MiB in a single
    // message and the client's decoder refuses it. Chunking is what keeps the
    // upload under this ceiling; if this ever stops failing, the round-trip
    // tests stop proving anything.
    let oversized = vec![UploadChunk {
        data: pattern(6 * CHUNK),
        filename: "report.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
    }];
    let (client, _shutdown) = serve(UploadOnlyBackend::serving_chunks(oversized)).await;

    let err = client
        .download_upload("task-1", "file-1")
        .await
        .expect_err("6 MiB in one message must exceed the decoding cap");

    assert_eq!(err.code(), tonic::Code::OutOfRange, "{err:?}");
}

#[tokio::test]
async fn a_stream_with_no_chunks_is_a_failed_download_not_an_empty_file() {
    // A file with no name cannot be written, so "no chunks at all" has to
    // surface as an error rather than as a nameless empty upload.
    let (client, _shutdown) = serve(UploadOnlyBackend::serving_chunks(Vec::new())).await;

    let err = client
        .download_upload("task-1", "file-1")
        .await
        .expect_err("an empty stream must not read as a successful download");

    assert_eq!(err.code(), tonic::Code::Internal);
}
