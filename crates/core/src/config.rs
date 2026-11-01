//! Configuration module for the Moatless Vibe backend service.
//!
//! Loads configuration from environment variables with sensible defaults.
//! Config is grouped into logical sub-structs for readability and to enable
//! passing only relevant config to subsystems.

use std::env;
use std::fmt;
use std::str::FromStr;

use moatless_types::adapter::{LinearConfig, MessageBufferConfig};
use moatless_types::agent::CodexOAuthConfig;
use moatless_types::auth::{AllowedOrigin, AuthConfig, AuthProvider, OidcConfig};
use moatless_types::model::CodexModel;
use moatless_types::sandbox::SandboxProviderType;
use moatless_types::secret::SecretsConfig;
use moatless_types::ui_rpc::T3UiRpcConfig;

use crate::feature_flags::FeatureFlags;
use crate::sandbox_identities::SandboxIdentities;

pub use crate::http_config::HttpConfig;
pub use crate::model_gateway_config::ModelGatewayConfig;
pub use crate::s3_config::S3Config;

pub use crate::decision_config::{
    DEFAULT_JEV_MODEL, DecisionConfig, DecisionProviderConfig, JevConfig,
};

/// Codex model a turn runs when neither its message nor its task names one,
/// unless env `DEFAULT_CODEX_MODEL` overrides it.
pub const DEFAULT_CODEX_MODEL: &str = CodexModel::Gpt61Sol.id();

/// Default GitHub REST API base URL.
pub const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";

fn github_api_base_from_env() -> String {
    env_opt("GITHUB_API_BASE_URL")
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_API_BASE.to_string())
}

/// Minimum length accepted for an operator-supplied `ADMIN_PASSWORD`.
pub const MIN_ADMIN_PASSWORD_LEN: usize = 12;

/// Parse an environment variable with a default fallback. Logs a warning if the
/// env var is set but cannot be parsed.
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

fn oidc_config_from_env() -> Option<OidcConfig> {
    let issuer = env_opt("OIDC_ISSUER")?;
    Some(OidcConfig {
        provider_id: env_str("OIDC_PROVIDER_ID", "company"),
        name: env_str("OIDC_NAME", "Company SSO"),
        issuer,
        client_id: env_opt("OIDC_CLIENT_ID").unwrap_or_default(),
        client_secret: env_opt("OIDC_CLIENT_SECRET").unwrap_or_default(),
        callback_url: env_opt("OIDC_CALLBACK_URL").unwrap_or_default(),
        auto_provision: env_bool("OIDC_AUTO_PROVISION", true),
        auto_link_verified_email: env_bool("OIDC_AUTO_LINK_VERIFIED_EMAIL", true),
        allowed_domains: env_opt("OIDC_ALLOWED_DOMAINS")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|domain| !domain.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Parse a JSON array of `{"name":"...", "value":"..."}` objects from an env
/// var.
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

/// Read an environment variable as `Option<String>`.
fn env_opt(key: &str) -> Option<String> {
    env::var(key).ok()
}

/// Read a comma-separated environment variable, dropping blank entries.
fn env_csv(key: &str) -> Vec<String> {
    env_opt(key)
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Read an environment variable with a string default.
fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

/// OpenAI's Codex refresh-token endpoint.
pub const CODEX_REFRESH_URL_DEFAULT: &str = "https://auth.openai.com/oauth/token";

/// `CODEX_REFRESH_TOKEN_URL_OVERRIDE` (a local mock), else the default.
pub fn codex_refresh_url_from_env() -> String {
    env_str(
        "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
        CODEX_REFRESH_URL_DEFAULT,
    )
}

/// Origins the CSRF check trusts beyond `FRONTEND_URL`, from
/// `CSRF_TRUSTED_ORIGINS`.
///
/// Never default this from `T3_UI_RPC_ALLOWED_ORIGINS`: it carries wildcard
/// preview origins serving agent-authored code, which under a shared
/// `COOKIE_DOMAIN` could drive cookie-authenticated writes.
fn csrf_trusted_origins_from_env() -> Vec<AllowedOrigin> {
    moatless_types::auth::parse_allowed_origins(
        &env_opt("CSRF_TRUSTED_ORIGINS").unwrap_or_default(),
    )
}

/// One Plugin the deployment declares, from `PLUGIN_REPOSITORIES_JSON`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredPlugin {
    pub name: String,
    pub git_url: String,
}

/// Parse `PLUGIN_REPOSITORIES_JSON`, a JSON array of `{"name", "gitUrl"}`.
/// Malformed JSON fails the load: read as empty, it would disable them all.
fn parse_declared_plugins(raw: Option<String>) -> Result<Vec<DeclaredPlugin>, ConfigError> {
    let Some(raw) = raw.filter(|r| !r.trim().is_empty()) else {
        return Ok(vec![]);
    };
    serde_json::from_str(raw.trim())
        .map_err(|e| ConfigError::Invalid(format!("PLUGIN_REPOSITORIES_JSON: {e}")))
}

/// Read an environment variable as a boolean (true if "true" or "1").
fn env_bool(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) if default => v != "false" && v != "0",
        Ok(v) => v == "true" || v == "1",
        Err(_) => default,
    }
}

/// HTTP and gRPC server ports.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Port to listen on (default: 8080)
    pub port: u16,
    /// Metrics port (default: 9090)
    pub metrics_port: u16,
    /// gRPC port for sandbox communication (default: 50051)
    pub grpc_port: u16,
    /// This replica's claim and lease identity: `HOSTNAME`, else unique per
    /// process. A restarted pod keeps its hostname and so releases its claims at
    /// once; the fallback is unique because a shared one lets replicas steal
    /// live claims.
    pub worker_identity: String,
}

/// Kubernetes compute substrate used for dynamically created sandbox pods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KubernetesComputeMode {
    /// Ordinary Kubernetes nodes with the full pod feature set.
    #[default]
    Standard,
    /// Amazon EKS Fargate: no privileged containers, hostPath, or EBS volumes.
    Fargate,
}

impl KubernetesComputeMode {
    fn from_env() -> Result<Self, ConfigError> {
        match env_str("K8S_COMPUTE_MODE", "standard")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "standard" => Ok(Self::Standard),
            "fargate" => Ok(Self::Fargate),
            value => Err(ConfigError::Invalid(format!(
                "unknown K8S_COMPUTE_MODE '{value}'; expected standard or fargate"
            ))),
        }
    }

    #[must_use]
    pub fn is_fargate(self) -> bool {
        self == Self::Fargate
    }
}

