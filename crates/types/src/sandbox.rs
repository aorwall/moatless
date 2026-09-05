//! Sandbox vocabulary: which provider runs a Sandbox, what survives its
//! removal, and the live status of a Sandbox and of the servers inside it.
//!
//! `AgentStatus` lives here, not in `agent`, because it belongs to the
//! live-status cluster: [`LiveStatus`] names it alongside [`SandboxStatus`],
//! and its consumers are the sandbox band and the HTTP schema chain —
//! disjoint from those of `agent::AgentSubType`.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The timestamp for a Sandbox that has never received durable lifecycle
/// intent. Every real Message timestamp sorts after this sentinel.
pub fn sandbox_intent_epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from(std::time::UNIX_EPOCH)
}

/// Which sandbox provider implementation to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProviderType {
    Kubernetes,
    Docker,
}

impl fmt::Display for SandboxProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kubernetes => write!(f, "kubernetes"),
            Self::Docker => write!(f, "docker"),
        }
    }
}

impl FromStr for SandboxProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "kubernetes" | "k8s" => Ok(Self::Kubernetes),
            "docker" => Ok(Self::Docker),
            _ => Err(format!("unknown sandbox provider: {s}")),
        }
    }
}

/// Typed policy for task archive retention.
/// Runtime resources (Deployment, Service, `IngressRoutes`, **and the PVC**) are
/// always removed regardless of this policy; it governs S3 archives and
/// managed provider-local disk archives (opt-in Kubernetes node hostPaths and
/// Docker task directories).
///
/// Persisted alongside `Removed` intent in the same atomic write so a crashed
/// remove can be retried with the *original* policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveRetention {
    /// Soft removal: keep archives so the task can be revived (reopen).
    RetainArchives,
    /// Permanent deletion: remove all archives too. Never superseded by a
    /// later message-wake — deletion wins.
    DeleteArchives,
}

impl ArchiveRetention {
    pub fn retains(self) -> bool {
        matches!(self, Self::RetainArchives)
    }

    /// The close-time policy from the `SANDBOX_RETAIN_STORAGE_ON_CLOSE` flag:
    /// retain archives (revivable) when set, delete them otherwise. One helper
    /// so the close/auto-close/adapter-close sites can't drift.
    pub fn on_close(retain_archives: bool) -> Self {
        if retain_archives {
            Self::RetainArchives
        } else {
            Self::DeleteArchives
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RetainArchives => "retain_archives",
            Self::DeleteArchives => "delete_archives",
        }
    }

    /// Parse the persisted slug back into a policy. `None` for an unknown value.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "retain_archives" => Some(Self::RetainArchives),
            "delete_archives" => Some(Self::DeleteArchives),
            _ => None,
        }
    }
}

/// Sandbox status — valid status values for sandbox lifecycle events.
///
/// These match the frontend `SandboxStatus` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    NotCreated,
    Initializing,
    Ready,
    Stopped,
    Removing,
    Removed,
    Error,
}

impl SandboxStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotCreated => "not_created",
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Stopped => "stopped",
            Self::Removing => "removing",
            Self::Removed => "removed",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for SandboxStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for SandboxStatus {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "not_created" => Ok(Self::NotCreated),
            "initializing" => Ok(Self::Initializing),
            "ready" => Ok(Self::Ready),
            "stopped" => Ok(Self::Stopped),
            "removing" => Ok(Self::Removing),
            "removed" => Ok(Self::Removed),
            "error" => Ok(Self::Error),
            // Accept legacy "fatal" as Error for backwards compatibility
            "fatal" => Ok(Self::Error),
            other => Err(format!("unknown SandboxStatus: {other}")),
        }
    }
}

/// Matches TS `ServerStatusEnumSchema`: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Stopped,
    Installing,
    Starting,
    Started,
    Failed,
}

impl ServerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Installing => "installing",
            Self::Starting => "starting",
            Self::Started => "started",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for ServerStatus {
    /// Lenient: an unrecognised value reads as [`ServerStatus::Stopped`] rather
    /// than failing.
    fn from(s: &str) -> Self {
        match s {
            "installing" => Self::Installing,
            "starting" => Self::Starting,
            "started" => Self::Started,
            "failed" => Self::Failed,
            _ => Self::Stopped,
        }
    }
}

