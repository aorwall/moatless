use async_trait::async_trait;
use futures::stream::BoxStream;
use moatless_types::sandbox::LiveStatus;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::sandbox::SandboxError;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::sandbox::ServerEntry;
use moatless_types::task::TaskStatus;

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

/// The narrowed sandbox status the T3 contract observer returns: the two fields
/// the T3 surface was designed for. Deliberately distinct from the wire response
/// body `moatless_api_schemas::sandbox::SandboxStatusFullResponse`; the backend
/// projects the wider response down to this. See `t3code::projection`.
#[derive(Debug, Clone)]
pub struct ContractSandboxStatus {
    pub sandbox_status: SandboxStatus,
    pub servers: Vec<ServerEntry>,
}

/// A Task's sandbox beyond its lifecycle state: what the T3 sandbox panel shows
/// under Image, Containers and Runtime events.
///
/// A second narrowing of the same backend response [`ContractSandboxStatus`]
/// narrows, and a wider one, because the panel asks a different question from
/// the status indicator. Kept apart from `ContractSandboxStatus` so the
/// indicator's two-field shape stays what it is; see `t3code::projection`.
#[derive(Debug, Clone)]
pub struct SandboxDetail {
    /// The image the sandbox is running. `None` where no sandbox is
    /// provisioned. The image a redeploy *would* install is not here: no status
    /// reader can see it, and [`super::SandboxOperator::expected_image`] is
    /// what answers it.
    pub current_image: Option<String>,
    /// Per-container CPU and memory come with each entry. There is no aggregate
    /// beside them: `moatless_api_schemas::sandbox::ResourceMetrics` is written
    /// by no adapter, so the panel sums these instead. See
    /// `docs/deferred-changes.md`.
    pub containers: Vec<SandboxContainer>,
    pub runtime_events: Vec<SandboxRuntimeEvent>,
}

/// One container of a sandbox, as the runtime reports it.
#[derive(Debug, Clone)]
pub struct SandboxContainer {
    pub name: String,
    /// `running`, `waiting`, `terminated`, or `unknown`.
    pub state: String,
    pub restart_count: i32,
    pub exit_code: Option<i32>,
    pub terminated_reason: Option<String>,
    pub last_exit_code: Option<i32>,
    pub last_terminated_reason: Option<String>,
    pub waiting_reason: Option<String>,
    /// `None` where the runtime publishes no metrics — a docker-compose
    /// deployment serves none, and a Kubernetes one without metrics-server
    /// does not either — so a panel hides the number rather than showing zero.
    pub cpu_millicores: Option<u64>,
    pub memory_mb: Option<u64>,
}

/// One runtime event about a sandbox — a scheduling failure, an image pull, a
/// crash-loop back-off.
#[derive(Debug, Clone)]
pub struct SandboxRuntimeEvent {
    pub reason: String,
    pub message: String,
    /// `normal` or `warning`.
    pub severity: Option<String>,
    pub count: Option<i32>,
    pub last_timestamp: Option<String>,
}

pub type ServerLogStream = BoxStream<'static, Result<String, SandboxError>>;

#[async_trait]
pub trait SandboxObserver: Send + Sync {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<ContractSandboxStatus, SandboxError>;

    /// The wider read behind the T3 sandbox panel, from the same backend
    /// response [`Self::sandbox_status`] narrows to two fields.
    async fn detail(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<SandboxDetail, SandboxError>;

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
