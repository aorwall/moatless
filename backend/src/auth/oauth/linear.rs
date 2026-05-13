//! Linear workspace OAuth provider.
//!
//! Unlike the user-login providers in this module, Linear OAuth in moatless is
//! used to install the moatless agent as a workspace-scoped app
//! (`actor=app`). The resulting tokens are stored at the workspace level
//! (`provider_tokens.team_id = <linear org uuid>`) and used by the Linear
//! Agents API client to post `agentActivity` / `agentSessionUpdate`.
//!
//! This struct implements `OAuthProvider` for consistency with the existing
//! OAuthProviders registry. `generate_auth_url` + `exchange_code_for_tokens`
//! are the two methods callers need; `get_user_info` returns the viewer
//! (the authenticated agent identity).

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;

use super::provider::OAuthProvider;
use crate::auth::error::AuthError;
use crate::auth::models::{OAuthTokens, ProviderUserInfo};
use crate::config::Config;

const LINEAR_AUTHORIZE_URL: &str = "https://linear.app/oauth/authorize";
const LINEAR_TOKEN_URL: &str = "https://api.linear.app/oauth/token";
const LINEAR_GRAPHQL_URL: &str = "https://api.linear.app/graphql";

/// Scopes requested for the workspace install.
///
/// `app:assignable` + `app:mentionable` enable the Agents UX. `read` /
/// `write` give the agent permission to read/write workspace content.
const LINEAR_OAUTH_SCOPE: &str = "read,write,app:assignable,app:mentionable";

#[derive(Clone)]
pub struct LinearOAuthProvider {
    client_id: String,
    client_secret: String,
    callback_url: String,
    http: Client,
}

impl LinearOAuthProvider {
    /// Build from config. Returns `None` when client_id / client_secret /
    /// callback_url are not set.
    pub fn from_config(config: &Config) -> Option<Self> {
        let client_id = config.linear.oauth_client_id.clone()?;
        let client_secret = config.linear.oauth_client_secret.clone()?;
        let callback_url = config.linear.oauth_callback_url.clone()?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?;
        Some(Self {
            client_id,
            client_secret,
            callback_url,
            http,
        })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn callback_url(&self) -> &str {
        &self.callback_url
    }
}

#[derive(serde::Deserialize)]
struct LinearTokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(serde::Deserialize)]
struct ViewerWrapper {
    data: Option<ViewerData>,
}

#[derive(serde::Deserialize)]
struct ViewerData {
    viewer: Option<Viewer>,
    organization: Option<OrgInfo>,
}

#[derive(serde::Deserialize)]
struct Viewer {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(serde::Deserialize)]
struct OrgInfo {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

/// Result of `LinearOAuthProvider::fetch_install_identity` — the workspace +
/// agent metadata recorded at install time.
#[derive(Debug, Clone)]
pub struct LinearInstallIdentity {
    pub organization_id: String,
    pub organization_name: Option<String>,
    pub agent_viewer_id: String,
    pub agent_viewer_name: Option<String>,
    pub agent_viewer_email: Option<String>,
}

impl LinearOAuthProvider {
    /// Fetch `viewer` and `organization` in one round-trip to record both
    /// the agent identity and the workspace this token belongs to.
    pub async fn fetch_install_identity(
        &self,
        access_token: &str,
    ) -> Result<LinearInstallIdentity, AuthError> {
        let resp = self
            .http
            .post(LINEAR_GRAPHQL_URL)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .header("User-Agent", "moatless-vibe-backend")
            .json(&serde_json::json!({
                "query": "query { viewer { id name email } organization { id name } }"
            }))
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("Linear viewer request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "Linear viewer query returned {status}"
            )));
        }

        let parsed: ViewerWrapper = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Linear viewer parse failed: {e}")))?;

        let data = parsed
            .data
            .ok_or_else(|| AuthError::OAuth("Linear viewer response missing data".into()))?;
        let viewer = data
            .viewer
            .ok_or_else(|| AuthError::OAuth("Linear viewer missing".into()))?;
        let org = data
            .organization
            .ok_or_else(|| AuthError::OAuth("Linear organization missing".into()))?;

        Ok(LinearInstallIdentity {
            organization_id: org.id,
            organization_name: org.name,
            agent_viewer_id: viewer.id,
            agent_viewer_name: viewer.name,
            agent_viewer_email: viewer.email,
        })
    }
}

#[async_trait]
impl OAuthProvider for LinearOAuthProvider {
    fn generate_auth_url(&self, state: &str) -> Option<String> {
        let params = [
            ("actor", "app"),
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.callback_url.as_str()),
            ("state", state),
            ("scope", LINEAR_OAUTH_SCOPE),
        ];
        let query = serde_urlencoded::to_string(params).ok()?;
        Some(format!("{LINEAR_AUTHORIZE_URL}?{query}"))
    }

    async fn exchange_code_for_tokens(&self, code: &str) -> Result<OAuthTokens, AuthError> {
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("redirect_uri", self.callback_url.as_str()),
        ];
        let resp = self
            .http
            .post(LINEAR_TOKEN_URL)
            .form(&form)
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("Linear token exchange failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::OAuth(format!(
                "Linear token exchange returned {status}: {body}"
            )));
        }

        let data: LinearTokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse Linear token response: {e}")))?;

        if let Some(err) = data.error {
            let desc = data.error_description.unwrap_or_default();
            return Err(AuthError::OAuth(format!(
                "Linear OAuth error: {err}: {desc}"
            )));
        }

        Ok(OAuthTokens {
            access_token: data.access_token,
            token_type: data.token_type.unwrap_or_else(|| "Bearer".into()),
            scope: data.scope,
            id_token: None,
            refresh_token: data.refresh_token,
            expires_in: data.expires_in,
            refresh_token_expires_in: None,
        })
    }

    async fn get_user_info(&self, tokens: &OAuthTokens) -> Result<ProviderUserInfo, AuthError> {
        let identity = self.fetch_install_identity(&tokens.access_token).await?;
        Ok(ProviderUserInfo {
            id: identity.agent_viewer_id.clone(),
            login: identity.agent_viewer_name.clone().unwrap_or(identity.agent_viewer_id),
            email: identity.agent_viewer_email,
            name: identity.agent_viewer_name,
            avatar_url: None,
        })
    }
}