/// Server capabilities (deterministic from server name)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub address_bar: bool,
    pub inspector: bool,
    pub reload: bool,
    pub open_external: bool,
    pub viewport: bool,
    pub console: bool,
}

/// Single server entry in the workspace status response
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub name: String,
    pub label: String,
    pub port: u16,
    pub status: ServerStatus,
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub default: bool,
    pub capabilities: ServerCapabilities,
}

/// Observed Agent lifecycle while its Sandbox is ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Running,
    /// Parked on a person: the run is open, and nothing is running until it is
    /// answered.
    Waiting,
    Idle,
    Unknown,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for AgentStatus {
    /// Lenient: an unrecognised value reads as [`AgentStatus::Unknown`] rather
    /// than failing, and legacy `"stopped"` reads as [`AgentStatus::Idle`].
    fn from(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "idle" | "stopped" => Self::Idle,
            _ => Self::Unknown,
        }
    }
}

/// Lifecycle of a command registered through `moat cmd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum ManagedCommandState {
    Running,
    Exited,
    /// Killed by the sandbox because it outlived its deadline.
    TimedOut,
    /// Killed on an explicit `moat cmd kill`.
    Killed,
}

impl ManagedCommandState {
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// A registered command, narrowed to what a status reader needs to say one is
/// still running and name it.
///
/// The full shape is `moatless_api_schemas::sandbox::ManagedCommand`, which the
/// status-read response carries. This is the half of it that belongs on a
/// status polled every couple of seconds: `pid`, the whole command line, and
/// the log path answer questions a status indicator does not ask.
///
/// Every time here is an instant, never an elapsed duration. A reader computes
/// "12m in" from `started_at_unix_ms` itself, because a duration recomputed on
/// the producing side changes on every read, and a push that only sends changes
/// would then send one every poll forever.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CommandSummary {
    pub id: String,
    /// Human label, defaulted by the sandbox to a prefix of the command line.
    pub label: String,
    pub state: ManagedCommandState,
    pub started_at_unix_ms: i64,
    /// When the sandbox kills it for outliving its deadline.
    pub deadline_unix_ms: i64,
    /// Set once the command reached a terminal state. 124 is a timeout kill,
    /// matching coreutils `timeout`.
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub exit_code: Option<i32>,
}

/// The complete live status of a Task's Sandbox and Agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LiveStatus {
    pub sandbox_status: SandboxStatus,
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub agent_status: Option<AgentStatus>,
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub sandbox_error: Option<String>,
    /// Commands registered through `moat cmd`, running and recently finished.
    ///
    /// The agent reports `idle` while one of these runs — its turn ended and
    /// the sandbox is carrying the work — so a reader with the agent status
    /// alone cannot tell a finished Task from one that is mid-build. Empty
    /// whenever the sandbox is not `Ready`, or when nothing is registered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandSummary>,
}

impl LiveStatus {
    /// Attach the registered commands read from the same agent-status call.
    ///
    /// Separate from construction because `derive_live_status` owns the rules
    /// that turn raw signals into display status, and these are carried
    /// through rather than derived from anything.
    #[must_use]
    pub fn with_commands(mut self, commands: Vec<CommandSummary>) -> Self {
        self.commands = commands;
        self
    }
}

/// Server sub-types — server container lifecycle.
///
/// `Failed` covers all error conditions (was: Failed, Error, Timeout — consolidated).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServerSubType {
    Installing,
    Starting,
    Started,
    Stopped,
    Failed,
}

impl ServerSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Installing => "installing",
            Self::Starting => "starting",
            Self::Started => "started",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for ServerSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ServerSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "installing" => Ok(Self::Installing),
            "starting" => Ok(Self::Starting),
            "started" => Ok(Self::Started),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown ServerSubType: {other}")),
        }
    }
}

/// Server event payload — matches frontend `ServerEventPayloadSchema`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerEventPayload {
    pub server_name: String,
    pub status: ServerSubType,
    /// Human-readable contextual detail (e.g. "`ContainerCreating`", "`CrashLoopBackOff`").
    /// Always set by the publisher. `#[serde(default)]` handles old events without this field.
    #[serde(default)]
    pub message: String,
}

