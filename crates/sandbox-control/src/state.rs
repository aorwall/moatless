//! Capabilities shared by sandbox request operations.

use std::sync::Arc;

use super::lifecycle::Lifecycle;
use moatless_ports::sandbox::WorkspaceSnapshotSource;

use crate::workspace_repository::WorkspaceRepositoryAccess;
use moatless_ports::task::TurnStateReader;
use moatless_sandbox_runtime::client::SandboxConnector;
use moatless_sandbox_runtime::status_reader::SandboxStatusReader;

/// State shared across sandbox API handlers.
#[derive(Clone)]
pub struct SandboxApiState {
    pub observer: Arc<dyn SandboxStatusReader>,
    pub connector: Arc<dyn SandboxConnector>,
    /// Shared durable lifecycle handle — the sole writer of Sandbox intent.
    ///
    /// `provision`/`stop`/`restart`/`redeploy`/`cleanup` drive transitions
    /// through it; `None` only in no-database mode, where those return a clean
    /// "unavailable" error.
    pub lifecycle: Option<Arc<Lifecycle>>,
    /// The Task band's half of the running rule, folded into `live_status` so
    /// this surface and the T3 conversation list agree.
    ///
    /// `None` in no-database mode, where there are no turn rows and the raw
    /// heartbeat is all there is.
    pub turns: Option<Arc<dyn TurnStateReader>>,
    /// The workspace as S3 last stored it, for reading a Task's files while no
    /// sandbox is running.
    ///
    /// `None` when the deployment stores no snapshots (no S3 bucket
    /// configured); a stopped Task then answers as before — sandbox not ready.
    pub snapshots: Option<Arc<dyn WorkspaceSnapshotSource>>,
    /// Mints direct access to a task's workspace repository, for an
    /// administrator opening it with the `restic` CLI.
    ///
    /// `None` unless the deployment configures the repository role, the master
    /// secret, and a bucket.
    pub workspace_repository: Option<Arc<WorkspaceRepositoryAccess>>,
}
