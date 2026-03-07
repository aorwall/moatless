//! Task request/response types.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::models::{TaskStatus, Visibility};
use crate::scope::Scope;
use crate::shared_models::{AgentMode, AgentType};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    pub repository_id: String,
    pub agent_type: Option<AgentType>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub branch: Option<String>,
    pub message: Option<String>,
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
    #[serde(default)]
    pub contexts: Vec<MessageContextInput>,
    #[serde(default)]
    pub file_ids: Vec<String>,
}

/// Data for an element context (selected UI component).
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ElementContextData {
    pub component_id: String,
    pub path: String,
    pub line: i32,
    pub column: i32,
    pub component_name: String,
}

/// Data for a Figma import context.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FigmaContextData {
    pub import_id: String,
    pub file_name: String,
    pub node_name: String,
    pub has_image: bool,
}

/// Data for a preview URL context.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreviewUrlContextData {
    pub path: String,
    pub url: Option<String>,
}

/// A context attached to a message, discriminated by `type`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "type")]
pub enum MessageContextInput {
    /// Selected UI element context.
    #[serde(rename = "element", rename_all = "camelCase")]
    Element {
        reference_id: String,
        data: ElementContextData,
    },
    /// Figma import context.
    #[serde(rename = "figma", rename_all = "camelCase")]
    Figma {
        reference_id: String,
        data: FigmaContextData,
    },
    /// Preview URL context.
    #[serde(rename = "previewUrl", rename_all = "camelCase")]
    PreviewUrl {
        reference_id: String,
        data: PreviewUrlContextData,
    },
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskMetadataRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVisibilityRequest {
    pub visibility: Visibility,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetTagsRequest {
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub status: Option<TaskStatus>,
    pub repository_id: Option<String>,
    pub created_by: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskResponse {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub agent_type: AgentType,
    pub repository_id: String,
    pub repository_name: Option<String>,
    pub branch: Option<String>,
    pub created_at: String,
    pub last_message_at: String,
    pub total_cost_usd: f64,
    pub site_url: Option<String>,
    pub template_id: Option<String>,
    pub vibe: Option<bool>,
    pub created_by: Option<String>,
    pub created_by_display_name: Option<String>,
    pub visibility: Visibility,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub tags: Vec<TagResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_running: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagResponse {
    pub tag_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub status: TaskStatus,
    pub task: TaskResponse,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    pub items: Vec<TaskResponse>,
    pub pagination: PaginationResponse,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginationResponse {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_element_context() {
        let json = serde_json::json!({
            "type": "element",
            "referenceId": "ref-1",
            "data": {
                "componentId": "comp-1",
                "path": "src/App.tsx",
                "line": 10,
                "column": 5,
                "componentName": "App"
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).unwrap();
        match ctx {
            MessageContextInput::Element { reference_id, data } => {
                assert_eq!(reference_id, "ref-1");
                assert_eq!(data.component_id, "comp-1");
                assert_eq!(data.path, "src/App.tsx");
                assert_eq!(data.line, 10);
                assert_eq!(data.column, 5);
                assert_eq!(data.component_name, "App");
            }
            _ => panic!("Expected Element variant"),
        }
    }

    #[test]
    fn deserialize_figma_context() {
        let json = serde_json::json!({
            "type": "figma",
            "referenceId": "ref-2",
            "data": {
                "importId": "imp-1",
                "fileName": "design.fig",
                "nodeName": "Button",
                "hasImage": true
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).unwrap();
        match ctx {
            MessageContextInput::Figma { reference_id, data } => {
                assert_eq!(reference_id, "ref-2");
                assert_eq!(data.import_id, "imp-1");
                assert_eq!(data.file_name, "design.fig");
                assert_eq!(data.node_name, "Button");
                assert!(data.has_image);
            }
            _ => panic!("Expected Figma variant"),
        }
    }

    #[test]
    fn deserialize_preview_url_context() {
        let json = serde_json::json!({
            "type": "previewUrl",
            "referenceId": "ref-3",
            "data": {
                "path": "/dashboard",
                "url": "https://example.com/dashboard"
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).unwrap();
        match ctx {
            MessageContextInput::PreviewUrl { reference_id, data } => {
                assert_eq!(reference_id, "ref-3");
                assert_eq!(data.path, "/dashboard");
                assert_eq!(data.url.as_deref(), Some("https://example.com/dashboard"));
            }
            _ => panic!("Expected PreviewUrl variant"),
        }
    }

    #[test]
    fn deserialize_preview_url_context_without_url() {
        let json = serde_json::json!({
            "type": "previewUrl",
            "referenceId": "ref-4",
            "data": {
                "path": "/about"
            }
        });

        let ctx: MessageContextInput = serde_json::from_value(json).unwrap();
        match ctx {
            MessageContextInput::PreviewUrl { data, .. } => {
                assert_eq!(data.path, "/about");
                assert!(data.url.is_none());
            }
            _ => panic!("Expected PreviewUrl variant"),
        }
    }

    #[test]
    fn deserialize_unknown_context_type_fails() {
        let json = serde_json::json!({
            "type": "unknown",
            "referenceId": "ref-5",
            "data": {}
        });

        let result = serde_json::from_value::<MessageContextInput>(json);
        assert!(result.is_err());
    }
}
