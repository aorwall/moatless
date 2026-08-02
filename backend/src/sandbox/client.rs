//! gRPC client wrapper for calling sandbox pods.
//!
//! Replaces all HTTP calls to sandbox pods (health, status, execute-agent,
//! stop-agent, notify-messages, search-files) with gRPC.
//! Preview servers run as separate K8s/Docker containers, but their live
//! start-command / env overrides are pushed through the sandbox
//! (`apply_server_config`), which owns the control files the sidecars read.

use std::collections::BTreeMap;
use std::time::Duration;

use sandbox_proto::sandbox_service_client::SandboxServiceClient;
use sandbox_proto::{
    ApplyServerConfigRequest, ExecuteAgentRequest, GetAgentStatusRequest, GetAgentStatusResponse,
    GetFileTreeRequest, GetFileTreeResponse, GetGitDiffPreviewRequest, GetGitDiffPreviewResponse,
    GetGitStatusRequest, GetGitStatusResponse, GetServerConfigRequest, ListFilesRequest,
    ListGitRefsRequest, ListGitRefsResponse, NotifyMessagesRequest, ReadFileRequest,
    ReadFileResponse, RefreshCredentialRequest, SearchFilesByNameRequest,
    SearchFilesByNameResponse, StopAgentRequest, TerminalAttachRequest, TerminalAttachStreamEvent,
    TerminalClearRequest, TerminalCloseRequest, TerminalOpenRequest, TerminalResizeRequest,
    TerminalRestartRequest, TerminalSessionSnapshot, TerminalWriteRequest, WriteFileRequest,
};
use tonic::Streaming;
use tonic::transport::Channel;
use tracing::debug;

use super::error::SandboxError;
use super::models::ServerOverride;
use super::schemas::{
    BackgroundProcess as SchemaBackgroundProcess,
    BackgroundWatchJobInfo as SchemaBackgroundWatchJobInfo,
    ScheduledJobInfo as SchemaScheduledJobInfo, ScheduledJobKind,
    WakeupJobInfo as SchemaWakeupJobInfo,
};

impl SchemaBackgroundProcess {
    /// Convert a proto `BackgroundProcess` into the API schema.
    pub fn from_proto(proto: sandbox_proto::BackgroundProcess) -> Self {
        Self {
            pid: proto.pid,
            command: proto.command,
        }
    }
}