/// Sandbox pod configuration (images, resources, storage, idle reaper).
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Kubernetes compute substrate. Env `K8S_COMPUTE_MODE` (`standard` or
    /// `fargate`).
    pub compute_mode: KubernetesComputeMode,
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
    /// CPU request for the sandbox (agent) container (env
    /// `K8S_RESOURCE_REQUEST_CPU`)
    pub sandbox_resources_cpu_request: String,
    /// CPU limit for the sandbox (agent) container (env
    /// `K8S_RESOURCE_LIMIT_CPU`)
    pub sandbox_resources_cpu_limit: String,
    /// Memory request for the sandbox (agent) container (env
    /// `K8S_RESOURCE_REQUEST_MEMORY`)
    pub sandbox_resources_memory_request: String,
    /// Memory limit for the sandbox (agent) container (env
    /// `K8S_RESOURCE_LIMIT_MEMORY`)
    pub sandbox_resources_memory_limit: String,
    /// Optional ephemeral-storage request for the sandbox container, which the
    /// scheduler counts against node disk. EKS Fargate uses it to provision
    /// pod-local disk. Env `K8S_SANDBOX_EPHEMERAL_STORAGE`.
    pub ephemeral_storage: Option<String>,
    /// Optional ephemeral-storage limit; the kubelet evicts a container that
    /// exceeds it. Unset means no ceiling. Ignored under Fargate, where the
    /// request doubles as the limit. Env `K8S_SANDBOX_EPHEMERAL_STORAGE_LIMIT`.
    pub ephemeral_storage_limit: Option<String>,
    /// Runs the sandbox and server containers as non-root via a pod
    /// securityContext (env `K8S_SANDBOX_RUN_AS_NON_ROOT`, default `false`).
    ///
    /// The `dind` sidecar still runs as root.
    pub run_as_non_root: bool,
    /// Size cap for the `shared-data` emptyDir mounted at `/opt/moatless/data`;
    /// without it, growth is unbounded and can evict the node.
    ///
    /// Env `K8S_SANDBOX_SHARED_DATA_SIZE` (default `2Gi`).
    pub shared_data_size: String,
    /// Persists cargo's downloaded state across pod replacement via
    /// `workspace-data` subPaths. Env `K8S_SANDBOX_PERSIST_TOOLCHAIN` (default
    /// `false`).
    ///
    /// Costs PVC space — bump the repo's storage size when enabling.
    pub persist_toolchain: bool,
    /// Whether PVC storage is enabled for sandbox pods
    pub storage_enabled: bool,
    /// PVC storage size (e.g., "3Gi")
    pub storage_size: String,
    /// PVC storage class
    pub storage_class: Option<String>,
    /// Whether sandboxes get a per-task PVC when the repo doesn't set
    /// `resources.pvc_enabled` (env `K8S_SANDBOX_DEFAULT_PVC`, default `true`).
    /// A per-repo switch overrides this; [`Self::storage_enabled`] overrides
    /// both.
    pub default_pvc_enabled: bool,
    /// Whether sandboxes snapshot their workspace to S3 when the repo doesn't
    /// set `resources.s3_enabled` (env `K8S_SANDBOX_DEFAULT_S3`, default
    /// `true`).
    ///
    /// Requires the S3 uploads bucket configured; without it the sandbox's sync
    /// calls no-op.
    pub default_s3_enabled: bool,
    /// Master secret every task's workspace-repository password is derived
    /// from.
    ///
    /// Without it no restic repository can be opened or written: the sandbox
    /// falls back to the tar, and a stopped task's files are unreadable. Env
    /// `SANDBOX_WORKSPACE_REPOSITORY_SECRET`.
    pub workspace_repository_secret: Option<String>,
    /// IAM role the backend assumes to hand a sandbox direct S3 access to its
    /// own workspace repository.
    ///
    /// Each session is narrowed to that task's prefix, so the role itself stays
    /// bucket-wide; without it the sandbox cannot write. Env
    /// `SANDBOX_WORKSPACE_REPOSITORY_ROLE_ARN`.
    pub workspace_repository_role_arn: Option<String>,
    /// Whether Kubernetes node-local workspace/Docker archives are tracked and
    /// managed across pod lifecycles.
    ///
    /// When disabled, `node` storage still uses hostPath, but the backend skips
    /// archive `ConfigMap`s, node pinning, and cleanup. Env
    /// `K8S_SANDBOX_NODE_ARCHIVE_ENABLED` (default `false`).
    pub node_archive_enabled: bool,
    /// When `true`, closing a task preserves its managed archives (S3, Docker,
    /// node-local disk) instead of deleting them, so it can reopen.
    ///
    /// K8s runtime resources still tear down on close; permanent deletion
    /// always removes storage regardless. Env
    /// `SANDBOX_RETAIN_STORAGE_ON_CLOSE` (default `true`).
    pub retain_storage_on_close: bool,
    /// Pod startup timeout in seconds
    pub pod_startup_timeout_secs: u64,
    /// How long a sandbox's init clone may receive nothing before git abandons
    /// it, forwarded as `GIT_CLONE_STALL_TIMEOUT_SECS`. `None` leaves the
    /// sandbox's own default. Env `K8S_SANDBOX_GIT_CLONE_STALL_TIMEOUT_SECS`.
    pub git_clone_stall_timeout_secs: Option<u64>,
    /// Secret for signing task auth JWTs
    pub task_token_secret: Option<String>,
    /// TTL (seconds) for the long-lived user auth token delivered over
    /// `GetCredentials`.
    ///
    /// Re-minted every fetch, so it only needs to outlive the 15-min refresher
    /// gap. Env `SANDBOX_USER_TOKEN_TTL_SECS` (default 604800 = 7 days).
    pub user_token_ttl_secs: u64,
    /// TTL (seconds) for the short-lived bootstrap token placed on the pod
    /// spec.
    ///
    /// Only authenticates the initial `GetCredentials` call, so a minutes-scale
    /// value bounds the blast radius of the one token left in etcd. Env
    /// `SANDBOX_BOOTSTRAP_TOKEN_TTL_SECS` (default 1800 = 30 min).
    pub bootstrap_token_ttl_secs: u64,
    /// Whether to also emit the long-lived `USER_AUTH_TOKEN` env var on the pod
    /// spec, for sandbox images still reading the token from env.
    ///
    /// Flip to `false` once every image reads the credential channel instead,
    /// removing the long-lived secret from etcd. Env
    /// `SANDBOX_EMIT_LEGACY_USER_TOKEN` (default `true`).
    pub emit_legacy_user_token_env: bool,
    /// Whether the sandbox gRPC verification layer accepts legacy claim-less
    /// tokens (minted before the per-task `taskId` binding existed).
    ///
    /// `true` falls back to the DB task-access check; `false` requires a
    /// matching `taskId` claim. Env `SANDBOX_ACCEPT_CLAIMLESS_TOKENS` (default
    /// `true`).
    pub accept_claimless_tokens: bool,
    /// Directory inside sandbox pods that attached uploads are written to (env
    /// `SANDBOX_FILE_MOUNT_PATH`, default `/opt/moatless/workspace/uploads`).
    ///
    /// The single source of truth for the path: the agent prompt's "Saved to:"
    /// line and the pod's `UPLOAD_DIR` env both derive from it.
    pub file_mount_path: String,
    /// Whether the idle sandbox reaper is enabled (default: true)
    pub idle_reaper_enabled: bool,
    /// Whether the cron scheduler is enabled. A shadow backend shares the real
    /// deployment's database and Leases, so it must not fire scheduled jobs.
    /// Env `CRON_SCHEDULER_ENABLED` (default: true).
    pub cron_scheduler_enabled: bool,
    /// Whether to attach a liveness probe restarting a hung-but-alive sandbox
    /// pod (agent actor wedged while `/health` still reports Ready).
    ///
    /// Default off: a false-positive restart kills an in-flight agent turn,
    /// worse than a rare hang. Env `K8S_SANDBOX_LIVENESS_ENABLED`.
    pub liveness_enabled: bool,
    /// Seconds between liveness probes. Env `K8S_SANDBOX_LIVENESS_PERIOD_SECS`
    /// (default 15).
    pub liveness_period_secs: i32,
    /// Seconds a liveness probe waits for `/livez`. Env
    /// `K8S_SANDBOX_LIVENESS_TIMEOUT_SECS` (default 10).
    pub liveness_timeout_secs: i32,
    /// Consecutive failed liveness probes before a restart. Env
    /// `K8S_SANDBOX_LIVENESS_FAILURE_THRESHOLD` (default 20).
    pub liveness_failure_threshold: i32,
    /// Stop sandboxes idle longer than this (minutes, default: 15)
    pub idle_timeout_minutes: u64,
    /// How often to check for idle sandboxes (seconds, default: 300 = 5 min)
    pub idle_check_interval_secs: u64,
    /// Auto-close an Open task this many days after it was settled. `0`
    /// disables auto-close. Env: `TASK_CLOSE_AFTER_SETTLED_DAYS` (default: 7).
    pub close_after_settled_days: u64,
    /// Settle a task idle this many hours, for a task that names no
    /// `settle_after_hours` of its own. `0` never settles. Env:
    /// `TASK_SETTLE_AFTER_HOURS` (default: 0).
    pub settle_after_hours: u64,
    /// Reclaim a Stopped sandbox's Deployment and PVC after this many hours,
    /// leaving the S3 snapshot as the only copy; a later start restores it.
    ///
    /// Only pvc+s3-tier tasks are eligible. `0` disables. Env
    /// `SANDBOX_COLD_AFTER_HOURS` (default 24); `tasks.cold_after_hours` overrides.
    pub cold_after_hours: u64,
    /// Whether the reaper's orphan-GC pass is enabled (default: true). When on,
    /// the reaper deletes labeled sandbox K8s resources whose Task row no
    /// longer exists. Env: `SANDBOX_ORPHAN_GC_ENABLED`.
    pub orphan_gc_enabled: bool,
    /// How often the background Codex refresher sweeps `provider_tokens`.
    ///
    /// `0` disables the sweep entirely (env-var fallback still works). Env
    /// `CODEX_REFRESHER_INTERVAL_SECS` (default 3600 = 1h).
    pub codex_refresher_interval_secs: u64,
    /// Maximum container restart count before auto-stopping a crash-looping
    /// sandbox (default: 5). Set to 0 to disable crash-loop auto-stop.
    pub max_crash_restart_count: i32,
    /// JSON-encoded tolerations for sandbox pods (e.g.,
    /// `[{"key":"kubernetes.azure.com/scalesetpriority","operator":"Equal","value":"spot","effect":"NoSchedule"}]`)
    pub tolerations_json: Option<String>,
    /// JSON-encoded node selector for sandbox pods (e.g.,
    /// `{"karpenter.sh/capacity-type":"spot"}`)
    pub node_selector_json: Option<String>,
    /// Kubernetes `ServiceAccount` name for sandbox pods (set via
    /// `K8S_SANDBOX_SERVICE_ACCOUNT`)
    pub service_account_name: Option<String>,
    /// Default Claude model for sandbox agents (forwarded as
    /// `DEFAULT_CLAUDE_CODE_MODEL` env var)
    pub default_model: Option<String>,
    /// Delivered with every Codex turn naming no model: the sandbox has none.
    pub default_codex_model: String,
    /// Extra env vars injected into every sandbox pod (parsed from
    /// `K8S_SANDBOX_EXTRA_ENV` JSON)
    pub extra_env: Vec<(String, String)>,
    /// Comma-separated Claude tools every sandbox run removes, forwarded as
    /// `SANDBOX_CLAUDE_DISALLOWED_TOOLS`. `None` keeps the sandbox's own
    /// default; an empty string removes none. Env
    /// `K8S_SANDBOX_CLAUDE_DISALLOWED_TOOLS`.
    pub claude_disallowed_tools: Option<String>,
    /// Host directory node-local sandbox storage is rooted at, holding
    /// `workspaces/<task-id>` and `docker/<task-id>`.
    ///
    /// The node archive manager and the node-storage janitor both prune under
    /// it. Env `K8S_SANDBOX_CACHE_HOST_ROOT` (default `/mnt/cache`).
    pub cache_host_root: String,
    /// Image for the privileged `dind` sidecar attached when a repo sets
    /// `dockerAccess: true`. Env `K8S_SANDBOX_DOCKER_DIND_IMAGE`.
    ///
    /// Must be allowlisted by the `restrict-image-registries` Kyverno
    /// policy, and Docker 25 or newer: the sidecar passes `--feature=` to
    /// `dockerd`, which an older one rejects at startup rather than ignoring.
    pub docker_dind_image: String,
    /// Size cap for the `dind` sidecar's `/var/lib/docker` emptyDir when
    /// `resources.dockerStorageMode` is `emptyDir`.
    ///
    /// The default `node` mode uses hostPath instead, bounded by node disk
    /// capacity. Env `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE` (default `10Gi`).
    pub docker_dind_storage_size: String,
    /// CPU request for the `dind` sidecar. Env
    /// `K8S_SANDBOX_DOCKER_DIND_CPU_REQUEST`.
    pub docker_dind_cpu_request: String,
    /// CPU limit for the `dind` sidecar. Env
    /// `K8S_SANDBOX_DOCKER_DIND_CPU_LIMIT`.
    pub docker_dind_cpu_limit: String,
    /// Memory request for the `dind` sidecar. Env
    /// `K8S_SANDBOX_DOCKER_DIND_MEMORY_REQUEST`.
    pub docker_dind_memory_request: String,
    /// Memory limit for the `dind` sidecar. Containers started by
    /// Testcontainers run as children of dockerd inside this container, so
    /// their memory charges against this limit. Env
    /// `K8S_SANDBOX_DOCKER_DIND_MEMORY_LIMIT`.
    pub docker_dind_memory_limit: String,
    /// In-cluster egress forward-proxy URL. Pods whose `network_profile` is
    /// listed in [`SandboxConfig::egress_proxy_profiles`] get
    /// `HTTPS_PROXY`/`HTTP_PROXY` pointed at it.
    ///
    /// Pairs with a per-profile `NetworkPolicy` that blocks direct egress and
    /// forces this path. Env `SANDBOX_EGRESS_PROXY_URL` (default unset).
    pub egress_proxy_url: Option<String>,
    /// Network profiles that receive the egress proxy env (see
    /// [`SandboxConfig::egress_proxy_url`]); a pod gets it only when its
    /// `network_profile` is listed here.
    ///
    /// Env `SANDBOX_EGRESS_PROXY_PROFILES` (comma-separated). Opt-in: empty
    /// means no pod gets the proxy even with a URL set.
    pub egress_proxy_profiles: Vec<String>,
    /// Network profiles the chart's templates select, beyond
    /// [`SandboxConfig::egress_proxy_profiles`]; see
    /// [`SandboxConfig::identities`]. Env `SANDBOX_NETWORK_PROFILES`
    /// (comma-separated).
    pub network_profiles: Vec<String>,
    /// Service accounts a Workspace may run its sandboxes as; see
    /// [`SandboxConfig::identities`]. Env
    /// `K8S_ALLOWED_SANDBOX_SERVICE_ACCOUNTS` (comma-separated).
    pub allowed_service_accounts: Vec<String>,
    /// `NO_PROXY` value paired with [`SandboxConfig::egress_proxy_url`] so
    /// in-cluster traffic (gRPC, DNS, kube-apiserver, link-local metadata)
    /// bypasses the proxy.
    ///
    /// Env `SANDBOX_EGRESS_PROXY_NO_PROXY` (default covers localhost, the
    /// cluster domain, RFC-1918, and the link-local metadata IP).
    pub egress_proxy_no_proxy: String,
    /// The backend's own values for [`LOCAL_PASSTHROUGH_ENV`], which a local
    /// sandbox process (`SANDBOX_PROVIDER=local`) inherits.
    pub local_passthrough_env: Vec<(String, String)>,
}

