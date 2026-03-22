//! Plugin repository request/response types.

use serde::{Deserialize, Serialize};

/// Request to create a new plugin repo.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreatePluginRepoRequest {
    pub name: String,
    pub git_url: String,
}

/// Plugin repo response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PluginRepoResponse {
    pub id: String,
    pub name: String,
    pub git_url: String,
}
