//! Event handler payload types.

use serde::Deserialize;

/// Parsed agent event from Redis stream payload.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub event_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub sub_type: String,
    pub task_id: String,
    pub timestamp: String,
    pub is_error: bool,
    pub agent_type: Option<String>,
    pub turn_number: Option<i32>,
    pub payload: serde_json::Value,
}

/// SDK result message payload (embedded in agent.completed events).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SdkResultMessage {
    pub is_error: Option<bool>,
    pub subtype: Option<String>,
    pub result: Option<serde_json::Value>,
    pub total_cost_usd: Option<f64>,
    pub usage: Option<UsageInfo>,
}

/// Usage info from SDK result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
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
            "type": "agent.completed",
            "subType": "result",
            "taskId": "task-1",
            "timestamp": "2025-01-01T00:00:00Z",
            "isError": false,
            "agentType": "claude",
            "turnNumber": 3,
            "payload": { "key": "value" }
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
        assert_eq!(evt.event_id, "evt-1");
        assert_eq!(evt.event_type, "agent.completed");
        assert_eq!(evt.sub_type, "result");
        assert_eq!(evt.task_id, "task-1");
        assert!(!evt.is_error);
        assert_eq!(evt.agent_type.as_deref(), Some("claude"));
        assert_eq!(evt.turn_number, Some(3));
    }

    #[test]
    fn deserialize_agent_event_minimal() {
        let v = json!({
            "eventId": "e",
            "type": "agent.message",
            "subType": "msg",
            "taskId": "t",
            "timestamp": "now",
            "isError": true,
            "payload": {}
        });
        let evt: AgentEvent = serde_json::from_value(v).unwrap();
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
        assert_eq!(r.result.as_ref().and_then(|v| v.as_str()), Some("Out of memory"));
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
}
