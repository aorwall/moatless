//! Access-settings wire contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitHubProviderTokenStatusResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    /// How the stored credential mints a token: `github_app` for a GitHub App
    /// user token, `github_app_installation` for an installation, `secret_ref`
    /// for a Personal Access Token held in the secret store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// Whether a Personal Access Token is standing in front of a GitHub App
    /// credential that `DELETE /settings/github/pat` can put back. A PAT with
    /// nothing behind it is the only credential there is, not an override.
    pub has_pat_override: bool,
    /// Whether `GET /auth/github-app/connect` can run on this deployment. False
    /// leaves a Personal Access Token as the only way to connect.
    pub app_connect_available: bool,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveGitHubProviderTokenRequest {
    pub token: String,
}

impl std::fmt::Debug for SaveGitHubProviderTokenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveGitHubProviderTokenRequest")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitHubInstallationOption {
    pub installation_id: u64,
    pub account_login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_type: Option<String>,
    /// Permission name to level (`read`/`write`) for this installation, as
    /// GitHub reports it. A permission the App was never granted is absent
    /// rather than `"none"`, so an operator reading this list sees a gap as a
    /// missing row.
    #[serde(default)]
    pub permissions: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
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
    /// the key itself is never returned. Absent for the login App, whose key is
    /// not stored.
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

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitHubAppsResponse {
    pub apps: Vec<GitHubAppOption>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
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

impl std::fmt::Debug for RegisterGitHubAppRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterGitHubAppRequest")
            .field("github_app_key", &self.github_app_key)
            .field("app_id", &self.app_id)
            .field("private_key_pem", &"[REDACTED]")
            .field("default_installation_id", &self.default_installation_id)
            .finish()
    }
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RotateGitHubAppKeyRequest {
    /// The replacement signing key. Proved before the stored one is replaced,
    /// so a bad key leaves the App exactly as it was.
    pub private_key_pem: String,
}

impl std::fmt::Debug for RotateGitHubAppKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RotateGitHubAppKeyRequest")
            .field("private_key_pem", &"[REDACTED]")
            .finish()
    }
}

/// What GitHub confirmed about an App that was just registered or rotated.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
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

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemoveGitHubAppResponse {
    pub github_app_key: String,
    /// Bot users that named this App and can no longer obtain a token. They are
    /// left in place so past work stays attributed, and named here so the
    /// consequence is not silent.
    pub orphaned_bot_users: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexAgentHarnessCredentialStatusResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    /// "`device_code`" | "`auth_json`" | "`api_key`" — which save path produced
    /// the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// Email reported by ChatGPT account on the `id_token` (device-code flow
    /// only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// ChatGPT plan type (e.g., "plus", "pro").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// True when the stored credential hit a permanent refresh failure and the
    /// user must sign in again. Absent when no credential exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_reconnect: Option<bool>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveCodexAgentHarnessCredentialRequest {
    pub auth_json: String,
}

