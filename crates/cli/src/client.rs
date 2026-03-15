//! HTTP client wrapper with session cookie / API key auth.
//!
//! Wraps `moatless_api_client::ApiClient` with auth header injection.

use moatless_api_client::{ApiClient, ApiError};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, COOKIE};
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
    #[error("{0}")]
    Other(String),
}

impl From<ApiError> for ClientError {
    fn from(err: ApiError) -> Self {
        match err {
            ApiError::Http(e) => ClientError::Http(e),
            ApiError::Api { status, body } => {
                if status == 401 {
                    ClientError::Unauthorized
                } else {
                    ClientError::Api { status, body }
                }
            }
            ApiError::Unauthorized => ClientError::Unauthorized,
            ApiError::InvalidJson(msg) => ClientError::Api {
                status: 500,
                body: format!("Invalid response: {msg}"),
            },
        }
    }
}

pub struct MoatClient {
    base_url: String,
    auth_mode: ClientAuthMode,
    verbose: bool,
    /// Typed API client.
    typed_client: ApiClient,
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
    /// Create a client with no auth headers. Reads `MOATLESS_API_URL` from env.
    pub fn new_unauthenticated() -> Result<Self, ClientError> {
        Self::from_headers(HeaderMap::new())
    }

    /// Create a client that simulates a specific user via ForwardAuth dev-mode headers.
    pub fn new_as_user(user_id: &str, login: &str, role: &str) -> Result<Self, ClientError> {
        let headers = Self::user_headers(user_id, login, role)?;
        Self::from_headers(headers)
    }

    /// Create a client that simulates a user with team membership via ForwardAuth dev-mode headers.
    pub fn new_as_user_with_teams(
        user_id: &str,
        login: &str,
        role: &str,
        team_ids: &[&str],
    ) -> Result<Self, ClientError> {
        let mut headers = Self::user_headers(user_id, login, role)?;
        if !team_ids.is_empty() {
            headers.insert(
                "X-Team-Ids",
                HeaderValue::from_str(&team_ids.join(","))
                    .map_err(|e| ClientError::Api { status: 0, body: e.to_string() })?,
            );
        }
        Self::from_headers(headers)
    }

    fn user_headers(user_id: &str, login: &str, role: &str) -> Result<HeaderMap, ClientError> {
        let mut headers = HeaderMap::new();
        let map_err = |e: reqwest::header::InvalidHeaderValue| ClientError::Api {
            status: 0,
            body: e.to_string(),
        };
        headers.insert("X-User-Id", HeaderValue::from_str(user_id).map_err(map_err)?);
        headers.insert("X-User-Login", HeaderValue::from_str(login).map_err(map_err)?);
        headers.insert("X-User-Role", HeaderValue::from_str(role).map_err(map_err)?);
        Ok(headers)
    }

