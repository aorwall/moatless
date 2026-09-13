//! Request/response types for agent endpoints.

use serde::{Deserialize, Serialize};

// =============================================================================
// ResumeSession
// =============================================================================

/// Session resume mode: resume the last session or a specific session by ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeSession {
    /// Resume the most recent session (`true` in JSON / proto bool).
    Last,
    /// Resume a specific session by its ID (`"<session-id>"` in JSON).
    WithId(String),
}

/// Where a fork's first turn branches from — the parent's session, and the cut
/// inside it.
///
/// Exactly one anchor is set: Claude keys by message uuid (no turn id), Codex's
/// `thread/fork` takes `lastTurnId`. Neither is set for a fork at the session's
/// head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkSource {
    pub source_session_id: String,
    pub source_message_uuid: Option<String>,
    pub source_agent_turn_id: Option<String>,
}

impl Serialize for ResumeSession {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Last => serializer.serialize_bool(true),
            Self::WithId(id) => serializer.serialize_str(id),
        }
    }
}

impl<'de> Deserialize<'de> for ResumeSession {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Bool(true) => Ok(Self::Last),
            serde_json::Value::String(id) => Ok(Self::WithId(id)),
            other => Err(serde::de::Error::custom(format!(
                "expected true or string for ResumeSession, got {other}"
            ))),
        }
    }
}

// =============================================================================
// ImageSourceType
// =============================================================================

/// Type of image source (re-export from SDK not possible since sandbox has its
/// own `ImageSource` struct for proto conversion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSourceType {
    Base64,
    Url,
    #[serde(other)]
    Unknown,
}

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
    pub source_type: ImageSourceType,
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
    /// Session ID to resume, or `true` to resume last session.
    #[serde(default)]
    pub resume: Option<ResumeSession>,
    /// Agent type identifier.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Agent mode identifier.
    #[serde(default)]
    pub agent_mode: Option<String>,
    /// Reasoning effort, in the agent's own vocabulary. `None` leaves the agent
    /// on its own default.
    #[serde(default)]
    pub effort: Option<String>,
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
    /// Parked on a question only a person can answer. Distinct from `Running`
    /// because nothing is running: the turn is held open on the answer, and a
    /// sandbox nobody has answered is reclaimable on the ordinary idle TTL.
    Waiting,
}

/// Who initiated an agent stop. Carried on the terminal `agent.stopped` event
/// payload as `source` so the backend can attribute the stop instead of
/// assuming every stop was user-requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSource {
    User,
    IdleReaper,
    Cleanup,
    System,
    Error,
}

impl StopSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::IdleReaper => "idle_reaper",
            Self::Cleanup => "cleanup",
            Self::System => "system",
            Self::Error => "error",
        }
    }
}

impl From<sandbox_proto::AgentStopSource> for StopSource {
    fn from(s: sandbox_proto::AgentStopSource) -> Self {
        match s {
            // Legacy backends predate the field; a bare StopAgent has always
            // meant a user-visible stop.
            sandbox_proto::AgentStopSource::Unspecified => Self::User,
            sandbox_proto::AgentStopSource::User => Self::User,
            sandbox_proto::AgentStopSource::IdleReaper => Self::IdleReaper,
            sandbox_proto::AgentStopSource::Cleanup => Self::Cleanup,
            sandbox_proto::AgentStopSource::System => Self::System,
            sandbox_proto::AgentStopSource::Error => Self::Error,
        }
    }
}

