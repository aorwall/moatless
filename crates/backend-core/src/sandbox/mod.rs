use std::collections::HashMap;

use chrono::{DateTime, Utc};

pub use moatless_types::repository::Repository;
pub use moatless_types::sandbox::{ArchiveRetention, SandboxDesiredState, SandboxStopReason};
pub use moatless_types::task::{CheckoutMode, Task, TaskStatus};
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::Serialize;

pub use moatless_types::sandbox::{AgentStatus, LiveStatus, SandboxStatus, ServerCapabilities, ServerEntry, ServerStatus};
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
    Provider { kind: ProviderKind, message: String },
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

#[derive(Debug, Clone)]
pub struct SandboxStatusFullResponse {
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

/// Wake `task_id`'s sandbox, returning the failure text (if any) for the caller
/// to persist or surface.
///
/// The one seam a message-send path uses, so its wake-failure handling is
/// reachable in a test through a [`SandboxWaker`] fake. It proves the seam — the
/// task id reaches the port unaltered and a failure is surfaced rather than
/// swallowed — not the wake behaviour itself, which lives in the sandbox-control
/// implementation and is recorded as still-unproven in `docs/deferred-changes.md`.
pub async fn wake_for_message(waker: &dyn SandboxWaker, task_id: &str) -> Option<String> {
    waker.wake(task_id).await.err()
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
            (
                ProviderKind::Kubernetes,
                "Kubernetes API error: 404 Not Found",
            ),
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

#[cfg(test)]
mod waker_tests {
    use super::*;
    use std::sync::Mutex;

    /// Records the ids it is woken with and returns a canned outcome.
    struct RecordingSandboxWaker {
        calls: Mutex<Vec<String>>,
        outcome: Result<(), String>,
    }

    #[async_trait]
    impl SandboxWaker for RecordingSandboxWaker {
        async fn wake(&self, task_id: &str) -> Result<(), String> {
            self.calls.lock().expect("lock").push(task_id.to_string());
            self.outcome.clone()
        }
    }

    /// The load-bearing assertion: the *task* id — not a message id or a turn id
    /// threaded nearby on the send path — is what reaches the port. That is the
    /// bug class this seam exists to catch; the success/None result is secondary.
    #[tokio::test]
    async fn wake_for_message_wakes_the_task_it_was_given() {
        let waker = RecordingSandboxWaker {
            calls: Mutex::new(Vec::new()),
            outcome: Ok(()),
        };

        let surfaced = wake_for_message(&waker, "task-42").await;

        assert_eq!(*waker.calls.lock().expect("lock"), vec!["task-42".to_string()]);
        assert_eq!(surfaced, None, "a successful wake surfaces no error");
    }

    /// Thinner — this is `Result::err` plumbing, kept for intent, not the
    /// assertion doing the work: a wake failure is handed back for the caller to
    /// persist rather than dropped.
    #[tokio::test]
    async fn wake_for_message_surfaces_the_failure_text() {
        let waker = RecordingSandboxWaker {
            calls: Mutex::new(Vec::new()),
            outcome: Err("sandbox unreachable".to_string()),
        };

        let surfaced = wake_for_message(&waker, "task-7").await;

        assert_eq!(*waker.calls.lock().expect("lock"), vec!["task-7".to_string()]);
        assert_eq!(surfaced.as_deref(), Some("sandbox unreachable"));
    }
}
