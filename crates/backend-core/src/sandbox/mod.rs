use async_trait::async_trait;
use futures::stream::BoxStream;
use moatless_api_schemas::enums::{SandboxDesiredState, SandboxStopReason, TaskStatus};
use serde::{Deserialize, Serialize};

pub use moatless_events::SandboxStatus;
pub use sandbox_proto::client::{ClientError, SandboxClient, ServerOverride};

/// Which container runtime a sandbox is running on.
///
/// The contract classifies a provider failure by runtime instead of naming the
/// runtime's own error type, so a caller can depend on this contract without
/// compiling `kube` or `bollard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Kubernetes,
    Docker,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Kubernetes => "Kubernetes",
            Self::Docker => "Docker",
        })
    }
}

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
    /// The container runtime under a sandbox failed. `kind` says which runtime,
    /// and `message` is the description that runtime's boundary already
    /// rendered — it carries no credential or wire detail, because the boundary
    /// is the only place that sees them.
    #[error("{message}")]
    Provider {
        kind: ProviderKind,
        message: String,
    },
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

/// The sandbox's own HTTP control port.
pub const SANDBOX_HTTP_PORT: u16 = 8001;
/// The dind Docker socket inside the sandbox.
pub const SANDBOX_DIND_PORT: u16 = 2375;

/// Whether `port` belongs to the sandbox's own control plane rather than to
/// anything a workspace may expose.
///
/// The rule lives beside `SandboxPortPublisher` because it is part of that
/// trait's contract rather than any one runtime's: routing the authenticated
/// preview edge onto the control port, the agent gRPC channel or the Docker
/// socket would be a sandbox escape, and that is true of every runtime — the
/// one that publishes nothing externally included, where the collision instead
/// shadows the sandbox's own listener inside the container. Takes the gRPC port
/// rather than a `Config` so the rule sits in the same band as the trait.
#[must_use]
pub fn is_reserved_sandbox_port(port: u16, grpc_port: u16) -> bool {
    port == SANDBOX_HTTP_PORT || port == grpc_port || port == SANDBOX_DIND_PORT
}

/// Make one port on an already-running sandbox reachable from outside it.
///
/// Deliberately not a sixth verb on `SandboxRuntime`: that trait's verbs all
/// move a sandbox through its desired-state machine and carry an operation
/// generation. Publishing a port changes no desired state, bumps no generation,
/// and is not something the reconciler drives — a Script asks for it as it
/// starts.
///
/// Implementations must be idempotent: the same call for a port that is already
/// published succeeds and returns the same URL, and must refuse a reserved port
/// (`is_reserved_sandbox_port`) before doing anything else — see that function.
#[async_trait]
pub trait SandboxPortPublisher: Send + Sync {
    /// Publish `port`, returning the external URL when the deployment has one.
    ///
    /// `Ok(None)` means the port is not externally addressable but nothing went
    /// wrong that the caller must act on — no proxy domain is configured, the
    /// runtime does not publish ports at all, or an edge write failed in a way
    /// that costs only the URL. An `Err` means the caller should not proceed.
    async fn publish_port(
        &self,
        task_id: &str,
        name: &str,
        port: u16,
    ) -> Result<Option<String>, SandboxError>;
}

/// One container's state and live resource usage, in the fleet-wide read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDiagnosticsContainer {
    pub name: String,
    /// Container state: "running", "waiting", "terminated", "unknown"
    pub state: String,
    pub restart_count: i32,
    /// `None` when the K8s Metrics API has nothing for this container yet, or
    /// is unavailable — never an error on its own.
    pub cpu_millicores: Option<u64>,
    pub memory_mb: Option<u64>,
}

/// One Sandbox's status and live resource usage, as answered by
/// [`ClusterDiagnostics::list_sandbox_diagnostics`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDiagnosticsEntry {
    pub task_id: String,
    /// Left `None` by every implementation: the seam is a pure infrastructure
    /// read with no database behind it, the same boundary [`SandboxObserver`]
    /// draws. A caller that wants a Task's title reads it separately.
    pub title: Option<String>,
    pub sandbox_status: SandboxStatus,
    /// The pod's scheduled node, when the runtime has one (K8s only).
    pub node_name: Option<String>,
    pub containers: Vec<SandboxDiagnosticsContainer>,
    /// Sum of `containers[].cpuMillicores`. `None` when metrics were not
    /// available for any container, so a caller can tell "zero usage" apart
    /// from "unknown usage".
    pub total_cpu_millicores: Option<u64>,
    pub total_memory_mb: Option<u64>,
}

/// Fleet-wide Sandbox status and live resource usage.
///
/// Deployment-scoped rather than per-Task: it answers "what does every Sandbox
/// in this deployment look like right now," which is a different question from
/// [`SandboxObserver`]'s one-Task-at-a-time read, and it is why this is its own
/// seam rather than a method added there. Read-only and idempotent — safe to
/// call on any replica and safe to retry.
#[async_trait]
pub trait ClusterDiagnostics: Send + Sync {
    async fn list_sandbox_diagnostics(&self) -> Result<Vec<SandboxDiagnosticsEntry>, SandboxError>;
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

    /// The classification is the machine-readable half; the message is what a
    /// caller renders or logs, and it reads the same whichever runtime failed.
    #[test]
    fn a_provider_failure_renders_its_message_and_keeps_its_kind() {
        for (kind, message) in [
            (ProviderKind::Kubernetes, "Kubernetes API error: 404 Not Found"),
            (ProviderKind::Docker, "Docker API error during inspect"),
        ] {
            let err = SandboxError::Provider {
                kind,
                message: message.to_string(),
            };
            assert_eq!(err.to_string(), message);
            assert!(matches!(err, SandboxError::Provider { kind: k, .. } if k == kind));
        }
    }

    #[test]
    fn provider_kinds_name_their_runtime() {
        assert_eq!(ProviderKind::Kubernetes.to_string(), "Kubernetes");
        assert_eq!(ProviderKind::Docker.to_string(), "Docker");
    }
}