impl SchemaScheduledJobInfo {
    /// Convert a proto `ScheduledJobInfo` into the API schema.
    ///
    /// Returns `None` for forward-compat unknown variants (proto `kind` is `None`)
    /// so callers can `flat_map` and silently drop them.
    pub fn from_proto(proto: sandbox_proto::ScheduledJobInfo) -> Option<Self> {
        let sandbox_proto::ScheduledJobInfo {
            id,
            registered_at_unix_ms,
            kind,
        } = proto;
        let kind = match kind? {
            sandbox_proto::scheduled_job_info::Kind::Wakeup(w) => {
                ScheduledJobKind::Wakeup(SchemaWakeupJobInfo {
                    reason: w.reason,
                    prompt_preview: w.prompt_preview,
                    wakeup_at_unix_ms: w.wakeup_at_unix_ms,
                })
            }
            sandbox_proto::scheduled_job_info::Kind::BackgroundWatch(b) => {
                ScheduledJobKind::BackgroundWatch(SchemaBackgroundWatchJobInfo {
                    process_count: b.process_count,
                    commands: b.commands,
                })
            }
        };
        Some(Self {
            id,
            registered_at_unix_ms,
            kind,
        })
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

/// Classify gRPC errors into appropriate `SandboxError` variants.
///
/// - `Unavailable`/`DeadlineExceeded`/`Cancelled` → `NotReady` (503)
///   (tonic's transport timeout surfaces as `Cancelled` with "Timeout expired")
/// - `NotFound` → `NotFound` (404)
/// - `FailedPrecondition`/`InvalidArgument` → `BadRequest` (400)
/// - `Unimplemented` → `Unsupported` (the sandbox image predates the method)
/// - everything else → `Internal` (500)
pub fn classify_grpc_error(e: tonic::Status, operation: &str) -> SandboxError {
    match e.code() {
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded | tonic::Code::Cancelled => {
            SandboxError::NotReady(format!("Sandbox unreachable: {operation}"))
        }
        tonic::Code::NotFound => SandboxError::NotFound(e.message().to_string()),
        tonic::Code::FailedPrecondition | tonic::Code::InvalidArgument => {
            SandboxError::BadRequest(e.message().to_string())
        }
        // A sandbox from an image older than the method. The call arrived and
        // was refused, so this is neither unreachable nor retryable.
        tonic::Code::Unimplemented => SandboxError::Unsupported {
            operation: operation.to_string(),
            source: e,
        },
        _ => SandboxError::Grpc {
            operation: operation.to_string(),
            source: e,
        },
    }
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
    pub async fn stop_agent(
        &self,
        source: sandbox_proto::AgentStopSource,
    ) -> Result<(), tonic::Status> {
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

#[cfg(test)]
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
        // An unclassified status keeps the tonic error on the source chain.
        let err = classify_grpc_error(tonic::Status::internal("boom"), "read_file");
        assert!(matches!(err, SandboxError::Grpc { .. }), "got {err:?}");
        let source = std::error::Error::source(&err).expect("tonic status is the source");
        assert!(source.to_string().contains("boom"));
    }

    /// Backend and sandbox images roll out separately, so any method added to
    /// `SandboxService` meets sandboxes that predate it. That is version skew a
    /// redeploy fixes, not an unreachable sandbox to wait on.
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
    fn from_proto_maps_wakeup_kind() {
        let proto = sandbox_proto::ScheduledJobInfo {
            id: "job-2".into(),
            registered_at_unix_ms: 1_700_000_000_000,
            kind: Some(sandbox_proto::scheduled_job_info::Kind::Wakeup(
                sandbox_proto::WakeupJobInfo {
                    reason: "Check build".into(),
                    prompt_preview: "Resume the loop".into(),
                    wakeup_at_unix_ms: 1_700_000_300_000,
                },
            )),
        };

        let job = SchemaScheduledJobInfo::from_proto(proto).expect("wakeup kind");
        assert_eq!(job.id, "job-2");
        assert_eq!(job.registered_at_unix_ms, 1_700_000_000_000);
        match job.kind {
            ScheduledJobKind::Wakeup(w) => {
                assert_eq!(w.reason, "Check build");
                assert_eq!(w.prompt_preview, "Resume the loop");
                assert_eq!(w.wakeup_at_unix_ms, 1_700_000_300_000);
            }
            ScheduledJobKind::BackgroundWatch(_) => panic!("expected wakeup kind"),
        }
    }

    #[test]
    fn from_proto_maps_background_watch_kind() {
        let proto = sandbox_proto::ScheduledJobInfo {
            id: "job-bg".into(),
            registered_at_unix_ms: 1_700_000_000_000,
            kind: Some(sandbox_proto::scheduled_job_info::Kind::BackgroundWatch(
                sandbox_proto::BackgroundWatchJobInfo {
                    process_count: 2,
                    commands: vec!["cargo build".into(), "sleep 30".into()],
                },
            )),
        };

        let job = SchemaScheduledJobInfo::from_proto(proto).expect("background-watch kind");
        assert_eq!(job.id, "job-bg");
        match job.kind {
            ScheduledJobKind::BackgroundWatch(b) => {
                assert_eq!(b.process_count, 2);
                assert_eq!(b.commands, vec!["cargo build", "sleep 30"]);
            }
            ScheduledJobKind::Wakeup(_) => panic!("expected background-watch kind"),
        }
    }

    #[test]
    fn background_process_from_proto_maps_fields() {
        let mapped = SchemaBackgroundProcess::from_proto(sandbox_proto::BackgroundProcess {
            pid: 4321,
            command: "cargo test --no-run".into(),
        });
        assert_eq!(mapped.pid, 4321);
        assert_eq!(mapped.command, "cargo test --no-run");
    }

    #[test]
    fn from_proto_drops_unknown_kind() {
        let proto = sandbox_proto::ScheduledJobInfo {
            id: "job-3".into(),
            registered_at_unix_ms: 0,
            kind: None,
        };

        assert!(SchemaScheduledJobInfo::from_proto(proto).is_none());
    }

    #[test]
    fn collect_filters_unknown_jobs() {
        let resp = sandbox_proto::GetAgentStatusResponse {
            status: "idle".into(),
            session_id: None,
            last_error: None,
            busy_processes: false,
            background_processes: vec![],
            scheduled_jobs: vec![
                sandbox_proto::ScheduledJobInfo {
                    id: "good".into(),
                    registered_at_unix_ms: 1,
                    kind: Some(sandbox_proto::scheduled_job_info::Kind::Wakeup(
                        sandbox_proto::WakeupJobInfo {
                            reason: "go".into(),
                            prompt_preview: "p".into(),
                            wakeup_at_unix_ms: 2,
                        },
                    )),
                },
                sandbox_proto::ScheduledJobInfo {
                    id: "unknown".into(),
                    registered_at_unix_ms: 3,
                    kind: None,
                },
            ],
        };

        let jobs: Vec<_> = resp
            .scheduled_jobs
            .into_iter()
            .filter_map(SchemaScheduledJobInfo::from_proto)
            .collect();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "good");
    }
}
