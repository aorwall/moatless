//! Canonical event type enums and payloads shared by sandbox and backend.
//!
//! This crate defines the typed event system used across three boundaries:
//! - Sandbox → Backend (via gRPC / sandbox-proto)
//! - Backend → Redis → Frontend (via SSE / NDJSON)
//!
//! All event types serialize to the exact strings the frontend expects.

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
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Sandbox sub-types — valid status values for sandbox lifecycle events.
///
/// These match the frontend `EnvironmentStatusSchema` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SandboxSubType {
    NotCreated,
    Initializing,
    Ready,
    Stopped,
    Removing,
    Error,
}

impl SandboxSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotCreated => "not_created",
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Stopped => "stopped",
            Self::Removing => "removing",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for SandboxSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Server sub-types — server container lifecycle.
///
/// `Failed` covers all error conditions (was: Failed, Error, Timeout — consolidated).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Agent sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AgentSubType {
    Started,
    Stopped,
    Message,
    Completed,
    Error,
    PendingResponse,
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
        }
    }

    /// Whether this sub-type represents an error condition.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }
}

impl std::fmt::Display for AgentSubType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Task sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// TaskTurn sub-types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TaskTurnSubType {
    Active,
    Completed,
    Error,
    Stopped,
}

impl TaskTurnSubType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Stopped => "stopped",
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

/// Server event payload — matches frontend `ServerEventPayloadSchema`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerEventPayload {
    pub server_name: String,
    pub status: ServerSubType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    pub payload: serde_json::Value,
}

/// A typed sub-type that can be any of the specific sub-type enums.
///
/// Used by the publisher to accept any sub-type and serialize it correctly.
#[derive(Clone, Debug)]
pub enum SubType {
    Sandbox(SandboxSubType),
    Server(ServerSubType),
    Agent(AgentSubType),
    Task(TaskSubType),
    TaskTurn(TaskTurnSubType),
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
            Self::Raw(s) => s.as_str(),
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            Self::Sandbox(SandboxSubType::Error) => true,
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
impl From<SandboxSubType> for SubType {
    fn from(s: SandboxSubType) -> Self {
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

    #[test]
    fn sandbox_sub_type_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&SandboxSubType::NotCreated).unwrap(),
            "\"not_created\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxSubType::Initializing).unwrap(),
            "\"initializing\""
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
            error: None,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["serverName"], "preview");
        assert_eq!(json["status"], "started");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn sub_type_is_error() {
        assert!(SubType::from(SandboxSubType::Error).is_error());
        assert!(SubType::from(ServerSubType::Failed).is_error());
        assert!(SubType::from(AgentSubType::Error).is_error());
        assert!(!SubType::from(AgentSubType::Completed).is_error());
        assert!(!SubType::from(SandboxSubType::Ready).is_error());
    }

    #[test]
    fn event_type_display() {
        assert_eq!(EventType::Task.to_string(), "task");
        assert_eq!(EventType::TaskTurn.to_string(), "taskTurn");
        assert_eq!(EventType::Sandbox.to_string(), "sandbox");
    }
}
