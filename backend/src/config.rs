//! Configuration module for the Moatless Vibe backend service.
//!
//! Loads configuration from environment variables with sensible defaults.
//! Config is grouped into logical sub-structs for readability and to enable
//! passing only relevant config to subsystems.

use std::env;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

/// Which sandbox provider implementation to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProviderType {
    Kubernetes,
    Docker,
}

impl fmt::Display for SandboxProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kubernetes => write!(f, "kubernetes"),
            Self::Docker => write!(f, "docker"),
        }
    }
}

impl FromStr for SandboxProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "kubernetes" | "k8s" => Ok(Self::Kubernetes),
            "docker" => Ok(Self::Docker),
            _ => Err(format!("unknown sandbox provider: {s}")),
        }
    }
}

/// Supported auth providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProvider {
    GitHub,
    GitHubApp,
    Auth0,
}

impl fmt::Display for AuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitHub => write!(f, "github"),
            Self::GitHubApp => write!(f, "github_app"),
            Self::Auth0 => write!(f, "auth0"),
        }
    }
}

impl FromStr for AuthProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "github" => Ok(Self::GitHub),
            "github_app" | "github-app" => Ok(Self::GitHubApp),
            "auth0" => Ok(Self::Auth0),
            _ => Err(format!("unknown auth provider: {s}")),
        }
    }
}

/// Parse an environment variable with a default fallback.
/// Logs a warning if the env var is set but cannot be parsed.
fn env_or<T: FromStr>(key: &str, default: T) -> T {
    match env::var(key) {
        Ok(v) => {
            if let Ok(parsed) = v.parse() {
                parsed
            } else {
                tracing::warn!(key, value = %v, "Failed to parse env var, using default");
                default
            }
        }
        Err(_) => default,
    }
}

/// Parse a JSON array of `{"name":"...", "value":"..."}` objects from an env var.
fn parse_extra_env(key: &str) -> Vec<(String, String)> {
    let raw = match env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => return vec![],
    };

    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        value: String,
    }

    match serde_json::from_str::<Vec<Entry>>(&raw) {
        Ok(entries) => entries.into_iter().map(|e| (e.name, e.value)).collect(),
        Err(e) => {
            tracing::warn!(key, error = %e, "Failed to parse extra env JSON");
            vec![]
        }
    }
}

/// Parse the generic sandbox package-manager cache list from an env var.
///
/// Shape (JSON): `[{"name":"maven","mountPath":"/maven-cache",
/// "env":{"MAVEN_OPTS":"-Dmaven.repo.local=/maven-cache"}}]`. Each entry is a
/// self-describing cache: a name (→ a subdir on the shared NVMe volume), the
/// in-container mount path, and the env var(s) that point the tool at it.
/// Adding npm/bun/pnpm later is one more JSON object, no Rust change.
/// `env` is parsed into a key-sorted `Vec` (via `BTreeMap`) for deterministic
/// container specs. Returns an empty vec on parse failure with a warning so a
/// typo can't take down startup.
fn parse_cache_volumes(key: &str) -> Vec<SandboxCacheVolume> {
    let raw = match env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => return vec![],
    };

    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        #[serde(rename = "mountPath")]
        mount_path: String,
        #[serde(default)]
        env: std::collections::BTreeMap<String, String>,
    }

    match serde_json::from_str::<Vec<Entry>>(&raw) {
        Ok(entries) => entries
            .into_iter()
            .map(|e| SandboxCacheVolume {
                name: e.name,
                mount_path: e.mount_path,
                env: e.env.into_iter().collect(),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(key, error = %e, "Failed to parse sandbox cache volumes JSON");
            vec![]
        }
    }
}

/// Read an environment variable as `Option<String>`.
fn env_opt(key: &str) -> Option<String> {
    env::var(key).ok()
}

/// Read an environment variable with a string default.
fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Parse the `GITHUB_APPS_JSON` env var into a list of `GitHubAppConfig`.
///
/// `\\n` sequences in `private_key` are converted to real newlines for
/// PEM-friendly env-var usage. Returns an empty vec on parse failure with a
/// warning so a typo in one JSON blob can't take down startup.
fn parse_github_apps_json(raw: Option<String>) -> Vec<GitHubAppConfig> {
    let Some(raw) = raw else {
        return vec![];
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return vec![];
    }
    match serde_json::from_str::<Vec<GitHubAppConfig>>(trimmed) {
        Ok(mut apps) => {
            for app in &mut apps {
                if app.private_key.contains("\\n") {
                    app.private_key = app.private_key.replace("\\n", "\n");
                }
            }
            apps
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse GITHUB_APPS_JSON — ignoring");
            vec![]
        }
    }
}

/// Read an environment variable as a boolean (true if "true" or "1").
fn env_bool(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) if default => v != "false" && v != "0",
        Ok(v) => v == "true" || v == "1",
        Err(_) => default,
    }
}

// =========================================================================
// Sub-structs
// =========================================================================

/// HTTP and gRPC server ports.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Port to listen on (default: 8080)
    pub port: u16,
    /// Metrics port (default: 9090)
    pub metrics_port: u16,
    /// gRPC port for sandbox communication (default: 50051)
    pub grpc_port: u16,
}

/// A node-local package-manager cache mounted into sandbox pods.
///
/// Each entry maps to a `subPath` (`<cache_host_root>/<name>`) on the single
/// shared `pkg-cache` volume, surfaced in the container at `mount_path`, with
/// `env` pointing the tool at it (e.g.
/// `MAVEN_OPTS=-Dmaven.repo.local=/maven-cache`). This is backend-agnostic: only
/// the volume source (node-local hostPath today, EFS later) changes — the
/// per-entry mount/env logic is reused either way.
///
/// Caches each tool's *download cache/store* (maven local repo, npm `_cacache`,
/// bun cache, pnpm store) — never per-project `node_modules`, which must not be
/// shared across repos.
#[derive(Debug, Clone)]
pub struct SandboxCacheVolume {
    /// Cache name → subdirectory under the shared cache root (e.g. `maven`).
    pub name: String,
    /// In-container mount path (e.g. `/maven-cache`).
    pub mount_path: String,
    /// Env vars that point the tool at the cache, sorted by key.
    pub env: Vec<(String, String)>,
}

