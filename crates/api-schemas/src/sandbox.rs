//! Sandbox API request/response schemas.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// =============================================================================
// Repository checkouts (backend → sandbox provisioning contract)
// =============================================================================

/// Env var carrying the JSON-encoded [`RepoCheckout`] list the sandbox clones.
///
/// The backend writes it into the sandbox container spec; the sandbox reads it
/// at boot. Absent on a pod provisioned before multi-repo support, where the
/// legacy single-repo `GIT_CLONE_URL`/`GIT_BRANCH`/`GIT_PROVIDER` trio is the
/// only checkout.
pub const WORKSPACE_REPOS_ENV: &str = "WORKSPACE_REPOS";

/// One repository the sandbox checks out, as declared by the task's workspace.
///
/// A workspace holds 0..n repository placements, so a sandbox holds 0..n
/// checkouts. Each is cloned into `{WORKSPACE_DIR}/{mount_name}`; exactly one is
/// primary and its directory is the sandbox's `repo_dir` (git status/diff, the
/// file API, setup commands, preview servers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RepoCheckout {
    /// Remote URL to clone. Credential-free on K8s — the sandbox authenticates
    /// via the credential channel using [`Self::provider`].
    pub clone_url: String,
    /// Branch to check out. `None`/empty clones the remote's default.
    #[serde(default)]
    pub branch: Option<String>,
    /// Git provider name (`github`, `gitlab`, `gitness`) — selects which
    /// provider token authenticates this clone.
    pub provider: String,
    /// Subdirectory under `WORKSPACE_DIR` this repo is cloned into. Always a
    /// single safe path segment (see [`sanitize_mount_name`]).
    pub mount_name: String,
    /// The "main" repo: diff, PR, `AGENTS.md`, setup commands, servers.
    #[serde(default)]
    pub is_primary: bool,
}

impl RepoCheckout {
    /// Absolute checkout directory: `{workspace_dir}/{mount_name}`.
    pub fn dir(&self, workspace_dir: &str) -> String {
        format!(
            "{}/{}",
            workspace_dir.trim_end_matches('/'),
            self.mount_name
        )
    }
}

/// Derive a mount name from a clone URL: the last path segment with any trailing
/// `.git` and slashes stripped. Falls back to `"repo"` for URLs that don't yield
/// a usable segment.
///
/// Handles `https://host/owner/repo(.git)` and `git@host:owner/repo(.git)`. This
/// is the default when a placement has no explicit mount name, and must stay
/// stable: an existing sandbox's primary checkout lives at this path on its
/// persistent volume.
pub fn derive_mount_name(clone_url: &str) -> String {
    let name = clone_url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".git");
    sanitize_mount_name(name).unwrap_or_else(|| "repo".to_string())
}

/// Reduce a user-authored mount name to a single safe path segment, or `None`
/// when nothing usable remains.
///
/// Mount names come from the UI and from repositories' committed
/// `.moatless/workspaces.json`, and they are joined onto `WORKSPACE_DIR` — so
/// separators and `..` must never survive. Characters outside
/// `[A-Za-z0-9._-]` become `-`, and a name that is only dots is rejected.
pub fn sanitize_mount_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return None;
    }
    Some(cleaned)
}

// =============================================================================
// Sandbox model types (shared between schemas and backend)
// =============================================================================

pub use moatless_events::SandboxStatus;

/// Observed Agent lifecycle status for a ready Sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Running,
    /// Parked on a person: the run is open, and nothing is running until it is
    /// answered. Must stay in step with `backend::sandbox::live_status`, which
    /// is what serializes the value this deserializes.
    Waiting,
    Idle,
    Unknown,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Running => write!(f, "running"),
            AgentStatus::Waiting => write!(f, "waiting"),
            AgentStatus::Idle => write!(f, "idle"),
            AgentStatus::Unknown => write!(f, "unknown"),
        }
    }
}

impl From<&str> for AgentStatus {
    fn from(s: &str) -> Self {
        match s {
            "running" => AgentStatus::Running,
            "waiting" => AgentStatus::Waiting,
            "idle" | "stopped" => AgentStatus::Idle,
            _ => AgentStatus::Unknown,
        }
    }
}

#[cfg(test)]
mod agent_status_tests {
    use super::AgentStatus;

    /// Every status the backend can serialize must round-trip here — a missing
    /// variant fails the whole `/sandbox/status` response, not just the field.
    #[test]
    fn every_backend_status_deserializes() {
        for wire in ["running", "waiting", "idle", "unknown"] {
            let json = format!("\"{wire}\"");
            let parsed: AgentStatus =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("{wire}: {e}"));
            assert_eq!(parsed.to_string(), wire);
            assert_eq!(AgentStatus::from(wire), parsed);
        }
    }
}

/// Small live-status resource shared by list, sidebar, and dashboard views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct LiveStatus {
    pub sandbox_status: SandboxStatus,
    pub agent_status: Option<AgentStatus>,
    pub sandbox_error: Option<String>,
}

/// Matches TS ServerStatusEnumSchema: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Stopped,
    Installing,
    Starting,
    Started,
    Failed,
}

