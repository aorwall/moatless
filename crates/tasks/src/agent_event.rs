//! Parsed agent-event vocabulary — the typed event a Redis stream payload
//! deserializes into, and the dispatch key derived from its type/sub-type.
//!
//! Consumed by the server-band event handler (which parses and dispatches) and
//! by the tasks-band reaper (which constructs a synthetic error event for
//! orphaned turns). Placed at the tasks band so neither consumer reaches up
//! into the other.

use moatless_types::agent::AgentSubType;
use moatless_types::event::EventType;
use moatless_types::sandbox::{SandboxStatus, ServerSubType};
use moatless_types::task::{TaskSubType, TaskTurnSubType};
use serde::Deserialize;

/// Typed event dispatch key — replaces string-concat `format!("{}.{}", type, sub)`.
///
/// `Hash` so a subscriber registry can key on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventKey {
    Agent(AgentSubType),
    Task(TaskSubType),
    TaskTurn(TaskTurnSubType),
    Sandbox(SandboxStatus),
    Server(ServerSubType),
    /// Unrecognised event_type + sub_type pair.
    Unknown(String, String),
}

/// Parsed agent event from Redis stream payload.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub event_id: String,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub sub_type: String,
    pub task_id: String,
    pub timestamp: String,
    pub is_error: bool,
    pub agent_type: Option<String>,
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
}

impl AgentEvent {
    /// Parse `event_type` + `sub_type` into a typed `EventKey`.
    pub fn event_key(&self) -> EventKey {
        match self.event_type {
            EventType::Agent => AgentSubType::try_from(self.sub_type.as_str())
                .map_or_else(|_| self.unknown(), EventKey::Agent),
            EventType::Task => TaskSubType::try_from(self.sub_type.as_str())
                .map_or_else(|_| self.unknown(), EventKey::Task),
            EventType::TaskTurn => TaskTurnSubType::try_from(self.sub_type.as_str())
                .map_or_else(|_| self.unknown(), EventKey::TaskTurn),
            EventType::Sandbox => SandboxStatus::try_from(self.sub_type.as_str())
                .map_or_else(|_| self.unknown(), EventKey::Sandbox),
            EventType::Server => ServerSubType::try_from(self.sub_type.as_str())
                .map_or_else(|_| self.unknown(), EventKey::Server),
            EventType::Resources => self.unknown(),
            // Derived, UI-only events are gated out before dispatch (see
            // `EventHandlerConsumer::process_response`); never reached here.
            EventType::Message => self.unknown(),
        }
    }

    fn unknown(&self) -> EventKey {
        EventKey::Unknown(self.event_type.to_string(), self.sub_type.clone())
    }
}

/// Whether an event `(type, sub_type)` is an `agent.message` source event. The
/// derived `message.upsert` / `subagentProgress` events (`type = "message"`) are
/// the Chat's content delivery and are a different pair.
pub fn is_agent_message(event_type: &str, sub_type: &str) -> bool {
    event_type == "agent" && sub_type == "message"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_agent_event_full() {
        let v = json!({
            "eventId": "evt-1",
            "type": "agent",
            "subType": "completed",
            "taskId": "task-1",
            "timestamp": "2025-01-01T00:00:00Z",
            "isError": false,
            "agentType": "claude",
            "turnNumber": 3,
            "payload": { "key": "value" }
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(evt.event_id, "evt-1");
        assert_eq!(evt.event_type, EventType::Agent);
        assert_eq!(evt.sub_type, "completed");
        assert_eq!(evt.task_id, "task-1");
        assert!(!evt.is_error);
        assert_eq!(evt.agent_type.as_deref(), Some("claude"));
        assert_eq!(evt.turn_number, Some(3));
    }

    #[test]
    fn deserialize_agent_event_minimal() {
        let v = json!({
            "eventId": "e",
            "type": "agent",
            "subType": "message",
            "taskId": "t",
            "timestamp": "now",
            "isError": true,
            "payload": {}
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(evt.event_type, EventType::Agent);
        assert!(evt.is_error);
        assert!(evt.agent_type.is_none());
        assert!(evt.turn_number.is_none());
    }

    #[test]
    fn event_key_agent_completed() {
        let v = json!({
            "eventId": "e",
            "type": "agent",
            "subType": "completed",
            "taskId": "t",
            "timestamp": "now",
            "isError": false,
            "payload": {}
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(evt.event_key(), EventKey::Agent(AgentSubType::Completed));
    }

    #[test]
    fn event_key_task_turn_error() {
        let v = json!({
            "eventId": "e",
            "type": "taskTurn",
            "subType": "error",
            "taskId": "t",
            "timestamp": "now",
            "isError": true,
            "payload": {}
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(evt.event_key(), EventKey::TaskTurn(TaskTurnSubType::Error));
    }

    #[test]
    fn event_key_unknown_sub_type() {
        let v = json!({
            "eventId": "e",
            "type": "agent",
            "subType": "bizarre",
            "taskId": "t",
            "timestamp": "now",
            "isError": false,
            "payload": {}
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(
            evt.event_key(),
            EventKey::Unknown("agent".into(), "bizarre".into())
        );
    }

    #[test]
    fn event_key_task_metadata_updated() {
        let v = json!({
            "eventId": "e",
            "type": "task",
            "subType": "metadata.updated",
            "taskId": "t",
            "timestamp": "now",
            "isError": false,
            "payload": {}
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(evt.event_key(), EventKey::Task(TaskSubType::MetadataUpdated));
    }
}
