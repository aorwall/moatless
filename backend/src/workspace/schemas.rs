//! Workspace request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate, with local `From`
//! conversions from the DB row models.

pub use moatless_api_schemas::workspace::*;

use super::models::{Workspace, WorkspaceRepo};

impl From<WorkspaceRepo> for WorkspaceRepoResponse {
    fn from(r: WorkspaceRepo) -> Self {
        Self {
            id: r.id,
            workspace_id: r.workspace_id,
            repository_id: r.repository_id,
            branch: r.branch,
            mount_name: r.mount_name,
            is_primary: r.is_primary,
            position: r.position,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// Build a [`WorkspaceResponse`] from a workspace row plus its placements.
/// (A free function rather than `From<(..)>` — the orphan rule forbids
/// implementing a foreign trait for a foreign type keyed on a tuple.)
pub fn workspace_response(w: Workspace, repos: Vec<WorkspaceRepo>) -> WorkspaceResponse {
    WorkspaceResponse {
            id: w.id,
            name: w.name,
            description: w.description,
            kind: w.kind,
            docker_image: w.docker_image,
            docker_access: w.docker_access,
            sandbox_service_account: w.sandbox_service_account,
            system_prompt: w.system_prompt,
            setup_commands: w.setup_commands.and_then(|v| serde_json::from_value(v).ok()),
            env_vars: w.env_vars.and_then(|v| serde_json::from_value(v).ok()),
            servers: w.servers.and_then(|v| serde_json::from_value(v).ok()),
            plugins: w.plugins.and_then(|v| serde_json::from_value(v).ok()),
            resources: w.resources.and_then(|v| serde_json::from_value(v).ok()),
            network_profile: w.network_profile,
            synced_from_git: w.source == "git",
            source: w.source,
            source_repository_id: w.source_repository_id,
            scope: w.scope,
            team_id: w.team_id,
            owner_user_id: w.owner_user_id,
            created_by: w.created_by,
            created_at: w.created_at.map(|t| t.to_rfc3339()),
            updated_at: w.updated_at.map(|t| t.to_rfc3339()),
            repos: repos.into_iter().map(Into::into).collect(),
    }
}
