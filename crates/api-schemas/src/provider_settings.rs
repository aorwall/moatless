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
    /// True for the App the deployment configures for signing in. It is listed
    /// so admins can see it, and it cannot be rotated or removed from here.
    #[serde(default)]
    pub deployment_configured: bool,
    /// Short digest of the stored signing key. Enough to tell two keys apart;
    /// the key itself is never returned. Absent for the login App, whose key
    /// is not stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_fingerprint: Option<String>,
    /// The bot login this App would have, as GitHub reported it when the App
    /// was last proved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_login: Option<String>,
    /// The Bot user staffing this App, when one has been created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
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
pub struct RegisterGitHubAppRequest {
    /// Short stable name the App will be known by, chosen at registration.
    pub github_app_key: String,
    /// GitHub's own numeric id for the App.
    pub app_id: u64,
    /// PEM-encoded signing key. Proved against GitHub before anything is
    /// stored, and never returned by any read path afterwards.
    pub private_key_pem: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_installation_id: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RotateGitHubAppKeyRequest {
    /// The replacement signing key. Proved before the stored one is replaced,
    /// so a bad key leaves the App exactly as it was.
    pub private_key_pem: String,
}

/// What GitHub confirmed about an App that was just registered or rotated.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHubAppRegistrationResponse {
    pub github_app_key: String,
    pub app_id: u64,
    pub name: String,
    pub slug: String,
    /// The bot login this App would have. Creating the Bot user is a separate
    /// act — an App that only reaches repositories never runs as anyone.
    pub bot_login: String,
    pub key_fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RemoveGitHubAppResponse {
    pub github_app_key: String,
    /// Bot users that named this App and can no longer obtain a token. They
    /// are left in place so past work stays attributed, and named here so the
    /// consequence is not silent.
    pub orphaned_bot_users: Vec<String>,
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

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdapterAppSummary {
    pub adapter_kind: String,
    pub app_key: String,
    pub display_name: String,
    pub config: serde_json::Value,
    pub secret_fingerprints: std::collections::HashMap<String, String>,
    pub revision: i32,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdapterAppsResponse {
    pub apps: Vec<AdapterAppSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveSlackAdapterAppRequest {
    pub bot_token: String,
    pub signing_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveTelegramAdapterAppRequest {
    pub bot_token: String,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SaveLinearAdapterAppRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_callback_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "adapterKind", rename_all = "snake_case")]
pub enum SaveAdapterAppRequest {
    Slack(SaveSlackAdapterAppRequest),
    Telegram(SaveTelegramAdapterAppRequest),
    Linear(SaveLinearAdapterAppRequest),
}
