//! Repository request/response types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::{Repository, RepositoryKind, RepositoryProvider};
use crate::sandbox::models::ServerConfig;
use crate::scope::Scope;
use crate::task::models::Visibility;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub visibility: Option<Visibility>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
    pub servers: Option<Vec<ServerConfig>>,
    pub plugins: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub setup_commands: Option<Vec<String>>,
    pub server_start_commands: Option<Vec<String>>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRepositoryRequest {
    pub name: Option<String>,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub visibility: Option<Visibility>,
    pub servers: Option<Vec<ServerConfig>>,
    pub plugins: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub setup_commands: Option<Vec<String>>,
    pub server_start_commands: Option<Vec<String>>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryResponse {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub visibility: Visibility,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub servers: Option<Vec<ServerConfig>>,
    pub plugins: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub setup_commands: Option<Vec<String>>,
    pub server_start_commands: Option<Vec<String>>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BranchListResponse {
    pub branches: Vec<BranchEntry>,
    pub default_branch: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BranchEntry {
    pub name: String,
    pub is_default: bool,
}

impl From<Repository> for RepositoryResponse {
    fn from(r: Repository) -> Self {
        // Compute visibility from scope for backwards compat
        let visibility = match r.scope {
            Scope::Global => Visibility::Public,
            _ => r.visibility.unwrap_or(Visibility::Private),
        };
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
            visibility,
            scope: r.scope,
            team_id: r.team_id,
            servers: r.servers.and_then(|v| serde_json::from_value(v).ok()),
            plugins: r.plugins.and_then(|v| serde_json::from_value(v).ok()),
            env_vars: r.env_vars.and_then(|v| serde_json::from_value(v).ok()),
            setup_commands: r.setup_commands.and_then(|v| serde_json::from_value(v).ok()),
            server_start_commands: r.server_start_commands.and_then(|v| serde_json::from_value(v).ok()),
            startup_timeout_seconds: r.startup_timeout_seconds,
            build_command: r.build_command,
            build_output_directory: r.build_output_directory,
            enable_inspector: r.enable_inspector,
            auto_reload: r.auto_reload,
            created_by: r.created_by,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.map(|t| t.to_rfc3339()),
        }
    }
}
