//! Capabilities shared by sandbox request operations.

use std::sync::Arc;

use super::lifecycle::Lifecycle;
use moatless_ports::sandbox::WorkspaceSnapshotSource;
use moatless_ports::task::TurnStateReader;
use moatless_sandbox_runtime::client::SandboxConnector;
use moatless_sandbox_runtime::status_reader::SandboxStatusReader;

/// State shared across sandbox API handlers.
#[derive(Clone)]
pub struct SandboxApiState {
    pub observer: Arc<dyn SandboxStatusReader>,
    pub connector: Arc<dyn SandboxConnector>,
    /// Shared durable lifecycle handle — the sole writer of Sandbox intent. The
    /// lifecycle HTTP handlers (`provision`/`stop`/`restart`/`redeploy`/`cleanup`)
    /// drive transitions through it. `None` only in the no-database mode, where
    /// those handlers return a clean "unavailable" error.
    pub lifecycle: Option<Arc<Lifecycle>>,
    /// The Task band's half of the running rule. `live_status` folds it into
    /// the runtime's answer so this surface and the T3 conversation list decide
    /// "is the agent working" from the same rule. `None` in the no-database
    /// mode, where there are no turn rows to read and the raw heartbeat is all
    /// there is.
    pub turns: Option<Arc<dyn TurnStateReader>>,
    /// The workspace as S3 last stored it, for reading a Task's files while no
    /// sandbox is running. `None` when the deployment stores no snapshots (no S3
    /// bucket configured), in which case a stopped Task answers as before —
    /// sandbox not ready.
    pub snapshots: Option<Arc<dyn WorkspaceSnapshotSource>>,
}