/// Sandbox pod configuration (images, resources, storage, idle reaper).
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Kubernetes namespace for sandbox pods
    pub namespace: String,
    /// Default sandbox container image
    pub image: String,
    /// Image pull policy for sandbox pod containers (default: "Always")
    pub image_pull_policy: String,
    /// gRPC port on sandbox pods (default: 50052)
    pub grpc_port: u16,
    /// CPU limit for server containers
    pub resources_cpu_limit: String,
    /// Memory limit for server containers
    pub resources_memory_limit: String,
    /// CPU request for server containers
    pub resources_cpu_request: String,
    /// Memory request for server containers
    pub resources_memory_request: String,
    /// CPU request for the sandbox (agent) container (env `K8S_RESOURCE_REQUEST_CPU`)
    pub sandbox_resources_cpu_request: String,
    /// CPU limit for the sandbox (agent) container (env `K8S_RESOURCE_LIMIT_CPU`)
    pub sandbox_resources_cpu_limit: String,
    /// Memory request for the sandbox (agent) container (env `K8S_RESOURCE_REQUEST_MEMORY`)
    pub sandbox_resources_memory_request: String,
    /// Memory limit for the sandbox (agent) container (env `K8S_RESOURCE_LIMIT_MEMORY`)
    pub sandbox_resources_memory_limit: String,
    /// Run the sandbox and server containers as the non-root `moatless` user
    /// (UID/GID 1000) via a pod-level securityContext (`runAsNonRoot`, `fsGroup`,
    /// `seccompProfile: RuntimeDefault`). The privileged `dind` sidecar and
    /// `nvme-setup` init container keep running as root. Defaults to `false` so
    /// the hardening can roll out per-environment. Env
    /// `K8S_SANDBOX_RUN_AS_NON_ROOT`.
    pub run_as_non_root: bool,
    /// Size cap for the `shared-data` emptyDir mounted at `/opt/moatless/data`
    /// across the sandbox and server containers. Without a cap this emptyDir can
    /// grow unbounded and evict the node. Env `K8S_SANDBOX_SHARED_DATA_SIZE`
    /// (default `2Gi`).
    pub shared_data_size: String,
    /// Whether PVC storage is enabled for sandbox pods
    pub storage_enabled: bool,
    /// PVC storage size (e.g., "3Gi")
    pub storage_size: String,
    /// PVC storage class
    pub storage_class: Option<String>,
    /// Default workspace storage backend for repos that don't set
    /// `resources.storage_mode` (env `K8S_SANDBOX_DEFAULT_STORAGE_MODE`).
    /// One of `ebs` / `ephemeral` / `s3`; unknown values fall back to `ebs`.
    /// A per-repo `storage_mode` always overrides this.
    pub default_storage_mode: String,
    /// Pod startup timeout in seconds
    pub pod_startup_timeout_secs: u64,
    /// Secret for signing task auth JWTs
    pub task_token_secret: Option<String>,
    /// TTL (seconds) for the long-lived user auth token delivered to sandboxes
    /// over `GetCredentials`. Re-minted on every credential fetch, so this only
    /// needs to outlive the 15-min refresher gap. Env `SANDBOX_USER_TOKEN_TTL_SECS`
    /// (default: 604800 = 7 days). Replaces the former hardcoded 90-day expiry.
    pub user_token_ttl_secs: u64,
    /// TTL (seconds) for the short-lived bootstrap token placed on the pod spec.
    /// It only authenticates the sandbox's initial `GetCredentials` call, so a
    /// minutes-scale value bounds the blast radius of the one token still in
    /// etcd. Env `SANDBOX_BOOTSTRAP_TOKEN_TTL_SECS` (default: 1800 = 30 min).
    pub bootstrap_token_ttl_secs: u64,
    /// Whether to also emit the long-lived `USER_AUTH_TOKEN` env var on the pod
    /// spec (in addition to the bootstrap token). Kept `true` during the
    /// sandbox-image rollout so pods still running the old image — which read the
    /// token from env — keep working. Flip to `false` once every sandbox image
    /// consumes the credential-channel token, removing the long-lived secret from
    /// etcd entirely. Env `SANDBOX_EMIT_LEGACY_USER_TOKEN` (default: true).
    pub emit_legacy_user_token_env: bool,
    /// Mount path inside sandbox pods for uploaded files (default: "/mnt/uploads")
    pub file_mount_path: String,
    /// Whether the idle sandbox reaper is enabled (default: true)
    pub idle_reaper_enabled: bool,
    /// Stop sandboxes idle longer than this (minutes, default: 15)
    pub idle_timeout_minutes: u64,
    /// How often to check for idle sandboxes (seconds, default: 300 = 5 min)
    pub idle_check_interval_secs: u64,
    /// Auto-close Open tasks inactive for this many days. `0` disables
    /// auto-close. Env: `TASK_INACTIVE_TIMEOUT_DAYS` (default: 7).
    pub inactive_task_timeout_days: u64,
    /// Whether the reaper's orphan-GC pass is enabled (default: true). When on,
    /// the reaper deletes labeled sandbox K8s resources (Deployment/Service/
    /// IngressRoute/PVC) whose task no longer exists or was closed past the
    /// retention window. Env: `SANDBOX_ORPHAN_GC_ENABLED`.
    pub orphan_gc_enabled: bool,
    /// Retention window (hours) before a closed task's leftover sandbox
    /// resources are orphan-GC'd. Resources for tasks deleted from the DB are
    /// collected immediately regardless of this window. Env:
    /// `SANDBOX_ORPHAN_GC_RETENTION_HOURS` (default: 24).
    pub orphan_gc_retention_hours: u64,
    /// How often the background Codex refresher sweeps `provider_tokens`.
    /// Setting this to 0 disables the sweep entirely (env-var fallback
    /// still works; just no proactive backend refresh). Env:
    /// `CODEX_REFRESHER_INTERVAL_SECS` (default: 3600 = 1h).
    pub codex_refresher_interval_secs: u64,
    /// Maximum container restart count before auto-stopping a crash-looping sandbox (default: 5).
    /// Set to 0 to disable crash-loop auto-stop.
    pub max_crash_restart_count: i32,
    /// JSON-encoded tolerations for sandbox pods (e.g., `[{"key":"kubernetes.azure.com/scalesetpriority","operator":"Equal","value":"spot","effect":"NoSchedule"}]`)
    pub tolerations_json: Option<String>,
    /// JSON-encoded node selector for sandbox pods (e.g., `{"karpenter.sh/capacity-type":"spot"}`)
    pub node_selector_json: Option<String>,
    /// Kubernetes ServiceAccount name for sandbox pods (set via K8S_SANDBOX_SERVICE_ACCOUNT)
    pub service_account_name: Option<String>,
    /// Default Claude model for sandbox agents (forwarded as DEFAULT_CLAUDE_CODE_MODEL env var)
    pub default_model: Option<String>,
    /// Default Codex model for sandbox agents (forwarded as DEFAULT_CODEX_MODEL env var)
    pub default_codex_model: Option<String>,
    /// Extra env vars injected into every sandbox pod (parsed from K8S_SANDBOX_EXTRA_ENV JSON)
    pub extra_env: Vec<(String, String)>,
    /// Generic package-manager caches mounted into sandbox pods (parsed from
    /// `K8S_SANDBOX_CACHE_VOLUMES` JSON). Empty disables the feature entirely —
    /// no extra volume, no init container. See [`SandboxCacheVolume`].
    pub cache_volumes: Vec<SandboxCacheVolume>,
    /// Host directory the shared `pkg-cache` volume is rooted at; the node-local
    /// NVMe is formatted and mounted here by the `nvme-setup` init container, and
    /// each cache lives in a `<root>/<name>` subdir. Env
    /// `K8S_SANDBOX_CACHE_HOST_ROOT` (default `/mnt/cache`).
    pub cache_host_root: String,
    /// Image for the privileged `nvme-setup` init container that formats/mounts
    /// the instance-store NVMe. Must be an allowlisted ref under the
    /// `restrict-image-registries` Kyverno policy (the ECR-mirrored dind image).
    /// Env `K8S_SANDBOX_CACHE_INIT_IMAGE`.
    pub cache_init_image: String,
    /// Image for the privileged `dind` (docker-in-docker) sidecar attached to
    /// sandbox pods whose repo config sets `dockerAccess: true`. This is what
    /// lets Testcontainers-based tests run inside the sandbox. Must be an
    /// allowlisted ref under the `restrict-image-registries` Kyverno policy
    /// (the ECR-mirrored dind image). Env `K8S_SANDBOX_DOCKER_DIND_IMAGE`.
    pub docker_dind_image: String,
    /// Size cap for the `dind` sidecar's `/var/lib/docker` emptyDir (image
    /// layer + container storage). Env `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE`
    /// (default `10Gi`).
    pub docker_dind_storage_size: String,
    /// CPU request for the `dind` sidecar. Env `K8S_SANDBOX_DOCKER_DIND_CPU_REQUEST`.
    pub docker_dind_cpu_request: String,
    /// CPU limit for the `dind` sidecar. Env `K8S_SANDBOX_DOCKER_DIND_CPU_LIMIT`.
    pub docker_dind_cpu_limit: String,
    /// Memory request for the `dind` sidecar. Env `K8S_SANDBOX_DOCKER_DIND_MEMORY_REQUEST`.
    pub docker_dind_memory_request: String,
    /// Memory limit for the `dind` sidecar. Containers started by Testcontainers
    /// run as children of dockerd inside this container, so their memory charges
    /// against this limit. Env `K8S_SANDBOX_DOCKER_DIND_MEMORY_LIMIT`.
    pub docker_dind_memory_limit: String,
}