impl From<&str> for ServerStatus {
    fn from(s: &str) -> Self {
        match s {
            "stopped" => ServerStatus::Stopped,
            "installing" => ServerStatus::Installing,
            "starting" => ServerStatus::Starting,
            "started" => ServerStatus::Started,
            "failed" => ServerStatus::Failed,
            _ => ServerStatus::Stopped,
        }
    }
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => write!(f, "stopped"),
            Self::Installing => write!(f, "installing"),
            Self::Starting => write!(f, "starting"),
            Self::Started => write!(f, "started"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Server snapshot from sandbox GET /status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServerSnapshot {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub enable_inspector: bool,
    pub status: ServerStatus,
    pub error: Option<String>,
    pub pid: Option<u32>,
}

/// Resource metrics from sandbox GET /status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetrics {
    pub memory_usage_mb: f64,
    pub memory_limit_mb: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub cpu_usage_percent: Option<f64>,
    pub event_loop_latency_ms: f64,
    pub uptime_seconds: f64,
}

/// Server capabilities (deterministic from server name)
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub address_bar: bool,
    pub inspector: bool,
    pub reload: bool,
    pub open_external: bool,
    pub viewport: bool,
    pub console: bool,
}

// =============================================================================
// API response schemas
// =============================================================================

/// Single server entry in the workspace status response
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
}

/// Full sandbox status response to frontend (matches TS SandboxStatusFullResponse)
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatusFullResponse {
    pub sandbox_status: SandboxStatus,
    pub sandbox_error: Option<String>,
    pub agent_status: Option<AgentStatus>,
    pub servers: Vec<ServerEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_diagnostics: Option<Vec<ContainerDiagnostics>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_events: Option<Vec<RuntimeEvent>>,
}

/// Server status response proxied from sandbox pod.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub servers: Vec<ServerSnapshot>,
}

/// A live override of one preview server's start command and env.
///
/// Stored only as control files on the sandbox's workspace volume — there is no
/// database copy — so every read of one needs a running sandbox.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unset_env: Vec<String>,
}

/// Update one preview server's live config.
///
/// `port` and `dockerImage` exist only so a caller that sends them gets an
/// explicit "redeploy required" rather than a silent no-op.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateServerConfigRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_commands: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unset_env: Option<Vec<String>>,
    pub restart: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<String>,
}

/// Result of a live server config change or restart.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerConfigResponse {
    pub server_name: String,
    pub config_override: ServerOverride,
    pub generation: u64,
    pub restarted: bool,
}

/// One server's override as it currently exists on the workspace volume.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverrideEntry {
    pub server_name: String,
    pub config_override: ServerOverride,
    /// The counter the sidecar watches; `0` means it has never been restarted.
    pub generation: u64,
}

/// Every override in effect on a task. Servers with no override are omitted.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverridesResponse {
    pub overrides: Vec<ServerOverrideEntry>,
}

// ── File operation DTOs ──────────────────────────────────────────

/// File tree response with git change summary.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileTreeResponse {
    pub tree: Vec<FileTreeNodeDto>,
    pub git_summary: GitSummaryDto,
}

/// A node in the file tree (file or directory).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub children: Option<Vec<FileTreeNodeDto>>,
}

/// Git change summary across the workspace.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WriteFileRequestBody {
    pub path: String,
    pub content: String,
}

/// Response for listing files.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ListFilesResponse {
    pub files: Vec<String>,
}

/// Request body for searching files by name.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SearchByNameRequest {
    pub query: String,
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    #[serde(default)]
    pub max_results: Option<i32>,
}

/// Response for searching files by name.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SearchByNameResponse {
    pub files: Vec<FileNameMatchDto>,
    pub total_matches: i32,
}

/// A file name match in the search results.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileNameMatchDto {
    pub path: String,
    pub matches: i32,
}

// ── Moatless tool names ──────────────────────────────────────────

/// Response from provisioning a sandbox.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub status: String,
}

/// All available Moatless tool names injected into the agent sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_mount_name_handles_common_url_shapes() {
        assert_eq!(
            derive_mount_name("https://github.com/owner/repo.git"),
            "repo"
        );
        assert_eq!(derive_mount_name("https://github.com/owner/repo"), "repo");
        assert_eq!(derive_mount_name("git@github.com:owner/repo.git"), "repo");
        assert_eq!(derive_mount_name("https://host/owner/repo/"), "repo");
        assert_eq!(
            derive_mount_name("https://gitlab.com/group/sub/my-service.git"),
            "my-service"
        );
    }

    #[test]
    fn derive_mount_name_falls_back_for_unusable_urls() {
        assert_eq!(derive_mount_name(""), "repo");
        assert_eq!(derive_mount_name("/"), "repo");
        assert_eq!(derive_mount_name("https://host/../"), "repo");
    }

    /// A mount name is joined onto `WORKSPACE_DIR`, so traversal and separators
    /// must not survive sanitization.
    #[test]
    fn sanitize_mount_name_strips_path_traversal() {
        assert_eq!(sanitize_mount_name("../../etc"), Some("..-..-etc".into()));
        assert_eq!(sanitize_mount_name("a/b"), Some("a-b".into()));
        assert_eq!(sanitize_mount_name(".."), None);
        assert_eq!(sanitize_mount_name("."), None);
        assert_eq!(sanitize_mount_name("   "), None);
        assert_eq!(sanitize_mount_name("/"), None);
    }

    #[test]
    fn sanitize_mount_name_keeps_ordinary_names() {
        assert_eq!(sanitize_mount_name("my-service"), Some("my-service".into()));
        assert_eq!(sanitize_mount_name("Api_2.0"), Some("Api_2.0".into()));
    }

    #[test]
    fn checkout_dir_joins_workspace_dir() {
        let checkout = RepoCheckout {
            clone_url: "https://github.com/owner/service.git".into(),
            branch: None,
            provider: "github".into(),
            mount_name: "service".into(),
            is_primary: true,
        };
        assert_eq!(
            checkout.dir("/opt/moatless/workspace"),
            "/opt/moatless/workspace/service"
        );
        assert_eq!(
            checkout.dir("/opt/moatless/workspace/"),
            "/opt/moatless/workspace/service"
        );
    }
}