/// Sandbox event payload — typed payload for sandbox lifecycle events.
///
/// Used by the pod watcher and provider to publish structured events
/// instead of raw `serde_json::Value`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SandboxEventPayload {
    /// Human-readable description of the event.
    #[serde(default)]
    pub message: String,
    /// K8s lifecycle event name (e.g. "`pod_ready`", "`container_oom_killed`").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_event: Option<String>,
    /// Error detail when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Exit code of the terminated sandbox container, when known.
    ///
    /// Lets consumers distinguish OOM (137) from an application crash (1) or a
    /// graceful SIGTERM (143). `#[serde(default)]` keeps old events readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// K8s termination reason of the sandbox container (e.g. "`OOMKilled`",
    /// "Error", "Completed"), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminated_reason: Option<String>,
    /// Node the pod was scheduled on (`pod.spec.node_name`). Enables node-level
    /// correlation of otherwise-unrelated tasks that stopped together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// Why the pod was disrupted, from
    /// `pod.status.conditions[DisruptionTarget].reason` (`PreemptionByScheduler` |
    /// `DeletionByTaintManager` | `TerminationByKubelet` | `EvictionByEvictionAPI` |
    /// `DeletionByPodGC`), falling back to `pod.status.reason`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disruption_reason: Option<String>,
    /// Who caused an intentional stop: "reaper" | "user" | "close" | "restart"
    /// | "`crash_autostop`" | "eviction". Null for events that aren't stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    /// Seconds the pod spent `Pending` before it was scheduled onto a node.
    /// Attached to `pod_scheduled` events so slow scheduling is readable directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_seconds: Option<i64>,
}

/// Resources event payload — typed shape for `EventType::Resources` events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResourcesEventPayload {
    #[serde(default)]
    pub message: String,
}

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

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Kubernetes => "Kubernetes",
            Self::Docker => "Docker",
        })
    }
}

