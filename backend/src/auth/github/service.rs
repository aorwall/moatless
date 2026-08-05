//! GitHub App server-side authentication service.
//!
//! Provides three capabilities:
//! 1. **Installation tokens** — server-to-server API calls as the App itself.
//! 2. **User token refresh** — refresh expired GitHub App user access tokens.
//! 3. **Bot identity discovery** — resolve `{slug}[bot]` login and noreply email
//!    for use as git author / committer when running "act-as-App" mode.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use jsonwebtoken::{EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::adapter_apps::dao::GitHubAppRecord;
use crate::auth::error::AuthError;
use crate::auth::models::OAuthTokens;
use crate::config::{Config, GitHubAppConfig};

/// Cached installation access token.
struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

/// Resolved bot identity for a configured GitHub App.
///
/// Populated lazily on first use via `GET /app` and `GET /users/{slug}[bot]`.
#[derive(Clone, Debug)]
pub struct BotIdentity {
    /// `{app-slug}[bot]` — what git/GitHub display as the author.
    pub login: String,
    /// Numeric bot user id (different from `app_id`).
    pub user_id: u64,
    /// `{user_id}+{slug}[bot]@users.noreply.github.com`.
    pub email: String,
}

/// GitHub App service for installation tokens and user token refresh.
pub struct GitHubAppService {
    /// Stable key used by loop config to select this App (e.g. `"dev-bot"`).
    key: String,
    app_id: u64,
    private_key: EncodingKey,
    client_id: Option<String>,
    client_secret: Option<String>,
    default_installation_id: Option<u64>,
    api_base: String,
    http: reqwest::Client,
    token_cache: RwLock<HashMap<u64, CachedToken>>,
    repository_token_cache: RwLock<HashMap<(u64, String), CachedToken>>,
    /// Cached `installation_id` per `owner` (org or user login).
    installation_by_owner: RwLock<HashMap<String, u64>>,
    /// Cached bot identity (populated lazily).
    bot_identity: RwLock<Option<BotIdentity>>,
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
struct InstallationRepositoriesResponse {
    repositories: Vec<InstallationRepository>,
}

#[derive(Deserialize)]
struct InstallationRepository {
    id: u64,
    name: String,
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

#[derive(Clone, Debug, Deserialize)]
pub struct Installation {
    pub id: u64,
    pub account: InstallationAccount,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InstallationAccount {
    pub login: String,
    #[serde(rename = "type")]
    pub account_type: Option<String>,
}

#[derive(Clone, Deserialize)]
struct AppMetadata {
    slug: String,
    #[serde(default)]
    name: Option<String>,
    /// GitHub's numeric id for the App the signing key actually belongs to.
    /// Absent from older recorded fixtures, which is why it is optional.
    #[serde(default)]
    id: Option<u64>,
}

/// What GitHub confirmed about an App when its signing key was proved.
#[derive(Clone, Debug)]
pub struct ProvedApp {
    pub app_id: u64,
    pub name: String,
    pub slug: String,
    pub bot_login: String,
}

#[derive(Deserialize)]
struct GitHubUserMetadata {
    id: u64,
}

#[derive(Deserialize)]
struct OwnerInstallation {
    id: u64,
}

impl GitHubAppService {
    /// Default registry key used by the implicit single-App config (the
    /// `GITHUB_APP_*` env vars). Code paths that operate on "the user-auth App"
    /// resolve via `GitHubAppRegistry::default_service()` which targets this key.
    pub const DEFAULT_KEY: &'static str = "default";

    /// Create from the legacy single-App env-var config. Returns `None` if
    /// `github_app_id` or `github_app_private_key` is not set.
    pub fn from_config(config: &Config) -> Option<Self> {
        let app_id = config.auth.github_app_id?;
        let pem = config.auth.github_app_private_key.as_ref()?;

        Self::build(
            Self::DEFAULT_KEY.to_string(),
            app_id,
            pem,
            config.auth.github_app_client_id.clone(),
            config.auth.github_app_client_secret.clone(),
            config.auth.github_app_default_installation_id,
        )
    }

