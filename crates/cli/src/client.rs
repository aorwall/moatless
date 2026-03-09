//! HTTP client wrapper with session cookie / API key auth.
//!
//! Wraps the progenitor-generated `moatless_api_client::Client` with auth
//! header injection.  Exposes `api()` for typed endpoints.

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
    #[error("Typed API error ({status}): {message}")]
    TypedApi { status: u16, message: String },
}

pub struct MoatClient {
    base_url: String,
    auth_mode: ClientAuthMode,
    verbose: bool,
    /// Progenitor-generated typed client.
    typed_client: moatless_api_client::Client,
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
        let typed_client = moatless_api_client::Client::new_with_client(&base_url, http);

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
            self.typed_client =
                moatless_api_client::Client::new_with_client(&self.base_url, http);
        }
    }

    /// Access the typed progenitor-generated API client.
    pub fn api(&self) -> &moatless_api_client::Client {
        &self.typed_client
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
}

/// Convert a progenitor `Error<E>` into our `ClientError`.
pub fn map_api_error<E: std::fmt::Debug>(err: progenitor_client::Error<E>) -> ClientError {
    match err {
        progenitor_client::Error::InvalidRequest(msg) => ClientError::Api {
            status: 400,
            body: msg,
        },
        progenitor_client::Error::CommunicationError(e) => ClientError::Http(e),
        progenitor_client::Error::ErrorResponse(rv) => {
            let status = rv.status().as_u16();
            if status == 401 {
                ClientError::Unauthorized
            } else {
                ClientError::TypedApi {
                    status,
                    message: format!("{:?}", rv.into_inner()),
                }
            }
        }
        progenitor_client::Error::InvalidResponsePayload(_, e) => ClientError::Api {
            status: 500,
            body: format!("Invalid response: {e}"),
        },
        progenitor_client::Error::UnexpectedResponse(resp) => {
            let status = resp.status().as_u16();
            if status == 401 {
                ClientError::Unauthorized
            } else {
                ClientError::TypedApi {
                    status,
                    message: format!("Unexpected response (status {status})"),
                }
            }
        }
        progenitor_client::Error::PreHookError(e) => ClientError::Api {
            status: 500,
            body: format!("Pre-hook error: {e}"),
        },
        _ => ClientError::Api {
            status: 500,
            body: format!("Unknown error: {err:?}"),
        },
    }
}
