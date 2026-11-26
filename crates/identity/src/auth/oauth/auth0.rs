//! Auth0 OAuth provider implementation.

use async_trait::async_trait;

use super::provider::OAuthProvider;
use crate::auth::error::AuthError;
use crate::auth::models::{OAuthTokens, ProviderUserInfo};
use moatless_core::config::Config;

pub struct Auth0OAuthProvider {
    domain: String,
    client_id: String,
    client_secret: String,
    callback_url: String,
    audience: Option<String>,
    http: reqwest::Client,
}

impl Auth0OAuthProvider {
    /// Create from config. Returns None if Auth0 is not configured.
    #[expect(
        clippy::disallowed_methods,
        reason = "sets the timeouts `Config::http` holds"
    )]
    pub fn from_config(config: &Config) -> Option<Self> {
        let domain = config.auth.auth0_domain.clone()?;
        let client_id = config.auth.auth0_client_id.clone()?;
        let client_secret = config.auth.auth0_client_secret.clone()?;
        let callback_url = config.auth.auth0_callback_url.clone()?;
        let http = reqwest::Client::builder()
            .connect_timeout(config.http.identity_provider.connect)
            .timeout(config.http.identity_provider.total)
            .build()
            .ok()?;
        Some(Self {
            domain,
            client_id,
            client_secret,
            callback_url,
            audience: config.auth.auth0_audience.clone(),
            http,
        })
    }
}

#[derive(serde::Deserialize)]
struct Auth0TokenResponse {
    access_token: String,
    id_token: Option<String>,
    token_type: String,
    scope: Option<String>,
    error: Option<String>,
}

#[derive(serde::Deserialize)]
struct Auth0UserInfo {
    sub: String,
    nickname: Option<String>,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
}

#[async_trait]
impl OAuthProvider for Auth0OAuthProvider {
    fn generate_auth_url(&self, state: &str) -> Option<String> {
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.callback_url),
            ("state", state),
            ("scope", "openid profile email"),
        ];

        // Audience requires owned string for the reference
        let audience_str;
        if let Some(ref audience) = self.audience {
            audience_str = audience.clone();
            params.push(("audience", &audience_str));
        }

        let query = serde_urlencoded::to_string(&params).ok()?;
        let domain = &self.domain;
        Some(format!("https://{domain}/authorize?{query}"))
    }

    async fn exchange_code_for_tokens(&self, code: &str) -> Result<OAuthTokens, AuthError> {
        let domain = &self.domain;
        let resp = self
            .http
            .post(format!("https://{domain}/oauth/token"))
            .json(&serde_json::json!({
                "grant_type": "authorization_code",
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "code": code,
                "redirect_uri": self.callback_url,
            }))
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("Auth0 token exchange failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "Auth0 token exchange returned {status}"
            )));
        }

        let data: Auth0TokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse Auth0 token response: {e}")))?;

        if let Some(err) = data.error {
            return Err(AuthError::OAuth(format!("Auth0 OAuth error: {err}")));
        }

        Ok(OAuthTokens {
            access_token: data.access_token,
            token_type: data.token_type,
            scope: data.scope,
            id_token: data.id_token,
            refresh_token: None,
            expires_in: None,
            refresh_token_expires_in: None,
        })
    }

    async fn get_user_info(&self, tokens: &OAuthTokens) -> Result<ProviderUserInfo, AuthError> {
        let domain = &self.domain;
        let resp = self
            .http
            .get(format!("https://{domain}/userinfo"))
            .header("Authorization", format!("Bearer {}", tokens.access_token))
            .send()
            .await
            .map_err(|e| AuthError::OAuth(format!("Auth0 user info request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AuthError::OAuth(format!(
                "Auth0 user info returned {status}"
            )));
        }

        let user: Auth0UserInfo = resp
            .json()
            .await
            .map_err(|e| AuthError::OAuth(format!("Failed to parse Auth0 user response: {e}")))?;

        let login = user
            .nickname
            .or(user.email.clone())
            .unwrap_or_else(|| user.sub.clone());

        Ok(ProviderUserInfo {
            id: user.sub,
            login,
            email: user.email,
            name: user.name,
            avatar_url: user.picture,
        })
    }

    fn generate_logout_url(&self, return_to: &str) -> Option<String> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("returnTo", return_to),
        ];
        let query = serde_urlencoded::to_string(params).ok()?;
        let domain = &self.domain;
        Some(format!("https://{domain}/v2/logout?{query}"))
    }
}
