//! Repository request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::repository::*;

use super::models::Repository;

impl From<Repository> for RepositoryResponse {
    fn from(r: Repository) -> Self {
        Self {
            id: r.id,
            name: r.name,
            kind: r.kind,
            remote_url: r.remote_url,
            default_branch: r.default_branch,
            docker_image: r.docker_image,
            system_prompt: r.system_prompt,
            description: r.description,
            provider: r.provider,
            scope: r.scope,
            team_id: r.team_id,
            owner_user_id: r.owner_user_id,
            servers: r.servers.and_then(|v| serde_json::from_value(v).ok()),
            plugins: r.plugins.and_then(|v| serde_json::from_value(v).ok()),
            env_vars: r.env_vars.and_then(|v| serde_json::from_value(v).ok()),
            setup_commands: r
                .setup_commands
                .and_then(|v| serde_json::from_value(v).ok()),
            server_start_commands: r
                .server_start_commands
                .and_then(|v| serde_json::from_value(v).ok()),
            startup_timeout_seconds: r.startup_timeout_seconds,
            build_command: r.build_command,
            build_output_directory: r.build_output_directory,
            enable_inspector: r.enable_inspector,
            auto_reload: r.auto_reload,
            resources: r.resources.and_then(|v| serde_json::from_value(v).ok()),
            created_by: r.created_by,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.map(|t| t.to_rfc3339()),
        }
    }
}
