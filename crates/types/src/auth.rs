//! Authentication settings: which provider signs a user in, and the
//! credentials each one needs.
//!
//! The redacting `Debug` impls are what keeps secrets out of logs, so every
//! one of them travels with its struct.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

/// Supported auth providers.
///
/// `Local` is the default: email/password against the `user_credentials`
/// table, with a bootstrap admin seeded at startup. OIDC deployments may keep
/// a separately enabled break-glass administrator credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthProvider {
    #[default]
    Local,
    GitHubApp,
    Auth0,
    Oidc,
}

impl fmt::Display for AuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::GitHubApp => write!(f, "github_app"),
            Self::Auth0 => write!(f, "auth0"),
            Self::Oidc => write!(f, "oidc"),
        }
    }
}

impl FromStr for AuthProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            // Legacy spellings of the removed no-auth mode.
            "" | "disabled" | "none" => {
                tracing::warn!(
                    "AUTH_PROVIDER={s:?} is deprecated and now means local email/password \
                     auth. Set AUTH_PROVIDER=local (or leave it unset) instead."
                );
                Ok(Self::Local)
            }
            "github_app" | "github-app" => Ok(Self::GitHubApp),
            "auth0" => Ok(Self::Auth0),
            "oidc" => Ok(Self::Oidc),
            _ => Err(format!("unknown auth provider: {s}")),
        }
    }
}

/// Authentication and session settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone)]
pub struct AuthConfig {
    /// Active auth provider. Defaults to [`AuthProvider::Local`]
    /// (email/password against `user_credentials`).
    pub provider: AuthProvider,
    /// Required GitHub organization login. When set, only members of this org can log in.
    pub github_required_org: Option<String>,
    /// Auth0 domain (e.g., "myapp.us.auth0.com")
    pub auth0_domain: Option<String>,
    /// Auth0 client ID
    pub auth0_client_id: Option<String>,
    /// Auth0 client secret
    pub auth0_client_secret: Option<String>,
    /// Auth0 callback URL
    pub auth0_callback_url: Option<String>,
    /// Auth0 audience
    pub auth0_audience: Option<String>,
    /// Auth0 JWT claim key for groups (default: "https://moatless.ai/groups")
    pub auth0_groups_claim: String,
    /// Standards-based OpenID Connect provider. Auth0 settings above remain a
    /// compatibility input and are translated into this representation.
    pub oidc: Option<OidcConfig>,
    /// Session max age in seconds (default: 604800 = 7 days)
    pub session_max_age_secs: u64,
    /// Hard auth-session lifetime regardless of activity.
    pub session_absolute_age_secs: u64,
    /// Frontend URL for redirects (default: "http://localhost:3000")
    pub frontend_url: String,
    /// Cookie domain (e.g., ".moatless.xyz"). None for localhost.
    pub cookie_domain: Option<String>,
    /// Session cookie name (default: "moatless_session")
    pub session_cookie_name: String,
    /// Whether the app is running in production mode (affects cookie Secure flag)
    pub is_production: bool,
    /// Login of the bootstrap admin seeded at startup under
    /// [`AuthProvider::Local`] (env: `ADMIN_USERNAME`, default `"admin"`).
    pub admin_username: String,
    /// Email address used by the bootstrap and break-glass administrator.
    pub admin_email: Option<String>,
    /// Password for the bootstrap admin (env: `ADMIN_PASSWORD`). When unset a
    /// random one is generated on first boot and logged once at WARN.
    pub admin_password: Option<String>,
    /// Register the unadvertised administrator-only password login route.
    pub break_glass_enabled: bool,
    /// Auth cache TTL in seconds (default: 30).
    pub cache_ttl_secs: u64,
    /// Secret for signing session JWTs (env: SESSION_JWT_SECRET).
    /// Falls back to `sandbox.task_token_secret` when not set.
    pub session_jwt_secret: Option<String>,
    /// Raw `mvk_`-prefixed API key for a test user auto-created on startup.
    /// When set, upserts a dedicated `test-api-user` and inserts the API key.
    /// No OAuth login required.
    pub test_api_key: Option<String>,
    /// Default team slug. On login, if a user has no teams and a default team exists, they are added to it.
    pub default_team_slug: Option<String>,

    // --- GitHub App (user authorization flow) ---
    /// GitHub App client ID (from App settings, different from OAuth App).
    pub github_app_client_id: Option<String>,
    /// GitHub App client secret.
    pub github_app_client_secret: Option<String>,
    /// GitHub App OAuth callback URL.
    pub github_app_callback_url: Option<String>,

    // --- GitHub App (installation token flow) ---
    /// GitHub App numeric ID.
    pub github_app_id: Option<u64>,
    /// PEM-encoded private key for signing App JWTs.
    pub github_app_private_key: Option<String>,
    /// Default installation ID for server-to-server operations.
    pub github_app_default_installation_id: Option<u64>,

