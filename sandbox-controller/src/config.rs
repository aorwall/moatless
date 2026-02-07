//! Configuration module for the sandbox controller service.
//!
//! Loads configuration from environment variables with sensible defaults.

use std::env;
use std::fmt;

/// Supported auth providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProvider {
    GitHub,
    Auth0,
}

impl fmt::Display for AuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitHub => write!(f, "github"),
            Self::Auth0 => write!(f, "auth0"),
        }
    }
}

impl AuthProvider {
    /// Parse from string (case-insensitive).
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "github" => Some(Self::GitHub),
            "auth0" => Some(Self::Auth0),
            _ => None,
        }
    }
}

/// Sandbox controller configuration
#[derive(Debug, Clone)]
pub struct Config {
    /// Port to listen on (default: 8080)
    pub port: u16,

    /// Metrics port (default: 9090)
    pub metrics_port: u16,

    /// Kubernetes namespace for sandbox pods
    pub sandbox_namespace: String,

    /// Path to worker pod YAML template
    pub worker_pod_template_path: Option<String>,

    /// Whether PVC storage is enabled for worker pods
    pub k8s_worker_storage_enabled: bool,

    /// PVC storage size (e.g., "5Gi")
    pub k8s_worker_storage_size: String,

    /// PVC storage class
    pub k8s_worker_storage_class: Option<String>,

    /// Pod startup timeout in seconds
    pub k8s_pod_startup_timeout_secs: u64,

    /// Secret for signing task auth JWTs
    pub task_token_secret: Option<String>,

    // =========================================================================
    // Proxy / Traefik IngressRoute settings
    // =========================================================================
    /// Domain for proxy URLs (e.g., "moatless.xyz"). If None, IngressRoutes are not created.
    pub proxy_domain: Option<String>,

    /// Protocol for proxy URLs (default: "https")
    pub proxy_protocol: String,

    /// Name of the Traefik ForwardAuth Middleware CRD (default: "preview-auth")
    pub traefik_auth_middleware_name: String,

    /// Namespace where the Traefik auth Middleware lives (defaults to sandbox_namespace)
    pub traefik_auth_middleware_namespace: String,

    /// Traefik entryPoint name for IngressRoutes (default: "websecure")
    pub traefik_entry_point: String,

    // =========================================================================
    // Database (optional — enables auto-provisioning on /start)
    // =========================================================================
    /// PostgreSQL connection URL. When set, the controller can read task/repository
    /// config from the database and auto-provision workspaces on `/start`.
    pub database_url: Option<String>,

    // =========================================================================
    // Redis (optional — enables environment event publishing)
    // =========================================================================
    /// Redis connection URL. When set, the controller publishes environment
    /// lifecycle events (initializing, ready, error, stopped) to Redis Streams.
    pub redis_url: Option<String>,

    /// Maximum stream length for MAXLEN ~ trimming (default: 10000)
    pub redis_max_stream_length: usize,

    // =========================================================================
    // Auth settings
    // =========================================================================
    /// Auth provider, or None (dev mode — auth disabled)
    pub auth_provider: Option<AuthProvider>,

    /// GitHub OAuth client ID
    pub github_client_id: Option<String>,
    /// GitHub OAuth client secret
    pub github_client_secret: Option<String>,
    /// GitHub OAuth callback URL
    pub github_callback_url: Option<String>,
    /// GitHub OAuth scopes (default: "user:email repo")
    pub github_oauth_scope: String,

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

    /// Session max age in seconds (default: 604800 = 7 days)
    pub session_max_age_secs: u64,
    /// Frontend URL for redirects (default: "http://localhost:3000")
    pub frontend_url: String,
    /// Cookie domain (e.g., ".moatless.xyz"). None for localhost.
    pub cookie_domain: Option<String>,
    /// Session cookie name (default: "moatless_session")
    pub session_cookie_name: String,
    /// Whether the app is running in production mode (affects cookie Secure flag)
    pub is_production: bool,

