//! Canonical event type enums and payloads shared by sandbox and backend.
//!
//! This crate defines the typed event system used across three boundaries:
//! - Sandbox → Backend (via gRPC / sandbox-proto)
//! - Backend → Redis → Frontend (via SSE / NDJSON)
//!
//! All event types serialize to the exact strings the frontend expects.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use serde::{Deserialize, Serialize};

/// Top-level event types.
///
/// Serializes to the exact strings the frontend `EventTypeSchema` expects.
/// Renamed: "environment" → "sandbox" to avoid confusion with K8s environments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum EventType {
    Task,
    TaskTurn,
    Sandbox,
    Agent,
    Server,
    Resources,
    /// Derived, UI-only events (`message.upsert` / `message.subagentProgress`).
    /// Assembled server-side and relayed to clients, never persisted to
    /// `task_events`.
    Message,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::TaskTurn => "taskTurn",
            Self::Sandbox => "sandbox",
            Self::Agent => "agent",
            Self::Server => "server",
            Self::Resources => "resources",
            Self::Message => "message",
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for EventType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "task" => Ok(Self::Task),
            "taskTurn" => Ok(Self::TaskTurn),
            "sandbox" => Ok(Self::Sandbox),
            "agent" => Ok(Self::Agent),
            "server" => Ok(Self::Server),
            "resources" => Ok(Self::Resources),
            "message" => Ok(Self::Message),
            other => Err(format!("unknown EventType: {other}")),
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

/// Matches TS ServerStatusEnumSchema: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
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

/// Agent sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AgentSubType {
    Started,
    Stopped,
    Message,
    Completed,
    Error,
    PendingResponse,
    SessionInit,
    Retrying,
    Scheduled,
    /// Sandbox restarted mid-run and is replaying the previously interrupted
    /// turn via `--resume <session_id>`.
    RecoveryStarted,
    /// Recovery attempts exhausted for an interrupted turn — the run has been
    /// abandoned and an Aborted Result row was inserted in its place.
    RecoveryExhausted,
    /// A session resume failed because the transcript is unavailable (e.g.
    /// stale session id after a pod restart); the agent fell back to a fresh
    /// session. Previous conversation context is lost. Warning, not an error.
    SessionContextLost,
    /// Lightweight liveness marker published (rate-limited) while a
    /// long-running tool call streams progress. Carries no conversation
    /// content — it exists so the reaper's activity floor keeps advancing
    /// during a multi-minute tool run instead of freezing at the last
    /// message. SSE-only downstream; never persisted as a message.
    Activity,
    /// What the harness said about its own context window: how much of it the
    /// session occupies, and how large it is where the harness states one.
    ContextWindow,
}

impl AgentSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Stopped => "stopped",
            Self::Message => "message",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::PendingResponse => "pending_response",
            Self::SessionInit => "session_init",
            Self::Retrying => "retrying",
            Self::Scheduled => "scheduled",
            Self::RecoveryStarted => "recovery_started",
            Self::RecoveryExhausted => "recovery_exhausted",
            Self::SessionContextLost => "session_context_lost",
            Self::Activity => "activity",
            Self::ContextWindow => "context_window",
        }
    }

    /// Whether this sub-type represents an error condition.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error | Self::RecoveryExhausted)
    }
}

impl std::fmt::Display for AgentSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for AgentSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "started" => Ok(Self::Started),
            "stopped" => Ok(Self::Stopped),
            "message" => Ok(Self::Message),
            "completed" => Ok(Self::Completed),
            "error" => Ok(Self::Error),
            "pending_response" => Ok(Self::PendingResponse),
            "session_init" => Ok(Self::SessionInit),
            "retrying" => Ok(Self::Retrying),
            "scheduled" => Ok(Self::Scheduled),
            "recovery_started" => Ok(Self::RecoveryStarted),
            "recovery_exhausted" => Ok(Self::RecoveryExhausted),
            "session_context_lost" => Ok(Self::SessionContextLost),
            "activity" => Ok(Self::Activity),
            "context_window" => Ok(Self::ContextWindow),
            other => Err(format!("unknown AgentSubType: {other}")),
        }
    }
}

