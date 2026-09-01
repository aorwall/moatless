//! Sandbox API request/response schemas.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::live_status::SandboxStatus;
use super::models::{ServerOverride, ServerSnapshot};

// The sandbox status-read response and its parts are request/response bodies, so
// they live in `moatless-api-schemas` (their single home); re-exported here so
// this crate's `schemas::` paths keep naming them.
pub use moatless_api_schemas::sandbox::{
    BackgroundProcess, BackgroundWatchJobInfo, ContainerDiagnostics, RuntimeEvent,
    SandboxStatusFullResponse, ScheduledJobInfo, ScheduledJobKind, WakeupJobInfo,
};
pub use moatless_types::sandbox::ServerEntry;

/// Provision response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub status: SandboxStatus,
}

/// Server status response proxied from sandbox pod.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub servers: Vec<ServerSnapshot>,
}

/// Update one preview server's live config.
///
/// Only fields that a sidecar can pick up on an in-place restart are accepted.
/// `port` and `dockerImage` are declared so a caller that sends them gets an
/// explicit "redeploy required" instead of a silent no-op.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateServerConfigRequest {
    /// Replaces the workspace's start command. Joined with `&&`, as at
    /// provision time.
    #[serde(default)]
    pub start_commands: Option<Vec<String>>,
    /// Env entries to set, merged into any existing override.
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    /// Keys to remove from the server's environment. Also drops them from the
    /// override's `env`, so setting and unsetting the same key is coherent.
    #[serde(default)]
    pub unset_env: Option<Vec<String>>,
    /// Restart the server so the change takes effect (default `true`).
    #[serde(default = "default_restart")]
    pub restart: bool,
    /// Not applicable live — the port is wired into the Service and
    /// IngressRoute at provision time.
    #[serde(default)]
    pub port: Option<u16>,
    /// Not applicable live — the image is the container's identity.
    #[serde(default)]
    pub docker_image: Option<String>,
}

fn default_restart() -> bool {
    true
}

/// Result of a live server config change or restart.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfigResponse {
    pub server_name: String,
    /// The override after the call — empty when it was reset.
    pub config_override: ServerOverride,
    /// Sidecar restart generation after the call.
    pub generation: u64,
    pub restarted: bool,
}

/// One server's override as it currently exists on the workspace volume.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerOverrideEntry {
    pub server_name: String,
    pub config_override: ServerOverride,
    /// The counter the sidecar watches; `0` means it has never been restarted.
    pub generation: u64,
}

/// Every override in effect on a task. Servers with no override are omitted.
///
/// Read from the sandbox's workspace volume, which is the only place an
/// override exists — a stopped sandbox cannot answer this.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerOverridesResponse {
    pub overrides: Vec<ServerOverrideEntry>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_binary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
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

/// A sandbox file-tree node carried a `FileNodeType` integer the wire contract
/// does not define, so the node cannot be classified as a file or a directory.
///
/// Carries the offending value and the node's path: the tree arrives recursively,
/// so the failing node is not necessarily the one the caller converted.
#[derive(Debug, thiserror::Error)]
#[error("unknown FileNodeType {node_type} for file tree node '{path}'")]
pub struct UnknownFileNodeType {
    pub path: String,
    pub node_type: i32,
}

/// Widening a protobuf file-tree node onto the API DTO is partial: `node_type`
/// is an open integer on the wire, and an unrecognized value must not be
/// reported to clients as a file.
impl TryFrom<sandbox_proto::FileTreeNode> for FileTreeNodeDto {
    type Error = UnknownFileNodeType;