    /// Create from a structured `GitHubAppConfig` entry. Returns `None` if the
    /// PEM private key fails to parse — the caller logs and continues with the
    /// remaining apps.
    pub fn from_app_config(entry: &GitHubAppConfig) -> Option<Self> {
        Self::build(
            entry.key.clone(),
            entry.app_id,
            &entry.private_key,
            entry.client_id.clone(),
            entry.client_secret.clone(),
            entry.default_installation_id,
        )
    }

    /// Create from a stored App registration, whose signing key has already
    /// been decrypted. A bot App carries no OAuth client credentials: it never
    /// authorizes a person, so it has nothing to authorize them with.
    ///
    /// Returns `None` only when the stored key no longer parses, which means
    /// the record was written by a version that did not prove keys.
    pub fn from_record(record: &GitHubAppRecord, private_key_pem: &str) -> Option<Self> {
        Self::build(
            record.app_key.clone(),
            record.app_id,
            private_key_pem,
            None,
            None,
            record.default_installation_id,
        )
    }

    fn build(
        key: String,
        app_id: u64,
        pem: &str,
        client_id: Option<String>,
        client_secret: Option<String>,
        default_installation_id: Option<u64>,
    ) -> Option<Self> {
        let private_key = match EncodingKey::from_rsa_pem(pem.as_bytes()) {
            Ok(k) => k,
            Err(e) => {
                warn!(app_key = %key, error = %e, "Failed to parse GitHub App private key PEM");
                return None;
            }
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?;

        info!(app_key = %key, app_id, "GitHub App service configured");

        Some(Self {
            key,
            app_id,
            private_key,
            client_id,
            client_secret,
            default_installation_id,
            api_base: crate::config::api_base(),
            http,
            token_cache: RwLock::new(HashMap::new()),
            repository_token_cache: RwLock::new(HashMap::new()),
            installation_by_owner: RwLock::new(HashMap::new()),
            bot_identity: RwLock::new(None),
        })
    }

    /// Stable registry key for this App.
    pub fn key(&self) -> &str {
        &self.key
    }

    #[cfg(test)]
    fn with_api_base(mut self, api_base: String) -> Self {
        self.api_base = api_base.trim().trim_end_matches('/').to_string();
        self
    }

    /// Generate a short-lived JWT for authenticating as the GitHub App.
    fn generate_app_jwt(&self) -> Result<String, AuthError> {
        let now = Utc::now().timestamp();
        let claims = AppJwtClaims {
            iss: self.app_id.to_string(),
            iat: now - 60,  // clock skew buffer
            exp: now + 600, // 10 minutes max per GitHub docs
        };

        let header = Header::new(jsonwebtoken::Algorithm::RS256);
        encode(&header, &claims, &self.private_key)
            .map_err(|e| AuthError::OAuth(format!("Failed to generate GitHub App JWT: {e}")))
    }

    /// Get an installation access token, using cache when available.
    pub async fn get_installation_token(&self, installation_id: u64) -> Result<String, AuthError> {
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
                "{}/app/installations/{installation_id}/access_tokens",
                self.api_base
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

        let data: InstallationTokenResponse = resp.json().await.map_err(|e| {
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

    /// Mint an installation token restricted by GitHub to one repository.
    pub async fn get_repository_installation_token(
        &self,
        installation_id: u64,
        owner: &str,
        repository: &str,
    ) -> Result<String, AuthError> {
        let cache_key = (installation_id, repository.to_ascii_lowercase());
        {
            let cache = self.repository_token_cache.read().await;
            if let Some(cached) = cache.get(&cache_key) {
                let margin = Utc::now() + chrono::Duration::minutes(5);
                if cached.expires_at > margin {
                    debug!(
                        installation_id,
                        repository, "Using cached repository-restricted installation token"
                    );
                    return Ok(cached.token.clone());
                }
            }
        }

        let app_jwt = self.generate_app_jwt()?;
        let resp = self
            .http
            .post(format!(
                "{}/app/installations/{installation_id}/access_tokens",
                self.api_base
            ))
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .json(&serde_json::json!({ "repositories": [repository] }))
            .send()
            .await
            .map_err(|e| {
                AuthError::OAuth(format!(
                    "GitHub repository installation token request failed: {e}"
                ))
            })?;

        let data = if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            let body = resp.text().await.unwrap_or_default();
            debug!(
                installation_id,
                owner,
                repository,
                github_response = %body,
                "GitHub rejected repository-name restricted token; retrying with repository id"
            );
            let repository_id = self
                .find_installation_repository_id(installation_id, owner, repository)
                .await?;
            self.get_repository_installation_token_by_id(installation_id, repository_id)
                .await?
        } else {
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AuthError::OAuth(format!(
                    "GitHub repository installation token request returned {status}: {body}"
                )));
            }

            resp.json().await.map_err(|e| {
                AuthError::OAuth(format!(
                    "Failed to parse GitHub repository installation token response: {e}"
                ))
            })?
        };
        let expires_at = data
            .expires_at
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now() + chrono::Duration::hours(1));
        self.repository_token_cache.write().await.insert(
            cache_key,
            CachedToken {
                token: data.token.clone(),
                expires_at,
            },
        );

        info!(
            installation_id,
            repository, "Obtained repository-restricted GitHub installation token"
        );
        Ok(data.token)
    }

