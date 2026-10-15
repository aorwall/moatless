//! gRPC client wrapper for calling sandbox pods.
//!
//! Replaces all HTTP calls to sandbox pods (health, status, execute-agent,
//! stop-agent, notify-messages, search-files) with gRPC.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use moatless_types::sandbox::SandboxError;

use crate::sandbox_service_client::SandboxServiceClient;
use crate::{
    ApplyTaskScriptRequest, ExecuteAgentRequest, GetAgentStatusRequest, GetAgentStatusResponse,
    GetFileTreeRequest, GetFileTreeResponse, GetGitDiffPreviewRequest, GetGitDiffPreviewResponse,
    GetGitStatusRequest, GetGitStatusResponse, GetTaskScriptsRequest, JoinTaskRequest,
    ListFilesRequest, ListGitRefsRequest, ListGitRefsResponse, NotifyMessagesRequest,
    ReadFileRequest, ReadFileResponse, RefreshCredentialRequest, SearchFilesByNameRequest,
    SearchFilesByNameResponse, StopAgentRequest, SyncWorkspaceRequest, SyncWorkspaceResponse,
    TaskScript, TerminalAttachRequest, TerminalAttachStreamEvent, TerminalClearRequest,
    TerminalCloseRequest, TerminalOpenRequest, TerminalResizeRequest, TerminalRestartRequest,
    TerminalSessionSnapshot, TerminalWriteRequest, WriteFileRequest,
};
use tonic::Streaming;
use tonic::transport::Channel;
use tracing::debug;

/// Every backend ↔ sandbox gRPC server and client, raised from tonic's 4 MiB
/// default so binary files and trajectory lines carrying images or PDFs fit.
pub const MAX_GRPC_MESSAGE_SIZE: usize = 32 * 1024 * 1024;
const SHORT_RPC_TIMEOUT: Duration = Duration::from_secs(10);
const JOIN_TASK_TIMEOUT: Duration = Duration::from_mins(5);

/// Errors that can occur when constructing a [`SandboxClient`].
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid gRPC endpoint URI: {0}")]
    InvalidUri(#[from] tonic::codegen::http::uri::InvalidUri),
}

/// Wrapper around `SandboxServiceClient` for calling sandbox pods via gRPC.
#[derive(Clone)]
pub struct SandboxClient {
    client: SandboxServiceClient<Channel>,
}

impl SandboxClient {
    /// Create a gRPC client for a sandbox pod using K8s service DNS.
    ///
    /// Uses `connect_lazy()` — no upfront TCP connection.
    pub fn for_service(
        service_name: &str,
        namespace: &str,
        port: u16,
    ) -> Result<Self, ClientError> {
        let url = format!("http://{service_name}.{namespace}.svc.cluster.local:{port}");
        Self::connect(&url)
    }

