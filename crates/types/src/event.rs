//! The event envelope: the top-level event type, the wire shape streamed to the
//! frontend, and the sub-type union that carries each domain's own
//! discriminator.
//!
//! Crosses two boundaries: Sandbox → Backend (gRPC / sandbox-proto) and Backend
//! → event feed → Frontend (SSE / NDJSON). Each domain owns its own sub-types and
//! payloads — see `sandbox`, `agent`, `task`, `message` — and all serialize to
//! the exact strings the frontend expects.

// reason: test code may unwrap freely; production code keeps the workspace `deny`.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use serde::{Deserialize, Serialize};

use crate::agent::AgentSubType;
use crate::message::MessageSubType;
use crate::sandbox::{SandboxStatus, ServerSubType};
use crate::task::{TaskSubType, TaskTurnSubType};

crate::string_enum!(
    /// Top-level event types.
    ///
    /// Serializes to the exact strings the frontend `EventTypeSchema` expects.
    EventType [Hash] {
        Task => "task",
        TaskTurn => "taskTurn",
        Sandbox => "sandbox",
        Agent => "agent",
        Server => "server",
        Resources => "resources",
        /// Derived, UI-only events (`message.upsert` / `message.subagentProgress`).
        /// Assembled server-side and relayed to clients, never persisted to
        /// `task_events`.
        Message => "message",
    }
);

/// SSE event envelope — the shape of events streamed to the frontend.
///
/// The canonical wire format for all SSE events; the frontend dispatches on
/// `event_type` + `sub_type`.
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
    /// What this event says about whether the agent is working, if anything.
    ///
    /// Absent means *this event does not say* (client keeps its last known
    /// value), never "not working". See [`agent_working`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_working: Option<bool>,
    pub payload: serde_json::Value,
}

/// What one event says about whether the agent is working.
///
/// An event knows only the turn half of the question:
/// `moatless_types::running::derive_running` reduced to what a turn state alone
/// implies, plus `agent.started`/`message`/`activity`, which move no turn row
/// but do mean the run is working.
pub fn agent_working(event_type: &str, sub_type: &str) -> Option<bool> {
    if let Some(status) = crate::task::TurnStatus::from_event(event_type, sub_type) {
        return Some(status.is_open());
    }
    match (event_type, sub_type) {
        ("agent", "started" | "message" | "activity") => Some(true),
        _ => None,
    }
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
    fn a_turn_bearing_event_says_whether_its_turn_is_open() {
        for (event_type, sub_type) in [
            ("taskTurn", "active"),
            // Interrupted resumes on the same row — the pod went away, the turn
            // did not end. Reading it as settled is what makes a resuming turn
            // look finished.
            ("taskTurn", "turn_interrupted"),
        ] {
            assert_eq!(
                agent_working(event_type, sub_type),
                Some(true),
                "{event_type}.{sub_type}"
            );
        }

        for (event_type, sub_type) in [
            ("taskTurn", "completed"),
            ("taskTurn", "error"),
            ("taskTurn", "stopped"),
            ("agent", "completed"),
            ("agent", "error"),
            ("agent", "stopped"),
        ] {
            assert_eq!(
                agent_working(event_type, sub_type),
                Some(false),
                "{event_type}.{sub_type}"
            );
        }
    }

    #[test]
    fn output_from_a_run_in_flight_says_working() {
        for sub_type in ["started", "message", "activity"] {
            assert_eq!(agent_working("agent", sub_type), Some(true), "{sub_type}");
        }
    }

    #[test]
    fn an_event_that_says_nothing_says_nothing_rather_than_not_working() {
        // The distinction the whole field rests on: `None` leaves the client's
        // last answer standing. Collapsing it to `false` would make every
        // sandbox event settle a running thread.
        for (event_type, sub_type) in [
            ("agent", "session_init"),
            ("agent", "retrying"),
            ("sandbox", "ready"),
            ("sandbox", "stopped"),
            ("task", "metadata.updated"),
            ("message", "upsert"),
        ] {
            assert_eq!(
                agent_working(event_type, sub_type),
                None,
                "{event_type}.{sub_type}"
            );
        }
    }

    #[test]
    fn the_envelope_omits_the_answer_it_does_not_have() {
        let event = StreamEvent {
            event_id: "evt-1".to_string(),
            event_type: EventType::Sandbox,
            sub_type: "ready".to_string(),
            task_id: "task-1".to_string(),
            timestamp: "2026-09-01T00:00:00.000Z".to_string(),
            is_error: false,
            agent_type: None,
            turn_number: None,
            agent_working: agent_working("sandbox", "ready"),
            payload: serde_json::json!({}),
        };
        let wire = serde_json::to_value(&event).unwrap();

        assert!(
            wire.get("agentWorking").is_none(),
            "an absent answer is an absent key, not a null: {wire}"
        );
    }

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