/// Proxy / Traefik IngressRoute settings.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Domain for proxy URLs (e.g., "moatless.xyz"). If None, IngressRoutes are not created.
    pub domain: Option<String>,
    /// Protocol for proxy URLs (default: "https")
    pub protocol: String,
    /// Name of the Traefik ForwardAuth Middleware CRD (default: "preview-auth")
    pub traefik_auth_middleware_name: String,
    /// Namespace where the Traefik auth Middleware lives (defaults to sandbox namespace)
    pub traefik_auth_middleware_namespace: String,
    /// Traefik entryPoint name for IngressRoutes (default: "websecure")
    pub traefik_entry_point: String,
}

/// Redis connection and stream settings.
#[derive(Debug, Clone)]
pub struct RedisConfig {
    /// Redis connection URL. When set, enables event publishing and SSE consumers.
    pub url: Option<String>,
    /// Maximum stream length for MAXLEN ~ trimming (default: 10000)
    pub max_stream_length: usize,
}

/// SSE (Server-Sent Events) settings.
#[derive(Debug, Clone)]
pub struct SseConfig {
    /// Broadcast buffer capacity for SSE fan-out (default: 4096)
    pub broadcast_capacity: usize,
    /// Consumer ID for the SSE Redis consumer group (default: "sse-{HOSTNAME}")
    pub consumer_id: String,
}

/// Authentication and session settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone)]
pub struct AuthConfig {
    /// Auth provider, or None (dev mode - auth disabled)
    pub provider: Option<AuthProvider>,
    /// GitHub OAuth client ID
    pub github_client_id: Option<String>,
    /// GitHub OAuth client secret
    pub github_client_secret: Option<String>,
    /// GitHub OAuth callback URL
    pub github_callback_url: Option<String>,
    /// GitHub OAuth scopes (default: "user:email repo")
    pub github_oauth_scope: String,
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
    /// `review-bot`). Loaded from the `GITHUB_APPS_JSON` env var which holds a
    /// JSON array of [`GitHubAppConfig`] entries. A bot user's
    /// `provider_tokens` row stores the matching `key` in `provider_user_id`,
    /// so credential resolution picks the right App.
    pub github_apps: Vec<GitHubAppConfig>,
}

/// One GitHub App registration, used by the `GitHubAppRegistry`.
#[derive(Clone, Deserialize)]
pub struct GitHubAppConfig {
    /// Stable key referenced from trigger config (e.g. `"dev-bot"`).
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
            .field("github_client_id", &self.github_client_id)
            .field(
                "github_client_secret",
                &self.github_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("github_callback_url", &self.github_callback_url)
            .field("auth0_domain", &self.auth0_domain)
            .field("auth0_client_id", &self.auth0_client_id)
            .field(
                "auth0_client_secret",
                &self.auth0_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("auth0_callback_url", &self.auth0_callback_url)
            .field("session_max_age_secs", &self.session_max_age_secs)
            .field("frontend_url", &self.frontend_url)
            .field("cookie_domain", &self.cookie_domain)
            .field("is_production", &self.is_production)
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

/// S3 / file storage settings.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// S3 bucket name for file uploads.
    pub bucket_name: Option<String>,
    /// S3 key prefix for uploaded files (default: "uploads")
    pub key_prefix: String,
    /// S3 key prefix for sandbox Claude transcripts, stored under
    /// `<prefix>/<task_id>/transcripts.tar` (default: "transcripts").
    /// Reuses the uploads bucket — the backend IRSA already has read/write on it.
    /// Env: `TRANSCRIPT_S3_PREFIX`.
    pub transcript_prefix: String,
    /// S3 key prefix for the `s3` storage backend's workspace archives, stored
    /// under `<prefix>/<task_id>/workspace.tar`. Reuses the uploads bucket. The
    /// sandbox never gets S3 credentials — the backend presigns PUT/GET URLs
    /// scoped to this prefix and the sandbox transfers directly. Env:
    /// `WORKSPACE_S3_PREFIX` (default: "workspaces").
    pub workspace_prefix: String,
    /// S3 region (default: "us-east-1")
    pub region: String,
}

/// Slack adapter settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone)]
pub struct SlackConfig {
    /// Slack bot token
    pub bot_token: Option<String>,
    /// Slack signing secret for webhook verification
    pub signing_secret: Option<String>,
    /// Slack bot user ID for filtering own messages
    pub bot_user_id: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for SlackConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlackConfig")
            .field("bot_token", &self.bot_token.as_ref().map(|_| "[REDACTED]"))
            .field(
                "signing_secret",
                &self.signing_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("bot_user_id", &self.bot_user_id)
            .finish()
    }
}

/// Telegram Bot API integration settings.
///
/// Implements a custom `Debug` to redact the bot token.
#[derive(Clone)]
pub struct TelegramConfig {
    /// Telegram bot token from BotFather.
    pub bot_token: Option<String>,
    /// Ingress mode: "webhook" (default) or "polling".
    pub mode: String,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for TelegramConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramConfig")
            .field("bot_token", &self.bot_token.as_ref().map(|_| "[REDACTED]"))
            .field("mode", &self.mode)
            .finish()
    }
}

/// GitHub PR adapter settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone)]
pub struct GithubPrConfig {
    /// Global webhook secret for verifying GitHub webhook signatures.
    pub webhook_secret: Option<String>,
    /// Global GitHub token for API calls (PAT or GitHub App installation token).
    pub bot_token: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for GithubPrConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubPrConfig")
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("bot_token", &self.bot_token.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Linear adapter settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone)]
pub struct LinearConfig {
    /// Global webhook secret for verifying Linear webhook signatures.
    pub webhook_secret: Option<String>,
    /// OAuth client_id from the Linear OAuth2 app registration.
    pub oauth_client_id: Option<String>,
    /// OAuth client_secret from the Linear OAuth2 app registration.
    pub oauth_client_secret: Option<String>,
    /// OAuth redirect_uri registered on the Linear OAuth2 app.
    pub oauth_callback_url: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for LinearConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinearConfig")
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("oauth_client_id", &self.oauth_client_id)
            .field(
                "oauth_client_secret",
                &self.oauth_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("oauth_callback_url", &self.oauth_callback_url)
            .finish()
    }
}

/// Docker-specific sandbox configuration.
#[derive(Debug, Clone)]
pub struct DockerConfig {
    /// Docker network name for sandbox containers (default: "moatless")
    pub network: String,
    /// Default sandbox container image for Docker mode
    pub image: String,
    /// Host path for shared data volumes — must be the absolute host path
    /// because sandbox containers are siblings, not children
    pub data_dir: String,
    /// Container-internal path where data is mounted (for backend file ops)
    pub container_data_dir: String,
    /// Backend container hostname on the Docker network (for sandbox → backend gRPC)
    pub backend_hostname: String,
    /// Mount Docker socket in sandbox containers (default: false)
    pub enable_socket_mount: bool,
    /// Host path to Docker socket (default: "/var/run/docker.sock")
    pub socket_path: String,
    /// Additional Docker networks to connect sandbox containers to
    pub extra_networks: Vec<String>,
}

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL. When set, enables DB-backed features.
    pub url: Option<String>,
    /// Max connections in the pool (default: 5)
    pub max_connections: u32,
}