/// A sandbox operation failure, in the vocabulary every band shares.
///
/// The two transport variants box their source so this crate names no gRPC
/// type: a caller either matches the variant or moves `source` into its own
/// error's `#[source]`, and the underlying status still reaches a log through
/// the rendered source chain.
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
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("sandbox gRPC error in {operation}: {source}")]
    Grpc {
        operation: String,
        source: Box<dyn std::error::Error + Send + Sync>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every status the backend can serialize must round-trip here — a missing
    /// variant fails the whole `/sandbox/status` response, not just the field.
    #[test]
    fn every_agent_status_deserializes() {
        for wire in ["running", "waiting", "idle", "unknown"] {
            let json = format!("\"{wire}\"");
            let parsed: AgentStatus =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("{wire}: {e}"));
            assert_eq!(parsed.to_string(), wire);
            assert_eq!(AgentStatus::from(wire), parsed);
        }
    }

    #[test]
    fn archive_retention_retains_only_when_asked() {
        assert!(ArchiveRetention::RetainArchives.retains());
        assert!(!ArchiveRetention::DeleteArchives.retains());
    }

    #[test]
    fn archive_retention_db_str_round_trips() {
        for r in [
            ArchiveRetention::RetainArchives,
            ArchiveRetention::DeleteArchives,
        ] {
            assert_eq!(ArchiveRetention::from_db_str(r.as_str()), Some(r));
        }
        assert_eq!(ArchiveRetention::from_db_str("bogus"), None);
    }

    #[test]
    fn every_server_status_deserializes() {
        for wire in ["stopped", "installing", "starting", "started", "failed"] {
            let json = format!("\"{wire}\"");
            let parsed: ServerStatus =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("{wire}: {e}"));
            assert_eq!(parsed.to_string(), wire);
            assert_eq!(ServerStatus::from(wire), parsed);
        }
    }

    #[test]
    fn sandbox_status_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&SandboxStatus::NotCreated).unwrap(),
            "\"not_created\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxStatus::Initializing).unwrap(),
            "\"initializing\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxStatus::Removed).unwrap(),
            "\"removed\""
        );
    }

    #[test]
    fn sandbox_status_display_covers_every_variant() {
        for (status, text) in [
            (SandboxStatus::NotCreated, "not_created"),
            (SandboxStatus::Initializing, "initializing"),
            (SandboxStatus::Ready, "ready"),
            (SandboxStatus::Stopped, "stopped"),
            (SandboxStatus::Removing, "removing"),
            (SandboxStatus::Removed, "removed"),
            (SandboxStatus::Error, "error"),
        ] {
            assert_eq!(status.to_string(), text);
        }
    }

    #[test]
    fn server_status_from_unrecognized_is_stopped() {
        assert_eq!(ServerStatus::from("nonsense"), ServerStatus::Stopped);
    }

    #[test]
    fn server_sub_type_serializes() {
        assert_eq!(
            serde_json::to_string(&ServerSubType::Started).unwrap(),
            "\"started\""
        );
        assert_eq!(
            serde_json::to_string(&ServerSubType::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn server_event_payload_serializes() {
        let payload = ServerEventPayload {
            server_name: "preview".to_string(),
            status: ServerSubType::Started,
            message: String::new(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["serverName"], "preview");
        assert_eq!(json["status"], "started");
        assert_eq!(json["message"], "");
    }

    #[test]
    fn server_event_payload_with_message() {
        let payload = ServerEventPayload {
            server_name: "preview".to_string(),
            status: ServerSubType::Starting,
            message: "ContainerCreating".to_string(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["message"], "ContainerCreating");
    }

    #[test]
    fn server_event_payload_deserializes_without_message() {
        let json = serde_json::json!({
            "serverName": "preview",
            "status": "started"
        });
        let payload: ServerEventPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.message, "");
    }

    #[test]
    fn sandbox_status_try_from() {
        assert_eq!(
            SandboxStatus::try_from("not_created"),
            Ok(SandboxStatus::NotCreated)
        );
        assert_eq!(
            SandboxStatus::try_from("removed"),
            Ok(SandboxStatus::Removed)
        );
        assert_eq!(SandboxStatus::try_from("fatal"), Ok(SandboxStatus::Error));
        assert!(SandboxStatus::try_from("bad").is_err());
    }

    #[test]
    fn server_sub_type_try_from() {
        assert_eq!(ServerSubType::try_from("failed"), Ok(ServerSubType::Failed));
        assert!(ServerSubType::try_from("nah").is_err());
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

    /// A boxed transport source stays reachable through `Error::source`, which
    /// is how the underlying status reaches a log.
    #[test]
    fn a_transport_failure_keeps_its_source_in_the_chain() {
        let err = SandboxError::Grpc {
            operation: "read_file".to_string(),
            source: Box::new(std::io::Error::other("boom")),
        };
        assert!(err.to_string().contains("read_file"));
        assert_eq!(
            std::error::Error::source(&err).map(ToString::to_string),
            Some("boom".to_string()),
            "the transport status stays on the source chain"
        );
    }
}

crate::string_enum!(
    /// Persisted application intent for a task's sandbox lifecycle.
    SandboxDesiredState {
        Running => "running",
        Stopped => "stopped",
        Removed => "removed",
    }
);

crate::string_enum!(
    /// Reason the persisted sandbox intent moved away from `running`.
    SandboxStopReason {
        User => "user",
        IdleReaper => "idle_reaper",
        Cleanup => "cleanup",
        System => "system",
        Error => "error",
    }
);

#[cfg(test)]
mod wire_contract {
    use super::*;

    #[test]
    fn sandbox_intent_wire_strings() {
        for (value, wire) in [
            (SandboxDesiredState::Running, "running"),
            (SandboxDesiredState::Stopped, "stopped"),
            (SandboxDesiredState::Removed, "removed"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(SandboxDesiredState::try_from(wire), Ok(value));
            assert_eq!(serde_json::to_string(&value).unwrap(), format!("\"{wire}\""));
        }
        for (value, wire) in [
            (SandboxStopReason::User, "user"),
            (SandboxStopReason::IdleReaper, "idle_reaper"),
            (SandboxStopReason::Cleanup, "cleanup"),
            (SandboxStopReason::System, "system"),
            (SandboxStopReason::Error, "error"),
        ] {
            assert_eq!(value.as_str(), wire);
            assert_eq!(SandboxStopReason::try_from(wire), Ok(value));
            assert_eq!(serde_json::to_string(&value).unwrap(), format!("\"{wire}\""));
        }
    }
}