/// Task sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskSubType {
    Created,
    Open,
    Closed,
    Updated,
    #[serde(rename = "metadata.updated")]
    MetadataUpdated,
}

impl TaskSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Updated => "updated",
            Self::MetadataUpdated => "metadata.updated",
        }
    }
}

impl std::fmt::Display for TaskSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TaskSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "created" => Ok(Self::Created),
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "updated" => Ok(Self::Updated),
            "metadata.updated" => Ok(Self::MetadataUpdated),
            other => Err(format!("unknown TaskSubType: {other}")),
        }
    }
}

/// TaskTurn sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskTurnSubType {
    Active,
    Completed,
    Error,
    Stopped,
    /// The turn's sandbox pod terminated involuntarily (eviction, node loss)
    /// while the turn was active. Emitted immediately by the pod watcher so the
    /// interruption is visible without waiting for the delayed orphaned-turn
    /// reconciliation. The turn resumes once the pod reschedules — not a
    /// terminal error, so `is_error` stays false.
    ///
    /// Renamed explicitly: `rename_all = "snake_case"` would yield
    /// `"interrupted"`, but the wire value (and `as_str`/`TryFrom`/frontend) is
    /// `"turn_interrupted"` — pin it so the serde-derived OpenAPI schema agrees.
    #[serde(rename = "turn_interrupted")]
    Interrupted,
}

impl TaskTurnSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Stopped => "stopped",
            Self::Interrupted => "turn_interrupted",
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }
}

impl std::fmt::Display for TaskTurnSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for TaskTurnSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            "error" => Ok(Self::Error),
            "stopped" => Ok(Self::Stopped),
            "turn_interrupted" => Ok(Self::Interrupted),
            other => Err(format!("unknown TaskTurnSubType: {other}")),
        }
    }
}

/// Message sub-types — derived, UI-only events for the live Chat.
///
/// `upsert` replaces-or-inserts an assembled Message by `uuid`;
/// `subagentProgress` carries a subagent's absolute activity summary. Neither
/// is an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MessageSubType {
    Upsert,
    /// A subagent step advanced: carries the absolute `{ toolUseId,
    /// messageCount, tokens, status }` summary for one `Agent` tool call. UI-only
    /// — keeps the collapsed `Agent` card live without streaming transcript
    /// content. Intentionally camelCase (not the crate's snake_case subtype
    /// convention) so the wire value matches the frontend event contract.
    #[serde(rename = "subagentProgress")]
    SubagentProgress,
}

impl MessageSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::SubagentProgress => "subagentProgress",
        }
    }

    pub fn is_error(&self) -> bool {
        false
    }
}

impl std::fmt::Display for MessageSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for MessageSubType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        match s {
            "upsert" => Ok(Self::Upsert),
            "subagentProgress" => Ok(Self::SubagentProgress),
            other => Err(format!("unknown MessageSubType: {other}")),
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
    /// Human-readable contextual detail (e.g. "ContainerCreating", "CrashLoopBackOff").
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
    /// K8s lifecycle event name (e.g. "pod_ready", "container_oom_killed").
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
    /// K8s termination reason of the sandbox container (e.g. "OOMKilled",
    /// "Error", "Completed"), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminated_reason: Option<String>,
    /// Node the pod was scheduled on (`pod.spec.node_name`). Enables node-level
    /// correlation of otherwise-unrelated tasks that stopped together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// Why the pod was disrupted, from
    /// `pod.status.conditions[DisruptionTarget].reason` (PreemptionByScheduler |
    /// DeletionByTaintManager | TerminationByKubelet | EvictionByEvictionAPI |
    /// DeletionByPodGC), falling back to `pod.status.reason`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disruption_reason: Option<String>,
    /// Who caused an intentional stop: "reaper" | "user" | "close" | "restart"
    /// | "crash_autostop" | "eviction". Null for events that aren't stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    /// Seconds the pod spent `Pending` before it was scheduled onto a node.
    /// Attached to `pod_scheduled` events so slow scheduling is readable directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_seconds: Option<i64>,
}

