//! GitHub OAuth provider implementation.

use std::time::Duration;

use async_trait::async_trait;
use crate::auth::error::AuthError;
use crate::auth::types::{OAuthTokens, ProviderUserInfo};
use crate::config::Config;
use super::provider::OAuthProvider;

pub struct GitHubOAuthProvider {
    client_id: String,
    client_secret: String,
    callback_url: String,
    scope: String,
    http: reqwest::Client,
}

impl GitHubOAuthProvider {
    /// Create from config. Returns None if GitHub OAuth is not configured.
    pub fn from_config(config: &Config) -> Option<Self> {
        let client_id = config.github_client_id.clone()?;
        let client_secret = config.github_client_secret.clone()?;
        let callback_url = config.github_callback_url.clone()?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");
        Some(Self {
            client_id,
            client_secret,
            callback_url,
            scope: config.github_oauth_scope.clone(),
            http,
        })
    }
}

#[derive(serde::Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
    token_type: String,
    scope: Option<String>,
    error: Option<String>,
}

#[derive(serde::Deserialize)]
struct GitHubUser {
    id: u64,
    login: String,
    email: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
}

#[async_trait]
impl OAuthProvider for GitHubOAuthProvider {
    fn provider_id(&self) -> &str {
        "github"
    }

    fn generate_auth_url(&self, state: &str) -> Option<String> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.callback_url.as_str()),
            ("state", state),
            ("scope", self.scope.as_str()),
        ];
        let query = serde_urlencoded::to_string(params).ok()?;
        Some(format!(
            "https://github.com/login/oauth/authorize?{}",
            query
        ))
    }

    async fn exchange_code_for_tokens(&self, code: &str) -> Result<OAuthTokens, AuthError> {
        let resp = self
            .http
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "code": code,
            }))
            .send()
            .await
            .map_err(|e| AuthError::OAuthError(format!("GitHub token exchange failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(AuthError::OAuthError(format!(
                "GitHub token exchange returned {}",
                resp.status()
            )));
        }

        let data: GitHubTokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuthError(format!("Failed to parse GitHub token response: {}", e)))?;

        if let Some(err) = data.error {
            return Err(AuthError::OAuthError(format!("GitHub OAuth error: {}", err)));
        }

        Ok(OAuthTokens {
            access_token: data.access_token,
            token_type: data.token_type,
            scope: data.scope,
            id_token: None,
        })
    }

    async fn get_user_info(&self, tokens: &OAuthTokens) -> Result<ProviderUserInfo, AuthError> {
        let resp = self
            .http
            .get("https://api.github.com/user")
            .header("Authorization", format!("Bearer {}", tokens.access_token))
            .header("User-Agent", "moatless-sandbox-controller")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| AuthError::OAuthError(format!("GitHub user info request failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(AuthError::OAuthError(format!(
                "GitHub user info returned {}",
                resp.status()
            )));
        }

        let user: GitHubUser = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuthError(format!("Failed to parse GitHub user response: {}", e)))?;

        Ok(ProviderUserInfo {
            id: user.id.to_string(),
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
        })
    }
}
