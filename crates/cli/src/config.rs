//! Configuration from environment variables.
//!
//! Auth mode resolution (first match wins):
//! 1. **Sandbox**: `USER_AUTH_TOKEN` set (inside a sandbox pod) — exchanges
//!    the token for a session cookie.
//! 2. **Standalone with env API key**: `MOATLESS_API_KEY` set — used directly
//!    as `Authorization: Bearer <key>`. Intended for non-interactive contexts
//!    (CI, cron, scripts) where the credentials file isn't available.
//! 3. **Standalone with credentials file**: API key from
//!    `~/.config/moatless/credentials.json` (set by `moat login`).

/// Authentication mode.
#[derive(Debug, Clone)]
pub enum AuthMode {
    /// Inside a sandbox pod — uses USER_AUTH_TOKEN for session exchange.
    Sandbox {
        user_auth_token: String,
        task_id: Option<String>,
    },
    /// Standalone CLI — uses API key from credentials file.
    Standalone { api_key: Option<String> },
}

#[derive(Debug, Clone)]
pub struct Config {
    /// API URL. Resolution: MOATLESS_API_URL → BACKEND_API_URL → "https://moatless.dev"
    pub api_url: String,
    /// Authentication mode (sandbox vs standalone).
    pub auth_mode: AuthMode,
    /// Session file path (sandbox mode only).
    pub session_file: String,
    /// Session cookie name.
    pub cookie_name: String,
}

impl Config {
    pub fn from_env() -> Self {
        let env_url = std::env::var("MOATLESS_API_URL")
            .or_else(|_| std::env::var("BACKEND_API_URL"))
            .ok();

        // Fall back to the default profile's api_url (set by
        // `moat login --local`) before the hardcoded default.
        let api_url = env_url
            .or_else(|| {
                crate::credentials::default_profile_url()
                    .and_then(|(host, url)| url.or_else(|| Some(format!("https://{host}"))))
            })
            .unwrap_or_else(|| "https://moatless.dev".to_string());

        let auth_mode = if let Some(token) = sandbox_token() {
            let task_id = std::env::var("TASK_ID").ok();
            AuthMode::Sandbox {
                user_auth_token: token,
                task_id,
            }
        } else if let Ok(env_key) = std::env::var("MOATLESS_API_KEY") {
            // Non-interactive auth (CI, scripts) — use the env key directly,
            // skipping the credentials-file lookup.
            let trimmed = env_key.trim().to_string();
            let api_key = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            };
            AuthMode::Standalone { api_key }
        } else {
            // Standalone mode — try loading API key from credentials file
            let host = host_from_url(&api_url);
            let api_key = crate::credentials::get_api_key(&host);
            AuthMode::Standalone { api_key }
        };

        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let session_file = format!("{home}/.moat_session");

        let cookie_name =
            std::env::var("MOAT_COOKIE_NAME").unwrap_or_else(|_| "moatless_session".to_string());

        Self {
            api_url,
            auth_mode,
            session_file,
            cookie_name,
        }
    }

    /// Whether we're in sandbox mode.
    pub fn is_sandbox(&self) -> bool {
        matches!(self.auth_mode, AuthMode::Sandbox { .. })
    }

    /// Get the task ID (sandbox mode only).
    pub fn task_id(&self) -> Option<&str> {
        match &self.auth_mode {
            AuthMode::Sandbox { task_id, .. } => task_id.as_deref(),
            AuthMode::Standalone { .. } => None,
        }
    }

    /// Get the host portion of the API URL (for credential profiles).
    pub fn host(&self) -> String {
        host_from_url(&self.api_url)
    }
}

/// The sandbox token, when one is actually present.
///
/// An env var set to the empty string is not a credential — it is how a caller
/// says "make this request without one", which the E2E suite relies on to
/// assert that a route rejects anonymous callers.
pub fn sandbox_token() -> Option<String> {
    std::env::var("USER_AUTH_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
}

/// Extract host (with port if non-standard) from a URL.
pub fn host_from_url(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}