    /// Create a gRPC client from a URL.
    pub fn connect(url: &str) -> Result<Self, ClientError> {
        let channel = Channel::from_shared(url.to_string())?
            .connect_timeout(Duration::from_secs(5))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10))
            .connect_lazy();

        Ok(Self {
            client: SandboxServiceClient::new(channel)
                .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
                .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE),
        })
    }

    /// Execute agent on the sandbox.
    pub async fn execute_agent(&self, request: ExecuteAgentRequest) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, client.execute_agent(request)).await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "ExecuteAgent gRPC request timed out",
            )),
        }
    }

    /// Register a task as a member of an already-running warm sandbox.
    ///
    /// See `JoinTaskRequest` for what `agent_profile` selects and its fallback
    /// when absent.
    pub async fn join_task(
        &self,
        task_id: &str,
        checkout_mode: Option<&str>,
        agent_profile: Option<&str>,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(JOIN_TASK_TIMEOUT, async {
            client
                .join_task(JoinTaskRequest {
                    task_id: task_id.to_string(),
                    checkout_mode: checkout_mode.map(str::to_string),
                    agent_profile: agent_profile.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "JoinTask gRPC request timed out",
            )),
        }
    }

    #[cfg(test)]
    fn join_task_timeout() -> Duration {
        JOIN_TASK_TIMEOUT
    }

    /// Stop the running agent. `source` is echoed on the terminal
    /// `agent.stopped` event so the stop is attributed to its initiator.
    pub async fn stop_agent(&self, source: crate::AgentStopSource) -> Result<(), tonic::Status> {
        self.stop_agent_for_task(None, source).await
    }

    pub async fn stop_agent_for_task(
        &self,
        task_id: Option<&str>,
        source: crate::AgentStopSource,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client
                .stop_agent(StopAgentRequest {
                    source: source.into(),
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "StopAgent gRPC request timed out",
            )),
        }
    }

    /// Push the workspace to S3 now, ahead of an effect that destroys the live
    /// copy. Bounded by its own deadline: the caller is on the stop path and a
    /// sandbox that cannot answer must not hold the transition open.
    pub async fn sync_workspace(
        &self,
        timeout: Duration,
    ) -> Result<SyncWorkspaceResponse, tonic::Status> {
        let mut client = self.client.clone();
        match tokio::time::timeout(timeout, client.sync_workspace(SyncWorkspaceRequest {})).await {
            Ok(response) => Ok(response?.into_inner()),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Workspace sync gRPC request timed out",
            )),
        }
    }

    /// Get the current agent status from the sandbox.
    pub async fn get_agent_status(&self) -> Result<GetAgentStatusResponse, tonic::Status> {
        self.get_agent_status_for_task(None).await
    }

    pub async fn get_agent_status_for_task(
        &self,
        task_id: Option<&str>,
    ) -> Result<GetAgentStatusResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            client
                .get_agent_status(GetAgentStatusRequest {
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Get agent status gRPC request timed out",
            )),
        }
    }

    /// Notify the sandbox that new messages are available.
    ///
    /// Fire-and-forget: logs warnings on failure, never propagates errors.
    pub async fn notify_messages(&self) {
        self.notify_messages_for_task(None).await;
    }

    pub async fn notify_messages_for_task(&self, task_id: Option<&str>) {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client
                .notify_messages(NotifyMessagesRequest {
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(_)) => {
                debug!("Notified sandbox pod via gRPC");
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "Failed to notify sandbox pod via gRPC");
            }
            Err(_) => {
                tracing::warn!("NotifyMessages gRPC request timed out");
            }
        }
    }

    /// See `sandbox::files::tree::get_file_tree_with_changes` for the file-tree
    /// contract.
    pub async fn get_file_tree(
        &self,
        root_path: &str,
        recursive: bool,
    ) -> Result<GetFileTreeResponse, tonic::Status> {
        self.get_file_tree_for_task(None, root_path, recursive)
            .await
    }

    pub async fn get_file_tree_for_task(
        &self,
        task_id: Option<&str>,
        root_path: &str,
        recursive: bool,
    ) -> Result<GetFileTreeResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            client
                .get_file_tree(GetFileTreeRequest {
                    root_path: root_path.to_string(),
                    recursive,
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Get file tree gRPC request timed out",
            )),
        }
    }

    /// Read a file from the sandbox workspace.
    pub async fn read_file(
        &self,
        path: &str,
        include_changes: bool,
    ) -> Result<ReadFileResponse, tonic::Status> {
        self.read_file_for_task(None, path, include_changes).await
    }

    pub async fn read_file_for_task(
        &self,
        task_id: Option<&str>,
        path: &str,
        include_changes: bool,
    ) -> Result<ReadFileResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            client
                .read_file(ReadFileRequest {
                    path: path.to_string(),
                    include_changes,
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Read file gRPC request timed out",
            )),
        }
    }

    /// Where the symbol at `line`/`column` (1-based, UTF-16) of `path` is
    /// defined. The sandbox may start a language server first, so the deadline
    /// covers its start as well as the lookup.
    pub async fn go_to_definition_for_task(
        &self,
        task_id: Option<&str>,
        path: &str,
        line: u32,
        column: u32,
    ) -> Result<crate::GoToDefinitionResponse, tonic::Status> {
        let mut client = self.client.clone();
        let request = crate::GoToDefinitionRequest {
            path: path.to_string(),
            line,
            column,
            task_id: task_id.map(str::to_string),
        };
        match tokio::time::timeout(Duration::from_mins(1), client.go_to_definition(request)).await {
            Ok(result) => result.map(tonic::Response::into_inner),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Go-to-definition gRPC request timed out",
            )),
        }
    }

    /// Write a file in the sandbox workspace.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<(), tonic::Status> {
        self.write_file_for_task(None, path, content).await
    }

    pub async fn write_file_for_task(
        &self,
        task_id: Option<&str>,
        path: &str,
        content: &str,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client
                .write_file(WriteFileRequest {
                    path: path.to_string(),
                    content: content.to_string(),
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Write file gRPC request timed out",
            )),
        }
    }

    /// List files in the sandbox workspace.
    pub async fn list_files(
        &self,
        path: &str,
        recursive: bool,
        respect_gitignore: bool,
    ) -> Result<Vec<String>, tonic::Status> {
        self.list_files_for_task(None, path, recursive, respect_gitignore)
            .await
    }

    pub async fn list_files_for_task(
        &self,
        task_id: Option<&str>,
        path: &str,
        recursive: bool,
        respect_gitignore: bool,
    ) -> Result<Vec<String>, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client
                .list_files(ListFilesRequest {
                    path: path.to_string(),
                    recursive,
                    respect_gitignore,
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner().files),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "List files gRPC request timed out",
            )),
        }
    }

    /// Push a freshly stored credential to the sandbox.
    ///
    /// Used by `CredentialService::broadcast` after a rotation to keep every
    /// running pod's view fresh.
    pub async fn refresh_credential(
        &self,
        task_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            client
                .refresh_credential(RefreshCredentialRequest {
                    task_id: task_id.to_string(),
                    key: key.to_string(),
                    value: value.to_string(),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "RefreshCredential gRPC timed out",
            )),
        }
    }

    /// Declare one of a Task's own Scripts, replacing whatever it held under
    /// that name; `remove` drops it instead.
    ///
    /// Removing a Script that was never declared succeeds.
    pub async fn apply_task_script(
        &self,
        task_id: &str,
        script: TaskScript,
        remove: bool,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let request = ApplyTaskScriptRequest {
            task_id: task_id.to_string(),
            script: Some(script),
            remove,
        };
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client.apply_task_script(request).await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "ApplyTaskScript gRPC timed out",
            )),
        }
    }

    /// Every Script this Task has declared on itself, keyed by name.
    pub async fn get_task_scripts(
        &self,
        task_id: &str,
    ) -> Result<BTreeMap<String, TaskScript>, tonic::Status> {
        let mut client = self.client.clone();
        let request = GetTaskScriptsRequest {
            task_id: task_id.to_string(),
        };
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client.get_task_scripts(request).await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner().scripts.into_iter().collect()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "GetTaskScripts gRPC timed out",
            )),
        }
    }

    pub async fn terminal_open(
        &self,
        request: TerminalOpenRequest,
    ) -> Result<TerminalSessionSnapshot, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.terminal_open(request).await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "TerminalOpen gRPC request timed out",
            )),
        }
    }

    pub async fn terminal_attach(
        &self,
        request: TerminalAttachRequest,
    ) -> Result<Streaming<TerminalAttachStreamEvent>, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.terminal_attach(request).await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "TerminalAttach gRPC request timed out",
            )),
        }
    }

    pub async fn terminal_write(&self, request: TerminalWriteRequest) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.terminal_write(request).await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "TerminalWrite gRPC request timed out",
            )),
        }
    }

    pub async fn terminal_resize(
        &self,
        request: TerminalResizeRequest,
    ) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.terminal_resize(request).await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "TerminalResize gRPC request timed out",
            )),
        }
    }

    pub async fn terminal_clear(&self, request: TerminalClearRequest) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.terminal_clear(request).await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "TerminalClear gRPC request timed out",
            )),
        }
    }

    pub async fn terminal_restart(
        &self,
        request: TerminalRestartRequest,
    ) -> Result<TerminalSessionSnapshot, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.terminal_restart(request).await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "TerminalRestart gRPC request timed out",
            )),
        }
    }

    pub async fn terminal_close(&self, request: TerminalCloseRequest) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.terminal_close(request).await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "TerminalClose gRPC request timed out",
            )),
        }
    }

    /// Search files by name in the sandbox workspace.
    pub async fn search_files_by_name(
        &self,
        query: &str,
        case_sensitive: bool,
        max_results: i32,
    ) -> Result<SearchFilesByNameResponse, tonic::Status> {
        self.search_files_by_name_for_task(None, query, case_sensitive, max_results)
            .await
    }

    pub async fn search_files_by_name_for_task(
        &self,
        task_id: Option<&str>,
        query: &str,
        case_sensitive: bool,
        max_results: i32,
    ) -> Result<SearchFilesByNameResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client
                .search_files_by_name(SearchFilesByNameRequest {
                    query: query.to_string(),
                    case_sensitive,
                    max_results,
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Search files by name gRPC request timed out",
            )),
        }
    }

    /// Both review diff scopes of the sandbox workspace.
    ///
    /// Given a longer deadline than other file calls: a patch is the whole
    /// working tree of every repo, read with `git` rather than an index.
    pub async fn get_git_diff_preview(
        &self,
        base_ref: Option<String>,
        ignore_whitespace: bool,
    ) -> Result<GetGitDiffPreviewResponse, tonic::Status> {
        self.get_git_diff_preview_for_task(None, base_ref, ignore_whitespace)
            .await
    }

    pub async fn get_git_diff_preview_for_task(
        &self,
        task_id: Option<&str>,
        base_ref: Option<String>,
        ignore_whitespace: bool,
    ) -> Result<GetGitDiffPreviewResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            client
                .get_git_diff_preview(GetGitDiffPreviewRequest {
                    base_ref,
                    ignore_whitespace,
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Git diff preview gRPC request timed out",
            )),
        }
    }

    /// The primary repository's branch state.
    pub async fn get_git_status(&self) -> Result<GetGitStatusResponse, tonic::Status> {
        self.get_git_status_for_task(None).await
    }

    pub async fn get_git_status_for_task(
        &self,
        task_id: Option<&str>,
    ) -> Result<GetGitStatusResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client
                .get_git_status(GetGitStatusRequest {
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Git status gRPC request timed out",
            )),
        }
    }

    /// The primary repository's branches.
    pub async fn list_git_refs(&self) -> Result<ListGitRefsResponse, tonic::Status> {
        self.list_git_refs_for_task(None).await
    }

    pub async fn list_git_refs_for_task(
        &self,
        task_id: Option<&str>,
    ) -> Result<ListGitRefsResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(SHORT_RPC_TIMEOUT, async {
            client
                .list_git_refs(ListGitRefsRequest {
                    task_id: task_id.map(str::to_string),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Git ref listing gRPC request timed out",
            )),
        }
    }
}