    fn from_headers(headers: HeaderMap) -> Result<Self, ClientError> {
        let base_url = std::env::var("MOATLESS_API_URL")
            .unwrap_or_else(|_| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_string();
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        let typed_client = ApiClient::new(&base_url, http);
        Ok(Self {
            base_url,
            auth_mode: ClientAuthMode::ApiKey(String::new()),
            verbose: false,
            typed_client,
        })
    }

    pub fn new(config: &Config, verbose: bool) -> Self {
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

        let base_url = config.api_url.trim_end_matches('/').to_string();
        let http = Self::build_http_client(&auth_mode);
        let typed_client = ApiClient::new(&base_url, http);

        Self {
            base_url,
            auth_mode,
            verbose,
            typed_client,
        }
    }

    /// Build a `reqwest::Client` with default auth headers baked in.
    fn build_http_client(auth_mode: &ClientAuthMode) -> reqwest::Client {
        let mut default_headers = HeaderMap::new();

        match auth_mode {
            ClientAuthMode::ApiKey(key) => {
                if !key.is_empty() {
                    let bearer = format!("Bearer {key}");
                    if let Ok(v) = HeaderValue::from_str(&bearer) {
                        default_headers.insert(AUTHORIZATION, v);
                    }
                }
            }
            ClientAuthMode::Sandbox {
                user_auth_token,
                cookie_name,
                session_token,
            } => {
                // Set session cookie for /api/v1/ routes
                if let Some(ref token) = session_token {
                    let cookie = format!("{cookie_name}={token}");
                    if let Ok(v) = HeaderValue::from_str(&cookie) {
                        default_headers.insert(COOKIE, v);
                    }
                }
                // Also set X-User-Token for sandbox routes
                if let Ok(v) = HeaderValue::from_str(user_auth_token) {
                    default_headers.insert("X-User-Token", v);
                }
            }
        }

        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .default_headers(default_headers)
            .build()
            .unwrap_or_default()
    }

    pub fn set_session_token(&mut self, token: String) {
        if let ClientAuthMode::Sandbox {
            ref mut session_token,
            ..
        } = self.auth_mode
        {
            *session_token = Some(token);
            // Rebuild HTTP client and typed client with new session cookie.
            let http = Self::build_http_client(&self.auth_mode);
            self.typed_client = ApiClient::new(&self.base_url, http);
        }
    }

    /// Access the typed API client.
    pub fn api(&self) -> &ApiClient {
        &self.typed_client
    }

    /// Return the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Raw GET for the auth probe in `ensure_auth`.
    pub async fn raw_get(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("GET {url}");
        }
        let http = Self::build_http_client(&self.auth_mode);
        let resp = http.get(&url).send().await?;
        self.handle_response(resp).await
    }

    /// GET an arbitrary URL (not relative to base) with the same auth headers.
    /// Returns the response body as text. Only fails on auth errors, not app-level errors.
    pub async fn raw_get_url(&self, url: &str) -> Result<String, ClientError> {
        if self.verbose {
            eprintln!("GET {url}");
        }
        let http = Self::build_http_client(&self.auth_mode);
        let resp = http.get(url).send().await?;
        let status = resp.status().as_u16();
        if status == 401 {
            return Err(ClientError::Unauthorized);
        }
        if status == 403 {
            return Err(ClientError::Api {
                status,
                body: "Access denied".to_string(),
            });
        }
        Ok(resp.text().await?)
    }

    /// Raw PUT with JSON body.
    pub async fn raw_put(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("PUT {url}");
        }
        let http = Self::build_http_client(&self.auth_mode);
        let resp = http.put(&url).json(body).send().await?;
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

    /// Start a streaming GET request (no timeout, returns raw response for SSE/NDJSON).
    pub async fn raw_get_stream(&self, path: &str) -> Result<reqwest::Response, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("GET (stream) {url}");
        }
        // Build a client with no timeout for long-lived SSE connections.
        let http = Self::build_streaming_http_client(&self.auth_mode);
        let resp = http.get(&url).send().await?;
        let status = resp.status().as_u16();
        if status == 401 {
            return Err(ClientError::Unauthorized);
        }
        if status >= 400 {
            let body = resp.text().await?;
            return Err(ClientError::Api { status, body });
        }
        Ok(resp)
    }

    /// Build a `reqwest::Client` with no timeout (for SSE streaming).
    fn build_streaming_http_client(auth_mode: &ClientAuthMode) -> reqwest::Client {
        let mut default_headers = HeaderMap::new();
        match auth_mode {
            ClientAuthMode::ApiKey(key) => {
                if !key.is_empty() {
                    let bearer = format!("Bearer {key}");
                    if let Ok(v) = HeaderValue::from_str(&bearer) {
                        default_headers.insert(AUTHORIZATION, v);
                    }
                }
            }
            ClientAuthMode::Sandbox {
                user_auth_token,
                cookie_name,
                session_token,
            } => {
                if let Some(ref token) = session_token {
                    let cookie = format!("{cookie_name}={token}");
                    if let Ok(v) = HeaderValue::from_str(&cookie) {
                        default_headers.insert(COOKIE, v);
                    }
                }
                if let Ok(v) = HeaderValue::from_str(user_auth_token) {
                    default_headers.insert("X-User-Token", v);
                }
            }
        }
        reqwest::Client::builder()
            .default_headers(default_headers)
            .build()
            .unwrap_or_default()
    }
}
