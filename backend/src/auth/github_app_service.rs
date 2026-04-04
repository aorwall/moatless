//! GitHub App server-side authentication service.
//!
//! Provides two capabilities:
//! 1. **Installation tokens** — server-to-server API calls as the App itself.
//! 2. **User token refresh** — refresh expired GitHub App user access tokens.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::auth::error::AuthError;
use crate::auth::models::OAuthTokens;
use crate::config::Config;

/// Cached installation access token.
struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

/// GitHub App service for installation tokens and user token refresh.
pub struct GitHubAppService {
    app_id: u64,
    private_key: EncodingKey,
    client_id: Option<String>,
    client_secret: Option<String>,
    default_installation_id: Option<u64>,
    http: reqwest::Client,
    token_cache: RwLock<HashMap<u64, CachedToken>>,
}

/// JWT claims for GitHub App authentication.
#[derive(Serialize)]
struct AppJwtClaims {
    iss: String,
    iat: i64,
    exp: i64,
}

#[derive(Deserialize)]
struct InstallationTokenResponse {
    token: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct RefreshTokenResponse {
    access_token: String,
    token_type: String,
    scope: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    refresh_token_expires_in: Option<u64>,
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Deserialize)]
pub struct Installation {
    pub id: u64,
    pub account: InstallationAccount,
}

#[derive(Deserialize)]
pub struct InstallationAccount {
    pub login: String,
    #[serde(rename = "type")]
    pub account_type: Option<String>,
}

impl GitHubAppService {
    /// Create from config. Returns None if `github_app_id` or `github_app_private_key` is not set.
    pub fn from_config(config: &Config) -> Option<Self> {
        let app_id = config.auth.github_app_id?;
        let pem = config.auth.github_app_private_key.as_ref()?;

        let private_key = match EncodingKey::from_rsa_pem(pem.as_bytes()) {
            Ok(key) => key,
            Err(e) => {
                warn!(error = %e, "Failed to parse GitHub App private key PEM");
                return None;
            }
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?;

        info!(app_id = app_id, "GitHub App service configured");

        Some(Self {
            app_id,
            private_key,
            client_id: config.auth.github_app_client_id.clone(),
            client_secret: config.auth.github_app_client_secret.clone(),
            default_installation_id: config.auth.github_app_default_installation_id,
            http,
            token_cache: RwLock::new(HashMap::new()),
        })
    }

    /// Generate a short-lived JWT for authenticating as the GitHub App.
    fn generate_app_jwt(&self) -> Result<String, AuthError> {
        let now = Utc::now().timestamp();
        let claims = AppJwtClaims {
            iss: self.app_id.to_string(),
            iat: now - 60, // clock skew buffer
            exp: now + 600, // 10 minutes max per GitHub docs
        };

        let header = Header::new(jsonwebtoken::Algorithm::RS256);
        encode(&header, &claims, &self.private_key)
            .map_err(|e| AuthError::OAuth(format!("Failed to generate GitHub App JWT: {e}")))
    }

    /// Get an installation access token, using cache when available.
    pub async fn get_installation_token(
        &self,
        installation_id: u64,
    ) -> Result<String, AuthError> {
        // Check cache first
        {
            let cache = self.token_cache.read().await;
            if let Some(cached) = cache.get(&installation_id) {
                // Return cached token if it has at least 5 minutes left
                let margin = Utc::now() + chrono::Duration::minutes(5);
                if cached.expires_at > margin {
                    debug!(installation_id, "Using cached installation token");
                    return Ok(cached.token.clone());
                }
            }
        }

        // Generate a new installation token
        let app_jwt = self.generate_app_jwt()?;

        let resp = self
            .http
            .post(format!(
                "https://api.github.com/app/installations/{installation_id}/access_tokens"
            ))
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .send()
            .await
            .map_err(|e| {
                AuthError::OAuth(format!("GitHub installation token request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::OAuth(format!(
                "GitHub installation token request returned {status}: {body}"
            )));
        }

        let data: InstallationTokenResponse = resp
            .json()
            .await
            .map_err(|e| {
                AuthError::OAuth(format!(
                    "Failed to parse GitHub installation token response: {e}"
                ))
            })?;

        let expires_at = data
            .expires_at
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now() + chrono::Duration::hours(1));

        // Cache the token
        {
            let mut cache = self.token_cache.write().await;
            cache.insert(
                installation_id,
                CachedToken {
                    token: data.token.clone(),
                    expires_at,
                },
            );
        }

        info!(installation_id, "Obtained new GitHub installation token");
        Ok(data.token)
    }

    /// Get an installation token using the default installation ID.
    pub async fn get_default_installation_token(&self) -> Result<String, AuthError> {
        let id = self.default_installation_id.ok_or_else(|| {
            AuthError::Config("No default GitHub App installation ID configured".to_string())
        })?;
        self.get_installation_token(id).await
    }

    /// List all installations of this GitHub App.
    pub async fn list_installations(&self) -> Result<Vec<Installation>, AuthError> {
        let app_jwt = self.generate_app_jwt()?;

        let resp = self
            .http
            .get("https://api.github.com/app/installations")
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .send()
            .await
            .map_err(|e| {
                AuthError::OAuth(format!("GitHub list installations request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub list installations returned {status}"
            )));
        }

        resp.json().await.map_err(|e| {
            AuthError::OAuth(format!(
                "Failed to parse GitHub installations response: {e}"
            ))
        })
    }

    /// Refresh a GitHub App user access token using a refresh token.
    pub async fn refresh_user_token(
        &self,
        refresh_token: &str,
    ) -> Result<OAuthTokens, AuthError> {
        let client_id = self.client_id.as_deref().ok_or_else(|| {
            AuthError::Config("GitHub App client_id required for token refresh".to_string())
        })?;
        let client_secret = self.client_secret.as_deref().ok_or_else(|| {
            AuthError::Config("GitHub App client_secret required for token refresh".to_string())
        })?;

        let resp = self
            .http
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": client_id,
                "client_secret": client_secret,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            }))
            .send()
            .await
            .map_err(|e| {
                AuthError::OAuth(format!("GitHub App token refresh request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub App token refresh returned {status}"
            )));
        }

        let data: RefreshTokenResponse = resp
            .json()
            .await
            .map_err(|e| {
                AuthError::OAuth(format!(
                    "Failed to parse GitHub App refresh response: {e}"
                ))
            })?;

        if let Some(err) = data.error {
            let desc = data.error_description.unwrap_or_default();
            return Err(AuthError::OAuth(format!(
                "GitHub App token refresh error: {err}: {desc}"
            )));
        }

        info!("Refreshed GitHub App user access token");

        Ok(OAuthTokens {
            access_token: data.access_token,
            token_type: data.token_type,
            scope: data.scope,
            id_token: None,
            refresh_token: data.refresh_token,
            expires_in: data.expires_in,
            refresh_token_expires_in: data.refresh_token_expires_in,
        })
    }
}