/// Agent event payload — typed shape for `EventType::Agent` events.
///
/// Agent payloads are pass-through from the sandbox (Claude CLI output).
/// This struct is primarily for OpenAPI schema generation; at runtime the
/// backend continues to forward `serde_json::Value` payloads.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AgentEventPayload {
    /// The message role type: "assistant", "user", etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub message_type: Option<String>,
    /// For "message" subtype: the Claude SDK assistant message.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub message: Option<serde_json::Value>,
    /// For "completed" subtype: the SDK result.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub result: Option<serde_json::Value>,
    /// For "pending_response" subtype: the tool being invoked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub tool_input: Option<serde_json::Value>,
    /// Context window usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<i64>,
    /// Cumulative cost in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
    /// The agent session ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
}

/// Task event payload — typed shape for `EventType::Task` events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskEventPayload {
    #[serde(default)]
    pub message: String,
}

/// TaskTurn event payload — typed shape for `EventType::TaskTurn` events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaskTurnEventPayload {
    #[serde(default)]
    pub message: String,
}

/// Resources event payload — typed shape for `EventType::Resources` events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResourcesEventPayload {
    #[serde(default)]
    pub message: String,
}

/// SSE event envelope — the shape of events streamed to the frontend.
///
/// This is the canonical wire format for all SSE events. The frontend
/// receives this shape and dispatches on `event_type` + `sub_type`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    pub event_id: String,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub sub_type: String,
    pub task_id: String,
    pub timestamp: String,
    pub is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
}

/// A typed sub-type that can be any of the specific sub-type enums.
///
/// Used by the publisher to accept any sub-type and serialize it correctly.
#[derive(Clone, Debug)]
pub enum SubType {
    Sandbox(SandboxStatus),
    Server(ServerSubType),
    Agent(AgentSubType),
    Task(TaskSubType),
    TaskTurn(TaskTurnSubType),
    Message(MessageSubType),
    /// Raw string for event types not yet typed (e.g., resources).
    Raw(String),
}

impl SubType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Sandbox(s) => s.as_str(),
            Self::Server(s) => s.as_str(),
            Self::Agent(s) => s.as_str(),
            Self::Task(s) => s.as_str(),
            Self::TaskTurn(s) => s.as_str(),
            Self::Message(s) => s.as_str(),
            Self::Raw(s) => s.as_str(),
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            Self::Sandbox(SandboxStatus::Error) => true,
            Self::Server(ServerSubType::Failed) => true,
            Self::Agent(s) => s.is_error(),
            Self::TaskTurn(s) => s.is_error(),
            _ => false,
        }
    }
}

impl std::fmt::Display for SubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// Convenience From impls
impl From<SandboxStatus> for SubType {
    fn from(s: SandboxStatus) -> Self {
        Self::Sandbox(s)
    }
}

impl From<ServerSubType> for SubType {
    fn from(s: ServerSubType) -> Self {
        Self::Server(s)
    }
}

impl From<AgentSubType> for SubType {
    fn from(s: AgentSubType) -> Self {
        Self::Agent(s)
    }
}

impl From<TaskSubType> for SubType {
    fn from(s: TaskSubType) -> Self {
        Self::Task(s)
    }
}

impl From<TaskTurnSubType> for SubType {
    fn from(s: TaskTurnSubType) -> Self {
        Self::TaskTurn(s)
    }
}