    /// Additional GitHub Apps registered as bot identities (e.g. `dev-bot`,
    /// `review-bot`). Loaded from the `GITHUB_APPS_JSON` env var, which is now
    /// a one-time upgrade path only: the first start after upgrading copies
    /// each entry into `github_apps` and the variable is ignored from then on
    /// (see `GitHubAppRegistry::carry_over_env_apps`). It holds a
    /// JSON array of [`GitHubAppConfig`] entries. A bot user's
    /// `provider_tokens` row stores the matching `key` in `provider_user_id`,
    /// so credential resolution picks the right App.
    pub github_apps: Vec<GitHubAppConfig>,
}

#[derive(Clone)]
pub struct OidcConfig {
    pub provider_id: String,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub callback_url: String,
    pub auto_provision: bool,
    pub auto_link_verified_email: bool,
    pub allowed_domains: Vec<String>,
}

impl fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcConfig")
            .field("provider_id", &self.provider_id)
            .field("name", &self.name)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("callback_url", &self.callback_url)
            .field("auto_provision", &self.auto_provision)
            .field("auto_link_verified_email", &self.auto_link_verified_email)
            .field("allowed_domains", &self.allowed_domains)
            .finish()
    }
}

/// One GitHub App registration, used by the `GitHubAppRegistry`.
#[derive(Clone, Deserialize)]
pub struct GitHubAppConfig {
    /// Stable key referenced from loop config (e.g. `"dev-bot"`).
    pub key: String,
    /// GitHub App numeric ID.
    pub app_id: u64,
    /// PEM-encoded private key for signing App JWTs.
    pub private_key: String,
    /// Optional OAuth client id (only required if this App also acts as a
    /// user-auth provider — bot-only Apps can omit it).
    #[serde(default)]
    pub client_id: Option<String>,
    /// Optional OAuth client secret.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Optional explicit installation id. When omitted the registry resolves
    /// the installation per repo owner via the GitHub API.
    #[serde(default)]
    pub default_installation_id: Option<u64>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for GitHubAppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubAppConfig")
            .field("key", &self.key)
            .field("app_id", &self.app_id)
            .field("private_key", &"[REDACTED]")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("default_installation_id", &self.default_installation_id)
            .finish()
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("provider", &self.provider)
            .field("auth0_domain", &self.auth0_domain)
            .field("auth0_client_id", &self.auth0_client_id)
            .field(
                "auth0_client_secret",
                &self.auth0_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("auth0_callback_url", &self.auth0_callback_url)
            .field("oidc", &self.oidc)
            .field("session_max_age_secs", &self.session_max_age_secs)
            .field("session_absolute_age_secs", &self.session_absolute_age_secs)
            .field("frontend_url", &self.frontend_url)
            .field("cookie_domain", &self.cookie_domain)
            .field("is_production", &self.is_production)
            .field("admin_username", &self.admin_username)
            .field("admin_email", &self.admin_email)
            .field(
                "admin_password",
                &self.admin_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field("break_glass_enabled", &self.break_glass_enabled)
            .field(
                "session_jwt_secret",
                &self.session_jwt_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "test_api_key",
                &self.test_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("github_app_client_id", &self.github_app_client_id)
            .field(
                "github_app_client_secret",
                &self.github_app_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("github_app_callback_url", &self.github_app_callback_url)
            .field("github_app_id", &self.github_app_id)
            .field(
                "github_app_private_key",
                &self.github_app_private_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "github_app_default_installation_id",
                &self.github_app_default_installation_id,
            )
            .field("github_apps", &self.github_apps)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_config_debug_redacts_the_client_secret() {
        let oidc = OidcConfig {
            provider_id: "company".into(),
            name: "Company SSO".into(),
            issuer: "https://issuer".into(),
            client_id: "client-123".into(),
            client_secret: "real-oidc-secret".into(),
            callback_url: "https://app/callback".into(),
            auto_provision: true,
            auto_link_verified_email: false,
            allowed_domains: vec!["example.com".into()],
        };
        let debug = format!("{oidc:?}");
        assert!(!debug.contains("real-oidc-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn github_app_config_debug_redacts_the_private_key() {
        let app = GitHubAppConfig {
            key: "dev-bot".into(),
            app_id: 42,
            private_key: "-----BEGIN RSA PRIVATE KEY-----real".into(),
            client_id: Some("Iv1.abc".into()),
            client_secret: Some("real-app-secret".into()),
            default_installation_id: None,
        };
        let debug = format!("{app:?}");
        assert!(!debug.contains("BEGIN RSA PRIVATE KEY"));
        assert!(!debug.contains("real-app-secret"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("dev-bot")); // the key is an identifier, not a secret
    }
}
