//! HTTP client wrapper with session cookie / API key auth.
//!
//! Wraps `moatless_api_client::ApiClient` with auth header injection.

use moatless_api_client::{ApiClient, ApiError};
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderValue};
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
    /// The request succeeded but the body didn't decode — almost always version
    /// skew, since a long-lived sandbox's baked-in `moat` reads a backend that
    /// has moved on.
    ///
    /// Kept separate from [`ClientError::Api`] so the reader isn't sent chasing
    /// a server fault that never happened.
    #[error(
        "Could not read the response: {0}\n\
         The server answered, so this is most likely a `moat` older than the backend — \
         a sandbox uses the CLI baked into its image. `moat --version` names the commit this one \
         was built from; restart the sandbox to pick up a newer one."
    )]
    UndecodableResponse(String),
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
            ApiError::InvalidJson(msg) => ClientError::UndecodableResponse(msg),
        }
    }
}

pub struct MoatClient {
    base_url: String,
    auth_mode: ClientAuthMode,
    verbose: bool,
    /// How long to wait for a response before giving up.
    http_timeout: std::time::Duration,
    /// Typed API client.
    typed_client: ApiClient,
}

/// Long enough for a person at a terminal, short enough to notice a hang.
/// Endpoints that provision a sandbox before answering need `--timeout`.
const DEFAULT_HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A response whose status is part of the answer, not an error to raise.
pub struct RawResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

/// Internal auth mode for the client.
enum ClientAuthMode {
    /// Standalone: use `Authorization: Bearer mvk_...` on all requests.
    ApiKey(String),
    /// Session cookie, as a browser would send it. Raw `name=value` pair.
    SessionCookie(String),
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

    /// Create a client authenticated with an `mvk_` API key against `base_url`.
    pub fn new_with_api_key(base_url: &str, api_key: &str) -> Self {
        let auth_mode = ClientAuthMode::ApiKey(api_key.to_string());
        Self::from_auth_mode(base_url, auth_mode)
    }

    /// Create a client authenticated with a session cookie against `base_url`.
    ///
    /// `cookie` is the raw `name=value` pair from a `Set-Cookie` header.
    pub fn new_with_session_cookie(base_url: &str, cookie: &str) -> Self {
        let auth_mode = ClientAuthMode::SessionCookie(cookie.to_string());
        Self::from_auth_mode(base_url, auth_mode)
    }

    fn from_auth_mode(base_url: &str, auth_mode: ClientAuthMode) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let http = Self::build_http_client(&auth_mode, DEFAULT_HTTP_TIMEOUT);
        let typed_client = ApiClient::new(&base_url, http);
        Self {
            base_url,
            auth_mode,
            verbose: false,
            http_timeout: DEFAULT_HTTP_TIMEOUT,
            typed_client,
        }
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
            http_timeout: DEFAULT_HTTP_TIMEOUT,
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
        let http = Self::build_http_client(&auth_mode, DEFAULT_HTTP_TIMEOUT);
        let typed_client = ApiClient::new(&base_url, http);

