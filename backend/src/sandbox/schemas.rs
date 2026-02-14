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
