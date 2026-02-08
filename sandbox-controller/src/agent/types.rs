//! Agent request/response types.

use serde::{Deserialize, Serialize};

use crate::task::types::MessageContextInput;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SendMessageRequest {
    pub message: String,
    pub resume: Option<bool>,
    pub model: Option<String>,
    #[serde(default)]
    pub file_ids: Vec<String>,
    pub command_name: Option<String>,
    pub agent_mode: Option<String>,
    #[serde(default)]
    pub contexts: Vec<MessageContextInput>,
    // Tool response fields
    pub tool_use_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_response: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SendCommandRequest {
    pub command_name: String,
    pub resume: Option<bool>,
    pub model: Option<String>,
    #[serde(default)]
    pub file_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub status: String,
    pub timestamp: String,
}