    fn try_from(node: sandbox_proto::FileTreeNode) -> Result<Self, Self::Error> {
        let node_type = match sandbox_proto::FileNodeType::try_from(node.node_type) {
            Ok(sandbox_proto::FileNodeType::File) => "file",
            Ok(sandbox_proto::FileNodeType::Directory) => "directory",
            Err(_) => {
                return Err(UnknownFileNodeType {
                    path: node.path,
                    node_type: node.node_type,
                });
            }
        };

        // A nested failure fails the whole tree — a partially converted tree
        // would silently hide a subtree from the client.
        let children = node
            .children
            .into_iter()
            .map(FileTreeNodeDto::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        // Directories always report a (possibly empty) child list so clients can
        // tell "expandable but empty" from "not expandable".
        let children = if children.is_empty() && node_type == "file" {
            None
        } else {
            Some(children)
        };

        Ok(Self {
            name: node.name,
            path: node.path,
            node_type: node_type.to_string(),
            is_changed: node.is_changed,
            git_status: node.git_status,
            additions: node.additions,
            deletions: node.deletions,
            is_staged: node.is_staged,
            children,
        })
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
            is_binary: r.is_binary,
            media_type: r.media_type,
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

impl TryFrom<sandbox_proto::GetFileTreeResponse> for FileTreeResponse {
    type Error = UnknownFileNodeType;

    fn try_from(resp: sandbox_proto::GetFileTreeResponse) -> Result<Self, Self::Error> {
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

        Ok(Self {
            tree: resp
                .tree
                .into_iter()
                .map(FileTreeNodeDto::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            git_summary,
        })
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
mod file_tree_conversion_tests {
    use super::*;

    fn proto_node(
        path: &str,
        node_type: i32,
        children: Vec<sandbox_proto::FileTreeNode>,
    ) -> sandbox_proto::FileTreeNode {
        sandbox_proto::FileTreeNode {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.to_string(),
            node_type,
            is_changed: false,
            git_status: None,
            additions: None,
            deletions: None,
            is_staged: None,
            children,
        }
    }

    #[test]
    fn every_known_node_type_converts_to_its_dto_string() {
        for (node_type, expected) in [
            (sandbox_proto::FileNodeType::File, "file"),
            (sandbox_proto::FileNodeType::Directory, "directory"),
        ] {
            let dto = FileTreeNodeDto::try_from(proto_node("a", node_type as i32, vec![]))
                .expect("known node type converts");
            assert_eq!(dto.node_type, expected);
        }
    }

    #[test]
    fn directories_keep_an_empty_child_list_and_files_have_none() {
        let dir = FileTreeNodeDto::try_from(proto_node(
            "src",
            sandbox_proto::FileNodeType::Directory as i32,
            vec![],
        ))
        .expect("directory converts");
        assert_eq!(dir.children.as_ref().map(Vec::len), Some(0));

        let file = FileTreeNodeDto::try_from(proto_node(
            "a.txt",
            sandbox_proto::FileNodeType::File as i32,
            vec![],
        ))
        .expect("file converts");
        assert!(file.children.is_none());
    }

    #[test]
    fn unknown_node_type_is_rejected_with_the_offending_value() {
        let err = FileTreeNodeDto::try_from(proto_node("weird.sock", 7, vec![]))
            .expect_err("unknown node type must not be treated as a file");
        assert_eq!(err.node_type, 7);
        assert_eq!(err.path, "weird.sock");
        assert_eq!(
            err.to_string(),
            "unknown FileNodeType 7 for file tree node 'weird.sock'"
        );
    }

    #[test]
    fn nested_unknown_node_type_propagates_to_the_root_conversion() {
        let deep = proto_node(
            "src",
            sandbox_proto::FileNodeType::Directory as i32,
            vec![proto_node(
                "src/nested",
                sandbox_proto::FileNodeType::Directory as i32,
                vec![
                    proto_node(
                        "src/nested/ok.rs",
                        sandbox_proto::FileNodeType::File as i32,
                        vec![],
                    ),
                    proto_node("src/nested/mystery", 42, vec![]),
                ],
            )],
        );

        let err =
            FileTreeNodeDto::try_from(deep).expect_err("a nested failure must fail the whole tree");
        // The error names the failing descendant, not the node converted.
        assert_eq!(err.path, "src/nested/mystery");
        assert_eq!(err.node_type, 42);
    }

    #[test]
    fn response_conversion_propagates_nested_failures() {
        let resp = sandbox_proto::GetFileTreeResponse {
            tree: vec![proto_node(
                "src",
                sandbox_proto::FileNodeType::Directory as i32,
                vec![proto_node("src/mystery", -1, vec![])],
            )],
            git_summary: None,
        };

        let err = FileTreeResponse::try_from(resp).expect_err("nested failure fails the response");
        assert_eq!(err.path, "src/mystery");
        assert_eq!(err.node_type, -1);
    }

    #[test]
    fn valid_tree_converts_with_children_and_git_overlay_intact() {
        let resp = sandbox_proto::GetFileTreeResponse {
            tree: vec![sandbox_proto::FileTreeNode {
                is_changed: true,
                git_status: Some("modified".to_string()),
                additions: Some(3),
                deletions: Some(1),
                is_staged: Some(false),
                ..proto_node(
                    "src",
                    sandbox_proto::FileNodeType::Directory as i32,
                    vec![proto_node(
                        "src/main.rs",
                        sandbox_proto::FileNodeType::File as i32,
                        vec![],
                    )],
                )
            }],
            git_summary: None,
        };

        let dto = FileTreeResponse::try_from(resp).expect("valid tree converts");
        let root = &dto.tree[0];
        assert_eq!(root.node_type, "directory");
        assert!(root.is_changed);
        assert_eq!(root.git_status.as_deref(), Some("modified"));
        assert_eq!(root.additions, Some(3));
        assert_eq!(root.deletions, Some(1));
        assert_eq!(root.is_staged, Some(false));
        let children = root.children.as_ref().expect("directory has children");
        assert_eq!(children[0].path, "src/main.rs");
        assert_eq!(children[0].node_type, "file");
        // Absent git summary still yields the documented default.
        assert_eq!(dto.git_summary.compare_base, "HEAD");
        assert!(!dto.git_summary.has_changes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_status::AgentStatus;

    #[test]
    fn provision_response_serializes_camel_case() {
        let resp = ProvisionResponse {
            status: SandboxStatus::Initializing,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "initializing");
    }

    #[test]
    fn error_body_serializes() {
        let body = moatless_api_schemas::error::ErrorBody {
            error: "not found".into(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["error"], "not found");
    }

    #[test]
    fn scheduled_job_serializes_with_tagged_kind() {
        let wakeup = ScheduledJobInfo {
            id: "j2".into(),
            registered_at_unix_ms: 2,
            kind: ScheduledJobKind::Wakeup(WakeupJobInfo {
                reason: "ping".into(),
                prompt_preview: "wake up".into(),
                wakeup_at_unix_ms: 5,
            }),
        };
        let json = serde_json::to_value(&wakeup).unwrap();
        assert_eq!(json["id"], "j2");
        assert_eq!(json["registeredAtUnixMs"], 2);
        assert_eq!(json["kind"], "wakeup");
        assert_eq!(json["reason"], "ping");
        assert_eq!(json["promptPreview"], "wake up");
        assert_eq!(json["wakeupAtUnixMs"], 5);
    }

    #[test]
    fn empty_scheduled_jobs_omitted_from_response() {
        let resp = SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: None,
            agent_status: Some(AgentStatus::Idle),
            servers: vec![],
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: vec![],
            background_processes: vec![],
            sandbox_desired_state: None,
            current_sandbox_image: None,
            latest_sandbox_image: None,
            image_up_to_date: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("scheduledJobs").is_none());
        assert!(json.get("backgroundProcesses").is_none());
    }

    #[test]
    fn populated_scheduled_jobs_serialized_in_response() {
        let resp = SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: None,
            agent_status: Some(AgentStatus::Running),
            servers: vec![],
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: vec![ScheduledJobInfo {
                id: "j1".into(),
                registered_at_unix_ms: 1,
                kind: ScheduledJobKind::Wakeup(WakeupJobInfo {
                    reason: "ping".into(),
                    prompt_preview: "wake up".into(),
                    wakeup_at_unix_ms: 5,
                }),
            }],
            background_processes: vec![],
            sandbox_desired_state: None,
            current_sandbox_image: None,
            latest_sandbox_image: None,
            image_up_to_date: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        let arr = json
            .get("scheduledJobs")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["kind"], "wakeup");
        assert_eq!(arr[0]["reason"], "ping");
    }

    #[test]
    fn background_processes_serialized_camel_case() {
        let resp = SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: None,
            agent_status: Some(AgentStatus::Idle),
            servers: vec![],
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs: vec![],
            background_processes: vec![BackgroundProcess {
                pid: 1234,
                command: "cargo test --no-run".into(),
            }],
            sandbox_desired_state: None,
            current_sandbox_image: None,
            latest_sandbox_image: None,
            image_up_to_date: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        let arr = json
            .get("backgroundProcesses")
            .and_then(|v| v.as_array())
            .expect("backgroundProcesses present when non-empty");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["pid"], 1234);
        assert_eq!(arr[0]["command"], "cargo test --no-run");
    }

    #[test]
    fn background_watch_scheduled_job_serialized_with_camel_case_kind() {
        let job = ScheduledJobInfo {
            id: "bg1".into(),
            registered_at_unix_ms: 7,
            kind: ScheduledJobKind::BackgroundWatch(BackgroundWatchJobInfo {
                process_count: 3,
                commands: vec!["sleep 30".into()],
            }),
        };
        let json = serde_json::to_value(&job).unwrap();
        assert_eq!(json["kind"], "backgroundWatch");
        assert_eq!(json["processCount"], 3);
        assert_eq!(json["commands"][0], "sleep 30");
    }
}
