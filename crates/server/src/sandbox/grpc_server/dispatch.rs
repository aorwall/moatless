//! The `BackendSandboxService` surface: every method this transport serves.
//!
//! Each arm delegates to the sibling named for the surface it answers for, so
//! this list is the whole inbound API and holds no body of its own.

use tonic::{Request, Response, Status, Streaming};

use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;
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
    TranscriptChunk, UpdateCredentialRequest, UpdateCredentialResponse, UploadTranscriptResponse,
};

use super::state::BackendSandboxServiceImpl;
use super::{
    commands, credentials, events, git_push, messages, transcripts, uploads, workspace_sync,
};

#[tonic::async_trait]
impl BackendSandboxService for BackendSandboxServiceImpl {
    type DownloadUploadChunkedStream = uploads::UploadChunkStream;
    type DownloadTranscriptStream = transcripts::TranscriptChunkStream;

    async fn publish_event(
        &self,
        request: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        events::publish_event(self, request).await
    }

    async fn get_pending_messages(
        &self,
        request: Request<GetPendingMessagesRequest>,
    ) -> Result<Response<GetPendingMessagesResponse>, Status> {
        messages::get_pending_messages(self, request).await
    }

    async fn acknowledge_messages(
        &self,
        request: Request<AcknowledgeMessagesRequest>,
    ) -> Result<Response<AcknowledgeMessagesResponse>, Status> {
        messages::acknowledge_messages(self, request).await
    }

    async fn get_credentials(
        &self,
        request: Request<GetCredentialsRequest>,
    ) -> Result<Response<GetCredentialsResponse>, Status> {
        credentials::get_credentials(self, request).await
    }

    async fn update_credential(
        &self,
        request: Request<UpdateCredentialRequest>,
    ) -> Result<Response<UpdateCredentialResponse>, Status> {
        credentials::update_credential(self, request).await
    }

    async fn get_system_prompt(
        &self,
        request: Request<GetSystemPromptRequest>,
    ) -> Result<Response<GetSystemPromptResponse>, Status> {
        messages::get_system_prompt(self, request).await
    }

    async fn download_upload(
        &self,
        request: Request<DownloadUploadRequest>,
    ) -> Result<Response<DownloadUploadResponse>, Status> {
        uploads::download_upload(self, request).await
    }

    async fn download_upload_chunked(
        &self,
        request: Request<DownloadUploadRequest>,
    ) -> Result<Response<Self::DownloadUploadChunkedStream>, Status> {
        uploads::download_upload_chunked(self, request).await
    }

    async fn upload_transcript(
        &self,
        request: Request<Streaming<TranscriptChunk>>,
    ) -> Result<Response<UploadTranscriptResponse>, Status> {
        transcripts::upload_transcript(self, request).await
    }

    async fn download_transcript(
        &self,
        request: Request<DownloadTranscriptRequest>,
    ) -> Result<Response<Self::DownloadTranscriptStream>, Status> {
        transcripts::download_transcript(self, request).await
    }

    async fn get_workspace_sync_urls(
        &self,
        request: Request<GetWorkspaceSyncUrlsRequest>,
    ) -> Result<Response<GetWorkspaceSyncUrlsResponse>, Status> {
        workspace_sync::get_workspace_sync_urls(self, request).await
    }

    async fn get_workspace_repository_access(
        &self,
        request: Request<GetWorkspaceRepositoryAccessRequest>,
    ) -> Result<Response<GetWorkspaceRepositoryAccessResponse>, Status> {
        workspace_sync::get_workspace_repository_access(self, request).await
    }

    async fn report_workspace_sync(
        &self,
        request: Request<ReportWorkspaceSyncRequest>,
    ) -> Result<Response<ReportWorkspaceSyncResponse>, Status> {
        workspace_sync::report_workspace_sync(self, request).await
    }

    async fn record_git_push(
        &self,
        request: Request<RecordGitPushRequest>,
    ) -> Result<Response<RecordGitPushResponse>, Status> {
        git_push::record_git_push(self, request).await
    }

    async fn record_unreported_commands(
        &self,
        request: Request<RecordUnreportedCommandsRequest>,
    ) -> Result<Response<RecordUnreportedCommandsResponse>, Status> {
        commands::record_unreported_commands(self, request).await
    }

    async fn take_unreported_commands(
        &self,
        request: Request<TakeUnreportedCommandsRequest>,
    ) -> Result<Response<TakeUnreportedCommandsResponse>, Status> {
        commands::take_unreported_commands(self, request).await
    }

    async fn enqueue_sandbox_message(
        &self,
        request: Request<EnqueueSandboxMessageRequest>,
    ) -> Result<Response<EnqueueSandboxMessageResponse>, Status> {
        messages::enqueue_sandbox_message(self, request).await
    }

    async fn recover_interrupted_run(
        &self,
        request: Request<RecoverInterruptedRunRequest>,
    ) -> Result<Response<RecoverInterruptedRunResponse>, Status> {
        events::recover_interrupted_run(self, request).await
    }
}
