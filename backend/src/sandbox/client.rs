//! gRPC client wrapper for calling sandbox pods.
//!
//! Replaces all HTTP calls to sandbox pods (health, status, execute-agent,
//! stop-agent, notify-messages, search-files) with gRPC.
//! Server management is handled by K8s (separate containers).

use std::time::Duration;

use sandbox_proto::sandbox_service_client::SandboxServiceClient;
use sandbox_proto::{
    ExecuteAgentRequest, GetAgentStatusRequest, GetAgentStatusResponse, GetFileTreeRequest,
    GetFileTreeResponse, ListFilesRequest, NotifyMessagesRequest, ReadFileRequest,
    ReadFileResponse, RefreshCredentialRequest, SearchFilesByNameRequest,
    SearchFilesByNameResponse, StopAgentRequest, WriteFileRequest,
};
use tonic::transport::Channel;
use tracing::debug;

use super::error::SandboxError;
use super::schemas::{
    MonitorJobInfo as SchemaMonitorJobInfo, ScheduledJobInfo as SchemaScheduledJobInfo,
    ScheduledJobKind, WakeupJobInfo as SchemaWakeupJobInfo,
};

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
            sandbox_proto::scheduled_job_info::Kind::Monitor(m) => {
                ScheduledJobKind::Monitor(SchemaMonitorJobInfo {
                    description: m.description,
                    command_preview: m.command_preview,
                    timeout_ms: m.timeout_ms,
                    persistent: m.persistent,
                })
            }
            sandbox_proto::scheduled_job_info::Kind::Wakeup(w) => {
                ScheduledJobKind::Wakeup(SchemaWakeupJobInfo {
                    reason: w.reason,
                    prompt_preview: w.prompt_preview,
                    wakeup_at_unix_ms: w.wakeup_at_unix_ms,
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
/// Unavailable/connection errors → `NotReady` (503),
/// everything else → `Internal` (500).
pub fn classify_grpc_error(e: tonic::Status, operation: &str) -> SandboxError {
    match e.code() {
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
            SandboxError::NotReady(format!("Sandbox unreachable: {operation}"))
        }
        _ => SandboxError::Internal(format!("gRPC {operation} failed: {e}")),
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

    /// Stop the running agent.
    pub async fn stop_agent(&self) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        client.stop_agent(StopAgentRequest {}).await?;
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
    ) -> Result<GetFileTreeResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            client
                .get_file_tree(GetFileTreeRequest {
                    root_path: root_path.to_string(),
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
    ) -> Result<Vec<String>, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client
                .list_files(ListFilesRequest {
                    path: path.to_string(),
                    recursive,
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

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_proto_maps_monitor_kind() {
        let proto = sandbox_proto::ScheduledJobInfo {
            id: "job-1".into(),
            registered_at_unix_ms: 1_700_000_000_000,
            kind: Some(sandbox_proto::scheduled_job_info::Kind::Monitor(
                sandbox_proto::MonitorJobInfo {
                    description: "Tail logs".into(),
                    command_preview: "tail -f /var/log/app.log".into(),
                    timeout_ms: 30_000,
                    persistent: true,
                },
            )),
        };

        let job = SchemaScheduledJobInfo::from_proto(proto).expect("monitor kind");
        assert_eq!(job.id, "job-1");
        assert_eq!(job.registered_at_unix_ms, 1_700_000_000_000);
        match job.kind {
            ScheduledJobKind::Monitor(m) => {
                assert_eq!(m.description, "Tail logs");
                assert_eq!(m.command_preview, "tail -f /var/log/app.log");
                assert_eq!(m.timeout_ms, 30_000);
                assert!(m.persistent);
            }
            ScheduledJobKind::Wakeup(_) => panic!("expected monitor variant"),
        }
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
        match job.kind {
            ScheduledJobKind::Wakeup(w) => {
                assert_eq!(w.reason, "Check build");
                assert_eq!(w.prompt_preview, "Resume the loop");
                assert_eq!(w.wakeup_at_unix_ms, 1_700_000_300_000);
            }
            ScheduledJobKind::Monitor(_) => panic!("expected wakeup variant"),
        }
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
