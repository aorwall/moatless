use async_trait::async_trait;
use futures::stream::BoxStream;
use moatless_api_schemas::enums::{SandboxDesiredState, SandboxStopReason, TaskStatus};
use serde::{Deserialize, Serialize};

pub use moatless_events::SandboxStatus;
pub use sandbox_proto::client::{ClientError, SandboxClient, ServerOverride};

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox not ready: {0}")]
    NotReady(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unsupported sandbox operation {operation}: {source}")]
    Unsupported {
        operation: String,
        source: tonic::Status,
    },
    #[error("sandbox gRPC error in {operation}: {source}")]
    Grpc {
        operation: String,
        source: tonic::Status,
    },
    #[error("sandbox client error: {0}")]
    GrpcClient(String),
    #[error("{0}")]
    Other(String),
}

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
            source: e,
        },
        _ => SandboxError::Grpc {
            operation: operation.to_string(),
            source: e,
        },
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LogOptions {
    pub previous: bool,
    pub follow: bool,
    pub tail_lines: i64,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            previous: false,
            follow: true,
            tail_lines: 500,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    Stopped,
    Installing,
    Starting,
    Started,
    Failed,
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stopped => "stopped",
            Self::Installing => "installing",
            Self::Starting => "starting",
            Self::Started => "started",
            Self::Failed => "failed",
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ServerCapabilities {
    pub address_bar: bool,
    pub inspector: bool,
    pub reload: bool,
    pub open_external: bool,
    pub viewport: bool,
    pub console: bool,
}

#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub name: String,
    pub label: String,
    pub port: u16,
    pub status: ServerStatus,
    pub url: Option<String>,
    pub error: Option<String>,
    pub detail: Option<String>,
    pub default: bool,
    pub capabilities: ServerCapabilities,
}

#[derive(Debug, Clone)]
pub struct SandboxStatusFullResponse {
    pub sandbox_status: SandboxStatus,
    pub servers: Vec<ServerEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Running,
    Waiting,
    Idle,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatus {
    pub sandbox_status: SandboxStatus,
    pub agent_status: Option<AgentStatus>,
    pub sandbox_error: Option<String>,
}

pub type ServerLogStream = BoxStream<'static, Result<String, SandboxError>>;

#[async_trait]
pub trait SandboxObserver: Send + Sync {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxStatusFullResponse, SandboxError>;

    async fn live_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<LiveStatus, SandboxError>;

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        options: LogOptions,
    ) -> Result<ServerLogStream, SandboxError>;
}

#[async_trait]
pub trait SandboxIntentReader: Send + Sync {
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxError>;
}

#[async_trait]
pub trait SandboxStarter: Send + Sync {
    async fn start(&self, task_id: &str) -> Result<(), SandboxError>;
}

#[async_trait]
pub trait SandboxStopper: Send + Sync {
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxError>;
}

#[async_trait]
pub trait SandboxConnector: Send + Sync {
    async fn sandbox_client(&self, task_id: &str) -> Result<SandboxClient, SandboxError>;
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
}
