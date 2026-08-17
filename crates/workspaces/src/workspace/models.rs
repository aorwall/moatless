//! Workspace domain types — DB row structs.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use moatless_backend_core::scope::Scope;

/// Named command run on demand inside a Sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptConfig {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub command: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub env_vars: Option<HashMap<String, String>>,
}

/// Provider whose static credential can be pinned to a Workspace.
///
/// GitHub is not here. A GitHub token is derived from an identity, so a
/// Workspace names the user it runs as — `workspaces.run_as_user_id` — and the
/// token is resolved for that identity like any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceCredentialProvider {
    Gitlab,
    Gitness,
    Openai,
    Anthropic,
}

impl WorkspaceCredentialProvider {
    pub const ALL: &'static [Self] = &[Self::Gitlab, Self::Gitness, Self::Openai, Self::Anthropic];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gitlab => "gitlab",
            Self::Gitness => "gitness",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn env_key(self) -> &'static str {
        match self {
            Self::Gitlab => "GITLAB_TOKEN",
            Self::Gitness => "GITNESS_TOKEN",
            Self::Openai => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
        }
    }

    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|provider| provider.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl std::fmt::Display for WorkspaceCredentialProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for WorkspaceCredentialProvider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|provider| provider.as_str() == value)
            .ok_or_else(|| {
                format!(
                    "Unknown credential provider '{value}'. Must be one of: {}",
                    Self::names()
                )
            })
    }
}

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
    /// Named commands run on demand in the sandbox — an array of `ScriptConfig`.
    pub scripts: Option<serde_json::Value>,
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
    /// The identity every Task on this Workspace acts as on a git host. When
    /// set it outranks the Task's own owner, so a Workspace can act as a
    /// dedicated machine account instead of as whoever started the Task.
    pub run_as_user_id: Option<String>,
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
    /// The "main" repo for diff/PR/AGENTS.md. Exactly one per workspace when n>=1.
    pub is_primary: bool,
    /// Ordering within the workspace.
    pub position: i32,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_provider_maps_persisted_and_environment_values() {
        let mut environment_keys = Vec::new();
        for provider in WorkspaceCredentialProvider::ALL {
            assert_eq!(provider.as_str().parse(), Ok(*provider));
            assert!(provider.env_key().ends_with("TOKEN") || provider.env_key().ends_with("KEY"));
            environment_keys.push(provider.env_key());
        }
        environment_keys.sort_unstable();
        environment_keys.dedup();
        assert_eq!(
            environment_keys.len(),
            WorkspaceCredentialProvider::ALL.len()
        );
    }
}