    /// Auth cache TTL in seconds (default: 30). Controls how long verified
    /// sessions and API keys are cached in memory before re-querying the DB.
    pub auth_cache_ttl_secs: u64,
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        // Load .env file if present (ignored in production)
        let _ = dotenvy::dotenv();

        Self {
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080),

            metrics_port: env::var("METRICS_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(9090),

            sandbox_namespace: env::var("SANDBOX_NAMESPACE")
                .unwrap_or_else(|_| "workspaces".to_string()),

            worker_pod_template_path: env::var("WORKER_POD_TEMPLATE_PATH").ok(),

            k8s_worker_storage_enabled: env::var("K8S_WORKER_STORAGE_ENABLED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true),

            k8s_worker_storage_size: env::var("K8S_WORKER_STORAGE_SIZE")
                .unwrap_or_else(|_| "5Gi".to_string()),

            k8s_worker_storage_class: env::var("K8S_WORKER_STORAGE_CLASS").ok(),

            k8s_pod_startup_timeout_secs: env::var("K8S_POD_STARTUP_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120),

            task_token_secret: env::var("TASK_TOKEN_SECRET").ok(),

            proxy_domain: env::var("PROXY_DOMAIN").ok(),

            proxy_protocol: env::var("PROXY_PROTOCOL")
                .unwrap_or_else(|_| "https".to_string()),

            traefik_auth_middleware_name: env::var("TRAEFIK_AUTH_MIDDLEWARE_NAME")
                .unwrap_or_else(|_| "preview-auth".to_string()),

            traefik_auth_middleware_namespace: env::var("TRAEFIK_AUTH_MIDDLEWARE_NAMESPACE")
                .unwrap_or_else(|_| {
                    env::var("SANDBOX_NAMESPACE")
                        .unwrap_or_else(|_| "workspaces".to_string())
                }),

            traefik_entry_point: env::var("TRAEFIK_ENTRY_POINT")
                .unwrap_or_else(|_| "websecure".to_string()),

            database_url: env::var("DATABASE_URL").ok(),

            redis_url: env::var("REDIS_URL").ok(),

            redis_max_stream_length: env::var("REDIS_MAX_STREAM_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10_000),

            auth_provider: env::var("AUTH_PROVIDER")
                .ok()
                .filter(|v| v != "disabled")
                .and_then(|v| AuthProvider::from_str_opt(&v)),

            github_client_id: env::var("GITHUB_CLIENT_ID").ok(),
            github_client_secret: env::var("GITHUB_CLIENT_SECRET").ok(),
            github_callback_url: env::var("GITHUB_CALLBACK_URL").ok(),
            github_oauth_scope: env::var("GITHUB_OAUTH_SCOPE")
                .unwrap_or_else(|_| "user:email repo".to_string()),

            auth0_domain: env::var("AUTH0_DOMAIN").ok(),
            auth0_client_id: env::var("AUTH0_CLIENT_ID").ok(),
            auth0_client_secret: env::var("AUTH0_CLIENT_SECRET").ok(),
            auth0_callback_url: env::var("AUTH0_CALLBACK_URL").ok(),
            auth0_audience: env::var("AUTH0_AUDIENCE").ok(),
            auth0_groups_claim: env::var("AUTH0_GROUPS_CLAIM")
                .unwrap_or_else(|_| "https://moatless.ai/groups".to_string()),

            session_max_age_secs: env::var("SESSION_MAX_AGE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(604_800),
            frontend_url: env::var("FRONTEND_URL")
                .unwrap_or_else(|_| "http://localhost:3000".to_string()),
            cookie_domain: env::var("COOKIE_DOMAIN").ok(),
            session_cookie_name: env::var("SESSION_COOKIE_NAME")
                .unwrap_or_else(|_| "moatless_session".to_string()),
            is_production: env::var("APP_ENV")
                .or_else(|_| env::var("NODE_ENV"))
                .map(|v| v == "production")
                .unwrap_or(false),

            auth_cache_ttl_secs: env::var("AUTH_CACHE_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        }
    }
}