impl From<MessageSubType> for SubType {
    fn from(s: MessageSubType) -> Self {
        Self::Message(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_serializes_to_camel_case() {
        assert_eq!(
            serde_json::to_string(&EventType::TaskTurn).unwrap(),
            "\"taskTurn\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::Sandbox).unwrap(),
            "\"sandbox\""
        );
    }

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
    fn agent_sub_type_serializes() {
        assert_eq!(
            serde_json::to_string(&AgentSubType::PendingResponse).unwrap(),
            "\"pending_response\""
        );
    }

    #[test]
    fn agent_sub_type_session_context_lost_round_trips() {
        assert_eq!(
            AgentSubType::SessionContextLost.as_str(),
            "session_context_lost"
        );
        assert_eq!(
            serde_json::to_string(&AgentSubType::SessionContextLost).unwrap(),
            "\"session_context_lost\""
        );
        assert_eq!(
            AgentSubType::try_from("session_context_lost"),
            Ok(AgentSubType::SessionContextLost)
        );
        // A warning, not an error: the agent continues with a fresh session.
        assert!(!AgentSubType::SessionContextLost.is_error());
    }

    #[test]
    fn agent_sub_type_activity_round_trips() {
        assert_eq!(AgentSubType::Activity.as_str(), "activity");
        assert_eq!(
            serde_json::to_string(&AgentSubType::Activity).unwrap(),
            "\"activity\""
        );
        assert_eq!(
            AgentSubType::try_from("activity"),
            Ok(AgentSubType::Activity)
        );
        assert!(!AgentSubType::Activity.is_error());
    }

    #[test]
    fn task_sub_type_metadata_updated() {
        assert_eq!(
            serde_json::to_string(&TaskSubType::MetadataUpdated).unwrap(),
            "\"metadata.updated\""
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
    fn sub_type_is_error() {
        assert!(SubType::from(SandboxStatus::Error).is_error());
        assert!(SubType::from(ServerSubType::Failed).is_error());
        assert!(SubType::from(AgentSubType::Error).is_error());
        assert!(!SubType::from(AgentSubType::Completed).is_error());
        assert!(!SubType::from(SandboxStatus::Ready).is_error());
    }

    #[test]
    fn event_type_display() {
        assert_eq!(EventType::Task.to_string(), "task");
        assert_eq!(EventType::TaskTurn.to_string(), "taskTurn");
        assert_eq!(EventType::Sandbox.to_string(), "sandbox");
    }

    #[test]
    fn event_type_try_from() {
        assert_eq!(EventType::try_from("task"), Ok(EventType::Task));
        assert_eq!(EventType::try_from("taskTurn"), Ok(EventType::TaskTurn));
        assert_eq!(EventType::try_from("agent"), Ok(EventType::Agent));
        assert!(EventType::try_from("unknown").is_err());
    }

    #[test]
    fn agent_sub_type_try_from() {
        assert_eq!(
            AgentSubType::try_from("completed"),
            Ok(AgentSubType::Completed)
        );
        assert_eq!(
            AgentSubType::try_from("session_init"),
            Ok(AgentSubType::SessionInit)
        );
        assert!(AgentSubType::try_from("bogus").is_err());
    }

    #[test]
    fn task_sub_type_try_from() {
        assert_eq!(
            TaskSubType::try_from("metadata.updated"),
            Ok(TaskSubType::MetadataUpdated)
        );
        assert_eq!(TaskSubType::try_from("open"), Ok(TaskSubType::Open));
        assert!(TaskSubType::try_from("nope").is_err());
    }

    #[test]
    fn task_turn_sub_type_try_from() {
        assert_eq!(
            TaskTurnSubType::try_from("completed"),
            Ok(TaskTurnSubType::Completed)
        );
        assert_eq!(
            TaskTurnSubType::try_from("stopped"),
            Ok(TaskTurnSubType::Stopped)
        );
        assert_eq!(
            TaskTurnSubType::try_from("turn_interrupted"),
            Ok(TaskTurnSubType::Interrupted)
        );
        assert!(TaskTurnSubType::try_from("x").is_err());
    }

    #[test]
    fn task_turn_interrupted_round_trips_and_is_not_error() {
        // Wire string must match the frontend; interruption is recoverable, so
        // it must not carry the is_error flag (that path drives error handling).
        assert_eq!(TaskTurnSubType::Interrupted.as_str(), "turn_interrupted");
        assert_eq!(TaskTurnSubType::Interrupted.to_string(), "turn_interrupted");
        assert!(!TaskTurnSubType::Interrupted.is_error());
        assert!(!SubType::TaskTurn(TaskTurnSubType::Interrupted).is_error());
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
        // Legacy "fatal" maps to Error
        assert_eq!(SandboxStatus::try_from("fatal"), Ok(SandboxStatus::Error));
        assert!(SandboxStatus::try_from("bad").is_err());
    }

    #[test]
    fn server_sub_type_try_from() {
        assert_eq!(ServerSubType::try_from("failed"), Ok(ServerSubType::Failed));
        assert!(ServerSubType::try_from("nah").is_err());
    }
}