        Self {
            base_url,
            auth_mode,
            verbose,
            http_timeout: DEFAULT_HTTP_TIMEOUT,
            typed_client,
        }
    }

    /// Wait longer than the default for every request this client makes.
    ///
    /// Some endpoints do real work before answering — closing a task removes
    /// its sandbox, messaging a stopped one provisions a replacement — and can
    /// outrun the default against a remote cluster.
    pub fn set_http_timeout(&mut self, timeout: std::time::Duration) {
        self.http_timeout = timeout;
        let http = Self::build_http_client(&self.auth_mode, timeout);
        self.typed_client = ApiClient::new(&self.base_url, http);
    }

    /// Build a `reqwest::Client` with default auth headers baked in.
    fn build_http_client(
        auth_mode: &ClientAuthMode,
        timeout: std::time::Duration,
    ) -> reqwest::Client {
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
            ClientAuthMode::SessionCookie(cookie) => {
                if let Ok(v) = HeaderValue::from_str(cookie) {
                    default_headers.insert(COOKIE, v);
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
            .timeout(timeout)
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
            let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
            self.typed_client = ApiClient::new(&self.base_url, http);
        }
    }

    pub fn api(&self) -> &ApiClient {
        &self.typed_client
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Raw GET for the auth probe in `ensure_auth`.
    pub async fn raw_get(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("GET {url}");
        }
        let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
        let resp = http.get(&url).send().await?;
        self.handle_response(resp).await
    }

    /// GET an arbitrary URL (not relative to base) with the same auth headers.
    /// Returns the response body as text. Only fails on auth errors, not
    /// app-level errors.
    pub async fn raw_get_url(&self, url: &str) -> Result<String, ClientError> {
        if self.verbose {
            eprintln!("GET {url}");
        }
        let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
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

    /// Raw POST with JSON body.
    pub async fn raw_post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("POST {url}");
        }
        let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
        let resp = http.post(&url).json(body).send().await?;
        self.handle_response(resp).await
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
        let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
        let resp = http.put(&url).json(body).send().await?;
        self.handle_response(resp).await
    }

    /// Raw PATCH with JSON body.
    pub async fn raw_patch(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("PATCH {url}");
        }
        let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
        let resp = http.patch(&url).json(body).send().await?;
        self.handle_response(resp).await
    }

    /// Raw DELETE (no body).
    pub async fn raw_delete(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        let url = format!("{}{path}", self.base_url);
        if self.verbose {
            eprintln!("DELETE {url}");
        }
        let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
        let resp = http.delete(&url).send().await?;
        self.handle_response(resp).await
    }

    /// Make an arbitrary request and report the status alongside the body,
    /// without treating a rejection as an error.
    ///
    /// Every other raw helper collapses a 4xx into `ClientError` — right for a
    /// terminal user, wrong when the caller's question *is* "how was this
    /// rejected". Nothing here is retried or interpreted.
    pub async fn raw_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
        headers: &[(String, String)],
    ) -> Result<RawResponse, ClientError> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{path}", self.base_url)
        };
        let verb = reqwest::Method::from_bytes(method.to_uppercase().as_bytes()).map_err(|_| {
            ClientError::Api {
                status: 400,
                body: format!("Unsupported HTTP method '{method}'"),
            }
        })?;
        if self.verbose {
            eprintln!("{verb} {url}");
        }

        let http = Self::build_http_client(&self.auth_mode, self.http_timeout);
        let mut request = http.request(verb, &url);
        if let Some(body) = body {
            request = request.json(body);
        }
        for (name, value) in headers {
            request = request.header(name, value);
        }

        let resp = request.send().await?;
        let status = resp.status().as_u16();
        let text = resp.text().await?;
        let body = if text.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
        };
        Ok(RawResponse { status, body })
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

    /// Start a streaming GET request (no timeout, returns raw response for
    /// SSE/NDJSON).
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
            ClientAuthMode::SessionCookie(cookie) => {
                if let Ok(v) = HeaderValue::from_str(cookie) {
                    default_headers.insert(COOKIE, v);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A body that fails to decode is the CLI's problem, not the server's —
    /// reporting it as `API error (500)` would misdirect the reader to backend
    /// logs when the request actually returned 200.
    #[test]
    fn an_undecodable_body_is_not_reported_as_a_server_status() {
        let err = ClientError::from(ApiError::InvalidJson(
            "invalid type: null, expected u16".to_string(),
        ));

        assert!(
            matches!(err, ClientError::UndecodableResponse(_)),
            "expected UndecodableResponse, got {err:?}"
        );
        let shown = err.to_string();
        assert!(!shown.contains("500"), "must not invent a status: {shown}");
        assert!(shown.contains("invalid type: null, expected u16"));
        // The reader is told how to check the diagnosis and what to do about it.
        assert!(shown.contains("moat --version"), "{shown}");
        assert!(shown.contains("restart the sandbox"), "{shown}");
    }

    /// A real server status still travels as one.
    #[test]
    fn a_server_error_keeps_its_status() {
        let err = ClientError::from(ApiError::Api {
            status: 503,
            body: "upstream down".to_string(),
        });
        assert!(matches!(err, ClientError::Api { status: 503, .. }));
    }
}
