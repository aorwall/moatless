//! Plugin request/response types.

use serde::{Deserialize, Serialize};

/// Request to register a new plugin.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreatePluginRequest {
    pub name: String,
    pub git_url: String,
}

/// Plugin response.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PluginResponse {
    pub id: String,
    pub name: String,
    pub git_url: String,
}