impl SandboxConfig {
    /// The network profiles and service accounts a Workspace may name: every
    /// profile the chart selects, and the allowlisted accounts.
    pub fn identities(&self) -> SandboxIdentities {
        let mut profiles = self.network_profiles.clone();
        profiles.extend(self.egress_proxy_profiles.iter().cloned());
        SandboxIdentities::new(profiles, self.allowed_service_accounts.clone())
    }
}

/// The backend environment a local sandbox process takes: no model credential,
/// since its agent reaches every model through the backend's gateway.
const LOCAL_PASSTHROUGH_ENV: &[&str] = &["PATH"];

/// Proxy / Traefik `IngressRoute` settings.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Domain for proxy URLs (e.g., "moatless.xyz"). If None, `IngressRoute`s
    /// are not created.
    pub domain: Option<String>,
    /// Protocol for proxy URLs (default: "https")
    pub protocol: String,
    /// Name of the Traefik `ForwardAuth` Middleware CRD (default:
    /// "preview-auth")
    pub traefik_auth_middleware_name: String,
    /// Namespace where the Traefik auth Middleware lives (defaults to sandbox
    /// namespace)
    pub traefik_auth_middleware_namespace: String,
    /// Traefik entryPoint name for `IngressRoute`s (default: "websecure")
    pub traefik_entry_point: String,
    /// Namespaces whose Traefik ingress pods the per-sandbox preview
    /// `NetworkPolicy` admits to the preview ports.
    ///
    /// Deriving the policy's ports from the sandbox's own server map
    /// (`k8s/networkpolicy.rs`) is what keeps it from drifting out of sync.
    pub traefik_ingress_namespaces: Vec<String>,
}

