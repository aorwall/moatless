//! The event envelope: the top-level event type, the wire shape streamed to
//! the frontend, and the sub-type union that carries each domain's own
//! discriminator.
//!
//! The envelope crosses two boundaries:
//! - Sandbox → Backend (via gRPC / sandbox-proto)
//! - Backend → Redis → Frontend (via SSE / NDJSON)
//!
//! Each domain owns its own sub-types and payloads — see `sandbox`, `agent`,
//! `task`, and `message`. All of them serialize to the exact strings the
//! frontend expects.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use serde::{Deserialize, Serialize};

use crate::agent::AgentSubType;
use crate::message::MessageSubType;
use crate::sandbox::{SandboxStatus, ServerSubType};
use crate::task::{TaskSubType, TaskTurnSubType};

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
}
