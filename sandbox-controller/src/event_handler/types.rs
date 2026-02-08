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
