use std::collections::HashMap;

use chrono::{DateTime, Utc};

use async_trait::async_trait;
use futures::stream::BoxStream;
pub use moatless_types::repository::Repository;
pub use moatless_types::sandbox::{ArchiveRetention, SandboxDesiredState, SandboxStopReason};
pub use moatless_types::task::{CheckoutMode, Task, TaskStatus};
use serde::Serialize;

pub use moatless_types::sandbox::{
    AgentStatus, LiveStatus, ProviderKind, SandboxError, SandboxStatus, ServerCapabilities,
    ServerEntry, ServerStatus,
};

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

#[async_trait]
pub trait SandboxIntentReader: Send + Sync {
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxError>;
}

#[async_trait]
pub trait SandboxStarter: Send + Sync {
    async fn start(&self, task_id: &str) -> Result<(), SandboxError>;
}

/// Preflight seam: credential validation before a sandbox is provisioned.
/// Wired once in state assembly (`RealPreflight`, in `sandbox::preflight`) so
/// every creation path gets the same check for free; mocked in unit tests.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait PreflightPort: Send + Sync {
    async fn validate(&self, task: &Task, repo: &Repository) -> Result<(), PreflightRejection>;
}

/// Band-clean preflight failure returned by [`PreflightPort`]. The
/// sandbox-control implementation pre-computes both strings from its rich
/// `PreflightError` so the tasks band names no preflight-credential type:
/// `user_message` drives the 403 body + `sandbox.error` event, `detail` is the
/// underlying failure's `Display` for the log + event detail field.
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct PreflightRejection {
    pub user_message: String,
    pub detail: String,
}

/// Registers a task as a member of an already-running sandbox process.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SandboxJoiner: Send + Sync {
    async fn join_task(
        &self,
        task_id: &str,
        checkout_mode: CheckoutMode,
    ) -> Result<(), SandboxError>;
}

/// Ensure a task's sandbox is running and notify it of new work.
///
/// A closed task is a no-op. On failure the wake returns a human-readable
/// string so the caller can decide whether to surface it to the user.
#[async_trait]
pub trait SandboxWaker: Send + Sync {
    async fn wake(&self, task_id: &str) -> Result<(), String>;
}

/// Why a snapshot answered nothing.
#[derive(Debug)]
pub enum SnapshotError {
    /// No snapshot is stored for this task, so there is nothing to fall back
    /// to. A task whose sandbox has never synced, and every task that only ever
    /// wrote a tar.
    NoSnapshot,
    /// The snapshot exists and does not hold that path.
    NotFound,
    /// The store could not be read. Distinct from [`Self::NoSnapshot`] because
    /// an outage must not read to a caller as "this task has no files".
    Unavailable(String),
}

/// One file, as the snapshot stored it.
#[derive(Debug, Clone)]
pub struct SnapshotFile {
    pub content: Vec<u8>,
    /// When the snapshot this came from was written.
    pub synced_at: DateTime<Utc>,
}

/// One entry in a snapshot listing. A directory is implied by the paths under
/// it — the snapshot stores files — so `is_dir` marks one this listing
/// synthesised.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Workspace-relative path, `/`-separated.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct SnapshotListing {
    pub entries: Vec<SnapshotEntry>,
    pub synced_at: DateTime<Utc>,
}

/// The workspace as S3 last stored it, for a task whose sandbox is not running.
///
/// The three reads a stopped sandbox can still answer. There is no write: a
/// snapshot is what the workspace was, and changing it would put the store and
/// the next pod's restore at odds.
///
/// **Every method is scoped to one task**, so a caller that has already decided
/// the viewer may read that task has decided the whole access question.
#[async_trait]
pub trait WorkspaceSnapshotSource: Send + Sync {
    async fn read(&self, task_id: &str, path: &str) -> Result<SnapshotFile, SnapshotError>;

    /// Entries directly under `path`, or the whole subtree when `recursive`.
    async fn list(
        &self,
        task_id: &str,
        path: &str,
        recursive: bool,
    ) -> Result<SnapshotListing, SnapshotError>;

    /// Files whose path contains `query`. `limit` of 0 means no cap.
    async fn search(
        &self,
        task_id: &str,
        query: &str,
        case_sensitive: bool,
        limit: usize,
    ) -> Result<SnapshotListing, SnapshotError>;
}

/// Push one rotated credential to a task's running sandbox.
///
/// The narrow half of the connector the credential fan-out needs. It is its own
/// port so the tasks band, which does nothing else with a sandbox client, names
/// no gRPC type: the sandbox band implements it over the connector.
#[async_trait]
pub trait CredentialBroadcast: Send + Sync {
    async fn refresh_credential(
        &self,
        task_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), SandboxError>;
}