/// SSE (Server-Sent Events) settings.
#[derive(Debug, Clone)]
pub struct SseConfig {
    /// Broadcast buffer capacity for SSE fan-out (default: 4096)
    pub broadcast_capacity: usize,
}

/// Slack adapter settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone, Default)]
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

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: None,
            mode: "webhook".to_string(),
        }
    }
}

/// GitHub PR adapter settings.
///
/// Implements a custom `Debug` to redact secrets.
#[derive(Clone)]
pub struct GithubPrConfig {
    /// Global webhook secret for verifying GitHub webhook signatures.
    pub webhook_secret: Option<String>,
}

#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for GithubPrConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubPrConfig")
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl DecisionConfig {
    fn from_env() -> Result<Self, ConfigError> {
        Self::parse(
            env_opt("DECISION_PROVIDER"),
            env_opt("TYPESAFE_API_KEY"),
            env_opt("TYPESAFE_API_BASE_URL"),
            env_opt("DECISION_MODEL"),
        )
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
    /// Backend container hostname on the Docker network (for sandbox → backend
    /// gRPC)
    pub backend_hostname: String,
    /// Mount Docker socket in sandbox containers (default: false)
    pub enable_socket_mount: bool,
    /// Host path to Docker socket (default: "/var/run/docker.sock")
    pub socket_path: String,
    /// Additional Docker networks to connect sandbox containers to
    pub extra_networks: Vec<String>,
}

/// `SANDBOX_PROVIDER=local` configuration: each sandbox is a child process of
/// the backend, for isolated E2E runs on one host.
#[derive(Debug, Clone)]
pub struct LocalSandboxConfig {
    /// `LOCAL_SANDBOX_ROOT`, holding one directory per task. Keep it short: the
    /// credential socket under it must fit the 108-byte Unix socket path limit.
    pub root: String,
    /// `LOCAL_SANDBOX_BIN`, the `sandbox` binary each task runs.
    pub bin: String,
}

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL. When set, enables DB-backed features.
    pub url: Option<String>,
    /// Max connections in the pool (default: 5)
    pub max_connections: u32,
    /// Whether to apply pending migrations at startup (default: true). Env
    /// `RUN_MIGRATIONS=false` disables them — used by the shadow-dev backend so
    /// a dev instance never mutates the shared dev schema.
    pub run_migrations: bool,
}

/// Skill discovery configuration.
#[derive(Debug, Clone)]
pub struct SkillConfig {
    /// In-memory cache TTL for parsed skill lists.
    pub cache_ttl_secs: u64,
}

/// OTLP trace export settings (`backend/src/startup.rs::init_tracing`).
///
/// `otlp_endpoint: None` (default, `OTEL_EXPORTER_OTLP_ENDPOINT` unset) means
/// trace export is off — `init_tracing` omits the OTel layer entirely.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// OTLP/gRPC collector endpoint (e.g. `http://tempo:4317`).
    pub otlp_endpoint: Option<String>,
    /// Head sampling ratio for the parent-based sampler, applied only to traces
    /// with no sampling decision already made upstream.
    ///
    /// Env `OTEL_TRACES_SAMPLER_ARG` (matches the OTel SDK's own name for a
    /// ratio-sampler argument).
    pub sampling_ratio: f64,
    /// OTel `service.name` resource (env: `OTEL_SERVICE_NAME`).
    pub service_name: String,
}

impl OtelConfig {
    const DEFAULT_SAMPLING_RATIO: f64 = 0.1;
    const DEFAULT_SERVICE_NAME: &'static str = "moatless-vibe-backend";

    /// Reads directly from the environment without a `tracing::warn!` on bad
    /// values — this runs before `startup::init_tracing` installs a subscriber,
    /// so a warning has nowhere to land.
    ///
    /// [`Config::from_env`] reads the same variables into [`Config::otel`]
    /// afterwards, once logging is live.
    pub fn from_env() -> Self {
        Self {
            otlp_endpoint: env_opt("OTEL_EXPORTER_OTLP_ENDPOINT"),
            sampling_ratio: env::var("OTEL_TRACES_SAMPLER_ARG")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|ratio| ratio.is_finite() && *ratio >= 0.0)
                .unwrap_or(Self::DEFAULT_SAMPLING_RATIO),
            service_name: env_str("OTEL_SERVICE_NAME", Self::DEFAULT_SERVICE_NAME),
        }
    }
}

/// The `T3_*` settings; a non-boolean `T3_UI_RPC_ENABLED` or
/// `T3_UI_RPC_REVIEW_GUIDE_FORK_AUTHOR` fails startup.
fn t3_ui_rpc_from_env() -> Result<T3UiRpcConfig, ConfigError> {
    Ok(T3UiRpcConfig {
        enabled: parse_enabled("T3_UI_RPC_ENABLED", &env_str("T3_UI_RPC_ENABLED", ""))?,
        allowed_origins: moatless_types::auth::parse_allowed_origins(
            &env_opt("T3_UI_RPC_ALLOWED_ORIGINS").unwrap_or_default(),
        ),
        client_traces_endpoint: env_opt("T3_CLIENT_OTLP_TRACES_ENDPOINT"),
        cliproxy_url: env_opt("T3_UI_RPC_CLIPROXY_URL"),
        cliproxy_management_key: env_opt("T3_UI_RPC_CLIPROXY_MANAGEMENT_KEY"),
        cliproxy_label: env_opt("T3_UI_RPC_CLIPROXY_LABEL"),
        orchestration_protocol_version: orchestration_protocol_version_from_env()?,
        review_guide_fork_author: parse_enabled(
            "T3_UI_RPC_REVIEW_GUIDE_FORK_AUTHOR",
            &env_str("T3_UI_RPC_REVIEW_GUIDE_FORK_AUTHOR", ""),
        )?,
    })
}

