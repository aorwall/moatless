//! Configuration module for the Moatless Vibe backend service.
//!
//! Loads configuration from environment variables with sensible defaults.
//! Config is grouped into logical sub-structs for readability and to enable
//! passing only relevant config to subsystems.

use std::env;
use std::fmt;
use std::str::FromStr;

use moatless_types::adapter::LinearConfig;
use moatless_types::agent::CodexOAuthConfig;
use moatless_types::auth::{AllowedOrigin, AuthConfig, AuthProvider, GitHubAppConfig, OidcConfig};
use moatless_types::sandbox::SandboxProviderType;
use moatless_types::secret::SecretsConfig;

/// Default Codex model forwarded to sandbox agents when the deployment does not
/// override `DEFAULT_CODEX_MODEL`.
pub const DEFAULT_CODEX_MODEL: &str = "gpt-6-astra";

/// Default GitHub REST API base URL.
const DEFAULT_API_BASE: &str = "https://api.github.com";

/// Resolve the GitHub API base URL. `GITHUB_API_BASE_URL` overrides the
/// default — used by E2E tests to point the backend at a mock server (and
/// usable for GitHub Enterprise hosts). Empty values are treated as unset.
pub fn api_base() -> String {
    env::var("GITHUB_API_BASE_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

/// Minimum length accepted for an operator-supplied `ADMIN_PASSWORD`.
pub const MIN_ADMIN_PASSWORD_LEN: usize = 12;

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

/// Origins the CSRF check trusts beyond `FRONTEND_URL`, from
/// `CSRF_TRUSTED_ORIGINS`.
///
/// Deliberately not defaulted from `T3_UI_RPC_ALLOWED_ORIGINS`, which the
/// WebSocket upgrade reads: that list carries wildcard sandbox-preview origins,
/// and a preview serves agent-authored code. Under a `COOKIE_DOMAIN` covering
/// both, inheriting it would let a preview origin drive cookie-authenticated
/// writes. Name a REST origin here, exactly.
fn csrf_trusted_origins_from_env() -> Vec<AllowedOrigin> {
    moatless_types::auth::parse_allowed_origins(
        &env_opt("CSRF_TRUSTED_ORIGINS").unwrap_or_default(),
    )
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
    /// Kubernetes compute substrate. Env `K8S_COMPUTE_MODE` (`standard` or `fargate`).
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
    /// CPU request for the sandbox (agent) container (env `K8S_RESOURCE_REQUEST_CPU`)
    pub sandbox_resources_cpu_request: String,
    /// CPU limit for the sandbox (agent) container (env `K8S_RESOURCE_LIMIT_CPU`)
    pub sandbox_resources_cpu_limit: String,
    /// Memory request for the sandbox (agent) container (env `K8S_RESOURCE_REQUEST_MEMORY`)
    pub sandbox_resources_memory_request: String,
    /// Memory limit for the sandbox (agent) container (env `K8S_RESOURCE_LIMIT_MEMORY`)
    pub sandbox_resources_memory_limit: String,
    /// Optional ephemeral-storage request and limit for the sandbox container.
    /// EKS Fargate uses the request to provision pod-local disk. Env
    /// `K8S_SANDBOX_EPHEMERAL_STORAGE`.
    pub ephemeral_storage: Option<String>,
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
    /// Persist cargo's downloaded state (registry + git caches) across pod
    /// replacement by mounting `workspace-data` subPaths at
    /// `/opt/cargo/registry` and `/opt/cargo/git`. Costs PVC space — bump the
    /// repo's storage size when enabling (a warm registry is multiple Gi).
    /// Env `K8S_SANDBOX_PERSIST_TOOLCHAIN` (default `false`).
    pub persist_toolchain: bool,
    /// Whether PVC storage is enabled for sandbox pods
    pub storage_enabled: bool,
    /// PVC storage size (e.g., "3Gi")
    pub storage_size: String,
    /// PVC storage class
    pub storage_class: Option<String>,
    /// Whether sandboxes get a per-task PVC when the repo doesn't set
    /// `resources.pvc_enabled` (env `K8S_SANDBOX_DEFAULT_PVC`, default `true`).
    /// A per-repo switch overrides this; [`Self::storage_enabled`] overrides both.
    pub default_pvc_enabled: bool,
    /// Whether sandboxes snapshot their workspace to S3 when the repo doesn't set
    /// `resources.s3_enabled` (env `K8S_SANDBOX_DEFAULT_S3`, default `true`).
    /// A per-repo switch overrides this. Requires the S3 uploads bucket to be
    /// configured; without it the sandbox's sync calls no-op.
    pub default_s3_enabled: bool,
    /// Whether an S3-syncing sandbox stores its workspace as a per-file mirror
    /// (one object per file plus a manifest) rather than one tar. The mirror is
    /// what makes a stopped task's files reachable object-by-object, and ships
    /// only the delta on each sync. Backward-compatible either way: a reader
    /// tries the mirror then the tar, and a mirror-off sandbox still reads a tar
    /// a mirror-on one never wrote. Env `SANDBOX_WORKSPACE_MIRROR_ENABLED`
    /// (default `true`).
    pub default_workspace_mirror_enabled: bool,
    /// How a sandbox stores its workspace: `tar`, `mirror`, `restic`, or
    /// `restic+mirror`. Empty leaves the sandbox on
    /// [`Self::default_workspace_mirror_enabled`], which is what a fleet that
    /// has not adopted the repository format runs.
    ///
    /// `restic+mirror` is what a soak runs: both formats are written, so
    /// rolling back to `mirror` loses nothing.
    /// Env `SANDBOX_WORKSPACE_SNAPSHOT_FORMAT`.
    pub workspace_snapshot_format: Option<String>,
    /// Master secret every task's workspace-repository password is derived
    /// from (`moatless_core::workspace_repository::derive_repository_password`).
    /// Without it no restic repository can be opened or written, and the
    /// snapshot falls back to the mirror and the tar.
    /// Env `SANDBOX_WORKSPACE_REPOSITORY_SECRET`.
    pub workspace_repository_secret: Option<String>,
    /// IAM role the backend assumes to hand a sandbox direct S3 access to its
    /// own workspace repository. Each session is narrowed further by a session
    /// policy naming that one task's prefix, so the role itself stays
    /// bucket-wide. Without it the sandbox cannot write a repository.
    /// Env `SANDBOX_WORKSPACE_REPOSITORY_ROLE_ARN`.
    pub workspace_repository_role_arn: Option<String>,
    /// Whether Kubernetes node-local workspace/Docker archives are tracked and
    /// managed across pod lifecycles. When disabled, `node` storage still uses
    /// hostPath volumes, but the backend does not access archive `ConfigMap`s,
    /// pin replacements to recorded nodes, or run deterministic node cleanup.
    /// Env `K8S_SANDBOX_NODE_ARCHIVE_ENABLED` (default: `false`).
    pub node_archive_enabled: bool,
    /// When `true`, closing a task preserves its managed archives (S3 plus
    /// Docker and opt-in K8s node-local disk) instead of deleting them. This is
    /// what makes a closed task revivable via `POST /tasks/{id}/reopen` —
    /// retained S3 archives are restored and managed node archives are pinned
    /// on the next provision.
    /// K8s runtime resources (Deployment/Service/PVC/IngressRoutes) are still
    /// torn down.
    /// Env `SANDBOX_RETAIN_STORAGE_ON_CLOSE` (default: `true`). Permanent
    /// task deletion always removes storage regardless of this flag. Retained
    /// archives are bounded by the uploads bucket's own expiry rule, not by
    /// close.
    pub retain_storage_on_close: bool,
    /// Pod startup timeout in seconds
    pub pod_startup_timeout_secs: u64,
    /// Secret for signing task auth JWTs
    pub task_token_secret: Option<String>,
    /// TTL (seconds) for the long-lived user auth token delivered to sandboxes
    /// over `GetCredentials`. Re-minted on every credential fetch, so this only
    /// needs to outlive the 15-min refresher gap. Env `SANDBOX_USER_TOKEN_TTL_SECS`
    /// (default: 604800 = 7 days).
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
    /// Whether the sandbox gRPC verification layer accepts legacy claim-less
    /// tokens (those minted before the per-task `taskId` binding existed). When
    /// `true`, such a token falls back to the DB task-access check; when `false`,
    /// every token must carry a matching `taskId` claim. Grace flag for the
    /// rollout — flip to `false` once every live sandbox holds a task-bound
    /// token. Env `SANDBOX_ACCEPT_CLAIMLESS_TOKENS` (default: true).
    pub accept_claimless_tokens: bool,
    /// Directory inside sandbox pods that attached uploads are written to
    /// (env `SANDBOX_FILE_MOUNT_PATH`, default
    /// `/opt/moatless/workspace/uploads`). It sits under the sandbox's
    /// `WORKSPACE_DIR` so uploads ride the workspace snapshot and survive a
    /// restart. This is the single source of truth for the path: the agent
    /// prompt's "Saved to:" line is rendered from it, and the runtime providers
    /// pass it to the pod as `UPLOAD_DIR` so the sandbox writes where the prompt
    /// says.
    pub file_mount_path: String,
    /// Whether the idle sandbox reaper is enabled (default: true)
    pub idle_reaper_enabled: bool,
    /// Whether the cron scheduler is enabled. A shadow backend shares the real
    /// deployment's database and Leases, so it must not fire scheduled jobs.
    /// Env `CRON_SCHEDULER_ENABLED` (default: true).
    pub cron_scheduler_enabled: bool,
    /// Whether to attach a liveness probe to the sandbox container so K8s
    /// restarts a hung-but-alive pod (agent actor wedged while `/health` still
    /// reports Ready). Default OFF: a false-positive restart kills an in-flight
    /// agent turn, which is worse than a rare hang. Env `K8S_SANDBOX_LIVENESS_ENABLED`.
    pub liveness_enabled: bool,
    /// Stop sandboxes idle longer than this (minutes, default: 15)
    pub idle_timeout_minutes: u64,
    /// How often to check for idle sandboxes (seconds, default: 300 = 5 min)
    pub idle_check_interval_secs: u64,
    /// Auto-close Open tasks inactive for this many days. `0` disables
    /// auto-close. Env: `TASK_INACTIVE_TIMEOUT_DAYS` (default: 7).
    pub inactive_task_timeout_days: u64,
    /// Reclaim the Deployment and PVC of a sandbox that has been Stopped this
    /// many hours, leaving the S3 snapshot as the only copy. The Task stays
    /// Open and a later start provisions a fresh volume and restores into it.
    /// Only tasks on the pvc+s3 tier are eligible — without a snapshot there is
    /// nothing to reclaim into. `0` disables the transition. Env:
    /// `SANDBOX_COLD_AFTER_HOURS` (default: 24). A per-task
    /// `tasks.cold_after_hours` overrides this.
    pub cold_after_hours: u64,
    /// Whether the reaper's orphan-GC pass is enabled (default: true). When on,
    /// the reaper deletes labeled sandbox K8s resources whose Task row no
    /// longer exists. Env: `SANDBOX_ORPHAN_GC_ENABLED`.
    pub orphan_gc_enabled: bool,
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
    /// Kubernetes `ServiceAccount` name for sandbox pods (set via `K8S_SANDBOX_SERVICE_ACCOUNT`)
    pub service_account_name: Option<String>,
    /// Default Claude model for sandbox agents (forwarded as `DEFAULT_CLAUDE_CODE_MODEL` env var)
    pub default_model: Option<String>,
    /// Default Codex model for sandbox agents (forwarded as `DEFAULT_CODEX_MODEL` env var)
    pub default_codex_model: Option<String>,
    /// Extra env vars injected into every sandbox pod (parsed from `K8S_SANDBOX_EXTRA_ENV` JSON)
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
    /// Size cap for the `dind` sidecar's `/var/lib/docker` emptyDir when
    /// `resources.dockerStorageMode` is `emptyDir`. The default `node` mode uses
    /// hostPath storage instead and is bounded by node disk capacity. Env
    /// `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE` (default `10Gi`).
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
    /// In-cluster egress forward-proxy URL (e.g.
    /// `http://moatless-vibe-egress-proxy.<ns>.svc.cluster.local:3128`). When
    /// set, sandbox pods whose `network_profile` is listed in
    /// [`SandboxConfig::egress_proxy_profiles`] get `HTTPS_PROXY`/`HTTP_PROXY`
    /// pointed at it, so all outbound traffic is funnelled through the proxy's
    /// hostname allowlist. Pairs with a per-profile `NetworkPolicy` that blocks
    /// direct internet egress and forces this path. Env
    /// `SANDBOX_EGRESS_PROXY_URL` (default: unset → no proxy injected).
    pub egress_proxy_url: Option<String>,
    /// Network profiles that receive the egress proxy env (see
    /// [`SandboxConfig::egress_proxy_url`]). A pod gets the proxy only when its
    /// `network_profile` value is in this list. Env
    /// `SANDBOX_EGRESS_PROXY_PROFILES` (comma-separated; default: empty →
    /// opt-in, no pod gets the proxy even if a URL is set).
    pub egress_proxy_profiles: Vec<String>,
    /// `NO_PROXY` value paired with [`SandboxConfig::egress_proxy_url`] so
    /// in-cluster traffic (backend gRPC, DNS, kube-apiserver, link-local
    /// metadata) bypasses the proxy. Env `SANDBOX_EGRESS_PROXY_NO_PROXY`
    /// (default covers localhost, the cluster service domain, RFC-1918 and the
    /// link-local metadata IP).
    pub egress_proxy_no_proxy: String,
}

/// Proxy / Traefik `IngressRoute` settings.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Domain for proxy URLs (e.g., "moatless.xyz"). If None, `IngressRoute`s are not created.
    pub domain: Option<String>,
    /// Protocol for proxy URLs (default: "https")
    pub protocol: String,
    /// Name of the Traefik `ForwardAuth` Middleware CRD (default: "preview-auth")
    pub traefik_auth_middleware_name: String,
    /// Namespace where the Traefik auth Middleware lives (defaults to sandbox namespace)
    pub traefik_auth_middleware_namespace: String,
    /// Traefik entryPoint name for `IngressRoute`s (default: "websecure")
    pub traefik_entry_point: String,
    /// Namespaces whose Traefik ingress pods (`app.kubernetes.io/name=traefik`)
    /// the per-sandbox preview `NetworkPolicy` admits to the preview ports. The
    /// policy is what makes a preview URL reachable once ingress `NetworkPolicy`s
    /// are enforced; deriving its ports from the sandbox's own server map (see
    /// `k8s/networkpolicy.rs`) is what keeps it from drifting out of sync.
    pub traefik_ingress_namespaces: Vec<String>,
}

/// Redis connection and stream settings.
#[derive(Debug, Clone)]
pub struct RedisConfig {
    /// Redis connection URL. When set, enables event publishing and SSE consumers.
    pub url: Option<String>,
    /// Maximum stream length for MAXLEN ~ trimming (default: 500).
    ///
    /// Sized for the live SSE tail, which is all the stream is for — history is
    /// served from `task_events` in Postgres. `MAXLEN` bounds entry count only;
    /// per-entry bytes are capped separately in `events.rs`.
    pub max_stream_length: usize,
    /// Whether the event-handler consumer runs (default: true). Its Redis
    /// consumer group is a shared constant (`event-handler`), so a second
    /// backend joining the same Redis would split the group's workload with the
    /// primary. Set `EVENT_HANDLER_ENABLED=false` on a shadow/dev backend that
    /// shares the primary's Redis so it never claims events meant for it.
    pub event_handler_enabled: bool,
}

/// SSE (Server-Sent Events) settings.
#[derive(Debug, Clone)]
pub struct SseConfig {
    /// Broadcast buffer capacity for SSE fan-out (default: 4096)
    pub broadcast_capacity: usize,
    /// Consumer ID for the SSE Redis consumer group (default: "sse-{HOSTNAME}")
    pub consumer_id: String,
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
    /// Whether to apply pending migrations at startup (default: true).
    /// Env `RUN_MIGRATIONS=false` disables them — used by the shadow-dev
    /// backend so a dev instance never mutates the shared dev schema.
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
/// `otlp_endpoint: None` (the default — `OTEL_EXPORTER_OTLP_ENDPOINT` unset)
/// means trace export is off: `init_tracing` omits the OTel layer entirely and
/// the process behaves exactly as it did before this config existed.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// OTLP/gRPC collector endpoint (e.g. `http://tempo:4317`).
    pub otlp_endpoint: Option<String>,
    /// Head sampling ratio for the parent-based sampler, applied only to
    /// traces with no sampling decision already made upstream (env:
    /// `OTEL_TRACES_SAMPLER_ARG`, matching the OTel SDK's own name for a
    /// ratio-sampler argument).
    pub sampling_ratio: f64,
    /// OTel `service.name` resource (env: `OTEL_SERVICE_NAME`).
    pub service_name: String,
}

impl OtelConfig {
    const DEFAULT_SAMPLING_RATIO: f64 = 0.1;
    const DEFAULT_SERVICE_NAME: &'static str = "moatless-vibe-backend";

    /// Read directly from the environment, without a `tracing::warn!` on a
    /// malformed value — this runs once from `main()` before
    /// `startup::init_tracing` installs a subscriber, and a warning emitted
    /// before that point has nowhere to land. [`Config::from_env`] reads the
    /// same variables into [`Config::otel`] afterwards, once logging is live.
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
    pub otel: OtelConfig,
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

        // Namespaces running the Traefik ingress pods that terminate preview
        // traffic. Defaults to the sandbox namespace (where preview IngressRoutes
        // are created) plus the shared `platform-ingress`; a comma-separated env
        // overrides. Duplicates are collapsed so a repeated `from` peer isn't
        // emitted when the two coincide.
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
                run_as_non_root: env_bool("K8S_SANDBOX_RUN_AS_NON_ROOT", false),
                shared_data_size: env_str("K8S_SANDBOX_SHARED_DATA_SIZE", "2Gi"),
                persist_toolchain: env_bool("K8S_SANDBOX_PERSIST_TOOLCHAIN", false),
                storage_enabled: env_bool("K8S_SANDBOX_STORAGE_ENABLED", true),
                storage_size: env_str("K8S_SANDBOX_STORAGE_SIZE", "3Gi"),
                storage_class: env_opt("K8S_SANDBOX_STORAGE_CLASS"),
                default_pvc_enabled: env_bool("K8S_SANDBOX_DEFAULT_PVC", true),
                default_s3_enabled: env_bool("K8S_SANDBOX_DEFAULT_S3", true),
                default_workspace_mirror_enabled: env_bool(
                    "SANDBOX_WORKSPACE_MIRROR_ENABLED",
                    true,
                ),
                workspace_snapshot_format: env_opt("SANDBOX_WORKSPACE_SNAPSHOT_FORMAT"),
                workspace_repository_secret: env_opt("SANDBOX_WORKSPACE_REPOSITORY_SECRET"),
                workspace_repository_role_arn: env_opt("SANDBOX_WORKSPACE_REPOSITORY_ROLE_ARN"),
                node_archive_enabled: env_bool("K8S_SANDBOX_NODE_ARCHIVE_ENABLED", false),
                retain_storage_on_close: env_bool("SANDBOX_RETAIN_STORAGE_ON_CLOSE", true),
                pod_startup_timeout_secs: env_or("K8S_POD_STARTUP_TIMEOUT_SECS", 120),
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
                idle_timeout_minutes: env_or("SANDBOX_IDLE_TIMEOUT_MINUTES", 15),
                idle_check_interval_secs: env_or("SANDBOX_IDLE_CHECK_INTERVAL_SECS", 300),
                inactive_task_timeout_days: env_or("TASK_INACTIVE_TIMEOUT_DAYS", 7),
                cold_after_hours: env_or("SANDBOX_COLD_AFTER_HOURS", 24),
                orphan_gc_enabled: env_bool("SANDBOX_ORPHAN_GC_ENABLED", true),
                codex_refresher_interval_secs: env_or("CODEX_REFRESHER_INTERVAL_SECS", 3600),
                max_crash_restart_count: env_or("SANDBOX_MAX_CRASH_RESTART_COUNT", 5),
                tolerations_json: env_opt("K8S_SANDBOX_TOLERATIONS"),
                node_selector_json: env_opt("K8S_SANDBOX_NODE_SELECTOR"),
                service_account_name: env_opt("K8S_SANDBOX_SERVICE_ACCOUNT"),
                default_model: env_opt("DEFAULT_CLAUDE_CODE_MODEL"),
                default_codex_model: Some(env_str("DEFAULT_CODEX_MODEL", DEFAULT_CODEX_MODEL)),
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
                docker_dind_storage_size: env_str("K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE", "10Gi"),
                docker_dind_cpu_request: env_str("K8S_SANDBOX_DOCKER_DIND_CPU_REQUEST", "500m"),
                docker_dind_cpu_limit: env_str("K8S_SANDBOX_DOCKER_DIND_CPU_LIMIT", "2"),
                docker_dind_memory_request: env_str(
                    "K8S_SANDBOX_DOCKER_DIND_MEMORY_REQUEST",
                    "1Gi",
                ),
                docker_dind_memory_limit: env_str("K8S_SANDBOX_DOCKER_DIND_MEMORY_LIMIT", "4Gi"),
                egress_proxy_url: env_opt("SANDBOX_EGRESS_PROXY_URL"),
                egress_proxy_profiles: env_opt("SANDBOX_EGRESS_PROXY_PROFILES")
                    .map(|s| {
                        s.split(',')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                egress_proxy_no_proxy: env_str(
                    "SANDBOX_EGRESS_PROXY_NO_PROXY",
                    "localhost,127.0.0.1,::1,.svc,.svc.cluster.local,.cluster.local,\
                     10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,169.254.169.254,kubernetes.default",
                ),
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

            database_url: database_url.clone(),

            database: DatabaseConfig {
                url: database_url,
                max_connections: env_or("DATABASE_MAX_CONNECTIONS", 5),
                run_migrations: env_bool("RUN_MIGRATIONS", true),
            },

            redis: RedisConfig {
                url: env_opt("REDIS_URL"),
                max_stream_length: env_or("REDIS_MAX_STREAM_LENGTH", 500),
                event_handler_enabled: env_bool("EVENT_HANDLER_ENABLED", true),
            },

            sse: SseConfig {
                broadcast_capacity: env_or("SSE_BROADCAST_CAPACITY", 4096),
                consumer_id: env::var("SSE_CONSUMER_ID").unwrap_or_else(|_| {
                    let hostname = env_str("HOSTNAME", "local");
                    format!("sse-{hostname}")
                }),
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
            },

            linear: LinearConfig {
                webhook_secret: env_opt("LINEAR_CHANNEL_WEBHOOK_SECRET"),
                oauth_client_id: env_opt("LINEAR_OAUTH_CLIENT_ID"),
                oauth_client_secret: env_opt("LINEAR_OAUTH_CLIENT_SECRET"),
                oauth_callback_url: env_opt("LINEAR_OAUTH_CALLBACK_URL"),
                api_base_url: env_opt("LINEAR_API_BASE_URL"),
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

            otel: OtelConfig::from_env(),
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration consistency.
    ///
    /// Made `pub(crate)` so tests can call it directly without env vars.
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        // Production must be able to authenticate sandbox gRPC calls. Without the
        // signing secret the verification layer would fall back to its dev bypass
        // (trusting every caller), so refuse to boot. Dev/local may omit it and
        // run unauthenticated.
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

        // Validate auth provider config consistency
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
                    if !self.sandbox.cache_volumes.is_empty() || self.sandbox.node_archive_enabled {
                        return Err(ConfigError::Invalid(
                            "K8S_COMPUTE_MODE=fargate does not support node-local cache volumes or archives"
                                .into(),
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

#[cfg(any(test, feature = "test-support"))]
impl Config {
    /// Minimal valid config for unit tests across the crate, and for the gRPC
    /// service's db-backed test constructors (which need a `Config` value but
    /// never read it). Public so downstream test tiers reach it off core's
    /// `test-support` feature; no shipping build enables that feature.
    pub fn test_minimal() -> Self {
        Config {
            server: ServerConfig {
                port: 8080,
                metrics_port: 9090,
                grpc_port: 50051,
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
                run_as_non_root: false,
                shared_data_size: "2Gi".into(),
                persist_toolchain: false,
                storage_enabled: false,
                storage_size: "1Gi".into(),
                storage_class: None,
                default_pvc_enabled: true,
                default_s3_enabled: true,
                default_workspace_mirror_enabled: true,
                workspace_snapshot_format: None,
                workspace_repository_secret: None,
                workspace_repository_role_arn: None,
                node_archive_enabled: false,
                retain_storage_on_close: false,
                pod_startup_timeout_secs: 10,
                task_token_secret: None,
                user_token_ttl_secs: 7 * 24 * 3600,
                bootstrap_token_ttl_secs: 1800,
                emit_legacy_user_token_env: true,
                accept_claimless_tokens: true,
                file_mount_path: "/tmp".into(),
                idle_reaper_enabled: false,
                cron_scheduler_enabled: false,
                liveness_enabled: false,
                idle_timeout_minutes: 15,
                idle_check_interval_secs: 300,
                inactive_task_timeout_days: 0,
                cold_after_hours: 0,
                orphan_gc_enabled: false,
                codex_refresher_interval_secs: 0,
                max_crash_restart_count: 5,
                node_selector_json: None,
                tolerations_json: None,
                service_account_name: None,
                default_model: None,
                default_codex_model: Some(DEFAULT_CODEX_MODEL.to_string()),
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
                egress_proxy_url: None,
                egress_proxy_profiles: vec![],
                egress_proxy_no_proxy: "localhost,.svc.cluster.local".into(),
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
            database_url: Some("postgres://localhost/test".into()),
            database: DatabaseConfig {
                url: Some("postgres://localhost/test".into()),
                max_connections: 5,
                run_migrations: true,
            },
            redis: RedisConfig {
                url: None,
                max_stream_length: 100,
                event_handler_enabled: true,
            },
            sse: SseConfig {
                broadcast_capacity: 16,
                consumer_id: "test".into(),
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
            },
            linear: LinearConfig {
                webhook_secret: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                oauth_callback_url: None,
                api_base_url: None,
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
            otel: OtelConfig {
                otlp_endpoint: None,
                sampling_ratio: 0.1,
                service_name: "test".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
    use std::sync::{Mutex, PoisonError};

    use super::*;

    // =========================================================================
    // CSRF trusted origins
    // =========================================================================

    /// Restore whatever the process had, so this test does not leak into the
    /// others sharing the environment.
    const ORIGIN_VARS: [&str; 2] = ["CSRF_TRUSTED_ORIGINS", "T3_UI_RPC_ALLOWED_ORIGINS"];

    /// The environment is process-wide and `cargo test` runs a binary's tests on
    /// several threads, so the two tests below would otherwise interleave: one
    /// sets `CSRF_TRUSTED_ORIGINS` while the other asserts it is absent.
    static ORIGIN_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn apply_origin_vars(values: [Option<String>; 2]) {
        for (name, value) in ORIGIN_VARS.into_iter().zip(values) {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }

    /// Run `check` with the two variables set, restoring what the process had.
    ///
    /// Restores through a panic as well: a failed assertion that leaked its
    /// variables would fail an unrelated test later, far from the cause.
    fn with_origin_vars(csrf: Option<&str>, websocket: Option<&str>, check: impl FnOnce()) {
        // A failing test poisons the lock. The guarded data is `()`, so there is
        // no torn state to protect the next test from.
        let _guard = ORIGIN_ENV_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        let previous = ORIGIN_VARS.map(|name| std::env::var(name).ok());
        apply_origin_vars([csrf.map(str::to_string), websocket.map(str::to_string)]);

        let outcome = catch_unwind(AssertUnwindSafe(check));

        apply_origin_vars(previous);
        if let Err(panic) = outcome {
            resume_unwind(panic);
        }
    }

    /// The WebSocket allow-list must not reach the CSRF check: it carries
    /// wildcard preview origins, and a preview serves agent-authored code.
    #[test]
    fn the_websocket_allow_list_is_not_a_source_of_csrf_trust() {
        with_origin_vars(
            None,
            Some("https://*.preview.example,https://t3.example"),
            || assert!(csrf_trusted_origins_from_env().is_empty()),
        );
    }

    #[test]
    fn csrf_trust_is_read_from_its_own_variable() {
        with_origin_vars(Some("https://t3.example"), None, || {
            assert_eq!(
                csrf_trusted_origins_from_env(),
                vec![AllowedOrigin::Exact("https://t3.example".into())]
            );
        });
    }

    // =========================================================================
    // AuthProvider parsing
    // =========================================================================

    #[test]
    fn auth_provider_from_str_github_returns_error() {
        assert!("github".parse::<AuthProvider>().is_err());
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
        // A typo must fail closed rather than silently disabling auth.
        assert!("guthub".parse::<AuthProvider>().is_err());
    }

    #[test]
    fn auth_provider_display() {
        assert_eq!(AuthProvider::GitHubApp.to_string(), "github_app");
        assert_eq!(AuthProvider::Auth0.to_string(), "auth0");
        assert_eq!(AuthProvider::Oidc.to_string(), "oidc");
    }

    #[test]
    fn config_defaults_codex_model_to_gpt_56_sol() {
        let previous = std::env::var("DEFAULT_CODEX_MODEL").ok();
        std::env::remove_var("DEFAULT_CODEX_MODEL");
        let model = env_str("DEFAULT_CODEX_MODEL", DEFAULT_CODEX_MODEL);
        match previous {
            Some(value) => std::env::set_var("DEFAULT_CODEX_MODEL", value),
            None => std::env::remove_var("DEFAULT_CODEX_MODEL"),
        }
        assert_eq!(model, DEFAULT_CODEX_MODEL);
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
        Config::test_minimal()
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
    fn auth_provider_defaults_to_local() {
        assert_eq!(AuthProvider::default(), AuthProvider::Local);
        assert!(minimal_config().validate().is_ok());
    }

    #[test]
    fn auth_provider_parses_legacy_no_auth_spellings_as_local() {
        for raw in ["", "disabled", "none", "local", "LOCAL"] {
            assert_eq!(raw.parse::<AuthProvider>().unwrap(), AuthProvider::Local);
        }
    }

    #[test]
    fn auth_provider_rejects_unknown_value() {
        assert!("guthub".parse::<AuthProvider>().is_err());
    }

    #[test]
    fn opaque_browser_sessions_do_not_require_a_signing_secret() {
        let mut config = minimal_config();
        config.auth.session_jwt_secret = None;
        config.sandbox.task_token_secret = None;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_local_accepts_task_token_secret_fallback() {
        let mut config = minimal_config();
        config.auth.session_jwt_secret = None;
        config.sandbox.task_token_secret = Some("fallback-secret".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_local_requires_database_url() {
        let mut config = minimal_config();
        config.database.url = None;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn validate_local_rejects_short_admin_password() {
        let mut config = minimal_config();
        config.auth.admin_password = Some("short".into());
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("ADMIN_PASSWORD"));
    }

    #[test]
    fn validate_local_accepts_long_admin_password() {
        let mut config = minimal_config();
        config.auth.admin_password = Some("a-sufficiently-long-password".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_production_with_auth_provider_passes() {
        let mut config = minimal_config();
        config.auth.is_production = true;
        config.auth.provider = AuthProvider::GitHubApp;
        config.auth.github_app_client_id = Some("id".into());
        config.auth.github_app_client_secret = Some("secret".into());
        config.auth.github_app_callback_url = Some("https://example.com/callback".into());
        config.sandbox.task_token_secret = Some("task-secret".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_production_without_task_token_secret_fails() {
        // Production must be able to authenticate sandbox gRPC calls; without the
        // signing secret the verification layer would fall back to its dev bypass.
        let mut config = minimal_config();
        config.auth.is_production = true;
        config.auth.provider = AuthProvider::GitHubApp;
        config.auth.github_app_client_id = Some("id".into());
        config.auth.github_app_client_secret = Some("secret".into());
        config.auth.github_app_callback_url = Some("https://example.com/callback".into());
        config.sandbox.task_token_secret = None;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("TASK_TOKEN_SECRET"));
    }

    #[test]
    fn validate_github_app_missing_callback_url_fails() {
        let mut config = minimal_config();
        config.auth.provider = AuthProvider::GitHubApp;
        config.auth.github_app_client_id = Some("id".into());
        config.auth.github_app_client_secret = Some("secret".into());
        config.auth.github_app_callback_url = None;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("GITHUB_APP_CALLBACK_URL"));
    }

    #[test]
    fn validate_github_app_complete_passes() {
        let mut config = minimal_config();
        config.auth.provider = AuthProvider::GitHubApp;
        config.auth.github_app_client_id = Some("id".into());
        config.auth.github_app_client_secret = Some("secret".into());
        config.auth.github_app_callback_url = Some("https://example.com/callback".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_auth0_missing_domain_fails() {
        let mut config = minimal_config();
        config.auth.provider = AuthProvider::Auth0;
        config.auth.auth0_domain = None;
        config.auth.auth0_client_id = Some("id".into());
        config.auth.auth0_client_secret = Some("secret".into());
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("AUTH0_DOMAIN"));
    }

    #[test]
    fn validate_auth0_complete_passes() {
        let mut config = minimal_config();
        config.auth.provider = AuthProvider::Auth0;
        config.auth.auth0_domain = Some("test.auth0.com".into());
        config.auth.auth0_client_id = Some("id".into());
        config.auth.auth0_client_secret = Some("secret".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_oidc_complete_passes() {
        let mut config = minimal_config();
        config.auth.provider = AuthProvider::Oidc;
        config.auth.oidc = Some(OidcConfig {
            provider_id: "company".into(),
            name: "Company SSO".into(),
            issuer: "https://id.example.com/".into(),
            client_id: "vibe".into(),
            client_secret: "secret".into(),
            callback_url: "https://moatless.example.com/api/v1/auth/oidc/company/callback".into(),
            auto_provision: true,
            auto_link_verified_email: true,
            allowed_domains: vec!["example.com".into()],
        });
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_oidc_rejects_empty_issuer() {
        let mut config = minimal_config();
        config.auth.provider = AuthProvider::Oidc;
        config.auth.oidc = Some(OidcConfig {
            provider_id: "company".into(),
            name: "Company SSO".into(),
            issuer: String::new(),
            client_id: "vibe".into(),
            client_secret: "secret".into(),
            callback_url: "https://moatless.example.com/api/v1/auth/oidc/company/callback".into(),
            auto_provision: true,
            auto_link_verified_email: true,
            allowed_domains: Vec::new(),
        });
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_empty_sandbox_namespace_fails() {
        let mut config = minimal_config();
        config.sandbox.namespace = String::new();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("SANDBOX_NAMESPACE"));
    }

    fn valid_fargate_config() -> Config {
        let mut config = minimal_config();
        config.sandbox.compute_mode = KubernetesComputeMode::Fargate;
        config.sandbox.default_pvc_enabled = false;
        config.sandbox.default_s3_enabled = true;
        config.sandbox.run_as_non_root = true;
        config.sandbox.ephemeral_storage = Some("40Gi".into());
        config.sandbox.sandbox_resources_cpu_request = "4".into();
        config.sandbox.sandbox_resources_cpu_limit = "4".into();
        config.sandbox.sandbox_resources_memory_request = "16Gi".into();
        config.sandbox.sandbox_resources_memory_limit = "16Gi".into();
        config.sandbox.resources_cpu_request = "1000m".into();
        config.sandbox.resources_cpu_limit = "1000m".into();
        config.sandbox.resources_memory_request = "1536Mi".into();
        config.sandbox.resources_memory_limit = "1536Mi".into();
        config
    }

    #[test]
    fn validate_fargate_contract_passes() {
        assert!(valid_fargate_config().validate().is_ok());
    }

    #[test]
    fn validate_fargate_rejects_default_pvc_storage() {
        let mut config = valid_fargate_config();
        config.sandbox.default_pvc_enabled = true;
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("DEFAULT_PVC=false"));
    }

    #[test]
    fn validate_fargate_rejects_disabled_s3_storage() {
        let mut config = valid_fargate_config();
        config.sandbox.default_s3_enabled = false;
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("DEFAULT_S3=true"));
    }

    #[test]
    fn validate_fargate_rejects_burstable_resources() {
        let mut config = valid_fargate_config();
        config.sandbox.sandbox_resources_cpu_request = "1".into();
        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("equal sandbox CPU"));
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
        auth.github_app_client_secret = Some("super_secret_123".into());
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