/// Skill discovery configuration.
#[derive(Debug, Clone)]
pub struct SkillConfig {
    /// In-memory cache TTL for parsed skill lists.
    pub cache_ttl_secs: u64,
}

/// Codex (OpenAI) device-code OAuth configuration.
///
/// Defaults reuse the Codex CLI's public client_id so the flow works out of
/// the box. Override via env vars to register a Moatless-owned OAuth client.
#[derive(Debug, Clone)]
pub struct CodexOAuthConfig {
    /// OAuth issuer base URL (default: `https://auth.openai.com`).
    pub issuer: String,
    /// OAuth client_id (default: Codex CLI's `app_EMoamEEZ73f0CkXaXp7hrann`).
    pub client_id: String,
}

/// Secrets feature bootstrap configuration.
///
/// Implements a custom `Debug` to redact encryption material.
#[derive(Clone)]
pub struct SecretsConfig {
    /// Base64 or raw 32-byte key used by the local envelope cipher.
    ///
    /// This is a bootstrap value: it stays outside the database and is only
    /// used until the deployment is moved to an external KMS-backed cipher.
    pub encryption_key: Option<String>,
    /// KMS key identifier or alias recorded on encrypted rows.
    pub kms_key_id: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for SecretsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretsConfig")
            .field(
                "encryption_key",
                &self.encryption_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("kms_key_id", &self.kms_key_id)
            .finish()
    }
}

/// Desired-vs-actual reconcile pass in the task reaper (Phase 4).
///
/// A slow, level-based backstop to the edge-triggered pod watcher. Runs inside
/// the existing leader-elected reaper cycle, reusing its cluster-wide
/// Deployments LIST. Each remediation class is independently gated and defaults
/// OFF so the pass ships dark and is turned on deliberately per environment.
#[derive(Debug, Clone)]
pub struct ReconcileConfig {
    /// Re-provision a Deployment that vanished out-of-band (e.g. deleted
    /// manually, or a watch event lost across a reconnect) for an Open task
    /// with an in-flight turn. Env `RECONCILE_REPROVISION_ENABLED` (default false).
    pub reprovision_enabled: bool,
    /// Scale a Deployment to 0 and surface an environment error when its pod is
    /// stuck `Pending` past the hard deadline (today they linger forever). Env
    /// `RECONCILE_PENDING_TIMEOUT_ENABLED` (default false).
    pub pending_timeout_enabled: bool,
    /// Hard deadline (minutes) a pod may sit `Pending` before the pending-timeout
    /// remediation fires. Env `RECONCILE_PENDING_TIMEOUT_MINUTES` (default 15).
    pub pending_timeout_minutes: u64,
    /// Max re-provision attempts per task (per leader lifetime) before giving up
    /// and surfacing an environment error. Env `RECONCILE_MAX_REPROVISION_ATTEMPTS`
    /// (default 3).
    pub max_reprovision_attempts: u32,
}

// =========================================================================
// Top-level Config
// =========================================================================

/// Top-level configuration grouping all sub-configs.
#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    /// Which sandbox provider to use (Kubernetes or Docker).
    pub sandbox_provider: SandboxProviderType,
    pub sandbox: SandboxConfig,
    /// Desired-vs-actual reconcile pass in the task reaper (all classes default OFF).
    pub reconcile: ReconcileConfig,
    pub docker: DockerConfig,
    pub proxy: ProxyConfig,
    /// PostgreSQL connection URL. When set, enables DB-backed features.
    pub database_url: Option<String>,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub sse: SseConfig,
    pub auth: AuthConfig,
    pub s3: S3Config,
    pub slack: SlackConfig,
    pub telegram: TelegramConfig,
    pub github_pr: GithubPrConfig,
    pub linear: LinearConfig,
    pub skill: SkillConfig,
    pub secrets: SecretsConfig,
    pub codex_oauth: CodexOAuthConfig,
    /// Global default user id that synced repo "loops" run as when a loop's
    /// `LOOP.md` frontmatter omits `runAsUserId` and the repo has no owner.
    /// Env: `MOATLESS_DEFAULT_RUN_AS_USER_ID`.
    pub default_run_as_user_id: Option<String>,
}