    async fn get_repository_installation_token_by_id(
        &self,
        installation_id: u64,
        repository_id: u64,
    ) -> Result<InstallationTokenResponse, AuthError> {
        let app_jwt = self.generate_app_jwt()?;
        let resp = self
            .http
            .post(format!(
                "{}/app/installations/{installation_id}/access_tokens",
                self.api_base
            ))
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .json(&serde_json::json!({ "repository_ids": [repository_id] }))
            .send()
            .await
            .map_err(|e| {
                AuthError::OAuth(format!(
                    "GitHub repository-id installation token request failed: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::OAuth(format!(
                "GitHub repository-id installation token request returned {status}: {body}"
            )));
        }

        resp.json().await.map_err(|e| {
            AuthError::OAuth(format!(
                "Failed to parse GitHub repository-id installation token response: {e}"
            ))
        })
    }

    async fn find_installation_repository_id(
        &self,
        installation_id: u64,
        owner: &str,
        repository: &str,
    ) -> Result<u64, AuthError> {
        let token = self.get_installation_token(installation_id).await?;
        let mut page = 1u32;
        loop {
            let resp = self
                .http
                .get(format!(
                    "{}/installation/repositories?per_page=100&page={page}",
                    self.api_base
                ))
                .header("Authorization", format!("Bearer {token}"))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "moatless-vibe-backend")
                .send()
                .await
                .map_err(|e| {
                    AuthError::OAuth(format!(
                        "GitHub installation repositories request failed: {e}"
                    ))
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AuthError::OAuth(format!(
                    "GitHub installation repositories request returned {status}: {body}"
                )));
            }

            let has_next_page = has_next_page(resp.headers());
            let data: InstallationRepositoriesResponse = resp.json().await.map_err(|e| {
                AuthError::OAuth(format!(
                    "Failed to parse GitHub installation repositories response: {e}"
                ))
            })?;
            if let Some(repository) = data
                .repositories
                .into_iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(repository))
            {
                return Ok(repository.id);
            }
            if !has_next_page {
                break;
            }
            page += 1;
        }

        Err(AuthError::Config(format!(
            "GitHub App installation {installation_id} for account '{owner}' does not have access \
             to repository '{repository}'. Update the App installation on GitHub to include that \
             repository, or bind a different installation."
        )))
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
            .get(format!("{}/app/installations", self.api_base))
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

