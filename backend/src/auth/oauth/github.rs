//! GitHub OAuth provider implementation.

use std::time::Duration;

use super::provider::OAuthProvider;
use crate::auth::error::AuthError;
use crate::auth::models::{OAuthTokens, ProviderUserInfo};
use crate::config::Config;
use async_trait::async_trait;

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
        let client_id = config.auth.github_client_id.clone()?;
        let client_secret = config.auth.github_client_secret.clone()?;
        let callback_url = config.auth.github_callback_url.clone()?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?;
        Some(Self {
            client_id,
            client_secret,
            callback_url,
            scope: config.auth.github_oauth_scope.clone(),
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
struct GitHubOrg {
    login: String,
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
    fn generate_auth_url(&self, state: &str) -> Option<String> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.callback_url.as_str()),
            ("state", state),
            ("scope", self.scope.as_str()),
        ];
        let query = serde_urlencoded::to_string(params).ok()?;
        Some(format!("https://github.com/login/oauth/authorize?{query}"))
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
            .map_err(|e| AuthError::OAuth(format!("GitHub token exchange failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub token exchange returned {status}"
            )));
        }

        let data: GitHubTokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse GitHub token response: {e}")))?;

        if let Some(err) = data.error {
            return Err(AuthError::OAuth(format!("GitHub OAuth error: {err}")));
        }

        Ok(OAuthTokens {
            access_token: data.access_token,
            token_type: data.token_type,
            scope: data.scope,
            id_token: None,
            refresh_token: None,
            expires_in: None,
            refresh_token_expires_in: None,
        })
    }

    async fn get_user_info(&self, tokens: &OAuthTokens) -> Result<ProviderUserInfo, AuthError> {
        let resp = self
            .http
            .get("https://api.github.com/user")
            .header("Authorization", format!("Bearer {}", tokens.access_token))
            .header("User-Agent", "moatless-vibe-backend")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("GitHub user info request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub user info returned {status}"
            )));
        }

        let user: GitHubUser = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse GitHub user response: {e}")))?;

        Ok(ProviderUserInfo {
            id: user.id.to_string(),
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
        })
    }

    async fn check_org_membership(&self, access_token: &str, org: &str) -> Result<bool, AuthError> {
        let mut page = 1u32;
        loop {
            let resp = self
                .http
                .get(format!(
                    "https://api.github.com/user/orgs?per_page=100&page={page}"
                ))
                .header("Authorization", format!("Bearer {access_token}"))
                .header("User-Agent", "moatless-vibe-backend")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(|e| AuthError::OAuth(format!("GitHub orgs request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                return Err(AuthError::OAuth(format!(
                    "GitHub orgs endpoint returned {status}"
                )));
            }

            let orgs: Vec<GitHubOrg> = resp.json().await.map_err(|e| {
                AuthError::OAuth(format!("Failed to parse GitHub orgs response: {e}"))
            })?;

            if orgs.is_empty() {
                break;
            }

            if orgs.iter().any(|o| o.login.eq_ignore_ascii_case(org)) {
                return Ok(true);
            }

            page += 1;
        }

        Ok(false)
    }
}
