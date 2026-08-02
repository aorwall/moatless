//! gRPC client wrapper for calling sandbox pods.
//!
//! Replaces all HTTP calls to sandbox pods (health, status, execute-agent,
//! stop-agent, notify-messages, search-files) with gRPC.
//! Preview servers run as separate K8s/Docker containers, but their live
//! start-command / env overrides are pushed through the sandbox
//! (`apply_server_config`), which owns the control files the sidecars read.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::sandbox_service_client::SandboxServiceClient;
use crate::{
    ApplyServerConfigRequest, ExecuteAgentRequest, GetAgentStatusRequest, GetAgentStatusResponse,
    GetFileTreeRequest, GetFileTreeResponse, GetGitDiffPreviewRequest, GetGitDiffPreviewResponse,
    GetGitStatusRequest, GetGitStatusResponse, GetServerConfigRequest, ListFilesRequest,
    ListGitRefsRequest, ListGitRefsResponse, NotifyMessagesRequest, ReadFileRequest,
    ReadFileResponse, RefreshCredentialRequest, SearchFilesByNameRequest,
    SearchFilesByNameResponse, StopAgentRequest, TerminalAttachRequest, TerminalAttachStreamEvent,
    TerminalClearRequest, TerminalCloseRequest, TerminalOpenRequest, TerminalResizeRequest,
    TerminalRestartRequest, TerminalSessionSnapshot, TerminalWriteRequest, WriteFileRequest,
};
use tonic::transport::Channel;
use tonic::Streaming;
use tracing::debug;

/// A live override of one preview server's start command and env.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unset_env: Vec<String>,
}

impl ServerOverride {
    pub fn is_empty(&self) -> bool {
        self.start_command.is_none() && self.env.is_empty() && self.unset_env.is_empty()
    }
}

/// Raise from tonic's 4 MB default so binary files (audio, images) can round-trip.
const MAX_GRPC_MESSAGE_SIZE: usize = 32 * 1024 * 1024;

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
            .timeout(Duration::from_secs(10))
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
        client.execute_agent(request).await?;
        Ok(())
    }

    /// Stop the running agent. `source` is echoed on the terminal
    /// `agent.stopped` event so the stop is attributed to its initiator.
    pub async fn stop_agent(&self, source: crate::AgentStopSource) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        client
            .stop_agent(StopAgentRequest {
                source: source.into(),
            })
            .await?;
        Ok(())
    }

    /// Get the current agent status from the sandbox.
    pub async fn get_agent_status(&self) -> Result<GetAgentStatusResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            client.get_agent_status(GetAgentStatusRequest {}).await
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
        let mut client = self.client.clone();
        match client.notify_messages(NotifyMessagesRequest {}).await {
            Ok(_) => {
                debug!("Notified sandbox pod via gRPC");
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to notify sandbox pod via gRPC");
            }
        }
    }

    /// Get the file tree with git change overlay.
    pub async fn get_file_tree(
        &self,
        root_path: &str,
        recursive: bool,
    ) -> Result<GetFileTreeResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            client
                .get_file_tree(GetFileTreeRequest {
                    root_path: root_path.to_string(),
                    recursive,
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
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            client
                .read_file(ReadFileRequest {
                    path: path.to_string(),
                    include_changes,
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

    /// Write a file in the sandbox workspace.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client
                .write_file(WriteFileRequest {
                    path: path.to_string(),
                    content: content.to_string(),
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
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client
                .list_files(ListFilesRequest {
                    path: path.to_string(),
                    recursive,
                    respect_gitignore,
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
    /// Used by `CredentialService::broadcast` after a credential is rotated
    /// (sandbox-side or via the UI) to keep every running pod's view fresh.
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

    /// Push a preview server's override into the sandbox and optionally restart
    /// it in place.
    ///
    /// The override is sent whole rather than as a delta, so the sandbox never
    /// merges. Callers that need a partial update read the current state with
    /// [`Self::get_server_config`] first. Returns the sidecar generation after
    /// the call.
    pub async fn apply_server_config(
        &self,
        task_id: &str,
        server_name: &str,
        ov: &ServerOverride,
        restart: bool,
    ) -> Result<u64, tonic::Status> {
        let mut client = self.client.clone();
        let request = ApplyServerConfigRequest {
            task_id: task_id.to_string(),
            server_name: server_name.to_string(),
            start_command: ov.start_command.clone(),
            env: ov.env.clone().into_iter().collect(),
            unset_env: ov.unset_env.clone(),
            restart,
        };
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            client.apply_server_config(request).await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner().generation),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "ApplyServerConfig gRPC timed out",
            )),
        }
    }

    /// Read every preview-server override currently on the workspace volume,
    /// keyed by server name, as `(override, generation)`.
    ///
    /// The control files are the only record of an override — nothing is
    /// persisted backend-side — so this is the authoritative read and it needs a
    /// running sandbox. Servers with no override are absent from the map.
    pub async fn get_server_config(
        &self,
        task_id: &str,
    ) -> Result<BTreeMap<String, (ServerOverride, u64)>, tonic::Status> {
        let mut client = self.client.clone();
        let request = GetServerConfigRequest {
            task_id: task_id.to_string(),
        };
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            client.get_server_config(request).await
        })
        .await;

        let resp = match result {
            Ok(Ok(resp)) => resp.into_inner(),
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(tonic::Status::deadline_exceeded(
                    "GetServerConfig gRPC timed out",
                ));
            }
        };

        Ok(resp
            .overrides
            .into_iter()
            .map(|(name, o)| {
                (
                    name,
                    (
                        ServerOverride {
                            start_command: o.start_command,
                            env: o.env.into_iter().collect(),
                            unset_env: o.unset_env,
                        },
                        o.generation,
                    ),
                )
            })
            .collect())
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
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client
                .search_files_by_name(SearchFilesByNameRequest {
                    query: query.to_string(),
                    case_sensitive,
                    max_results,
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
    /// Given a longer deadline than the other file calls: a patch is the whole
    /// working tree of every repository in the workspace, and the sandbox reads
    /// it with `git` rather than from an index.
    pub async fn get_git_diff_preview(
        &self,
        base_ref: Option<String>,
        ignore_whitespace: bool,
    ) -> Result<GetGitDiffPreviewResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            client
                .get_git_diff_preview(GetGitDiffPreviewRequest {
                    base_ref,
                    ignore_whitespace,
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
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.get_git_status(GetGitStatusRequest {}).await
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
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client.list_git_refs(ListGitRefsRequest {}).await
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
