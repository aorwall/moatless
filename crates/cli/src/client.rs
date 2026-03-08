//! HTTP client wrapper with session cookie / API key auth.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, COOKIE};
use serde::Serialize;
use thiserror::Error;

use crate::config::{AuthMode, Config};

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error ({status}): {body}")]
    Api { status: u16, body: String },
    #[error("Not authenticated — run `moat login`")]
    Unauthorized,
    #[error("Not logged in — run `moat login` to authenticate")]
    NotLoggedIn,
}

pub struct MoatClient {
    http: reqwest::Client,
    base_url: String,
    auth_mode: ClientAuthMode,
    verbose: bool,
}

/// Internal auth mode for the client.
enum ClientAuthMode {
    /// Standalone: use `Authorization: Bearer mvk_...` on all requests.
    ApiKey(String),
    /// Sandbox: use X-User-Token for sandbox routes, session cookie for others.
    Sandbox {
        user_auth_token: String,
        cookie_name: String,
        session_token: Option<String>,
    },
}

impl MoatClient {
    pub fn new(config: &Config, verbose: bool) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        let auth_mode = match &config.auth_mode {
            AuthMode::Standalone { api_key } => {
                ClientAuthMode::ApiKey(api_key.clone().unwrap_or_default())
            }
            AuthMode::Sandbox {
                user_auth_token, ..
            } => ClientAuthMode::Sandbox {
                user_auth_token: user_auth_token.clone(),
                cookie_name: config.cookie_name.clone(),
                session_token: None,
            },
        };

        Self {
            http,
            base_url: config.api_url.trim_end_matches('/').to_string(),
            auth_mode,
            verbose,
        }
    }

    pub fn set_session_token(&mut self, token: String) {
        if let ClientAuthMode::Sandbox {
            ref mut session_token,
            ..
        } = self.auth_mode
        {
            *session_token = Some(token);
        }
    }

    fn build_headers(&self, path: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();

        match &self.auth_mode {
            ClientAuthMode::ApiKey(key) => {
                if !key.is_empty() {
                    let bearer = format!("Bearer {key}");
                    if let Ok(v) = HeaderValue::from_str(&bearer) {
                        headers.insert(AUTHORIZATION, v);
                    }
                }
            }
            ClientAuthMode::Sandbox {
                user_auth_token,
                cookie_name,
                session_token,
            } => {
                // Sandbox routes use X-User-Token directly
                if path.starts_with("/api/sandbox/") {
                    if let Ok(v) = HeaderValue::from_str(user_auth_token) {
                        headers.insert("X-User-Token", v);
                    }
                } else if let Some(ref token) = session_token {
                    // Regular API routes use session cookie
                    let cookie = format!("{cookie_name}={token}");
                    if let Ok(v) = HeaderValue::from_str(&cookie) {
                        headers.insert(COOKIE, v);
                    }
                }
            }
        }

        headers
    }

    pub async fn get(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("GET {url}");
        }
        let resp = self
            .http
            .get(&url)
            .headers(self.build_headers(path))
            .send()
            .await?;
        self.handle_response(resp).await
    }

    pub async fn post(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("POST {url}");
        }
        let resp = self
            .http
            .post(&url)
            .headers(self.build_headers(path))
            .json(body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    pub async fn post_empty(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("POST {url}");
        }
        let resp = self
            .http
            .post(&url)
            .headers(self.build_headers(path))
            .send()
            .await?;
        self.handle_response(resp).await
    }

    pub async fn put(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("PUT {url}");
        }
        let resp = self
            .http
            .put(&url)
            .headers(self.build_headers(path))
            .json(body)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    /// Send a raw POST with custom headers (for token exchange).
    pub async fn post_with_headers(
        &self,
        path: &str,
        headers: HeaderMap,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("POST {url}");
        }
        let resp = self.http.post(&url).headers(headers).send().await?;
        self.handle_response(resp).await
    }

    async fn handle_response(
        &self,
        resp: reqwest::Response,
    ) -> Result<serde_json::Value, ClientError> {
        let status = resp.status().as_u16();

        if status == 401 {
            return Err(ClientError::Unauthorized);
        }

        let body = resp.text().await?;

        if status >= 400 {
            return Err(ClientError::Api { status, body });
        }

        // 204 No Content
        if body.is_empty() {
            return Ok(serde_json::json!({"status": "ok"}));
        }

        serde_json::from_str(&body).map_err(|_| ClientError::Api {
            status,
            body: format!("Invalid JSON: {body}"),
        })
    }
}
