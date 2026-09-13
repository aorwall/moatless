//! Authentication settings: which provider signs a user in, and the credentials
//! each one needs.
//!
//! The redacting `Debug` impls are what keeps secrets out of logs, so every one
//! of them travels with its struct.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use uuid::Uuid;

use crate::user::UserRole;

/// Supported auth providers.
///
/// `Local` is the default: email/password against `user_credentials`, with a
/// bootstrap admin seeded at startup; OIDC deployments may keep a separately
/// enabled break-glass administrator credential.
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
            // Legacy spellings of the removed no-auth mode: `""`, `"disabled"`, `"none"`.
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
    /// Required GitHub organization login. When set, only members of this org
    /// can log in.
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
    /// Auth0 JWT claim key for groups (default: "<https://moatless.ai/groups>")
    pub auth0_groups_claim: String,
    /// Standards-based OpenID Connect provider. Auth0 settings above remain a
    /// compatibility input and are translated into this representation.
    pub oidc: Option<OidcConfig>,
    /// Session max age in seconds (default: 604800 = 7 days)
    pub session_max_age_secs: u64,
    /// Hard auth-session lifetime regardless of activity.
    pub session_absolute_age_secs: u64,
    /// Frontend URL for redirects (default: "<http://localhost:3000>")
    pub frontend_url: String,
    /// Browser origins trusted for the double-submit CSRF check beyond
    /// [`Self::frontend_url`] (env: `CSRF_TRUSTED_ORIGINS`).
    ///
    /// The T3 web client reaches `/api` via Traefik on its own origin, so it
    /// needs an explicit entry; a wildcard host trusts every subdomain with the
    /// session cookie.
    pub csrf_trusted_origins: Vec<AllowedOrigin>,
    /// Cookie domain (e.g., ".moatless.xyz"). None for localhost.
    pub cookie_domain: Option<String>,
    /// Session cookie name (default: "`moatless_session`")
    pub session_cookie_name: String,
    /// Whether the app is running in production mode (affects cookie Secure
    /// flag)
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
    /// Secret for signing session JWTs (env: `SESSION_JWT_SECRET`). Falls back
    /// to `sandbox.task_token_secret` when not set.
    pub session_jwt_secret: Option<String>,
    /// Raw `mvk_`-prefixed API key for a test user auto-created on startup.
    /// When set, upserts a dedicated `test-api-user` and inserts the API key.
    /// No OAuth login required.
    pub test_api_key: Option<String>,
    /// Default team slug. On login, if a user has no teams and a default team
    /// exists, they are added to it.
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
    /// `review-bot`).
    ///
    /// `GITHUB_APPS_JSON` is a one-time upgrade path: the first start after
    /// upgrading copies each entry into this field and ignores the env var from
    /// then on (see `GitHubAppRegistry::carry_over_env_apps`). A bot user's
    /// `provider_tokens.provider_user_id` stores the matching `key`.
    pub github_apps: Vec<GitHubAppConfig>,
}

impl AuthConfig {
    /// The credentials the GitHub App user-authorization flow needs, present
    /// only when all three are configured.
    ///
    /// The one authority on whether that flow can run at all — both GitHub App
    /// *login* and *connect* go through it, so a deployment using another
    /// sign-in method can still offer GitHub access without a PAT.
    pub fn github_app_oauth(&self) -> Option<(&str, &str, &str)> {
        Some((
            self.github_app_client_id.as_deref()?,
            self.github_app_client_secret.as_deref()?,
            self.github_app_callback_url.as_deref()?,
        ))
    }
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

/// One entry of a browser origin allow-list.
///
/// Grants are list-specific, not interchangeable —
/// [`AuthConfig::csrf_trusted_origins`] admits a cookie write, the T3 WebSocket
/// list a socket. A `*.<domain>` entry admits any same-scheme host under it,
/// for sandbox previews whose per-workspace hostname isn't known in advance;
/// the wildcard never spans scheme or port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowedOrigin {
    Exact(String),
    /// `prefix` is `<scheme>://`, `suffix` is `.<domain>`.
    Subdomain { prefix: String, suffix: String },
}

impl AllowedOrigin {
    pub fn parse(entry: &str) -> Self {
        match entry.split_once("://*.") {
            Some((scheme, domain)) if !scheme.is_empty() && !domain.is_empty() => Self::Subdomain {
                prefix: format!("{scheme}://"),
                suffix: format!(".{domain}"),
            },
            _ => Self::Exact(entry.to_string()),
        }
    }

