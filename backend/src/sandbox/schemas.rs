//! Sandbox API request/response schemas.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::{
    AgentState, EnvironmentStatus, ResourceMetrics, SandboxState, ServerCapabilities,
    ServerSnapshot, ServerStatus,
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

/// Full sandbox status response to frontend (matches TS SandboxStatusFullResponse)
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatusFullResponse {
    pub sandbox_state: SandboxState,
    pub environment_status: EnvironmentStatus,
    pub environment_error: Option<String>,
    pub agent_state: AgentState,
    pub servers: Vec<ServerEntry>,
    pub sandbox_ready: bool,
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

// ── File operation DTOs ──────────────────────────────────────────

/// File tree response with git change summary.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileTreeResponse {
    pub tree: Vec<FileTreeNodeDto>,
    pub git_summary: GitSummaryDto,
}

/// A node in the file tree (file or directory).
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileTreeNodeDto {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub is_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_staged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(no_recursion)]
    pub children: Option<Vec<FileTreeNodeDto>>,
}

/// Git change summary across the workspace.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitSummaryDto {
    pub total_files: i32,
    pub total_additions: i32,
    pub total_deletions: i32,
    pub total_staged: i32,
    pub total_modified: i32,
    pub compare_base: String,
    pub has_changes: bool,
}

/// File content response with optional git change info.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileContentResponse {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_modified: Option<bool>,
}

/// Request body for writing a file.
#[derive(Debug, Deserialize, ToSchema)]
pub struct WriteFileRequestBody {
    pub path: String,
    pub content: String,
}

/// Response for listing files.
#[derive(Debug, Serialize, ToSchema)]
pub struct ListFilesResponse {
    pub files: Vec<String>,
}

/// Request body for searching files by name.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchByNameRequest {
    pub query: String,
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    #[serde(default)]
    pub max_results: Option<i32>,
}

/// Response for searching files by name.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchByNameResponse {
    pub files: Vec<FileNameMatchDto>,
    pub total_matches: i32,
}

/// A file name match in the search results.
#[derive(Debug, Serialize, ToSchema)]
pub struct FileNameMatchDto {
    pub path: String,
    pub matches: i32,
}

// ── Moatless tool names ──────────────────────────────────────────

/// All available Moatless tool names injected into the agent sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum MoatlessToolName {
    #[serde(rename = "updateTaskMetadata")]
    UpdateTaskMetadata,
    #[serde(rename = "install")]
    Install,
    #[serde(rename = "updateRepositoryConfig")]
    UpdateRepositoryConfig,
    #[serde(rename = "getRepositoryConfig")]
    GetRepositoryConfig,
    #[serde(rename = "startOrRestartPreviewServer")]
    StartOrRestartPreviewServer,
    #[serde(rename = "getPreviewServerLogs")]
    GetPreviewServerLogs,
    #[serde(rename = "navigatePreview")]
    NavigatePreview,
    #[serde(rename = "clickElement")]
    ClickElement,
    #[serde(rename = "evaluateJavaScript")]
    EvaluateJavaScript,
    #[serde(rename = "fillPreviewForm")]
    FillPreviewForm,
    #[serde(rename = "takePreviewSnapshot")]
    TakePreviewSnapshot,
}

// ── Proto conversions ────────────────────────────────────────────

impl From<sandbox_proto::FileTreeNode> for FileTreeNodeDto {
    fn from(node: sandbox_proto::FileTreeNode) -> Self {
        let node_type = if node.node_type == sandbox_proto::FileNodeType::Directory as i32 {
            "directory".to_string()
        } else {
            "file".to_string()
        };

        let children = if node.children.is_empty() {
            if node_type == "directory" {
                Some(Vec::new())
            } else {
                None
            }
        } else {
            Some(node.children.into_iter().map(FileTreeNodeDto::from).collect())
        };

        Self {
            name: node.name,
            path: node.path,
            node_type,
            is_changed: node.is_changed,
            git_status: node.git_status,
            additions: node.additions,
            deletions: node.deletions,
            is_staged: node.is_staged,
            children,
        }
    }
}

impl From<sandbox_proto::GitSummary> for GitSummaryDto {
    fn from(s: sandbox_proto::GitSummary) -> Self {
        Self {
            total_files: s.total_files,
            total_additions: s.total_additions,
            total_deletions: s.total_deletions,
            total_staged: s.total_staged,
            total_modified: s.total_modified,
            compare_base: s.compare_base,
            has_changes: s.has_changes,
        }
    }
}

impl From<sandbox_proto::ReadFileResponse> for FileContentResponse {
    fn from(r: sandbox_proto::ReadFileResponse) -> Self {
        Self {
            content: r.content,
            old_content: r.old_content,
            additions: r.additions,
            deletions: r.deletions,
            is_modified: r.is_modified,
        }
    }
}

impl From<sandbox_proto::FileNameMatch> for FileNameMatchDto {
    fn from(m: sandbox_proto::FileNameMatch) -> Self {
        Self {
            path: m.path,
            matches: m.matches,
        }
    }
}

impl From<sandbox_proto::GetFileTreeResponse> for FileTreeResponse {
    fn from(resp: sandbox_proto::GetFileTreeResponse) -> Self {
        let git_summary = resp.git_summary.map_or(
            GitSummaryDto {
                total_files: 0,
                total_additions: 0,
                total_deletions: 0,
                total_staged: 0,
                total_modified: 0,
                compare_base: "HEAD".to_string(),
                has_changes: false,
            },
            GitSummaryDto::from,
        );

        Self {
            tree: resp.tree.into_iter().map(FileTreeNodeDto::from).collect(),
            git_summary,
        }
    }
}

impl From<sandbox_proto::SearchFilesByNameResponse> for SearchByNameResponse {
    fn from(resp: sandbox_proto::SearchFilesByNameResponse) -> Self {
        Self {
            files: resp.files.into_iter().map(FileNameMatchDto::from).collect(),
            total_matches: resp.total_matches,
        }
    }
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