#[async_trait]
pub trait SandboxStopper: Send + Sync {
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxError>;
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

/// A lifecycle operation failure, band-clean and reduced to what the task API
/// surfaces. The sandbox-band implementation pre-classifies each variant from
/// the rich `LifecycleError` so the tasks band names no `sandbox` error type.
#[derive(Debug, thiserror::Error)]
pub enum SandboxOpError {
    /// The Task has no row — maps to a 404.
    #[error("task not found: {0}")]
    NotFound(String),
    /// The operation was rejected for the Task's current state (the `Closed`
    /// guard) — a client state error, maps to a 400.
    #[error("{0}")]
    NotAllowed(String),
    /// Another operation holds the per-Task lock — maps to a 409.
    #[error("{0}")]
    InProgress(String),
    /// Any other lifecycle failure — maps to a 500 with the rendered chain.
    #[error("{0}")]
    Internal(String),
}

/// The durable sandbox-lifecycle contract the tasks band drives.
///
/// The task closer, reaper, HTTP handlers, and service drive a Task's sandbox —
/// start it, stop it, remove it, read its desired state — but they are a band
/// below the `Lifecycle` orchestrator that owns those transitions. This port
/// inverts that edge: the consumers hold `Arc<dyn SandboxLifecycle>` and name no
/// `sandbox` type, and `Lifecycle` implements it at the composition root.
///
/// It is deliberately **not** [`SandboxStarter`]/[`SandboxStopper`]
/// /[`SandboxIntentReader`]: those flatten a lifecycle failure through
/// [`SandboxError`], which has no `Conflict` variant, so lock contention
/// (`OperationInProgress`) that the task API answers **409** would arrive as an
/// internal **500**. The tasks band needs that 409, so this contract's error
/// preserves the three client-visible outcomes. The
/// `LifecycleError -> SandboxOpError` mapping lives in the sandbox-band
/// implementation; `moatless_tasks::task::error::TaskError` maps
/// `SandboxOpError` the rest of the way (404 / 400 / 409 / 500).
///
/// Sized to the tasks-band cross-boundary surface: the concrete `Lifecycle` also
/// serves `restart`/`redeploy`/`expected_sandbox_image`, but those have no
/// tasks-band caller (only the sandbox HTTP handlers, on the legal side of the
/// boundary), so they stay off the port.
#[async_trait]
pub trait SandboxLifecycle: Send + Sync {
    /// Provision and start the Task's sandbox.
    async fn start(&self, task_id: &str) -> Result<(), SandboxOpError>;

    /// Stop the Task's sandbox, recording why.
    async fn stop(&self, task_id: &str, reason: SandboxStopReason) -> Result<(), SandboxOpError>;

    /// Stop the agent running inside the Task's sandbox, leaving the sandbox
    /// up. Not a lifecycle transition: nothing is recorded and the desired
    /// state is untouched, so the next turn runs in the same sandbox.
    ///
    /// Best-effort, like the graceful stop [`Self::stop`] issues before it
    /// tears compute down: a sandbox that cannot be reached is one whose agent
    /// is not running anyway. The terminal `agent.stopped` the caller publishes
    /// is what closes the turn.
    async fn stop_agent(&self, task_id: &str, reason: SandboxStopReason);

    /// Remove the Task's sandbox, applying the archive-retention policy. The
    /// caller deletes the Task row only on success.
    async fn remove(
        &self,
        task_id: &str,
        retention: ArchiveRetention,
    ) -> Result<(), SandboxOpError>;

    /// Reclaim the storage of a Task whose sandbox has been stopped long
    /// enough, leaving the durable archive as the only copy. The Task stays
    /// Open and its sandbox stays `Stopped` — a later start restores from the
    /// archive — so this is a cost transition, not a state one, and the reaper
    /// is its only caller.
    ///
    /// Idempotent: a Task whose storage is already reclaimed is not an error.
    async fn archive_storage(&self, task_id: &str) -> Result<(), SandboxOpError>;

    /// Which of `task_ids` have a stopped sandbox that still owns storage, and
    /// when each stopped. The candidate set for [`Self::archive_storage`]: the
    /// caller applies its own per-Task clock on top, because the threshold is a
    /// Task property and the storage is a sandbox one.
    async fn stopped_with_storage(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, SandboxOpError>;

    /// The persisted desired state for the Task's sandbox (absence reads as
    /// converged `Stopped`).
    async fn desired_state(&self, task_id: &str) -> Result<SandboxDesiredState, SandboxOpError>;
}

/// Just-in-time Codex credential refresh for the identity a sandbox acts as.
///
/// The refresh itself needs the tasks band — it yields to a running sandbox's
/// own refresher rather than racing it for the single-use refresh token, and it
/// fans a rotated blob out to every open codex sandbox in scope. This port
/// inverts that edge so the sandbox band, which only needs the resulting
/// `auth.json`, names no tasks-band type.
///
/// `Ok(None)` is "no codex credential at any scope", which is an ordinary
/// outcome: the caller leaves the slot empty and lets a scoped secret fill it.
/// `Err` is a failure to report.
#[async_trait]
pub trait CodexCredentials: Send + Sync {
    async fn ensure_fresh_codex_for(
        &self,
        user_id: &str,
        except_task_id: Option<&str>,
    ) -> Result<Option<String>, String>;
}
