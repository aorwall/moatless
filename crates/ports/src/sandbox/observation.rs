use async_trait::async_trait;
use moatless_types::sandbox::LiveStatus;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::sandbox::SandboxError;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::task::TaskStatus;

/// The narrowed sandbox status the T3 contract observer returns: the field the
/// T3 surface was designed for.
///
/// Distinct from the wire response body `SandboxStatusFullResponse`, which
/// `moatless_sandbox_control::observation` projects down to this.
#[derive(Debug, Clone)]
pub struct ContractSandboxStatus {
    pub sandbox_status: SandboxStatus,
}

/// A Task's sandbox beyond its lifecycle state: what the T3 sandbox panel shows
/// under Image, Containers and Runtime events.
///
/// A second, wider narrowing of the same response, made in
/// `moatless_sandbox_control::observation`; kept separate so
/// [`ContractSandboxStatus`]'s two-field shape stays fixed.
#[derive(Debug, Clone)]
pub struct SandboxDetail {
    /// The image the sandbox is running. `None` where no sandbox is
    /// provisioned.
    ///
    /// The image a redeploy *would* install is not here — no status reader can
    /// see it; [`super::SandboxOperator::expected_image`] answers that instead.
    pub current_image: Option<String>,
    /// Per-container CPU and memory come with each entry; there is no
    /// aggregate.
    ///
    /// `ResourceMetrics` is written by no adapter, so the panel sums these
    /// instead. See `docs/deferred-changes.md`.
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
    /// deployment serves none, and a Kubernetes one without metrics-server does
    /// not either — so a panel hides the number rather than showing zero.
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

#[async_trait]
pub trait SandboxObserver: Send + Sync {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<ContractSandboxStatus, SandboxError>;

    /// The wider read behind the T3 sandbox panel, from the same backend
    /// response [`Self::sandbox_status`] narrows to one field.
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
}
