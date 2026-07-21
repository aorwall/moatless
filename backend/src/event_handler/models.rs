//! Event handler payload types.

use moatless_events::{
    AgentSubType, EventType, SandboxStatus, ServerSubType, TaskSubType, TaskTurnSubType,
};
use serde::Deserialize;

/// Typed event dispatch key — replaces string-concat `format!("{}.{}", type, sub)`.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// SDK result message payload (embedded in agent.completed events).
///
/// Note: The sandbox serializes Rust structs with snake_case field names,
/// so multi-word fields need `#[serde(alias = "snake_case")]` to accept
/// both camelCase (from `rename_all`) and snake_case (from the sandbox).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SdkResultMessage {
    #[serde(alias = "session_id")]
    pub session_id: Option<String>,
    #[serde(alias = "is_error")]
    pub is_error: Option<bool>,
    pub subtype: Option<String>,
    #[serde(default, deserialize_with = "deserialize_value_as_string")]
    pub result: Option<String>,
    #[serde(alias = "total_cost_usd")]
    pub total_cost_usd: Option<f64>,
    pub usage: Option<UsageInfo>,
    #[serde(alias = "num_turns")]
    pub num_turns: Option<u32>,
    #[serde(alias = "duration_ms")]
    pub duration_ms: Option<u64>,
    #[serde(alias = "duration_api_ms")]
    pub duration_api_ms: Option<u64>,
    /// Tool calls the agent attempted but was denied permission for. Used to
    /// emit the `moatless_agent_permission_denials_total` metric. Defaults to
    /// empty when the runtime doesn't report denials.
    #[serde(default, alias = "permission_denials")]
    pub permission_denials: Vec<PermissionDenialInfo>,
}

/// One denied tool call from the SDK result's `permission_denials` array.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDenialInfo {
    #[serde(alias = "tool_name")]
    pub tool_name: Option<String>,
}

/// Deserialize any JSON value into `Option<String>`.
///
/// Strings pass through directly; other types are converted via `to_string()`.
/// `null` becomes `None`.
fn deserialize_value_as_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }))
}

/// Usage info from SDK result.
///
/// The sandbox sends snake_case field names, so each multi-word field needs
/// an alias to accept both camelCase and snake_case.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    #[serde(alias = "input_tokens")]
    pub input_tokens: Option<i64>,
    #[serde(alias = "output_tokens")]
    pub output_tokens: Option<i64>,
    #[serde(alias = "cache_creation_input_tokens")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(alias = "cache_read_input_tokens")]
    pub cache_read_input_tokens: Option<i64>,
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
    fn deserialize_sdk_result_success() {
        let v = json!({
            "isError": false,
            "subtype": "completed",
            "result": "All done",
            "totalCostUsd": 0.05,
            "usage": {
                "inputTokens": 1000,
                "outputTokens": 500,
                "cacheCreationInputTokens": null,
                "cacheReadInputTokens": 200
            }
        });
        let r: SdkResultMessage = serde_json::from_value(v).unwrap();
        assert_eq!(r.is_error, Some(false));
        assert_eq!(r.subtype.as_deref(), Some("completed"));
        assert_eq!(r.total_cost_usd, Some(0.05));
        let usage = r.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(1000));
        assert_eq!(usage.output_tokens, Some(500));
        assert!(usage.cache_creation_input_tokens.is_none());
        assert_eq!(usage.cache_read_input_tokens, Some(200));
    }

    #[test]
    fn deserialize_sdk_result_error() {
        let v = json!({
            "isError": true,
            "subtype": "error",
            "result": "Out of memory"
        });
        let r: SdkResultMessage = serde_json::from_value(v).unwrap();
        assert_eq!(r.is_error, Some(true));
        assert_eq!(r.result.as_deref(), Some("Out of memory"));
        assert!(r.total_cost_usd.is_none());
        assert!(r.usage.is_none());
    }

    #[test]
    fn deserialize_sdk_result_empty() {
        let v = json!({});
        let r: SdkResultMessage = serde_json::from_value(v).unwrap();
        assert!(r.is_error.is_none());
        assert!(r.subtype.is_none());
        assert!(r.result.is_none());
        assert!(r.total_cost_usd.is_none());
        assert!(r.usage.is_none());
    }

    #[test]
    fn deserialize_sdk_result_with_snake_case_fields() {
        let v = json!({
            "session_id": "sess-123",
            "is_error": false,
            "result": "done",
            "total_cost_usd": 0.0538,
            "num_turns": 5,
            "duration_ms": 3200,
            "duration_api_ms": 2800,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 10,
                "cache_creation_input_tokens": 20
            }
        });
        let r: SdkResultMessage = serde_json::from_value(v).unwrap();
        assert_eq!(r.session_id.as_deref(), Some("sess-123"));
        assert_eq!(r.is_error, Some(false));
        assert_eq!(r.result.as_deref(), Some("done"));
        assert_eq!(r.total_cost_usd, Some(0.0538));
        assert_eq!(r.num_turns, Some(5));
        assert_eq!(r.duration_ms, Some(3200));
        assert_eq!(r.duration_api_ms, Some(2800));
        let usage = r.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cache_read_input_tokens, Some(10));
        assert_eq!(usage.cache_creation_input_tokens, Some(20));
    }

    #[test]
    fn deserialize_sdk_result_non_string_value() {
        // Non-string JSON values should be converted to their string representation
        let v = json!({
            "result": 42
        });
        let r: SdkResultMessage = serde_json::from_value(v).unwrap();
        assert_eq!(r.result.as_deref(), Some("42"));
    }

    #[test]
    fn deserialize_sdk_result_null_result() {
        let v = json!({
            "result": null
        });
        let r: SdkResultMessage = serde_json::from_value(v).unwrap();
        assert!(r.result.is_none());
    }

    #[test]
    fn deserialize_usage_info_with_nulls() {
        let v = json!({
            "inputTokens": 42,
            "outputTokens": null,
            "cacheCreationInputTokens": null,
            "cacheReadInputTokens": null
        });
        let u: UsageInfo = serde_json::from_value(v).unwrap();
        assert_eq!(u.input_tokens, Some(42));
        assert!(u.output_tokens.is_none());
        assert!(u.cache_creation_input_tokens.is_none());
        assert!(u.cache_read_input_tokens.is_none());
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
        assert_eq!(
            evt.event_key(),
            EventKey::Task(TaskSubType::MetadataUpdated)
        );
    }
}
