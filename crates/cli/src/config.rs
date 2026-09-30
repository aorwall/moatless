//! Configuration from environment variables.
//!
//! Auth mode resolution, first match wins: a sandbox user token (refreshed copy
//! on disk, else the `USER_AUTH_TOKEN` baked into the pod at creation) is
//! exchanged for a session cookie; else `MOATLESS_API_KEY` is used directly as
//! `Authorization: Bearer <key>` for non-interactive contexts; else the API key
//! comes from `~/.config/moatless/credentials.json` (set by `moat login`).

/// Authentication mode.
#[derive(Debug, Clone)]
pub enum AuthMode {
    /// Inside a sandbox pod — uses `USER_AUTH_TOKEN` for session exchange.
    Sandbox {
        user_auth_token: String,
        task_id: Option<String>,
    },
    /// Standalone CLI — uses API key from credentials file.
    Standalone { api_key: Option<String> },
}

#[derive(Debug, Clone)]
pub struct Config {
    /// API URL. Resolution: `MOATLESS_API_URL` → `BACKEND_API_URL` →
    /// "<https://moatless.dev>"
    pub api_url: String,
    /// Authentication mode (sandbox vs standalone).
    pub auth_mode: AuthMode,
    /// Session file path (sandbox mode only).
    pub session_file: String,
    /// Session cookie name.
    pub cookie_name: String,
    /// What `moat login` fills its prompts from instead of asking.
    pub login: LoginEnv,
    /// The loopback port of this sandbox's own gRPC server: `GRPC_PORT`, else
    /// 50052.
    pub sandbox_grpc_port: String,
    /// Whether the caller already names a repository for `gh` through `GH_REPO`.
    pub gh_repo_set: bool,
    /// Whether `GH_TOKEN` or `GITHUB_TOKEN` holds a non-blank token.
    pub github_token_set: bool,
}

/// `moat login`'s non-interactive inputs, each `None` when unset or empty.
#[derive(Clone, Default)]
pub struct LoginEnv {
    /// `MOATLESS_EMAIL`, else `MOATLESS_USERNAME`.
    pub email: Option<String>,
    /// `MOATLESS_PASSWORD`.
    pub password: Option<String>,
}

impl std::fmt::Debug for LoginEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginEnv")
            .field("email", &self.email)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
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
            login: LoginEnv {
                email: std::env::var("MOATLESS_EMAIL")
                    .or_else(|_| std::env::var("MOATLESS_USERNAME"))
                    .ok()
                    .filter(|value| !value.is_empty()),
                password: std::env::var("MOATLESS_PASSWORD")
                    .ok()
                    .filter(|value| !value.is_empty()),
            },
            sandbox_grpc_port: std::env::var("GRPC_PORT").unwrap_or_else(|_| "50052".to_string()),
            gh_repo_set: std::env::var_os("GH_REPO").is_some(),
            github_token_set: std::env::var("GH_TOKEN").is_ok_and(|v| !v.trim().is_empty())
                || std::env::var("GITHUB_TOKEN").is_ok_and(|v| !v.trim().is_empty()),
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

/// Where the sandbox persists the refreshed user auth token (~15 min cycle),
/// which outlives the pod's fixed, staling `USER_AUTH_TOKEN` env. Mirrors the
/// sandbox's `RUNTIME_DIR`.
const PERSISTED_USER_TOKEN_PATH: &str = "/run/moatless/.user_auth_token";

/// The sandbox token, when one is actually present.
///
/// Prefers the on-disk refreshed copy over the pod's fixed env copy, which goes
/// stale past the token's TTL and never recovers; an env var explicitly set to
/// empty means "no credential" (relied on by the E2E suite for anonymous-reject
/// checks) and overrides any on-disk token.
pub fn sandbox_token() -> Option<String> {
    if matches!(std::env::var("USER_AUTH_TOKEN"), Ok(token) if token.trim().is_empty()) {
        return None;
    }

    persisted_sandbox_token().or_else(|| {
        std::env::var("USER_AUTH_TOKEN")
            .ok()
            .filter(|token| !token.trim().is_empty())
    })
}

/// Read the refreshed user token the sandbox persisted to disk, if any. Honors
/// a `RUNTIME_DIR` override for tests; defaults to the sandbox path.
fn persisted_sandbox_token() -> Option<String> {
    read_persisted_token(std::env::var("RUNTIME_DIR").ok().as_deref())
}

/// Load and trim the persisted token under `runtime_dir` (or the default path).
/// Returns `None` when the file is absent, unreadable, or blank.
fn read_persisted_token(runtime_dir: Option<&str>) -> Option<String> {
    let path = match runtime_dir {
        Some(dir) => format!("{}/.user_auth_token", dir.trim_end_matches('/')),
        None => PERSISTED_USER_TOKEN_PATH.to_string(),
    };
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim().to_string();
    if token.is_empty() { None } else { Some(token) }
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

impl Config {
    /// The task named by this invocation's environment.
    pub fn require_task_id(&self) -> Result<String, Box<dyn std::error::Error>> {
        self.task_id()
            .map(String::from)
            .ok_or_else(|| "TASK_ID environment variable is required for this command".into())
    }
}

#[cfg(test)]
mod tests {
    use super::read_persisted_token;

    #[test]
    fn persisted_token_is_read_and_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".user_auth_token"), "  fresh-token\n").unwrap();
        assert_eq!(
            read_persisted_token(Some(dir.path().to_str().unwrap())),
            Some("fresh-token".to_string())
        );
    }

    #[test]
    fn blank_persisted_token_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".user_auth_token"), "   \n").unwrap();
        assert_eq!(
            read_persisted_token(Some(dir.path().to_str().unwrap())),
            None
        );
    }

    #[test]
    fn missing_persisted_token_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_persisted_token(Some(dir.path().to_str().unwrap())),
            None
        );
    }
}
