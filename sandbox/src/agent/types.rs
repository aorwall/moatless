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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_status_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&AgentStatus::Idle).unwrap(), r#""idle""#);
        assert_eq!(serde_json::to_string(&AgentStatus::Running).unwrap(), r#""running""#);
        assert_eq!(serde_json::to_string(&AgentStatus::Stopped).unwrap(), r#""stopped""#);
    }

    #[test]
    fn content_block_text_roundtrip() {
        let block = ContentBlock::Text { text: "hello".to_string() };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");

        let back: ContentBlock = serde_json::from_value(json).unwrap();
        match back {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn content_block_image_roundtrip() {
        let block = ContentBlock::Image {
            source: ImageSource {
                source_type: "base64".to_string(),
                media_type: "image/png".to_string(),
                data: "abc123".to_string(),
            },
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["source"]["media_type"], "image/png");
    }

    #[test]
    fn extract_text_joins_text_blocks() {
        let req = AgentRunRequest {
            content: vec![
                ContentBlock::Text { text: "line one".to_string() },
                ContentBlock::Image {
                    source: ImageSource {
                        source_type: "base64".to_string(),
                        media_type: "image/png".to_string(),
                        data: "data".to_string(),
                    },
                },
                ContentBlock::Text { text: "line two".to_string() },
            ],
            model: None,
            system_prompt: None,
            resume: None,
            agent_type: None,
        };
        assert_eq!(req.extract_text(), "line one\nline two");
    }

    #[test]
    fn extract_text_empty_content() {
        let req = AgentRunRequest {
            content: vec![],
            model: None,
            system_prompt: None,
            resume: None,
            agent_type: None,
        };
        assert_eq!(req.extract_text(), "");
    }

    #[test]
    fn agent_run_request_deserializes_minimal() {
        let json = r#"{"content": [{"type": "text", "text": "hello"}]}"#;
        let req: AgentRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content.len(), 1);
        assert_eq!(req.extract_text(), "hello");
        assert!(req.model.is_none());
    }

    #[test]
    fn agent_run_request_deserializes_all_fields() {
        let json = r#"{
            "content": [],
            "model": "claude-sonnet-4-5-20250929",
            "resume": true,
            "agentType": "claude-code"
        }"#;
        let req: AgentRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model.as_deref(), Some("claude-sonnet-4-5-20250929"));
        assert_eq!(req.resume, Some(serde_json::Value::Bool(true)));
        assert_eq!(req.agent_type.as_deref(), Some("claude-code"));
    }

    #[test]
    fn agent_run_request_defaults_empty_content() {
        let json = r#"{}"#;
        let req: AgentRunRequest = serde_json::from_str(json).unwrap();
        assert!(req.content.is_empty());
    }
}

