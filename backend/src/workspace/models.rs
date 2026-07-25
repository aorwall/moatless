//! Workspace domain types — DB row structs.

use chrono::{DateTime, Utc};

use moatless_backend_core::scope::Scope;

// =============================================================================
// Workspace (DB)
// =============================================================================

/// Row from the `workspaces` table — the run-configuration + composition that a
/// Task references. Run-config columns mirror `repositories`; a shadow workspace
/// created by the Phase A backfill copies them 1:1 from its repository.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Derived/validated label from the `WorkspaceRepo` count: `blank` (0),
    /// `single` (1), or `multi` (n).
    pub kind: String,
    // --- Run configuration (mirrors repositories) ---
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<serde_json::Value>,
    pub env_vars: Option<serde_json::Value>,
    pub servers: Option<serde_json::Value>,
    pub plugins: Option<serde_json::Value>,
    pub resources: Option<serde_json::Value>,
    /// Per-workspace network profile (parity with `repositories`). Selects a
    /// static NetworkPolicy via the pod label; grants nothing on its own.
    pub network_profile: Option<String>,
    // --- Provenance ---
    /// `manual` (created via the API) or `git` (reconciled from a repo's
    /// `.moatless/workspaces.json` / `.moatless/config.json`). Git-synced
    /// workspaces are read-only in the UI and write API.
    pub source: String,
    /// The repository whose git tree declared this workspace (`source = 'git'`).
    pub source_repository_id: Option<String>,
    /// Stable natural key within the source repo — the workspace entry `key`
    /// (or slugified name), or `__config_json__` for the legacy config.json form.
    pub source_key: Option<String>,
    /// Path to the config file within the source repository that declared this
    /// workspace, e.g. `.moatless/workspaces.json` or `.moatless/config.json`.
    /// Retained even after the workspace is overridden (source='manual') so the
    /// origin remains visible.
    pub source_config_path: Option<String>,
    // --- Access control (mirrors repositories) ---
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub deleted: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// =============================================================================
// WorkspaceRepo (DB)
// =============================================================================

/// Row from the `workspace_repos` table — a repository's placement inside a
/// workspace (branch override, mount name, primary flag, ordering).
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct WorkspaceRepo {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    /// Override `repository.default_branch` for this placement (`None` = repo default).
    pub branch: Option<String>,
    /// Subdir under the workspace dir (`None` = default to the repo name at runtime).
    pub mount_name: Option<String>,
    /// The "main" repo for diff/PR/CLAUDE.md. Exactly one per workspace when n>=1.
    pub is_primary: bool,
    /// Ordering within the workspace.
    pub position: i32,
    pub created_at: Option<DateTime<Utc>>,
}
