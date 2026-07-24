//! Provider settings request/response types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHubConfigResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    /// True when a GitHub-OAuth user has an active PAT override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_pat_override: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SavePatRequest {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHubInstallationBindingRequest {
    pub github_app_key: String,
    pub installation_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHubInstallationBindingResponse {
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_app_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHubInstallationOption {
    pub installation_id: u64,
    pub account_login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHubAppOption {
    pub github_app_key: String,
    pub name: String,
    pub install_url: String,
    pub installations: Vec<GitHubInstallationOption>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHubAppsResponse {
    pub apps: Vec<GitHubAppOption>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexConfigResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    /// "device_code" | "auth_json" | "api_key" — which save path produced the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// Email reported by ChatGPT account on the id_token (device-code flow only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// ChatGPT plan type (e.g., "plus", "pro").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// True when the stored credential hit a permanent refresh failure and
    /// the user must sign in again. Absent when no credential exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_reconnect: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveCodexAuthRequest {
    pub auth_json: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceCodeStartResponse {
    pub device_auth_id: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceCodePollRequest {
    pub device_auth_id: String,
    pub user_code: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum CodexDeviceCodePollResponse {
    Pending,
    Success {
        connected: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        email: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        plan_type: Option<String>,
    },
    Expired,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct OAuthUrlResponse {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ExchangeTokenRequest {
    pub code: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ExchangeTokenResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitnessConfigResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveGitnessPATRequest {
    pub base_url: String,
    pub token: String,
}
