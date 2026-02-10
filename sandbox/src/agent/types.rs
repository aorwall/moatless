//! Request/response types for agent endpoints.

use serde::{Deserialize, Serialize};

/// Content block in an agent run request (text or image).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

/// Image source for content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// Request to execute the agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunRequest {
    /// Content blocks (text and/or images).
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// Optional model override.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional system prompt to append.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Session ID to resume, or true to resume last session.
    #[serde(default)]
    pub resume: Option<serde_json::Value>,
    /// Agent type identifier.
    #[serde(default)]
    pub agent_type: Option<String>,
}

impl AgentRunRequest {
    /// Extract text content from all text blocks, joined by newlines.
    pub fn extract_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Agent status as reported in /status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Running,
    Stopped,
}