impl std::fmt::Debug for SaveCodexAgentHarnessCredentialRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveCodexAgentHarnessCredentialRequest")
            .field("auth_json", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceCodeStartResponse {
    pub device_auth_id: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexDeviceCodePollRequest {
    pub device_auth_id: String,
    pub user_code: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
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

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitnessProviderTokenStatusResponse {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveGitnessProviderTokenRequest {
    pub base_url: String,
    pub token: String,
}

impl std::fmt::Debug for SaveGitnessProviderTokenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveGitnessProviderTokenRequest")
            .field("base_url", &self.base_url)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// One person's connection to one Forgejo instance.
///
/// There is no `login`, unlike every sibling here: a `read:repository` token
/// can't reach Forgejo's `user` endpoint, so validating one only learns that it
/// works.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForgejoProviderTokenStatusResponse {
    pub host: String,
    pub connected: bool,
    /// How the stored credential resolves: `secret_ref` for a personal access
    /// token held in the secret store, `forgejo_oauth` for an OAuth2
    /// credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// Whether a personal access token is standing in front of an OAuth
    /// credential that `DELETE /settings/forgejo/{host}/pat` can put back.
    pub has_pat_override: bool,
    /// Whether this instance has a registered OAuth2 application. False leaves
    /// a personal access token as the only way to connect.
    pub oauth_available: bool,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveForgejoProviderTokenRequest {
    /// The instance, as a host or a URL. Only its host is kept.
    pub base_url: String,
    /// A Forgejo personal access token must name its scopes, and this one needs
    /// `read:repository` to clone and `write:repository` to push.
    pub token: String,
}

impl std::fmt::Debug for SaveForgejoProviderTokenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveForgejoProviderTokenRequest")
            .field("base_url", &self.base_url)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// Which instance a Forgejo settings request is about.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ForgejoHostQuery {
    /// The instance, as a host or a URL. Only its host is kept.
    pub host: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForgejoInstanceSummary {
    pub host: String,
    pub client_id: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForgejoInstancesResponse {
    pub instances: Vec<ForgejoInstanceSummary>,
}

/// Every registered instance, each carrying the caller's own connection to it.
///
/// Not [`ForgejoInstancesResponse`], the admin view that names a `client_id`.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ForgejoConnectionsResponse {
    pub instances: Vec<ForgejoProviderTokenStatusResponse>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegisterForgejoInstanceRequest {
    pub client_id: String,
    /// Sealed under the instance row's id before it is stored, and never
    /// returned by any read path afterwards.
    pub client_secret: String,
}

impl std::fmt::Debug for RegisterForgejoInstanceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterForgejoInstanceRequest")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
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

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdapterAppsResponse {
    pub apps: Vec<AdapterAppSummary>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveSlackAdapterAppRequest {
    pub bot_token: String,
    pub signing_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
}

impl std::fmt::Debug for SaveSlackAdapterAppRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveSlackAdapterAppRequest")
            .field("bot_token", &"[REDACTED]")
            .field("signing_secret", &"[REDACTED]")
            .field("bot_user_id", &self.bot_user_id)
            .finish()
    }
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveTelegramAdapterAppRequest {
    pub bot_token: String,
    #[serde(default)]
    pub mode: Option<String>,
}

impl std::fmt::Debug for SaveTelegramAdapterAppRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveTelegramAdapterAppRequest")
            .field("bot_token", &"[REDACTED]")
            .field("mode", &self.mode)
            .finish()
    }
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
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

impl std::fmt::Debug for SaveLinearAdapterAppRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveLinearAdapterAppRequest")
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "oauth_client_id",
                &self.oauth_client_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "oauth_client_secret",
                &self.oauth_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("oauth_callback_url", &self.oauth_callback_url)
            .field("api_base_url", &self.api_base_url)
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "adapterKind", rename_all = "snake_case")]
pub enum SaveAdapterAppRequest {
    Slack(SaveSlackAdapterAppRequest),
    Telegram(SaveTelegramAdapterAppRequest),
    Linear(SaveLinearAdapterAppRequest),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_requests_redact_secrets_from_debug_output() {
        // Paired with the plaintext rather than checked for the word "secret":
        // `client_secret` is a field name, and a redacted rendering still prints it.
        let values = [
            (
                format!(
                    "{:?}",
                    SaveGitHubProviderTokenRequest {
                        token: "github-plaintext".to_string(),
                    }
                ),
                "github-plaintext",
            ),
            (
                format!(
                    "{:?}",
                    SaveGitnessProviderTokenRequest {
                        base_url: "https://gitness.example".to_string(),
                        token: "gitness-plaintext".to_string(),
                    }
                ),
                "gitness-plaintext",
            ),
            (
                format!(
                    "{:?}",
                    SaveForgejoProviderTokenRequest {
                        base_url: "https://git.acme.dev".to_string(),
                        token: "forgejo-plaintext".to_string(),
                    }
                ),
                "forgejo-plaintext",
            ),
            (
                format!(
                    "{:?}",
                    RegisterForgejoInstanceRequest {
                        client_id: "forgejo-client".to_string(),
                        client_secret: "forgejo-app-plaintext".to_string(),
                    }
                ),
                "forgejo-app-plaintext",
            ),
            (
                format!(
                    "{:?}",
                    SaveCodexAgentHarnessCredentialRequest {
                        auth_json: "codex-plaintext".to_string(),
                    }
                ),
                "codex-plaintext",
            ),
            (
                format!(
                    "{:?}",
                    RegisterGitHubAppRequest {
                        github_app_key: "example".to_string(),
                        app_id: 1,
                        private_key_pem: "github-app-plaintext".to_string(),
                        default_installation_id: None,
                    }
                ),
                "github-app-plaintext",
            ),
        ];

        for (value, plaintext) in values {
            assert!(value.contains("[REDACTED]"));
            assert!(!value.contains(plaintext));
        }
    }
}
