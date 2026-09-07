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

pub type ServerLogStream = BoxStream<'static, Result<String, SandboxError>>;

#[async_trait]
pub trait SandboxObserver: Send + Sync {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<ContractSandboxStatus, SandboxError>;

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