/// Convert proto `ContentBlock`s to internal `ContentBlock`s.
///
/// Shared by both `grpc_server.rs` (`execute_agent`) and `consumer.rs` (message
/// polling).
pub fn content_blocks_from_proto(
    proto_blocks: Vec<sandbox_proto::ContentBlock>,
) -> Vec<ContentBlock> {
    proto_blocks
        .into_iter()
        .filter_map(|block| {
            block.block.map(|b| match b {
                sandbox_proto::content_block::Block::Text(t) => ContentBlock::Text { text: t.text },
                sandbox_proto::content_block::Block::Image(img) => ContentBlock::Image {
                    source: ImageSource {
                        source_type: ImageSourceType::Url,
                        media_type: img.media_type,
                        data: img.url,
                    },
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&AgentStatus::Idle).unwrap(),
            r#""idle""#
        );
        assert_eq!(
            serde_json::to_string(&AgentStatus::Running).unwrap(),
            r#""running""#
        );
        assert_eq!(
            serde_json::to_string(&AgentStatus::Stopped).unwrap(),
            r#""stopped""#
        );
        assert_eq!(
            serde_json::to_string(&AgentStatus::Waiting).unwrap(),
            r#""waiting""#
        );
    }

    #[test]
    fn content_block_text_roundtrip() {
        let block = ContentBlock::Text {
            text: "hello".to_string(),
        };
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
                source_type: ImageSourceType::Base64,
                media_type: "image/png".to_string(),
                data: "abc123".to_string(),
            },
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["source"]["type"], "base64");
        assert_eq!(json["source"]["media_type"], "image/png");
    }

    #[test]
    fn extract_text_joins_text_blocks() {
        let req = AgentRunRequest {
            content: vec![
                ContentBlock::Text {
                    text: "line one".to_string(),
                },
                ContentBlock::Image {
                    source: ImageSource {
                        source_type: ImageSourceType::Base64,
                        media_type: "image/png".to_string(),
                        data: "data".to_string(),
                    },
                },
                ContentBlock::Text {
                    text: "line two".to_string(),
                },
            ],
            model: None,
            system_prompt: None,
            resume: None,
            agent_type: None,
            agent_mode: None,
            effort: None,
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
            agent_mode: None,
            effort: None,
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
        assert_eq!(req.resume, Some(ResumeSession::Last));
        assert_eq!(req.agent_type.as_deref(), Some("claude-code"));
    }

    #[test]
    fn agent_run_request_deserializes_resume_session_id() {
        let json = r#"{"resume": "session-abc-123"}"#;
        let req: AgentRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            req.resume,
            Some(ResumeSession::WithId("session-abc-123".to_string()))
        );
    }

    #[test]
    fn resume_session_serializes_correctly() {
        assert_eq!(
            serde_json::to_value(ResumeSession::Last).unwrap(),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            serde_json::to_value(ResumeSession::WithId("abc".to_string())).unwrap(),
            serde_json::Value::String("abc".to_string())
        );
    }

    #[test]
    fn agent_run_request_defaults_empty_content() {
        let json = r#"{}"#;
        let req: AgentRunRequest = serde_json::from_str(json).unwrap();
        assert!(req.content.is_empty());
    }

    #[test]
    fn content_blocks_from_proto_converts_text() {
        let proto = vec![sandbox_proto::ContentBlock {
            block: Some(sandbox_proto::content_block::Block::Text(
                sandbox_proto::TextBlock {
                    text: "hello".to_string(),
                },
            )),
        }];
        let blocks = content_blocks_from_proto(proto);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("Expected Text block"),
        }
    }

    #[test]
    fn content_blocks_from_proto_converts_image() {
        let proto = vec![sandbox_proto::ContentBlock {
            block: Some(sandbox_proto::content_block::Block::Image(
                sandbox_proto::ImageBlock {
                    url: "https://example.com/img.png".to_string(),
                    media_type: "image/png".to_string(),
                },
            )),
        }];
        let blocks = content_blocks_from_proto(proto);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Image { source } => {
                assert_eq!(source.source_type, ImageSourceType::Url);
                assert_eq!(source.media_type, "image/png");
                assert_eq!(source.data, "https://example.com/img.png");
            }
            _ => panic!("Expected Image block"),
        }
    }

    #[test]
    fn content_blocks_from_proto_skips_empty_block() {
        let proto = vec![sandbox_proto::ContentBlock { block: None }];
        let blocks = content_blocks_from_proto(proto);
        assert!(blocks.is_empty());
    }
}
