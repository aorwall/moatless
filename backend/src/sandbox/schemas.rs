//! Sandbox API request/response schemas.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::live_status::{AgentStatus, SandboxStatus};
use super::models::{
    ResourceMetrics, ServerCapabilities, ServerOverride, ServerSnapshot, ServerStatus,
};
use crate::task::models::SandboxDesiredState;

/// Provision response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub status: SandboxStatus,
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

/// A provider-agnostic runtime event associated with a sandbox.
///
/// In K8s mode, these are sourced from K8s Event objects (FailedScheduling,
/// BackOff, Pulling, etc.). Docker mode can populate from `docker events`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    /// Event reason (e.g. "BackOff", "FailedScheduling", "Pulling", "OOMKilled")
    pub reason: String,
    /// Human-readable message
    pub message: String,
    /// Severity: "normal" or "warning"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Number of times this event has occurred
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i32>,
    /// Last time the event occurred (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_timestamp: Option<String>,
}

/// Container-level diagnostics for troubleshooting sandbox issues.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerDiagnostics {
    pub name: String,
    /// Container state: "running", "waiting", "terminated", "unknown"
    pub state: String,
    pub restart_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminated_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_terminated_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_reason: Option<String>,
    /// CPU usage in millicores (from K8s Metrics API, if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_millicores: Option<u64>,
    /// Memory usage in megabytes (from K8s Metrics API, if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
}

/// Wakeup timer armed by the agent via `ScheduleWakeup`.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WakeupJobInfo {
    pub reason: String,
    pub prompt_preview: String,
    pub wakeup_at_unix_ms: i64,
}

/// Watch that re-triggers the agent once the `run_in_background` jobs captured
/// at the end of a turn have finished (subprocess mode only).
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundWatchJobInfo {
    pub process_count: u32,
    pub commands: Vec<String>,
}

/// Per-kind payload for a scheduled job.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScheduledJobKind {
    Wakeup(WakeupJobInfo),
    BackgroundWatch(BackgroundWatchJobInfo),
}

/// Snapshot of one scheduled job currently registered in the sandbox.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledJobInfo {
    pub id: String,
    pub registered_at_unix_ms: i64,
    #[serde(flatten)]
    pub kind: ScheduledJobKind,
}

/// A live background process (a `run_in_background` job or ad-hoc daemon) still
/// running in the sandbox after the agent went idle.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundProcess {
    pub pid: u32,
    pub command: String,
}

/// Full sandbox status response to frontend (matches TS SandboxStatusFullResponse)
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatusFullResponse {
    pub sandbox_status: SandboxStatus,
    #[schema(required = true)]
    pub sandbox_error: Option<String>,
    #[schema(required = true)]
    pub agent_status: Option<AgentStatus>,
    pub servers: Vec<ServerEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_diagnostics: Option<Vec<ContainerDiagnostics>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_events: Option<Vec<RuntimeEvent>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub scheduled_jobs: Vec<ScheduledJobInfo>,
    /// Live background processes (`run_in_background` jobs, ad-hoc daemons) still
    /// running in the sandbox. Empty when idle with no background work.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub background_processes: Vec<BackgroundProcess>,
    /// Desired sandbox lifecycle intent recorded on the task
    /// (`running`/`stopped`/`removed`). Reflects the last user/system action,
    /// independent of the live pod phase. `None` when it can't be resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_desired_state: Option<SandboxDesiredState>,
    /// Image the currently-provisioned sandbox container is running, read from
    /// the live Deployment/container. `None` when no sandbox exists yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_sandbox_image: Option<String>,
    /// Image a fresh (re)deploy would use for the sandbox container: the
    /// per-repo `docker_image` override when set, otherwise the configured
    /// default. Compare against `current_sandbox_image` to know whether a
    /// redeploy would change the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_sandbox_image: Option<String>,
    /// `true` when `current_sandbox_image` matches `latest_sandbox_image`.
    /// `None` when either image is unknown (e.g. no sandbox provisioned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_up_to_date: Option<bool>,
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
            Some(
                node.children
                    .into_iter()
                    .map(FileTreeNodeDto::from)
                    .collect(),
            )
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
            status: SandboxStatus::Initializing,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "initializing");
    }

    #[test]
    fn error_body_serializes() {
        let body = crate::error::ErrorBody {
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