/// Reads [`T3UiRpcConfig::orchestration_protocol_version`], which names the
/// values.
fn orchestration_protocol_version_from_env() -> Result<u8, ConfigError> {
    match env_str("T3_UI_RPC_ORCHESTRATION_PROTOCOL_VERSION", "1").trim() {
        "" | "1" => Ok(1),
        "2" => Ok(2),
        other => Err(ConfigError::Invalid(format!(
            "T3_UI_RPC_ORCHESTRATION_PROTOCOL_VERSION must be 1 or 2, got `{other}`"
        ))),
    }
}

fn parse_enabled(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(ConfigError::Invalid(format!("{key} must be true or false"))),
    }
}

/// The `MODEL_GATEWAY_*` settings.
fn model_gateway_from_env() -> ModelGatewayConfig {
    ModelGatewayConfig {
        hub_url: env_opt("MODEL_GATEWAY_HUB_URL").map(|url| url.trim_end_matches('/').to_string()),
        hub_key: env_opt("MODEL_GATEWAY_HUB_KEY"),
    }
}

/// Top-level configuration grouping all sub-configs.
#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    /// Which sandbox provider to use (Kubernetes or Docker).
    pub sandbox_provider: SandboxProviderType,
    pub sandbox: SandboxConfig,
    pub docker: DockerConfig,
    pub local_sandbox: LocalSandboxConfig,
    pub proxy: ProxyConfig,
    /// PostgreSQL connection URL. When set, enables DB-backed features.
    pub database_url: Option<String>,
    pub database: DatabaseConfig,
    /// Whether this process runs the fleet's event dispatcher (default: true).
    ///
    /// Every replica claims off one shared queue, so a second backend on the
    /// same database splits the fleet's workload — set
    /// `EVENT_HANDLER_ENABLED=false` on a shadow/dev backend sharing it.
    pub event_handler_enabled: bool,
    pub sse: SseConfig,
    pub auth: AuthConfig,
    pub s3: S3Config,
    pub slack: SlackConfig,
    pub telegram: TelegramConfig,
    pub github_pr: GithubPrConfig,
    /// GitHub REST API root, without a trailing `/`. `GITHUB_API_BASE_URL`
    /// overrides [`DEFAULT_GITHUB_API_BASE`], for an E2E mock server or a
    /// GitHub Enterprise host; an empty value counts as unset.
    pub github_api_base: String,
    pub linear: LinearConfig,
    pub decision: DecisionConfig,
    pub skill: SkillConfig,
    /// The Plugins the deployment owns; see `Plugins::reconcile_declared`.
    pub declared_plugins: Vec<DeclaredPlugin>,
    pub secrets: SecretsConfig,
    pub codex_oauth: CodexOAuthConfig,
    pub otel: OtelConfig,
    pub t3_ui_rpc: T3UiRpcConfig,
    pub feature_flags: FeatureFlags,
    pub message_buffer: MessageBufferConfig,
    pub http: HttpConfig,
    pub workspace_sync: WorkspaceSyncConfig,
    pub model_gateway: ModelGatewayConfig,
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

        // Namespaces running Traefik ingress pods that terminate preview
        // traffic. Defaults to the sandbox namespace plus the shared
        // `platform-ingress`; a comma-separated env overrides. Duplicates
        // collapse so a repeated `from` peer isn't emitted.
        let traefik_ingress_namespaces = {
            let mut namespaces = env_opt("TRAEFIK_INGRESS_NAMESPACES").map_or_else(
                || vec![sandbox_namespace.clone(), "platform-ingress".to_string()],
                |s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(String::from)
                        .collect::<Vec<_>>()
                },
            );
            namespaces.dedup();
            namespaces
        };

        let config = Self {
            server: ServerConfig {
                port: env_or("PORT", 8080),
                metrics_port: env_or("METRICS_PORT", 9090),
                grpc_port: env_or("GRPC_PORT", 50051),
                worker_identity: env_opt("HOSTNAME")
                    .unwrap_or_else(|| format!("unknown-host-{}", uuid::Uuid::new_v4())),
            },

            sandbox_provider,

            sandbox: SandboxConfig {
                compute_mode: KubernetesComputeMode::from_env()?,
                namespace: sandbox_namespace.clone(),
                image: env_str("SANDBOX_IMAGE", "aorwall/moatless-sandbox:latest"),
                image_pull_policy: env_str("SANDBOX_IMAGE_PULL_POLICY", "Always"),
                grpc_port: env_or("SANDBOX_GRPC_PORT", 50052),
                resources_cpu_limit: env_str("SERVER_RESOURCES_CPU_LIMIT", "1000m"),
                resources_memory_limit: env_str("SERVER_RESOURCES_MEMORY_LIMIT", "1536Mi"),
                resources_cpu_request: env_str("SERVER_RESOURCES_CPU_REQUEST", "200m"),
                resources_memory_request: env_str("SERVER_RESOURCES_MEMORY_REQUEST", "256Mi"),
                sandbox_resources_cpu_request: env_str("K8S_RESOURCE_REQUEST_CPU", "1"),
                sandbox_resources_cpu_limit: env_str("K8S_RESOURCE_LIMIT_CPU", "4"),
                sandbox_resources_memory_request: env_str("K8S_RESOURCE_REQUEST_MEMORY", "2Gi"),
                sandbox_resources_memory_limit: env_str("K8S_RESOURCE_LIMIT_MEMORY", "16Gi"),
                ephemeral_storage: env_opt("K8S_SANDBOX_EPHEMERAL_STORAGE"),
                ephemeral_storage_limit: env_opt("K8S_SANDBOX_EPHEMERAL_STORAGE_LIMIT"),
                run_as_non_root: env_bool("K8S_SANDBOX_RUN_AS_NON_ROOT", false),
                shared_data_size: env_str("K8S_SANDBOX_SHARED_DATA_SIZE", "2Gi"),
                persist_toolchain: env_bool("K8S_SANDBOX_PERSIST_TOOLCHAIN", false),
                storage_enabled: env_bool("K8S_SANDBOX_STORAGE_ENABLED", true),
                storage_size: env_str("K8S_SANDBOX_STORAGE_SIZE", "3Gi"),
                storage_class: env_opt("K8S_SANDBOX_STORAGE_CLASS"),
                default_pvc_enabled: env_bool("K8S_SANDBOX_DEFAULT_PVC", true),
                default_s3_enabled: env_bool("K8S_SANDBOX_DEFAULT_S3", true),
                workspace_repository_secret: env_opt("SANDBOX_WORKSPACE_REPOSITORY_SECRET"),
                workspace_repository_role_arn: env_opt("SANDBOX_WORKSPACE_REPOSITORY_ROLE_ARN"),
                node_archive_enabled: env_bool("K8S_SANDBOX_NODE_ARCHIVE_ENABLED", false),
                retain_storage_on_close: env_bool("SANDBOX_RETAIN_STORAGE_ON_CLOSE", true),
                pod_startup_timeout_secs: env_or("K8S_POD_STARTUP_TIMEOUT_SECS", 120),
                git_clone_stall_timeout_secs: env_opt("K8S_SANDBOX_GIT_CLONE_STALL_TIMEOUT_SECS")
                    .and_then(|raw| raw.trim().parse::<u64>().ok())
                    .filter(|secs| *secs > 0),
                task_token_secret: env_opt("TASK_TOKEN_SECRET"),
                user_token_ttl_secs: env_or("SANDBOX_USER_TOKEN_TTL_SECS", 7 * 24 * 3600),
                bootstrap_token_ttl_secs: env_or("SANDBOX_BOOTSTRAP_TOKEN_TTL_SECS", 1800),
                emit_legacy_user_token_env: env_bool("SANDBOX_EMIT_LEGACY_USER_TOKEN", true),
                accept_claimless_tokens: env_bool("SANDBOX_ACCEPT_CLAIMLESS_TOKENS", true),
                file_mount_path: env_str(
                    "SANDBOX_FILE_MOUNT_PATH",
                    "/opt/moatless/workspace/uploads",
                ),
                idle_reaper_enabled: env_bool("SANDBOX_IDLE_REAPER_ENABLED", true),
                cron_scheduler_enabled: env_bool("CRON_SCHEDULER_ENABLED", true),
                liveness_enabled: env_bool("K8S_SANDBOX_LIVENESS_ENABLED", false),
                liveness_period_secs: env_or("K8S_SANDBOX_LIVENESS_PERIOD_SECS", 15),
                liveness_timeout_secs: env_or("K8S_SANDBOX_LIVENESS_TIMEOUT_SECS", 10),
                liveness_failure_threshold: env_or("K8S_SANDBOX_LIVENESS_FAILURE_THRESHOLD", 20),
                idle_timeout_minutes: env_or("SANDBOX_IDLE_TIMEOUT_MINUTES", 15),
                idle_check_interval_secs: env_or("SANDBOX_IDLE_CHECK_INTERVAL_SECS", 300),
                close_after_settled_days: env_or("TASK_CLOSE_AFTER_SETTLED_DAYS", 7),
                settle_after_hours: env_or("TASK_SETTLE_AFTER_HOURS", 0),
                cold_after_hours: env_or("SANDBOX_COLD_AFTER_HOURS", 24),
                orphan_gc_enabled: env_bool("SANDBOX_ORPHAN_GC_ENABLED", true),
                codex_refresher_interval_secs: env_or("CODEX_REFRESHER_INTERVAL_SECS", 3600),
                max_crash_restart_count: env_or("SANDBOX_MAX_CRASH_RESTART_COUNT", 5),
                tolerations_json: env_opt("K8S_SANDBOX_TOLERATIONS"),
                node_selector_json: env_opt("K8S_SANDBOX_NODE_SELECTOR"),
                service_account_name: env_opt("K8S_SANDBOX_SERVICE_ACCOUNT"),
                default_model: env_opt("DEFAULT_CLAUDE_CODE_MODEL"),
                default_codex_model: env_str("DEFAULT_CODEX_MODEL", DEFAULT_CODEX_MODEL),
                extra_env: parse_extra_env("K8S_SANDBOX_EXTRA_ENV"),
                claude_disallowed_tools: env_opt("K8S_SANDBOX_CLAUDE_DISALLOWED_TOOLS"),
                cache_host_root: env_str("K8S_SANDBOX_CACHE_HOST_ROOT", "/mnt/cache"),
                docker_dind_image: env_str(
                    "K8S_SANDBOX_DOCKER_DIND_IMAGE",
                    "111111111111.dkr.ecr.eu-north-1.amazonaws.com/mirror/docker.io/library/docker:29-dind",
                ),
                docker_dind_storage_size: env_str("K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE", "10Gi"),
                docker_dind_cpu_request: env_str("K8S_SANDBOX_DOCKER_DIND_CPU_REQUEST", "500m"),
                docker_dind_cpu_limit: env_str("K8S_SANDBOX_DOCKER_DIND_CPU_LIMIT", "2"),
                docker_dind_memory_request: env_str(
                    "K8S_SANDBOX_DOCKER_DIND_MEMORY_REQUEST",
                    "1Gi",
                ),
                docker_dind_memory_limit: env_str("K8S_SANDBOX_DOCKER_DIND_MEMORY_LIMIT", "4Gi"),
                egress_proxy_url: env_opt("SANDBOX_EGRESS_PROXY_URL"),
                egress_proxy_profiles: env_csv("SANDBOX_EGRESS_PROXY_PROFILES"),
                network_profiles: env_csv("SANDBOX_NETWORK_PROFILES"),
                allowed_service_accounts: env_csv("K8S_ALLOWED_SANDBOX_SERVICE_ACCOUNTS"),
                egress_proxy_no_proxy: env_str(
                    "SANDBOX_EGRESS_PROXY_NO_PROXY",
                    "localhost,127.0.0.1,::1,.svc,.svc.cluster.local,.cluster.local,\
                     10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,169.254.169.254,kubernetes.default",
                ),
                local_passthrough_env: LOCAL_PASSTHROUGH_ENV
                    .iter()
                    .filter_map(|key| Some(((*key).to_string(), env::var(key).ok()?)))
                    .collect(),
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
                traefik_ingress_namespaces,
            },

            docker: DockerConfig {
                network: env_str("DOCKER_NETWORK", "moatless"),
                image: env_str("DOCKER_SANDBOX_IMAGE", "aorwall/moatless-sandbox:latest"),
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

            local_sandbox: LocalSandboxConfig {
                root: env_str("LOCAL_SANDBOX_ROOT", "/tmp/moatless-local"),
                bin: env_str("LOCAL_SANDBOX_BIN", "sandbox"),
            },

            database_url: database_url.clone(),

            database: DatabaseConfig {
                url: database_url,
                max_connections: env_or("DATABASE_MAX_CONNECTIONS", 5),
                run_migrations: env_bool("RUN_MIGRATIONS", true),
            },

            event_handler_enabled: env_bool("EVENT_HANDLER_ENABLED", true),

            sse: SseConfig {
                broadcast_capacity: env_or("SSE_BROADCAST_CAPACITY", 4096),
            },

            auth: AuthConfig {
                provider: match env_opt("AUTH_PROVIDER") {
                    Some(v) => v.parse().map_err(ConfigError::Invalid)?,
                    None => AuthProvider::default(),
                },
                github_required_org: env_opt("GITHUB_REQUIRED_ORG"),
                auth0_domain: env_opt("AUTH0_DOMAIN"),
                auth0_client_id: env_opt("AUTH0_CLIENT_ID"),
                auth0_client_secret: env_opt("AUTH0_CLIENT_SECRET"),
                auth0_callback_url: env_opt("AUTH0_CALLBACK_URL"),
                auth0_audience: env_opt("AUTH0_AUDIENCE"),
                auth0_groups_claim: env_str("AUTH0_GROUPS_CLAIM", "https://moatless.ai/groups"),
                oidc: oidc_config_from_env(),
                session_max_age_secs: env_or("SESSION_MAX_AGE", 604_800),
                session_absolute_age_secs: env_or("SESSION_ABSOLUTE_MAX_AGE", 2_592_000),
                frontend_url: env_str("FRONTEND_URL", "http://localhost:3000"),
                csrf_trusted_origins: csrf_trusted_origins_from_env(),
                cookie_domain: env_opt("COOKIE_DOMAIN"),
                session_cookie_name: env_str("SESSION_COOKIE_NAME", "moatless_session"),
                is_production: env::var("APP_ENV")
                    .or_else(|_| env::var("NODE_ENV"))
                    .is_ok_and(|v| v == "production"),
                admin_username: env_str("ADMIN_USERNAME", "admin"),
                admin_email: Some(env_str("ADMIN_EMAIL", "admin@localhost")),
                admin_password: env_opt("ADMIN_PASSWORD"),
                admin_user_ids: env_opt("ADMIN_USER_IDS")
                    .map(|s| {
                        s.split(',')
                            .map(str::trim)
                            .filter(|id| !id.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                break_glass_enabled: env_bool("AUTH_BREAK_GLASS_ENABLED", false),
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
            },

            s3: S3Config {
                bucket_name: env_opt("S3_BUCKET_NAME"),
                key_prefix: env_str("S3_KEY_PREFIX", "uploads"),
                transcript_prefix: env_str("TRANSCRIPT_S3_PREFIX", "transcripts"),
                workspace_prefix: env_str("WORKSPACE_S3_PREFIX", "workspaces"),
                artifact_prefix: env_str("ARTIFACT_S3_PREFIX", "artifacts"),
                region: env_str("S3_REGION", "us-east-1"),
            },

            // Adapter app credentials load from `adapter_apps` after the DB is
            // up; see `moatless_server::compose::adapter_config`.
            slack: SlackConfig::default(),
            telegram: TelegramConfig::default(),

            github_pr: GithubPrConfig {
                webhook_secret: env_opt("GITHUB_CHANNEL_WEBHOOK_SECRET"),
            },
            github_api_base: github_api_base_from_env(),

            linear: LinearConfig::default(),

            decision: DecisionConfig::from_env()?,

            skill: SkillConfig {
                cache_ttl_secs: env_or("BACKEND_SKILL_CACHE_TTL_SECONDS", 3_600),
            },

            declared_plugins: parse_declared_plugins(env_opt("PLUGIN_REPOSITORIES_JSON"))?,
            secrets: SecretsConfig {
                encryption_key: env_opt("SECRETS_ENCRYPTION_KEY"),
                kms_key_id: env_opt("SECRETS_KMS_KEY_ID").or_else(|| env_opt("AWS_KMS_KEY_ID")),
            },

            codex_oauth: CodexOAuthConfig {
                issuer: env_str("CODEX_OAUTH_ISSUER", "https://auth.openai.com"),
                client_id: env_str("CODEX_OAUTH_CLIENT_ID", "app_EMoamEEZ73f0CkXaXp7hrann"),
                refresh_url: codex_refresh_url_from_env(),
            },

            otel: OtelConfig::from_env(),
            t3_ui_rpc: t3_ui_rpc_from_env()?,
            feature_flags: FeatureFlags::from_lookup(env_opt),
            // `vars_os`, not `vars`: `vars` panics on any non-Unicode entry, even
            // an unrelated one, and would stop the backend starting.
            message_buffer: crate::message_buffer_config::parse(env::vars_os()),
            http: HttpConfig::from_vars(|key| env::var(key).ok()),
            workspace_sync: WorkspaceSyncConfig::parse(
                env_csv("GITHUB_ORG_SYNC_ORGS"),
                env_opt("GITHUB_ORG_SYNC_USER_ID"),
                env_opt("WORKSPACE_IDENTITY_GRANTS"),
            )
            .map_err(ConfigError::Invalid)?,
            model_gateway: model_gateway_from_env(),
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration consistency.
    ///
    /// Made `pub(crate)` so tests can call it directly without env vars.
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        self.workspace_sync
            .identity_grants
            .check(&self.sandbox.identities())
            .map_err(|e| ConfigError::Invalid(format!("WORKSPACE_IDENTITY_GRANTS: {e}")))?;

        // Production must authenticate sandbox gRPC calls: without the signing
        // secret, verification falls back to its dev bypass (trusting every
        // caller), so refuse to boot. Dev/local may omit it and run unauthenticated.
        if self.auth.is_production && self.sandbox.task_token_secret.is_none() {
            return Err(ConfigError::Invalid(
                "TASK_TOKEN_SECRET required in production (APP_ENV=production) — sandbox gRPC \
                 authentication cannot be enforced without it."
                    .into(),
            ));
        }

        // Local password auth stores credentials and opaque auth sessions in
        // Postgres. Every bootstrap administrator has an email because local
        // login is email-based.
        if self.auth.provider == AuthProvider::Local {
            if self.database.url.is_none() {
                return Err(ConfigError::Invalid(
                    "DATABASE_URL required when AUTH_PROVIDER=local — credentials are stored in \
                     the user_credentials table."
                        .into(),
                ));
            }
            if self.auth.admin_email.as_deref().is_none_or(str::is_empty) {
                return Err(ConfigError::Invalid(
                    "ADMIN_EMAIL required when AUTH_PROVIDER=local".into(),
                ));
            }
            if let Some(ref pw) = self.auth.admin_password {
                if pw.chars().count() < MIN_ADMIN_PASSWORD_LEN {
                    return Err(ConfigError::Invalid(format!(
                        "ADMIN_PASSWORD must be at least {MIN_ADMIN_PASSWORD_LEN} characters"
                    )));
                }
            }
        }

        if self.auth.provider == AuthProvider::GitHubApp {
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
        if self.auth.provider == AuthProvider::Auth0 {
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
        if self.auth.provider == AuthProvider::Oidc {
            let oidc = self.auth.oidc.as_ref().ok_or_else(|| {
                ConfigError::Invalid(
                    "OIDC_ISSUER, OIDC_CLIENT_ID, OIDC_CLIENT_SECRET, and OIDC_CALLBACK_URL \
                     are required when AUTH_PROVIDER=oidc"
                        .into(),
                )
            })?;
            if oidc.issuer.is_empty()
                || oidc.client_id.is_empty()
                || oidc.client_secret.is_empty()
                || oidc.callback_url.is_empty()
            {
                return Err(ConfigError::Invalid(
                    "OIDC_ISSUER, OIDC_CLIENT_ID, OIDC_CLIENT_SECRET, and OIDC_CALLBACK_URL must \
                     not be empty"
                        .into(),
                ));
            }
        }
        if self.auth.break_glass_enabled {
            if self.auth.admin_email.as_deref().is_none_or(str::is_empty)
                || self.auth.admin_password.is_none()
            {
                return Err(ConfigError::Invalid(
                    "ADMIN_EMAIL and ADMIN_PASSWORD are required when AUTH_BREAK_GLASS_ENABLED=true"
                        .into(),
                ));
            }
            if self
                .auth
                .admin_password
                .as_ref()
                .is_some_and(|password| password.chars().count() < MIN_ADMIN_PASSWORD_LEN)
            {
                return Err(ConfigError::Invalid(format!(
                    "ADMIN_PASSWORD must be at least {MIN_ADMIN_PASSWORD_LEN} characters"
                )));
            }
        }

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

        match self.sandbox_provider {
            SandboxProviderType::Kubernetes => {
                if self.sandbox.namespace.is_empty() {
                    return Err(ConfigError::Invalid(
                        "SANDBOX_NAMESPACE cannot be empty".into(),
                    ));
                }
                if self.sandbox.compute_mode.is_fargate() {
                    if self.sandbox.default_pvc_enabled || !self.sandbox.default_s3_enabled {
                        return Err(ConfigError::Invalid(
                            "K8S_COMPUTE_MODE=fargate requires K8S_SANDBOX_DEFAULT_PVC=false and K8S_SANDBOX_DEFAULT_S3=true"
                                .into(),
                        ));
                    }
                    if !self.sandbox.run_as_non_root {
                        return Err(ConfigError::Invalid(
                            "K8S_COMPUTE_MODE=fargate requires K8S_SANDBOX_RUN_AS_NON_ROOT=true"
                                .into(),
                        ));
                    }
                    if self.sandbox.node_archive_enabled {
                        return Err(ConfigError::Invalid(
                            "K8S_COMPUTE_MODE=fargate does not support node-local archives".into(),
                        ));
                    }
                    for (request, limit, name) in [
                        (
                            &self.sandbox.sandbox_resources_cpu_request,
                            &self.sandbox.sandbox_resources_cpu_limit,
                            "sandbox CPU",
                        ),
                        (
                            &self.sandbox.sandbox_resources_memory_request,
                            &self.sandbox.sandbox_resources_memory_limit,
                            "sandbox memory",
                        ),
                        (
                            &self.sandbox.resources_cpu_request,
                            &self.sandbox.resources_cpu_limit,
                            "server CPU",
                        ),
                        (
                            &self.sandbox.resources_memory_request,
                            &self.sandbox.resources_memory_limit,
                            "server memory",
                        ),
                    ] {
                        if request != limit {
                            return Err(ConfigError::Invalid(format!(
                                "K8S_COMPUTE_MODE=fargate requires equal {name} requests and limits"
                            )));
                        }
                    }
                    if self
                        .sandbox
                        .ephemeral_storage
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty())
                    {
                        return Err(ConfigError::Invalid(
                            "K8S_COMPUTE_MODE=fargate requires K8S_SANDBOX_EPHEMERAL_STORAGE"
                                .into(),
                        ));
                    }
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
            SandboxProviderType::Local => {}
        }

        if self.secrets.encryption_key.is_none() {
            return Err(ConfigError::Invalid(
                "SECRETS_ENCRYPTION_KEY is required".into(),
            ));
        }

        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Config {
    /// Minimal valid config for unit tests, and for gRPC service test
    /// constructors that need a `Config` value but never read it.
    ///
    /// Public so downstream test tiers reach it via core's `test-support`
    /// feature; no shipping build enables that feature.
    pub fn test_minimal() -> Self {
        Config {
            server: ServerConfig {
                port: 8080,
                metrics_port: 9090,
                grpc_port: 50051,
                worker_identity: "test-worker".into(),
            },
            sandbox_provider: SandboxProviderType::Kubernetes,
            sandbox: SandboxConfig {
                compute_mode: KubernetesComputeMode::Standard,
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
                ephemeral_storage: None,
                ephemeral_storage_limit: None,
                run_as_non_root: false,
                shared_data_size: "2Gi".into(),
                persist_toolchain: false,
                storage_enabled: false,
                storage_size: "1Gi".into(),
                storage_class: None,
                default_pvc_enabled: true,
                default_s3_enabled: true,
                workspace_repository_secret: None,
                workspace_repository_role_arn: None,
                node_archive_enabled: false,
                retain_storage_on_close: false,
                pod_startup_timeout_secs: 10,
                git_clone_stall_timeout_secs: None,
                task_token_secret: None,
                user_token_ttl_secs: 7 * 24 * 3600,
                bootstrap_token_ttl_secs: 1800,
                emit_legacy_user_token_env: true,
                accept_claimless_tokens: true,
                file_mount_path: "/tmp".into(),
                idle_reaper_enabled: false,
                cron_scheduler_enabled: false,
                liveness_enabled: false,
                liveness_period_secs: 15,
                liveness_timeout_secs: 10,
                liveness_failure_threshold: 20,
                idle_timeout_minutes: 15,
                idle_check_interval_secs: 300,
                close_after_settled_days: 0,
                settle_after_hours: 0,
                cold_after_hours: 0,
                orphan_gc_enabled: false,
                codex_refresher_interval_secs: 0,
                max_crash_restart_count: 5,
                node_selector_json: None,
                tolerations_json: None,
                service_account_name: None,
                default_model: None,
                default_codex_model: DEFAULT_CODEX_MODEL.to_string(),
                extra_env: vec![],
                claude_disallowed_tools: None,
                cache_host_root: "/mnt/cache".into(),
                docker_dind_image: "docker:dind".into(),
                docker_dind_storage_size: "10Gi".into(),
                docker_dind_cpu_request: "500m".into(),
                docker_dind_cpu_limit: "2".into(),
                docker_dind_memory_request: "1Gi".into(),
                docker_dind_memory_limit: "4Gi".into(),
                egress_proxy_url: None,
                egress_proxy_profiles: vec![],
                network_profiles: vec![],
                allowed_service_accounts: vec![],
                egress_proxy_no_proxy: "localhost,.svc.cluster.local".into(),
                local_passthrough_env: vec![],
            },
            proxy: ProxyConfig {
                domain: None,
                protocol: "http".into(),
                traefik_auth_middleware_name: "test".into(),
                traefik_auth_middleware_namespace: "test".into(),
                traefik_entry_point: "web".into(),
                traefik_ingress_namespaces: vec!["test".into()],
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
            local_sandbox: LocalSandboxConfig {
                root: "/tmp/moatless-local".into(),
                bin: "sandbox".into(),
            },
            database_url: Some("postgres://localhost/test".into()),
            database: DatabaseConfig {
                url: Some("postgres://localhost/test".into()),
                max_connections: 5,
                run_migrations: true,
            },
            event_handler_enabled: true,
            sse: SseConfig {
                broadcast_capacity: 16,
            },
            auth: AuthConfig {
                provider: AuthProvider::Local,
                github_required_org: None,
                auth0_domain: None,
                auth0_client_id: None,
                auth0_client_secret: None,
                auth0_callback_url: None,
                auth0_audience: None,
                auth0_groups_claim: String::new(),
                oidc: None,
                session_max_age_secs: 3600,
                session_absolute_age_secs: 7200,
                frontend_url: "http://localhost:3000".into(),
                csrf_trusted_origins: Vec::new(),
                cookie_domain: None,
                session_cookie_name: "test".into(),
                is_production: false,
                admin_username: "admin".into(),
                admin_email: Some("admin@example.com".into()),
                admin_password: None,
                admin_user_ids: Vec::new(),
                break_glass_enabled: false,
                cache_ttl_secs: 0,
                session_jwt_secret: Some("test-session-secret".into()),
                test_api_key: None,
                default_team_slug: None,
                github_app_client_id: None,
                github_app_client_secret: None,
                github_app_callback_url: None,
                github_app_id: None,
                github_app_private_key: None,
                github_app_default_installation_id: None,
            },
            s3: S3Config {
                bucket_name: None,
                key_prefix: "test".into(),
                transcript_prefix: "transcripts".into(),
                workspace_prefix: "workspaces".into(),
                artifact_prefix: "artifacts".into(),
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
            },
            github_api_base: DEFAULT_GITHUB_API_BASE.to_string(),
            linear: LinearConfig {
                webhook_secret: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                oauth_callback_url: None,
                api_base_url: None,
            },
            decision: DecisionConfig { provider: None },
            skill: SkillConfig { cache_ttl_secs: 60 },
            declared_plugins: vec![],
            secrets: SecretsConfig {
                encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
                kms_key_id: None,
            },
            codex_oauth: CodexOAuthConfig {
                issuer: "https://auth.openai.com".into(),
                client_id: "test-codex-client".into(),
                refresh_url: CODEX_REFRESH_URL_DEFAULT.into(),
            },
            otel: OtelConfig {
                otlp_endpoint: None,
                sampling_ratio: 0.1,
                service_name: "test".into(),
            },
            t3_ui_rpc: T3UiRpcConfig::default(),
            feature_flags: FeatureFlags::default(),
            message_buffer: MessageBufferConfig::default(),
            http: HttpConfig::default(),
            workspace_sync: WorkspaceSyncConfig::default(),
            model_gateway: ModelGatewayConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests;

pub use crate::workspace_identity_grants::WorkspaceSyncConfig;