    /// Verify that an installation belongs to this App and return its account.
    pub async fn get_installation(&self, installation_id: u64) -> Result<Installation, AuthError> {
        let app_jwt = self.generate_app_jwt()?;
        let resp = self
            .http
            .get(format!(
                "{}/app/installations/{installation_id}",
                self.api_base
            ))
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("GitHub installation lookup failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub installation lookup returned {status}"
            )));
        }
        resp.json().await.map_err(|e| {
            AuthError::OAuth(format!("Failed to parse GitHub installation response: {e}"))
        })
    }

    /// Prove that this signing key works, and that it belongs to the App the
    /// admin named.
    ///
    /// Run before an App is stored and before a rotated key replaces the one
    /// in place, so a stored App is always one that has already worked. The
    /// error messages are the ones an admin reads, so they name which part
    /// failed, say what to do about it, and never suggest something was saved.
    pub async fn prove(&self) -> Result<ProvedApp, AuthError> {
        let app_jwt = self.generate_app_jwt()?;
        let resp = self
            .http
            .get(format!("{}/app", self.api_base))
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .send()
            .await
            .map_err(|e| {
                AuthError::BadRequest(format!(
                    "GitHub could not be reached, so the App was not stored: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(refusal(status, &body, self.app_id));
        }

        let metadata: AppMetadata = resp.json().await.map_err(|e| {
            AuthError::BadRequest(format!("GitHub's response could not be read: {e}"))
        })?;

        // A key that signs successfully still proves nothing about *which*
        // App it is, and storing it under the wrong id produces a bot that
        // authenticates as somebody else.
        if let Some(actual) = metadata.id {
            if actual != self.app_id {
                return Err(AuthError::BadRequest(format!(
                    "This key belongs to a different App — '{}' (id {actual}), not id {}. \
                     Nothing was stored.",
                    metadata.name.as_deref().unwrap_or(&metadata.slug),
                    self.app_id
                )));
            }
        }

        Ok(ProvedApp {
            app_id: self.app_id,
            name: metadata
                .name
                .clone()
                .unwrap_or_else(|| metadata.slug.clone()),
            bot_login: format!("{}[bot]", metadata.slug),
            slug: metadata.slug,
        })
    }

    /// Resolve display metadata and the App installation link.
    pub async fn app_listing(&self) -> Result<(String, String), AuthError> {
        let app_jwt = self.generate_app_jwt()?;
        let resp = self
            .http
            .get(format!("{}/app", self.api_base))
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("GitHub /app lookup failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub /app returned {status} for app {}",
                self.key
            )));
        }
        let metadata: AppMetadata = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse /app response: {e}")))?;
        let name = metadata.name.unwrap_or_else(|| metadata.slug.clone());
        let install_url = format!(
            "https://github.com/apps/{}/installations/new",
            metadata.slug
        );
        Ok((name, install_url))
    }

    /// Refresh a GitHub App user access token using a refresh token.
    pub async fn refresh_user_token(&self, refresh_token: &str) -> Result<OAuthTokens, AuthError> {
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

        let data: RefreshTokenResponse = resp.json().await.map_err(|e| {
            AuthError::OAuth(format!("Failed to parse GitHub App refresh response: {e}"))
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

    /// Resolve the installation id for an `owner` (org or user), cached.
    ///
    /// Calls `GET /orgs/{owner}/installation` first; falls back to
    /// `GET /users/{owner}/installation` for user-owned repos. Returns
    /// `Config` error if the App isn't installed on that owner.
    pub async fn resolve_installation_id(&self, owner: &str) -> Result<u64, AuthError> {
        {
            let cache = self.installation_by_owner.read().await;
            if let Some(id) = cache.get(owner) {
                return Ok(*id);
            }
        }

        if let Some(id) = self.default_installation_id {
            // When a default is configured we still cache it per-owner so the
            // lookup path is uniform.
            self.installation_by_owner
                .write()
                .await
                .insert(owner.to_string(), id);
            return Ok(id);
        }

        let app_jwt = self.generate_app_jwt()?;

        for endpoint in [
            format!("{}/orgs/{owner}/installation", self.api_base),
            format!("{}/users/{owner}/installation", self.api_base),
        ] {
            let resp = self
                .http
                .get(&endpoint)
                .header("Authorization", format!("Bearer {app_jwt}"))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "moatless-vibe-backend")
                .send()
                .await
                .map_err(|e| AuthError::OAuth(format!("GitHub installation lookup failed: {e}")))?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                continue;
            }
            if !resp.status().is_success() {
                let status = resp.status();
                return Err(AuthError::OAuth(format!(
                    "GitHub installation lookup returned {status} for {owner}"
                )));
            }
            let parsed: OwnerInstallation = resp.json().await.map_err(|e| {
                AuthError::OAuth(format!("Failed to parse installation lookup: {e}"))
            })?;
            self.installation_by_owner
                .write()
                .await
                .insert(owner.to_string(), parsed.id);
            return Ok(parsed.id);
        }

        Err(AuthError::Config(format!(
            "GitHub App {} is not installed on owner {owner}",
            self.key
        )))
    }

    /// Convenience: resolve installation id for `owner` then mint a token.
    pub async fn installation_token_for_owner(&self, owner: &str) -> Result<String, AuthError> {
        let id = self.resolve_installation_id(owner).await?;
        self.get_installation_token(id).await
    }

    /// Resolve the bot identity for this App (cached).
    ///
    /// Calls `GET /app` (authenticated as the App JWT) to fetch the slug, then
    /// `GET /users/{slug}[bot]` (public) to fetch the bot user's numeric id.
    /// The committer/author email follows GitHub's noreply convention:
    /// `{user_id}+{slug}[bot]@users.noreply.github.com`.
    pub async fn get_bot_identity(&self) -> Result<BotIdentity, AuthError> {
        {
            let cached = self.bot_identity.read().await;
            if let Some(id) = cached.as_ref() {
                return Ok(id.clone());
            }
        }

        let app_jwt = self.generate_app_jwt()?;
        let meta_resp = self
            .http
            .get(format!("{}/app", self.api_base))
            .header("Authorization", format!("Bearer {app_jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("GitHub /app lookup failed: {e}")))?;
        if !meta_resp.status().is_success() {
            let status = meta_resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub /app returned {status} for app {}",
                self.key
            )));
        }
        let meta: AppMetadata = meta_resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse /app response: {e}")))?;

        let bot_login = format!("{}[bot]", meta.slug);
        let user_resp = self
            .http
            .get(format!("{}/users/{bot_login}", self.api_base))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "moatless-vibe-backend")
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("GitHub /users/{bot_login} failed: {e}")))?;
        if !user_resp.status().is_success() {
            let status = user_resp.status();
            return Err(AuthError::OAuth(format!(
                "GitHub /users/{bot_login} returned {status}"
            )));
        }
        let user: GitHubUserMetadata = user_resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse /users response: {e}")))?;

        let identity = BotIdentity {
            login: bot_login.clone(),
            user_id: user.id,
            email: format!("{}+{}@users.noreply.github.com", user.id, bot_login),
        };
        info!(
            app_key = %self.key,
            login = %identity.login,
            user_id = identity.user_id,
            "Resolved GitHub App bot identity"
        );
        *self.bot_identity.write().await = Some(identity.clone());
        Ok(identity)
    }
}