/// Configuration validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Invalid configuration: {0}")]
    Invalid(String),
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Load .env file if present (ignored in production)
        let _ = dotenvy::dotenv();

        let sandbox_namespace = env_str("SANDBOX_NAMESPACE", "workspaces");
        let database_url = env_opt("DATABASE_URL");

        let sandbox_provider = env_opt("SANDBOX_PROVIDER")
            .and_then(|v| v.parse().ok())
            .unwrap_or(SandboxProviderType::Kubernetes);

        let config = Self {
            server: ServerConfig {
                port: env_or("PORT", 8080),
                metrics_port: env_or("METRICS_PORT", 9090),
                grpc_port: env_or("GRPC_PORT", 50051),
            },

            sandbox_provider,

            sandbox: SandboxConfig {
                namespace: sandbox_namespace.clone(),
                image: env_str("SANDBOX_IMAGE", "aorwall/moatless-vibe-sandbox:latest"),
                image_pull_policy: env_str("SANDBOX_IMAGE_PULL_POLICY", "Always"),
                grpc_port: env_or("SANDBOX_GRPC_PORT", 50052),
                resources_cpu_limit: env_str("SERVER_RESOURCES_CPU_LIMIT", "1000m"),
                resources_memory_limit: env_str("SERVER_RESOURCES_MEMORY_LIMIT", "1536Mi"),
                resources_cpu_request: env_str("SERVER_RESOURCES_CPU_REQUEST", "200m"),
                resources_memory_request: env_str("SERVER_RESOURCES_MEMORY_REQUEST", "256Mi"),
                sandbox_resources_cpu_request: env_str("K8S_RESOURCE_REQUEST_CPU", "1"),
                sandbox_resources_cpu_limit: env_str("K8S_RESOURCE_LIMIT_CPU", "4"),
                sandbox_resources_memory_request: env_str("K8S_RESOURCE_REQUEST_MEMORY", "2Gi"),
                sandbox_resources_memory_limit: env_str("K8S_RESOURCE_LIMIT_MEMORY", "8Gi"),
                run_as_non_root: env_bool("K8S_SANDBOX_RUN_AS_NON_ROOT", false),
                shared_data_size: env_str("K8S_SANDBOX_SHARED_DATA_SIZE", "2Gi"),
                storage_enabled: env_bool("K8S_SANDBOX_STORAGE_ENABLED", true),
                storage_size: env_str("K8S_SANDBOX_STORAGE_SIZE", "3Gi"),
                storage_class: env_opt("K8S_SANDBOX_STORAGE_CLASS"),
                default_storage_mode: env_str("K8S_SANDBOX_DEFAULT_STORAGE_MODE", "ebs"),
                pod_startup_timeout_secs: env_or("K8S_POD_STARTUP_TIMEOUT_SECS", 120),
                task_token_secret: env_opt("TASK_TOKEN_SECRET"),
                user_token_ttl_secs: env_or("SANDBOX_USER_TOKEN_TTL_SECS", 7 * 24 * 3600),
                bootstrap_token_ttl_secs: env_or("SANDBOX_BOOTSTRAP_TOKEN_TTL_SECS", 1800),
                emit_legacy_user_token_env: env_bool("SANDBOX_EMIT_LEGACY_USER_TOKEN", true),
                file_mount_path: env_str("SANDBOX_FILE_MOUNT_PATH", "/mnt/uploads"),
                idle_reaper_enabled: env_bool("SANDBOX_IDLE_REAPER_ENABLED", true),
                idle_timeout_minutes: env_or("SANDBOX_IDLE_TIMEOUT_MINUTES", 15),
                idle_check_interval_secs: env_or("SANDBOX_IDLE_CHECK_INTERVAL_SECS", 300),
                inactive_task_timeout_days: env_or("TASK_INACTIVE_TIMEOUT_DAYS", 7),
                orphan_gc_enabled: env_bool("SANDBOX_ORPHAN_GC_ENABLED", true),
                orphan_gc_retention_hours: env_or("SANDBOX_ORPHAN_GC_RETENTION_HOURS", 24),
                codex_refresher_interval_secs: env_or("CODEX_REFRESHER_INTERVAL_SECS", 3600),
                max_crash_restart_count: env_or("SANDBOX_MAX_CRASH_RESTART_COUNT", 5),
                tolerations_json: env_opt("K8S_SANDBOX_TOLERATIONS"),
                node_selector_json: env_opt("K8S_SANDBOX_NODE_SELECTOR"),
                service_account_name: env_opt("K8S_SANDBOX_SERVICE_ACCOUNT"),
                default_model: env_opt("DEFAULT_CLAUDE_CODE_MODEL"),
                default_codex_model: env_opt("DEFAULT_CODEX_MODEL"),
                extra_env: parse_extra_env("K8S_SANDBOX_EXTRA_ENV"),
                cache_volumes: parse_cache_volumes("K8S_SANDBOX_CACHE_VOLUMES"),
                cache_host_root: env_str("K8S_SANDBOX_CACHE_HOST_ROOT", "/mnt/cache"),
                cache_init_image: env_str(
                    "K8S_SANDBOX_CACHE_INIT_IMAGE",
                    "111111111111.dkr.ecr.eu-north-1.amazonaws.com/mirror/docker.io/library/docker:29-dind",
                ),
                docker_dind_image: env_str(
                    "K8S_SANDBOX_DOCKER_DIND_IMAGE",
                    "111111111111.dkr.ecr.eu-north-1.amazonaws.com/mirror/docker.io/library/docker:29-dind",
                ),
                docker_dind_storage_size: env_str(
                    "K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE",
                    "10Gi",
                ),
                docker_dind_cpu_request: env_str("K8S_SANDBOX_DOCKER_DIND_CPU_REQUEST", "500m"),
                docker_dind_cpu_limit: env_str("K8S_SANDBOX_DOCKER_DIND_CPU_LIMIT", "2"),
                docker_dind_memory_request: env_str(
                    "K8S_SANDBOX_DOCKER_DIND_MEMORY_REQUEST",
                    "1Gi",
                ),
                docker_dind_memory_limit: env_str("K8S_SANDBOX_DOCKER_DIND_MEMORY_LIMIT", "4Gi"),
            },

            reconcile: ReconcileConfig {
                reprovision_enabled: env_bool("RECONCILE_REPROVISION_ENABLED", false),
                pending_timeout_enabled: env_bool("RECONCILE_PENDING_TIMEOUT_ENABLED", false),
                pending_timeout_minutes: env_or("RECONCILE_PENDING_TIMEOUT_MINUTES", 15),
                max_reprovision_attempts: env_or("RECONCILE_MAX_REPROVISION_ATTEMPTS", 3),
            },

            proxy: ProxyConfig {
                domain: env_opt("PROXY_DOMAIN"),
                protocol: env_str("PROXY_PROTOCOL", "https"),
                traefik_auth_middleware_name: env_str(
                    "TRAEFIK_AUTH_MIDDLEWARE_NAME",
                    "preview-auth",
                ),
                traefik_auth_middleware_namespace: env::var("TRAEFIK_AUTH_MIDDLEWARE_NAMESPACE")
                    .unwrap_or(sandbox_namespace),
                traefik_entry_point: env_str("TRAEFIK_ENTRY_POINT", "websecure"),
            },

            docker: DockerConfig {
                network: env_str("DOCKER_NETWORK", "moatless"),
                image: env_str(
                    "DOCKER_SANDBOX_IMAGE",
                    "aorwall/moatless-vibe-sandbox-js:latest",
                ),
                data_dir: env_str("DATA_DIR", "/data"),
                container_data_dir: env_str("CONTAINER_DATA_DIR", "/data"),
                backend_hostname: env_str("DOCKER_BACKEND_HOSTNAME", "moatless-backend"),
                enable_socket_mount: env_bool("DOCKER_ENABLE_SOCKET_MOUNT", false),
                socket_path: env_str("DOCKER_SOCKET_PATH", "/var/run/docker.sock"),
                extra_networks: env_opt("DOCKER_EXTRA_NETWORKS")
                    .map(|s| {
                        s.split(',')
                            .map(|n| n.trim().to_string())
                            .filter(|n| !n.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
            },

            database_url: database_url.clone(),

            database: DatabaseConfig {
                url: database_url,
                max_connections: env_or("DATABASE_MAX_CONNECTIONS", 5),
            },

            redis: RedisConfig {
                url: env_opt("REDIS_URL"),
                max_stream_length: env_or("REDIS_MAX_STREAM_LENGTH", 10_000),
            },

            sse: SseConfig {
                broadcast_capacity: env_or("SSE_BROADCAST_CAPACITY", 4096),
                consumer_id: env::var("SSE_CONSUMER_ID").unwrap_or_else(|_| {
                    let hostname = env_str("HOSTNAME", "local");
                    format!("sse-{hostname}")
                }),
            },

            auth: AuthConfig {
                provider: env_opt("AUTH_PROVIDER")
                    .filter(|v| v != "disabled")
                    .and_then(|v| v.parse().ok()),
                github_client_id: env_opt("GITHUB_CLIENT_ID"),
                github_client_secret: env_opt("GITHUB_CLIENT_SECRET"),
                github_callback_url: env_opt("GITHUB_CALLBACK_URL"),
                github_oauth_scope: env_str("GITHUB_OAUTH_SCOPE", "user:email repo"),
                github_required_org: env_opt("GITHUB_REQUIRED_ORG"),
                auth0_domain: env_opt("AUTH0_DOMAIN"),
                auth0_client_id: env_opt("AUTH0_CLIENT_ID"),
                auth0_client_secret: env_opt("AUTH0_CLIENT_SECRET"),
                auth0_callback_url: env_opt("AUTH0_CALLBACK_URL"),
                auth0_audience: env_opt("AUTH0_AUDIENCE"),
                auth0_groups_claim: env_str("AUTH0_GROUPS_CLAIM", "https://moatless.ai/groups"),
                session_max_age_secs: env_or("SESSION_MAX_AGE", 604_800),
                frontend_url: env_str("FRONTEND_URL", "http://localhost:3000"),
                cookie_domain: env_opt("COOKIE_DOMAIN"),
                session_cookie_name: env_str("SESSION_COOKIE_NAME", "moatless_session"),
                is_production: env::var("APP_ENV")
                    .or_else(|_| env::var("NODE_ENV"))
                    .map(|v| v == "production")
                    .unwrap_or(false),
                cache_ttl_secs: env_or("AUTH_CACHE_TTL_SECS", 30),
                session_jwt_secret: env_opt("SESSION_JWT_SECRET"),
                test_api_key: env_opt("TEST_API_KEY").map(|s| s.trim().to_string()),
                default_team_slug: env_opt("DEFAULT_TEAM_SLUG"),

                github_app_client_id: env_opt("GITHUB_APP_CLIENT_ID"),
                github_app_client_secret: env_opt("GITHUB_APP_CLIENT_SECRET"),
                github_app_callback_url: env_opt("GITHUB_APP_CALLBACK_URL"),
                github_app_id: env_opt("GITHUB_APP_ID").and_then(|v| v.parse().ok()),
                github_app_private_key: env_opt("GITHUB_APP_PRIVATE_KEY")
                    .map(|s| s.replace("\\n", "\n")),
                github_app_default_installation_id: env_opt("GITHUB_APP_DEFAULT_INSTALLATION_ID")
                    .and_then(|v| v.parse().ok()),
                github_apps: parse_github_apps_json(env_opt("GITHUB_APPS_JSON")),
            },

            s3: S3Config {
                bucket_name: env_opt("S3_BUCKET_NAME"),
                key_prefix: env_str("S3_KEY_PREFIX", "uploads"),
                transcript_prefix: env_str("TRANSCRIPT_S3_PREFIX", "transcripts"),
                workspace_prefix: env_str("WORKSPACE_S3_PREFIX", "workspaces"),
                region: env_str("S3_REGION", "us-east-1"),
            },

            slack: SlackConfig {
                bot_token: env_opt("SLACK_BOT_TOKEN"),
                signing_secret: env_opt("SLACK_SIGNING_SECRET"),
                bot_user_id: env_opt("SLACK_BOT_USER_ID"),
            },

            telegram: TelegramConfig {
                bot_token: env_opt("TELEGRAM_BOT_TOKEN"),
                mode: env_opt("TELEGRAM_MODE").unwrap_or_else(|| "webhook".into()),
            },

            github_pr: GithubPrConfig {
                webhook_secret: env_opt("GITHUB_CHANNEL_WEBHOOK_SECRET"),
                bot_token: env_opt("GITHUB_CHANNEL_TOKEN"),
            },

            linear: LinearConfig {
                webhook_secret: env_opt("LINEAR_CHANNEL_WEBHOOK_SECRET"),
                oauth_client_id: env_opt("LINEAR_OAUTH_CLIENT_ID"),
                oauth_client_secret: env_opt("LINEAR_OAUTH_CLIENT_SECRET"),
                oauth_callback_url: env_opt("LINEAR_OAUTH_CALLBACK_URL"),
            },

            skill: SkillConfig {
                cache_ttl_secs: env_or("BACKEND_SKILL_CACHE_TTL_SECONDS", 3_600),
            },

            secrets: SecretsConfig {
                encryption_key: env_opt("SECRETS_ENCRYPTION_KEY"),
                kms_key_id: env_opt("SECRETS_KMS_KEY_ID").or_else(|| env_opt("AWS_KMS_KEY_ID")),
            },

            codex_oauth: CodexOAuthConfig {
                issuer: env_str("CODEX_OAUTH_ISSUER", "https://auth.openai.com"),
                client_id: env_str("CODEX_OAUTH_CLIENT_ID", "app_EMoamEEZ73f0CkXaXp7hrann"),
            },

            default_run_as_user_id: env_opt("MOATLESS_DEFAULT_RUN_AS_USER_ID"),
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration consistency.
    ///
    /// Made `pub(crate)` so tests can call it directly without env vars.
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        // Validate auth provider config consistency
        if let Some(AuthProvider::GitHub) = self.auth.provider {
            if self.auth.github_client_id.is_none() {
                return Err(ConfigError::Invalid(
                    "GITHUB_CLIENT_ID required when AUTH_PROVIDER=github".into(),
                ));
            }
            if self.auth.github_client_secret.is_none() {
                return Err(ConfigError::Invalid(
                    "GITHUB_CLIENT_SECRET required when AUTH_PROVIDER=github".into(),
                ));
            }
        }
        if let Some(AuthProvider::GitHubApp) = self.auth.provider {
            if self.auth.github_app_client_id.is_none() {
                return Err(ConfigError::Invalid(
                    "GITHUB_APP_CLIENT_ID required when AUTH_PROVIDER=github_app".into(),
                ));
            }
            if self.auth.github_app_client_secret.is_none() {
                return Err(ConfigError::Invalid(
                    "GITHUB_APP_CLIENT_SECRET required when AUTH_PROVIDER=github_app".into(),
                ));
            }
            if self.auth.github_app_callback_url.is_none() {
                return Err(ConfigError::Invalid(
                    "GITHUB_APP_CALLBACK_URL required when AUTH_PROVIDER=github_app".into(),
                ));
            }
        }
        if let Some(AuthProvider::Auth0) = self.auth.provider {
            if self.auth.auth0_domain.is_none() {
                return Err(ConfigError::Invalid(
                    "AUTH0_DOMAIN required when AUTH_PROVIDER=auth0".into(),
                ));
            }
            if self.auth.auth0_client_id.is_none() {
                return Err(ConfigError::Invalid(
                    "AUTH0_CLIENT_ID required when AUTH_PROVIDER=auth0".into(),
                ));
            }
            if self.auth.auth0_client_secret.is_none() {
                return Err(ConfigError::Invalid(
                    "AUTH0_CLIENT_SECRET required when AUTH_PROVIDER=auth0".into(),
                ));
            }
        }

        // Validate test API key format
        if let Some(ref key) = self.auth.test_api_key {
            if !key.starts_with("mvk_") {
                return Err(ConfigError::Invalid(
                    "TEST_API_KEY must start with 'mvk_' prefix".into(),
                ));
            }
            if key.len() < 36 {
                return Err(ConfigError::Invalid(
                    "TEST_API_KEY must be at least 36 characters".into(),
                ));
            }
        }

        // Provider-specific validation
        match self.sandbox_provider {
            SandboxProviderType::Kubernetes => {
                if self.sandbox.namespace.is_empty() {
                    return Err(ConfigError::Invalid(
                        "SANDBOX_NAMESPACE cannot be empty".into(),
                    ));
                }
            }
            SandboxProviderType::Docker => {
                if self.docker.network.is_empty() {
                    return Err(ConfigError::Invalid(
                        "DOCKER_NETWORK cannot be empty when SANDBOX_PROVIDER=docker".into(),
                    ));
                }
                if self.docker.data_dir.is_empty() {
                    return Err(ConfigError::Invalid(
                        "DATA_DIR cannot be empty when SANDBOX_PROVIDER=docker".into(),
                    ));
                }
            }
        }

        // SECRETS_ENCRYPTION_KEY is always required
        if self.secrets.encryption_key.is_none() {
            return Err(ConfigError::Invalid(
                "SECRETS_ENCRYPTION_KEY is required".into(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // AuthProvider parsing
    // =========================================================================

    #[test]
    fn auth_provider_from_str_github() {
        assert_eq!(
            "github".parse::<AuthProvider>().unwrap(),
            AuthProvider::GitHub
        );
        assert_eq!(
            "GitHub".parse::<AuthProvider>().unwrap(),
            AuthProvider::GitHub
        );
        assert_eq!(
            "GITHUB".parse::<AuthProvider>().unwrap(),
            AuthProvider::GitHub
        );
    }

    #[test]
    fn auth_provider_from_str_auth0() {
        assert_eq!(
            "auth0".parse::<AuthProvider>().unwrap(),
            AuthProvider::Auth0
        );
        assert_eq!(
            "Auth0".parse::<AuthProvider>().unwrap(),
            AuthProvider::Auth0
        );
    }

    #[test]
    fn auth_provider_from_str_unknown_returns_error() {
        assert!("unknown".parse::<AuthProvider>().is_err());
        assert!("".parse::<AuthProvider>().is_err());
    }

    #[test]
    fn auth_provider_display() {
        assert_eq!(AuthProvider::GitHub.to_string(), "github");
        assert_eq!(AuthProvider::Auth0.to_string(), "auth0");
    }

    // =========================================================================
    // env_bool logic
    // =========================================================================

    #[test]
    fn env_bool_default_false_recognises_true_values() {
        // With default=false, only "true" and "1" return true
        std::env::set_var("_TEST_BOOL_1", "true");
        assert!(env_bool("_TEST_BOOL_1", false));

        std::env::set_var("_TEST_BOOL_2", "1");
        assert!(env_bool("_TEST_BOOL_2", false));

        std::env::set_var("_TEST_BOOL_3", "false");
        assert!(!env_bool("_TEST_BOOL_3", false));

        std::env::set_var("_TEST_BOOL_4", "anything");
        assert!(!env_bool("_TEST_BOOL_4", false));

        // Missing env var
        std::env::remove_var("_TEST_BOOL_MISSING");
        assert!(!env_bool("_TEST_BOOL_MISSING", false));

        // Cleanup
        for k in [
            "_TEST_BOOL_1",
            "_TEST_BOOL_2",
            "_TEST_BOOL_3",
            "_TEST_BOOL_4",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn env_bool_default_true_recognises_false_values() {
        // With default=true, only "false" and "0" return false
        std::env::set_var("_TEST_BOOL_T1", "false");
        assert!(!env_bool("_TEST_BOOL_T1", true));

        std::env::set_var("_TEST_BOOL_T2", "0");
        assert!(!env_bool("_TEST_BOOL_T2", true));

        std::env::set_var("_TEST_BOOL_T3", "anything");
        assert!(env_bool("_TEST_BOOL_T3", true));

        // Missing env var returns default (true)
        std::env::remove_var("_TEST_BOOL_T_MISSING");
        assert!(env_bool("_TEST_BOOL_T_MISSING", true));

        // Cleanup
        for k in ["_TEST_BOOL_T1", "_TEST_BOOL_T2", "_TEST_BOOL_T3"] {
            std::env::remove_var(k);
        }
    }

    // =========================================================================
    // Config validation
    // =========================================================================

    /// Helper to build a minimal valid Config for testing.
    fn minimal_config() -> Config {
        Config {
            server: ServerConfig {
                port: 8080,
                metrics_port: 9090,
                grpc_port: 50051,
            },
            sandbox_provider: SandboxProviderType::Kubernetes,
            sandbox: SandboxConfig {
                namespace: "test".into(),
                image: "test:latest".into(),
                image_pull_policy: "Never".into(),
                grpc_port: 50052,
                resources_cpu_limit: "500m".into(),
                resources_memory_limit: "512Mi".into(),
                resources_cpu_request: "100m".into(),
                resources_memory_request: "128Mi".into(),
                sandbox_resources_cpu_request: "1".into(),
                sandbox_resources_cpu_limit: "4".into(),
                sandbox_resources_memory_request: "2Gi".into(),
                sandbox_resources_memory_limit: "8Gi".into(),
                run_as_non_root: false,
                shared_data_size: "2Gi".into(),
                storage_enabled: false,
                storage_size: "1Gi".into(),
                storage_class: None,
                default_storage_mode: "ebs".into(),
                pod_startup_timeout_secs: 10,
                task_token_secret: None,
                user_token_ttl_secs: 7 * 24 * 3600,
                bootstrap_token_ttl_secs: 1800,
                emit_legacy_user_token_env: true,
                file_mount_path: "/tmp".into(),
                idle_reaper_enabled: false,
                idle_timeout_minutes: 15,
                idle_check_interval_secs: 300,
                inactive_task_timeout_days: 0,
                orphan_gc_enabled: false,
                orphan_gc_retention_hours: 24,
                codex_refresher_interval_secs: 0,
                max_crash_restart_count: 5,
                node_selector_json: None,
                tolerations_json: None,
                service_account_name: None,
                default_model: None,
                default_codex_model: None,
                extra_env: vec![],
                cache_volumes: vec![],
                cache_host_root: "/mnt/cache".into(),
                cache_init_image: "docker:dind".into(),
                docker_dind_image: "docker:dind".into(),
                docker_dind_storage_size: "10Gi".into(),
                docker_dind_cpu_request: "500m".into(),
                docker_dind_cpu_limit: "2".into(),
                docker_dind_memory_request: "1Gi".into(),
                docker_dind_memory_limit: "4Gi".into(),
            },
            reconcile: ReconcileConfig {
                reprovision_enabled: false,
                pending_timeout_enabled: false,
                pending_timeout_minutes: 15,
                max_reprovision_attempts: 3,
            },
            proxy: ProxyConfig {
                domain: None,
                protocol: "http".into(),
                traefik_auth_middleware_name: "test".into(),
                traefik_auth_middleware_namespace: "test".into(),
                traefik_entry_point: "web".into(),
            },
            docker: DockerConfig {
                network: "test-network".into(),
                image: "test-sandbox:latest".into(),
                data_dir: "/data".into(),
                container_data_dir: "/data".into(),
                backend_hostname: "moatless-backend".into(),
                enable_socket_mount: false,
                socket_path: "/var/run/docker.sock".into(),
                extra_networks: vec![],
            },
            database_url: None,
            database: DatabaseConfig {
                url: None,
                max_connections: 5,
            },
            redis: RedisConfig {
                url: None,
                max_stream_length: 100,
            },
            sse: SseConfig {
                broadcast_capacity: 16,
                consumer_id: "test".into(),
            },
            auth: AuthConfig {
                provider: None,
                github_client_id: None,
                github_client_secret: None,
                github_callback_url: None,
                github_oauth_scope: "".into(),
                github_required_org: None,
                auth0_domain: None,
                auth0_client_id: None,
                auth0_client_secret: None,
                auth0_callback_url: None,
                auth0_audience: None,
                auth0_groups_claim: "".into(),
                session_max_age_secs: 3600,
                frontend_url: "http://localhost:3000".into(),
                cookie_domain: None,
                session_cookie_name: "test".into(),
                is_production: false,
                cache_ttl_secs: 0,
                session_jwt_secret: None,
                test_api_key: None,
                default_team_slug: None,
                github_app_client_id: None,
                github_app_client_secret: None,
                github_app_callback_url: None,
                github_app_id: None,
                github_app_private_key: None,
                github_app_default_installation_id: None,
                github_apps: vec![],
            },
            s3: S3Config {
                bucket_name: None,
                key_prefix: "test".into(),
                transcript_prefix: "transcripts".into(),
                workspace_prefix: "workspaces".into(),
                region: "us-east-1".into(),
            },
            slack: SlackConfig {
                bot_token: None,
                signing_secret: None,
                bot_user_id: None,
            },
            telegram: TelegramConfig {
                bot_token: None,
                mode: "webhook".into(),
            },
            github_pr: GithubPrConfig {
                webhook_secret: None,
                bot_token: None,
            },
            linear: LinearConfig {
                webhook_secret: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                oauth_callback_url: None,
            },
            skill: SkillConfig { cache_ttl_secs: 60 },
            secrets: SecretsConfig {
                encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
                kms_key_id: None,
            },
            codex_oauth: CodexOAuthConfig {
                issuer: "https://auth.openai.com".into(),
                client_id: "test-codex-client".into(),
            },
            default_run_as_user_id: None,
        }
    }

    #[test]
    fn validate_missing_encryption_key_fails() {
        let mut config = minimal_config();
        config.secrets.encryption_key = None;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("SECRETS_ENCRYPTION_KEY"));
    }

    #[test]
    fn validate_encryption_key_present_passes() {
        let config = minimal_config();
        // minimal_config() already has encryption_key set
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_no_auth_provider_passes() {
        let config = minimal_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_github_missing_client_id_fails() {
        let mut config = minimal_config();
        config.auth.provider = Some(AuthProvider::GitHub);
        config.auth.github_client_id = None;
        config.auth.github_client_secret = Some("secret".into());
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("GITHUB_CLIENT_ID"));
    }

    #[test]
    fn validate_github_missing_client_secret_fails() {
        let mut config = minimal_config();
        config.auth.provider = Some(AuthProvider::GitHub);
        config.auth.github_client_id = Some("id".into());
        config.auth.github_client_secret = None;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("GITHUB_CLIENT_SECRET"));
    }

    #[test]
    fn validate_github_complete_passes() {
        let mut config = minimal_config();
        config.auth.provider = Some(AuthProvider::GitHub);
        config.auth.github_client_id = Some("id".into());
        config.auth.github_client_secret = Some("secret".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_github_app_missing_callback_url_fails() {
        let mut config = minimal_config();
        config.auth.provider = Some(AuthProvider::GitHubApp);
        config.auth.github_app_client_id = Some("id".into());
        config.auth.github_app_client_secret = Some("secret".into());
        config.auth.github_app_callback_url = None;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("GITHUB_APP_CALLBACK_URL"));
    }

    #[test]
    fn validate_github_app_complete_passes() {
        let mut config = minimal_config();
        config.auth.provider = Some(AuthProvider::GitHubApp);
        config.auth.github_app_client_id = Some("id".into());
        config.auth.github_app_client_secret = Some("secret".into());
        config.auth.github_app_callback_url = Some("https://example.com/callback".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_auth0_missing_domain_fails() {
        let mut config = minimal_config();
        config.auth.provider = Some(AuthProvider::Auth0);
        config.auth.auth0_domain = None;
        config.auth.auth0_client_id = Some("id".into());
        config.auth.auth0_client_secret = Some("secret".into());
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("AUTH0_DOMAIN"));
    }

    #[test]
    fn validate_auth0_complete_passes() {
        let mut config = minimal_config();
        config.auth.provider = Some(AuthProvider::Auth0);
        config.auth.auth0_domain = Some("test.auth0.com".into());
        config.auth.auth0_client_id = Some("id".into());
        config.auth.auth0_client_secret = Some("secret".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_empty_sandbox_namespace_fails() {
        let mut config = minimal_config();
        config.sandbox.namespace = String::new();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("SANDBOX_NAMESPACE"));
    }

    // =========================================================================
    // AuthConfig Debug redaction
    // =========================================================================

    #[test]
    fn validate_test_api_key_missing_prefix_fails() {
        let mut config = minimal_config();
        config.auth.test_api_key = Some("a".repeat(36));
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("TEST_API_KEY"));
        assert!(err.to_string().contains("mvk_"));
    }

    #[test]
    fn validate_test_api_key_too_short_fails() {
        let mut config = minimal_config();
        config.auth.test_api_key = Some("mvk_short".into());
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("TEST_API_KEY"));
        assert!(err.to_string().contains("36 characters"));
    }

    #[test]
    fn validate_test_api_key_valid_passes() {
        let mut config = minimal_config();
        config.auth.test_api_key = Some(format!("mvk_{}", "a".repeat(32)));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn auth_config_debug_redacts_secrets() {
        let config = minimal_config();
        let mut auth = config.auth;
        auth.github_client_secret = Some("super_secret_123".into());
        auth.auth0_client_secret = Some("another_secret".into());
        auth.test_api_key = Some(format!("mvk_{}", "x".repeat(32)));
        let debug = format!("{auth:?}");
        assert!(!debug.contains("super_secret_123"));
        assert!(!debug.contains("another_secret"));
        assert!(!debug.contains(&"x".repeat(32)));
        assert!(debug.contains("[REDACTED]"));
    }

    // =========================================================================
    // SlackConfig Debug redaction
    // =========================================================================

    #[test]
    fn slack_config_debug_redacts_secrets() {
        let slack = SlackConfig {
            bot_token: Some("xoxb-real-token".into()),
            signing_secret: Some("real-signing-secret".into()),
            bot_user_id: Some("U12345".into()),
        };
        let debug = format!("{slack:?}");
        assert!(!debug.contains("xoxb-real-token"));
        assert!(!debug.contains("real-signing-secret"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("U12345")); // bot_user_id should NOT be redacted
    }
}
