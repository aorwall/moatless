//! Repository request/response types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::Repository;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub kind: String,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub provider: Option<String>,
    pub visibility: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub servers: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub plugins: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub env_vars: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub setup_commands: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub server_start_commands: Option<serde_json::Value>,
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
    pub visibility: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub servers: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub plugins: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub env_vars: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub setup_commands: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub server_start_commands: Option<serde_json::Value>,
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
    pub kind: String,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub provider: Option<String>,
    pub visibility: String,
    #[schema(value_type = Option<Object>)]
    pub servers: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub plugins: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub env_vars: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub setup_commands: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub server_start_commands: Option<serde_json::Value>,
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
            visibility: r.visibility.unwrap_or_else(|| "public".to_string()),
            servers: r.servers,
            plugins: r.plugins,
            env_vars: r.env_vars,
            setup_commands: r.setup_commands,
            server_start_commands: r.server_start_commands,
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