/// Hands out a gRPC client for a task's sandbox.
///
/// Sits beside [`SandboxClient`] since that's what it returns: naming this
/// trait pulls in the generated protocol, not naming it keeps a consumer off
/// it.
#[async_trait]
pub trait SandboxConnector: Send + Sync {
    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError>;
}

/// Map a gRPC status onto the shared sandbox vocabulary.
pub fn classify_grpc_error(e: tonic::Status, operation: &str) -> SandboxError {
    match e.code() {
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded | tonic::Code::Cancelled => {
            SandboxError::NotReady(format!("Sandbox unreachable: {operation}"))
        }
        tonic::Code::NotFound => SandboxError::NotFound(e.message().to_string()),
        tonic::Code::FailedPrecondition | tonic::Code::InvalidArgument => {
            SandboxError::BadRequest(e.message().to_string())
        }
        tonic::Code::Unimplemented => SandboxError::Unsupported {
            operation: operation.to_string(),
            source: Box::new(e),
        },
        _ => SandboxError::Grpc {
            operation: operation.to_string(),
            source: Box::new(e),
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures must succeed"
)]
mod tests {
    use super::*;

    #[test]
    fn classify_unreachable_codes_as_not_ready() {
        for status in [
            tonic::Status::unavailable("connection refused"),
            tonic::Status::deadline_exceeded("deadline"),
            tonic::Status::cancelled("Timeout expired"),
        ] {
            let err = classify_grpc_error(status, "get_file_tree");
            assert!(matches!(err, SandboxError::NotReady(_)), "got {err:?}");
        }
    }

    #[test]
    fn classify_not_found_and_internal() {
        assert!(matches!(
            classify_grpc_error(tonic::Status::not_found("missing"), "read_file"),
            SandboxError::NotFound(_)
        ));
        let err = classify_grpc_error(tonic::Status::internal("boom"), "read_file");
        assert!(matches!(err, SandboxError::Grpc { .. }), "got {err:?}");
        let source = std::error::Error::source(&err).expect("tonic status is the source");
        assert!(source.to_string().contains("boom"));
    }

    #[test]
    fn classify_an_unimplemented_method_as_a_sandbox_too_old_to_serve_it() {
        let err = classify_grpc_error(
            tonic::Status::unimplemented("unknown service"),
            "list_git_refs",
        );
        assert!(
            matches!(err, SandboxError::Unsupported { .. }),
            "got {err:?}"
        );
        assert!(err.to_string().contains("list_git_refs"));
    }

    #[test]
    fn join_task_deadline_allows_clone_scale_materialization() {
        assert!(SandboxClient::join_task_timeout() > SHORT_RPC_TIMEOUT);
        assert!(SandboxClient::join_task_timeout() >= Duration::from_mins(2));
    }
}