    pub fn matches(&self, origin: &str) -> bool {
        match self {
            Self::Exact(allowed) => allowed == origin,
            Self::Subdomain { prefix, suffix } => origin
                .strip_prefix(prefix.as_str())
                // A browser sends a host and nothing else, but this header is
                // attacker-controlled: without these, `https://evil.example/
                // x.preview.moatless.xyz` ends with the suffix and passes.
                .is_some_and(|host| {
                    host.len() > suffix.len()
                        && host.ends_with(suffix.as_str())
                        && !host.contains(['/', '\\', '?', '#', '@', ':'])
                }),
        }
    }
}

/// Parse a comma-separated origin allow-list, as `T3_UI_RPC_ALLOWED_ORIGINS`
/// carries it. Blank entries are dropped, so an unset variable yields no
/// origins.
pub fn parse_allowed_origins(value: &str) -> Vec<AllowedOrigin> {
    value
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(AllowedOrigin::parse)
        .collect()
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
            .field("csrf_trusted_origins", &self.csrf_trusted_origins)
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

    #[test]
    fn wildcard_entry_admits_hosts_under_its_domain() {
        let allowed = AllowedOrigin::parse("https://*.preview.moatless.xyz");
        assert_eq!(
            allowed,
            AllowedOrigin::Subdomain {
                prefix: "https://".into(),
                suffix: ".preview.moatless.xyz".into(),
            }
        );
        for origin in [
            "https://8d38cf39-4659-4a5e-92b1-49e8dfc9efb1--5733.preview.moatless.xyz",
            "https://anything.preview.moatless.xyz",
            "https://deeper.nested.preview.moatless.xyz",
        ] {
            assert!(allowed.matches(origin), "{origin} should be admitted");
        }
    }

    #[test]
    fn wildcard_entry_refuses_everything_outside_its_domain() {
        let allowed = AllowedOrigin::parse("https://*.preview.moatless.xyz");
        for origin in [
            // the domain itself, which the wildcard label does not stand for
            "https://preview.moatless.xyz",
            // an empty label
            "https://.preview.moatless.xyz",
            // the parent domain, and a lookalike registered elsewhere
            "https://moatless.xyz",
            "https://evil-preview.moatless.xyz",
            "https://preview.moatless.xyz.evil.example",
            // a scheme the entry does not name
            "http://a.preview.moatless.xyz",
            // a port makes it a different origin
            "https://a.preview.moatless.xyz:8443",
            // a path or userinfo smuggling the suffix past a bare ends_with
            "https://evil.example/x.preview.moatless.xyz",
            "https://evil.example@x.preview.moatless.xyz",
            "https://evil.example?.preview.moatless.xyz",
            "https://evil.example#.preview.moatless.xyz",
            "https://evil.example\\.preview.moatless.xyz",
        ] {
            assert!(!allowed.matches(origin), "{origin} should be refused");
        }
    }

    #[test]
    fn entry_without_a_wildcard_label_stays_exact() {
        assert_eq!(
            AllowedOrigin::parse("https://moatless.xyz"),
            AllowedOrigin::Exact("https://moatless.xyz".into())
        );
        // A `*` that is not the whole leading label is not a wildcard.
        let partial = AllowedOrigin::parse("https://*preview.moatless.xyz");
        assert!(matches!(partial, AllowedOrigin::Exact(_)));
        assert!(!partial.matches("https://a.preview.moatless.xyz"));
    }

    #[test]
    fn parse_allowed_origins_drops_blanks_and_trims() {
        assert!(parse_allowed_origins("").is_empty());
        assert_eq!(
            parse_allowed_origins(" https://a.example , ,https://b.example "),
            vec![
                AllowedOrigin::Exact("https://a.example".into()),
                AllowedOrigin::Exact("https://b.example".into()),
            ]
        );
    }
}

/// The authenticated user identity, set by the auth middleware on every
/// successfully-validated request. Downstream extractors read this from
/// `request.extensions()` instead of headers.
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub user_id: Uuid,
    pub login: String,
    pub role: UserRole,
    pub provider: String,
    pub group_id: Option<String>,
}

/// Which credential the middleware accepted for the current request, injected
/// alongside [`AuthIdentity`].
///
/// Read by handlers whose defences depend on *how* the caller proved who they
/// are, not who they are — a cookie is attached by the browser to any request
/// anybody can cause it to make, while the other two require a client holding
/// them deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    SessionCookie,
    ApiKey,
    /// A short-lived ticket in the query string, accepted only on the WebSocket
    /// upgrade.
    WebSocketTicket,
    /// A sandbox-issued `X-User-Token` JWT, carried by the agent's CLI from
    /// inside a sandbox pod.
    ///
    /// A deliberately-held bearer credential, so like an API key it's exempt
    /// from CSRF; also marks the request as agent context for `source`-tagging
    /// downstream.
    UserToken,
}