fn has_next_page(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(reqwest::header::LINK)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|link| link.split(';').any(|part| part.trim() == r#"rel="next""#))
        })
}

/// Turn GitHub's refusal of `GET /app` into something an admin can act on.
///
/// The status alone is not enough to act on, and 404 in particular reads as a
/// broken deployment when it is nothing of the sort: GitHub answers `404
/// Integration not found` when the JWT is signed correctly but its issuer is
/// not an App id. That is what an admin sees after pasting the Installation ID
/// — the long number in the installation URL — into the App ID field, which is
/// the easiest of the three numbers on those pages to reach for.
fn refusal(status: reqwest::StatusCode, body: &str, app_id: u64) -> AuthError {
    let github_said = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message")?.as_str().map(str::to_string));

    AuthError::BadRequest(match status {
        // GitHub's own words are kept: if it ever answers 404 for some other
        // reason, the advice below would otherwise be the only thing an admin
        // sees, and it would be wrong.
        reqwest::StatusCode::NOT_FOUND => format!(
            "GitHub has no App with ID {app_id} (it answered '{}'). That number is the \
             App ID on the App's own settings page — not the Installation ID from the \
             installation URL, and not the client ID. Nothing was stored.",
            github_said.as_deref().unwrap_or("Not Found")
        ),
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => format!(
            "GitHub did not accept this key for App {app_id}. Check the key was generated \
             by this App and pasted whole, including the BEGIN and END lines. Nothing was \
             stored."
        ),
        _ => {
            let detail = github_said
                .as_deref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default();
            format!("GitHub refused the request with {status}{detail}, so the App was not stored.")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::Json;
    use axum::body::Bytes;
    use axum::extract::{Query, State};
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use tokio::sync::Mutex;

    /// GitHub's own words for the case this was written for.
    const INTEGRATION_NOT_FOUND: &str = r#"{"message":"Integration not found","status":"404"}"#;
    const TEST_PEM: &str = include_str!("testdata/github_app_test_key.pem");

    fn test_app() -> GitHubAppService {
        let config = GitHubAppConfig {
            key: "review-bot".to_string(),
            app_id: 42,
            private_key: TEST_PEM.to_string(),
            client_id: None,
            client_secret: None,
            default_installation_id: None,
        };
        GitHubAppService::from_app_config(&config).expect("test key parses")
    }

    fn token_response(token: &str) -> serde_json::Value {
        serde_json::json!({
            "token": token,
            "expires_at": "2099-01-01T00:00:00Z",
        })
    }

    async fn github_token_stub(
        repositories: Vec<serde_json::Value>,
    ) -> (SocketAddr, Arc<Mutex<Vec<serde_json::Value>>>) {
        github_token_stub_pages(vec![repositories]).await
    }

    async fn github_token_stub_pages(
        pages: Vec<Vec<serde_json::Value>>,
    ) -> (SocketAddr, Arc<Mutex<Vec<serde_json::Value>>>) {
        let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let pages = Arc::new(pages);
        let app = axum::Router::new()
            .route(
                "/app/installations/42/access_tokens",
                post(
                    |State(requests): State<Arc<Mutex<Vec<serde_json::Value>>>>,
                     body: Bytes| async move {
                        let request = if body.is_empty() {
                            serde_json::Value::Null
                        } else {
                            serde_json::from_slice(&body).expect("request body is json")
                        };
                        requests.lock().await.push(request.clone());

                        if request.get("repositories").is_some() {
                            return (
                                StatusCode::UNPROCESSABLE_ENTITY,
                                Json(serde_json::json!({
                                    "message": "There is at least one repository that does not exist or is not accessible to the parent installation.",
                                    "status": "422",
                                })),
                            );
                        }
                        if request.get("repository_ids").is_some() {
                            return (StatusCode::CREATED, Json(token_response("repo-id-token")));
                        }
                        (
                            StatusCode::CREATED,
                            Json(token_response("installation-token")),
                        )
                    },
                ),
            )
            .route(
                "/installation/repositories",
                get({
                    let pages = Arc::clone(&pages);
                    move |Query(query): Query<HashMap<String, String>>| {
                        let pages = Arc::clone(&pages);
                        async move {
                            let page = query
                                .get("page")
                                .and_then(|value| value.parse::<usize>().ok())
                                .unwrap_or(1);
                            let repositories = pages
                                .get(page.saturating_sub(1))
                                .cloned()
                                .unwrap_or_default();
                            let body = serde_json::json!({ "repositories": repositories });
                            if page < pages.len() {
                                (
                                    [(
                                        header::LINK,
                                        format!(
                                            "<http://github.test/installation/repositories?page={}>; rel=\"next\"",
                                            page + 1
                                        ),
                                    )],
                                    Json(body),
                                )
                                    .into_response()
                            } else {
                                Json(body).into_response()
                            }
                        }
                    }
                }),
            )
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub");
        let addr = listener.local_addr().expect("stub addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve stub");
        });
        (addr, requests)
    }

    #[test]
    fn a_404_says_which_number_is_wanted_rather_than_repeating_the_status() {
        let error = refusal(
            reqwest::StatusCode::NOT_FOUND,
            INTEGRATION_NOT_FOUND,
            150_038_583,
        );

        let message = error.to_string();
        assert!(message.contains("150038583"), "{message}");
        assert!(message.contains("Installation ID"), "{message}");
        assert!(message.contains("Integration not found"), "{message}");
        assert!(message.contains("Nothing was stored"), "{message}");
    }

    #[test]
    fn a_rejected_key_is_told_apart_from_an_app_that_does_not_exist() {
        let message = refusal(
            reqwest::StatusCode::UNAUTHORIZED,
            r#"{"message":"A JSON web token could not be decoded"}"#,
            42,
        )
        .to_string();

        assert!(message.contains("did not accept this key"), "{message}");
        assert!(!message.contains("Installation ID"), "{message}");
    }

    /// Anything else is unclassified, so GitHub's own message is the most
    /// useful thing we have and is passed through rather than swallowed.
    #[test]
    fn an_unclassified_refusal_carries_githubs_own_message() {
        let message = refusal(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            r#"{"message":"Service unavailable"}"#,
            42,
        )
        .to_string();

        assert!(message.contains("503"), "{message}");
        assert!(message.contains("Service unavailable"), "{message}");
    }

    #[test]
    fn a_body_that_is_not_json_still_produces_a_usable_message() {
        let message = refusal(
            reqwest::StatusCode::BAD_GATEWAY,
            "<html>upstream is having a day</html>",
            42,
        )
        .to_string();

        assert!(message.contains("502"), "{message}");
        assert!(!message.contains("<html>"), "{message}");
    }

    #[tokio::test]
    async fn repository_token_retries_422_with_repository_id() {
        let (addr, requests) = github_token_stub(vec![serde_json::json!({
            "id": 987,
            "name": "Repo",
        })])
        .await;

        let token = test_app()
            .with_api_base(format!("http://{addr}"))
            .get_repository_installation_token(42, "Acme", "repo")
            .await
            .expect("repository-id retry succeeds");

        assert_eq!(token, "repo-id-token");
        let requests = requests.lock().await;
        assert_eq!(requests.len(), 3, "name request, unscoped token, id retry");
        assert_eq!(requests[0], serde_json::json!({ "repositories": ["repo"] }));
        assert_eq!(requests[1], serde_json::Value::Null);
        assert_eq!(requests[2], serde_json::json!({ "repository_ids": [987] }));
    }

    #[tokio::test]
    async fn repository_token_follows_github_pagination_to_late_pages() {
        let mut pages: Vec<Vec<serde_json::Value>> = (0..20)
            .map(|index| {
                vec![serde_json::json!({
                    "id": index,
                    "name": format!("other-repo-{index}"),
                })]
            })
            .collect();
        pages.push(vec![serde_json::json!({
            "id": 987,
            "name": "repo",
        })]);
        let (addr, _requests) = github_token_stub_pages(pages).await;

        let token = test_app()
            .with_api_base(format!("http://{addr}"))
            .get_repository_installation_token(42, "Acme", "repo")
            .await
            .expect("late-page repository-id retry succeeds");

        assert_eq!(token, "repo-id-token");
    }

    #[tokio::test]
    async fn repository_token_names_installation_access_gap() {
        let (addr, _requests) = github_token_stub(vec![serde_json::json!({
            "id": 123,
            "name": "other-repo",
        })])
        .await;

        let error = test_app()
            .with_api_base(format!("http://{addr}"))
            .get_repository_installation_token(42, "Acme", "repo")
            .await
            .expect_err("missing selected repository is an access error");

        let message = error.to_string();
        assert!(
            message.contains("does not have access to repository 'repo'"),
            "{message}"
        );
        assert!(message.contains("installation 42"), "{message}");
        assert!(message.contains("account 'Acme'"), "{message}");
    }
}
