//! Sandbox API request/response schemas.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::{
    AgentState, EnvironmentStatus, ResourceMetrics, ServerCapabilities, ServerSnapshot,
    ServerStatus, WorkspaceState,
};

/// Generic error body returned by all error responses.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

/// Provision response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub status: EnvironmentStatus,
}

/// Status response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub status: EnvironmentStatus,
    pub pod_name: Option<String>,
    pub last_error: Option<String>,
}

/// Health response
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub healthy: bool,
}

/// Single server entry in the workspace status response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub name: String,
    pub label: String,
    pub port: u16,
    pub status: ServerStatus,
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub default: bool,
    pub capabilities: ServerCapabilities,
}

/// Full workspace status response to frontend (matches TS WorkspaceStatusResponse)
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatusFullResponse {
    pub workspace_state: WorkspaceState,
    pub environment_status: EnvironmentStatus,
    pub environment_error: Option<String>,
    pub agent_state: AgentState,
    pub servers: Vec<ServerEntry>,
    pub is_running: bool,
    pub workspace_ready: bool,
    pub can_execute: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceMetrics>,
}

/// Request body for file search in sandbox.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchFilesRequest {
    pub query: String,
    pub limit: Option<u32>,
}

/// Single file search result from sandbox.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchResult {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
}

/// Server status response proxied from sandbox pod.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub servers: Vec<ServerSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provision_response_serializes_camel_case() {
        let resp = ProvisionResponse {
            status: EnvironmentStatus::Initializing,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "initializing");
    }

    #[test]
    fn status_response_serializes_camel_case() {
        let resp = StatusResponse {
            status: EnvironmentStatus::Ready,
            pod_name: Some("sandbox-abc-xyz".into()),
            last_error: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "ready");
        assert_eq!(json["podName"], "sandbox-abc-xyz");
        assert!(json["lastError"].is_null());
    }

    #[test]
    fn health_response_serializes() {
        let resp = HealthResponse { healthy: true };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["healthy"], true);
    }

    #[test]
    fn error_body_serializes() {
        let body = ErrorBody { error: "not found".into() };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["error"], "not found");
    }

    #[test]
    fn file_search_result_roundtrip() {
        let result = FileSearchResult {
            path: "/src/main.rs".into(),
            name: "main.rs".into(),
            file_type: Some("file".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: FileSearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "/src/main.rs");
        assert_eq!(back.name, "main.rs");
        assert_eq!(back.file_type.as_deref(), Some("file"));
    }

    #[test]
    fn search_files_request_deserializes() {
        let json = r#"{"query": "main", "limit": 10}"#;
        let req: SearchFilesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "main");
        assert_eq!(req.limit, Some(10));
    }

    #[test]
    fn search_files_request_optional_limit() {
        let json = r#"{"query": "test"}"#;
        let req: SearchFilesRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "test");
        assert!(req.limit.is_none());
    }
}
