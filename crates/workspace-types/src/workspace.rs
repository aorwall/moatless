//! Workspace vocabulary: the composition and run configuration a Workspace
//! carries — its scripts, container sizing, and the providers
//! whose credential can be pinned to it.
//!
//! Every string field that is really a value (CPU/memory quantity, storage
//! medium, env-var name) is parsed at deserialization, strictly — an
//! unrecognised value fails the whole decode instead of becoming a default —
//! and serialized keeping the original spelling, so persisted JSON round-trips
//! byte for byte.

#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )
)]

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use moatless_types::access_control::Scope;

// =============================================================================
// Workspace
// =============================================================================

crate::string_enum!(
    /// How many repository placements a Workspace composes.
    ///
    /// Derived from the `WorkspaceRepo` count, never set by hand.
    WorkspaceKind {
        /// No placement yet.
        Blank => "blank",
        /// Exactly one placement.
        Single => "single",
        /// Two or more placements.
        Multi => "multi",
    }
);

crate::string_enum!(
    /// Who declared a Workspace.
    WorkspaceSource {
        /// Created through the API. Editable from the UI and the write API.
        Manual => "manual",
        /// Reconciled from a repository's `.moatless/workspaces.json`.
        /// Read-only until it is overridden.
        Git => "git",
    }
);

/// The glyph a client renders beside a Workspace name.
///
/// Mirrors the T3 client's `ProjectIconOverride`, the only surface that picks
/// one; its own schema enumerates legal glyph names and colours, not restated
/// here to avoid drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkspaceIcon {
    Lucide { name: String, color: String },
    Emoji { emoji: String },
}

/// The composition of one or more repositories plus the run configuration a
/// Task runs against.
///
/// Run-config fields stay `Option` to keep "unset" apart from "empty": a
/// Workspace declaring no scripts differs from one declaring an empty list,
/// reaching the frontend as `null` vs `[]`.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<WorkspaceIcon>,
    pub kind: WorkspaceKind,
    // --- Run configuration ---
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    /// Named commands run on demand in the sandbox.
    pub scripts: Option<Vec<ScriptConfig>>,
    pub resources: Option<ResourceConfig>,
    /// Language servers the agent's code-intelligence tool may start.
    pub language_servers: Option<Vec<LanguageServer>>,
    /// Selects a static `NetworkPolicy` through the pod label. Grants nothing
    /// on its own, and the set of valid names is deployment configuration, so
    /// this stays an open string.
    pub network_profile: Option<String>,
    /// Extra node labels a sandbox pod must match, merged over the
    /// deployment-wide selector rather than replacing it — dropping the global
    /// labels would strand the pod on the taint they pair with. Selects
    /// platform-owned capacity; provisions none.
    pub node_selector: Option<BTreeMap<String, String>>,
    /// The service proxies this Workspace opts into, by name. An opt-in
    /// grants nothing a definition's availability withholds.
    pub services: Option<Vec<String>>,
    // --- Task defaults ---
    /// Model catalog id (`opus`, `sonnet`) a new Task on this Workspace starts
    /// with. An open string: the catalog is deployment configuration.
    pub default_model: Option<String>,
    /// Reasoning-effort level paired with [`Self::default_model`] (`high`).
    pub default_effort: Option<String>,
    /// Key of the Linear team (`SP`) T3's Issues page lists for this
    /// Workspace. `None` gives the Workspace no issue tracker.
    pub linear_team: Option<String>,
    // --- Provenance ---
    pub source: WorkspaceSource,
    /// The repository whose git tree declared this Workspace.
    pub source_repository_id: Option<String>,
    /// Stable natural key within the source repository.
    pub source_key: Option<String>,
    /// Path to the config file within the source repository that declared this
    /// Workspace. Retained after an override so the origin stays visible.
    pub source_config_path: Option<String>,
    // --- Access control ---
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    /// The identity every Task on this Workspace acts as on a git host. When
    /// set it outranks the Task's own owner, so a Workspace can act as a
    /// dedicated machine account instead of as whoever started the Task.
    pub run_as_user_id: Option<String>,
    pub created_by: Option<String>,
    /// Soft-deleted Workspaces are normally hidden. A git-origin tombstone
    /// stays visible in admin so an operator can restore it from git without
    /// the automatic sync recreating it behind their back.
    pub deleted: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A single repository's placement inside a Workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceRepo {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    /// Overrides `Repository::default_branch` for this placement.
    pub branch: Option<String>,
    /// Subdirectory under the workspace directory. `None` defaults to the
    /// repository name at runtime.
    pub mount_name: Option<String>,
    /// The repository used for diffs, PRs, and AGENTS.md. Exactly one placement
    /// is primary once the Workspace holds any.
    pub is_primary: bool,
    /// Ordering within the Workspace.
    pub position: i32,
    pub created_at: Option<DateTime<Utc>>,
}

/// Env var carrying the resolved [`RepoCheckout`] list supplied by the backend.
pub const WORKSPACE_REPOS_ENV: &str = "WORKSPACE_REPOS";

/// Env var carrying the workspace volume's size limit, as a K8s quantity.
///
/// One number sizes both backends, so the writer exports it unconditionally:
/// an `emptyDir` is metered by the kubelet and evicts the pod that overruns
/// it, a PVC fails the write with `ENOSPC`, and `df` answers for neither.
pub const WORKSPACE_LIMIT_ENV: &str = "SANDBOX_WORKSPACE_LIMIT";

/// A [`WorkspaceRepo`] placement resolved for sandbox provisioning.
///
/// `WORKSPACE_REPOS` carries these primary first. Each is cloned into
/// `{WORKSPACE_DIR}/{mount_name}`; the primary supplies the sandbox's `repo_dir`
/// for git, files, and setup commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", into = "RepoCheckoutWire")]
pub struct RepoCheckout {
    /// Remote URL to clone. Credential-free on K8s — the sandbox authenticates
    /// via the credential channel using [`Self::provider`].
    pub clone_url: String,
    /// Branch to check out. `None`/empty clones the remote's default.
    #[serde(default)]
    pub branch: Option<String>,
    /// Repository ID for credential resolution; empty for a template checkout.
    #[serde(default)]
    pub repository_id: String,
    /// Declared git provider for token selection; empty when no known provider is set.
    pub provider: String,
    /// Subdirectory under `WORKSPACE_DIR` this repo is cloned into. Always a
    /// single safe path segment (see [`sanitize_mount_name`]).
    pub mount_name: String,
    /// The "main" repo: diff, PR, `AGENTS.md`, setup commands.
    #[serde(default)]
    pub is_primary: bool,
}

/// [`RepoCheckout`] as `WORKSPACE_REPOS` serializes it, plus a retired
/// `usesSsh: false`.
///
/// A Workspace pinning `docker_image` can run a sandbox older than migration
/// 0216, whose `RepoCheckout` requires `usesSsh`; it would refuse the whole
/// list rather than ignore the missing field.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepoCheckoutWire {
    uses_ssh: bool,
    clone_url: String,
    branch: Option<String>,
    repository_id: String,
    provider: String,
    mount_name: String,
    is_primary: bool,
}

impl From<RepoCheckout> for RepoCheckoutWire {
    fn from(checkout: RepoCheckout) -> Self {
        Self {
            uses_ssh: false,
            clone_url: checkout.clone_url,
            branch: checkout.branch,
            repository_id: checkout.repository_id,
            provider: checkout.provider,
            mount_name: checkout.mount_name,
            is_primary: checkout.is_primary,
        }
    }
}

impl RepoCheckout {
    /// Absolute checkout directory: `{workspace_dir}/{mount_name}`.
    pub fn dir(&self, workspace_dir: &str) -> String {
        format!(
            "{}/{}",
            workspace_dir.trim_end_matches('/'),
            self.mount_name
        )
    }
}

/// The provider whose checkouts always clone through the backend's git proxy
/// instead of from their own host.
///
/// A Forgejo token is never delivered into a pod: the pod addresses the
/// backend, which attaches the credential. The sandbox proxies a GitHub
/// checkout too; a GitLab checkout reaches its host directly.
pub const PROXIED_PROVIDER: &str = moatless_types::git_host::RepositoryProvider::Forgejo.as_str();

/// The proxied remote for `repository_id`:
/// `{backend_api_url}/git/{repository_id}`.
///
/// The path is the one the backend's `git_proxy` routes serve; the pod's task
/// token is the HTTP Basic password on it.
pub fn proxy_clone_url(backend_api_url: &str, repository_id: &str) -> String {
    format!(
        "{}/git/{repository_id}",
        backend_api_url.trim_end_matches('/')
    )
}

/// The remote naming the GitHub repository a proxied checkout's `origin`
/// reaches.
///
/// Plain `gh` resolves its repository from a remote on a GitHub host and finds
/// none in `origin`.
pub const GITHUB_REMOTE: &str = "github";

/// `git clone --config` values adding [`GITHUB_REMOTE`] at `upstream`.
/// `gh` refuses a branch without the tracking refspec; `skipFetchAll` keeps
/// `git fetch --all` from fetching the same repository twice.
pub fn github_remote_config(upstream: &str) -> [String; 3] {
    [
        format!("remote.{GITHUB_REMOTE}.url={upstream}"),
        format!("remote.{GITHUB_REMOTE}.fetch=+refs/heads/*:refs/remotes/{GITHUB_REMOTE}/*"),
        format!("remote.{GITHUB_REMOTE}.skipFetchAll=true"),
    ]
}

/// The repository id in a [`proxy_clone_url`] against `backend_api_url`.
///
/// Returns `None` when `url` names another origin — every checkout that clones
/// from its own git host.
pub fn proxy_clone_repository_id<'a>(backend_api_url: &str, url: &'a str) -> Option<&'a str> {
    let prefix = format!("{}/git/", backend_api_url.trim_end_matches('/'));
    let id = url.strip_prefix(&prefix)?.trim_end_matches('/');
    let id = id.strip_suffix(".git").unwrap_or(id);
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// Derive a mount name from a clone URL, falling back to `"repo"`.
///
/// This default must stay stable: an existing sandbox's primary checkout lives
/// at this path on its persistent volume.
pub fn derive_mount_name(clone_url: &str) -> String {
    let name = clone_url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".git");
    sanitize_mount_name(name).unwrap_or_else(|| "repo".to_string())
}

/// Reduce a user-authored mount name to a single safe path segment, or `None`
/// when nothing usable remains.
///
/// Mount names are joined onto `WORKSPACE_DIR`, so separators and `..` must
/// never survive: characters outside `[A-Za-z0-9._-]` become `-`, and an
/// all-dots name is rejected.
pub fn sanitize_mount_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return None;
    }
    Some(cleaned)
}

/// Named command run inside a Sandbox, on demand or at Sandbox start.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ScriptConfig {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub command: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub env_vars: Option<HashMap<String, String>>,
    /// Start this Script automatically when the Task's Sandbox reports Ready,
    /// rather than only when someone runs it. Reported to the T3 client as
    /// `runOnWorktreeCreate`.
    #[serde(default)]
    pub run_on_start: bool,
}

crate::string_enum!(
    /// A language server the sandbox can start for the agent's `LSP` tool.
    ///
    /// Closed because each value maps to a binary the sandbox image installs;
    /// the sandbox owns that mapping.
    LanguageServer [Hash] {
        /// TypeScript and JavaScript, through TypeScript 7's built-in server.
        Typescript => "typescript",
        /// Rust, through rust-analyzer.
        Rust => "rust",
        /// Java, through Eclipse JDT.LS; only the `soaplabs` recipe image has it.
        Java => "java",
    }
);

crate::string_enum!(
    /// A provider whose credential can be pinned to a Workspace.
    ///
    /// Only single static env-var credentials: Codex and Claude OAuth tokens
    /// refresh per user, so a pinned copy goes stale. GitHub is deliberately
    /// absent: a pin would outrank the App connection that authors Task posts.
    WorkspaceCredentialProvider [Hash] {
        Gitlab => "gitlab",
        Forgejo => "forgejo",
        Openai => "openai",
        Anthropic => "anthropic",
    }
);

impl WorkspaceCredentialProvider {
    /// The git hosts among these.
    ///
    /// A pin for one is the token every call against that host presents, so
    /// it's resolved by `moatless_identity::git_host_tokens::GitHostTokens`
    /// rather than only delivered as an environment variable.
    pub const GIT_HOSTS: &'static [Self] = &[Self::Gitlab, Self::Forgejo];

    /// A Forgejo pin is single-valued though a deployment may hold repositories
    /// on several Forgejo hosts, so it authenticates against all of them.
    ///
    /// Per-host Forgejo credentials are `provider_tokens` rows keyed
    /// `forgejo:{host}`, which a pin deliberately outranks.
    pub fn env_key(self) -> &'static str {
        match self {
            Self::Gitlab => "GITLAB_TOKEN",
            Self::Forgejo => "FORGEJO_TOKEN",
            Self::Openai => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
        }
    }

    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|provider| provider.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl WorkspaceCredentialProvider {
    /// The pin a Workspace may hold for `host`; `None` for GitHub, which a
    /// Workspace cannot pin.
    pub fn git_host(host: crate::repository::RepositoryProvider) -> Option<Self> {
        use crate::repository::RepositoryProvider as Host;
        match host {
            Host::Github => None,
            Host::Gitlab => Some(Self::Gitlab),
            Host::Forgejo => Some(Self::Forgejo),
        }
    }
}

/// A quantity that failed the Kubernetes quantity grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} quantity '{value}': {reason}")]
pub struct QuantityParseError {
    kind: &'static str,
    value: String,
    reason: &'static str,
}

// =============================================================================
// Cpu
// =============================================================================

/// A Kubernetes CPU quantity — `"250m"`, `"2"`, `"500000u"`, `"1000000000n"`.
///
/// Keeps the original spelling for pass-through to the Kubernetes API, and
/// exposes the parsed value in millicores for the Docker path and metrics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cpu {
    raw: String,
    millicores: u64,
}

impl Cpu {
    /// The quantity exactly as it was written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The quantity in millicores.
    pub fn millicores(&self) -> u64 {
        self.millicores
    }
}

impl FromStr for Cpu {
    type Err = QuantityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &'static str| QuantityParseError {
            kind: "CPU",
            value: value.to_string(),
            reason,
        };
        // Each suffix names how many of its units make one millicore.
        let (digits, scale) = split_suffix(value, &[("n", 1_000_000), ("u", 1_000), ("m", 1)]);
        // A suffixed form is an integer count of that unit. Without a suffix the
        // value is whole (possibly fractional) cores.
        let millicores = if let Some(per_millicore) = scale {
            let units: u64 = digits.parse().map_err(|_| invalid("not an integer"))?;
            units / per_millicore
        } else {
            let cores: f64 = digits.parse().map_err(|_| invalid("not a number"))?;
            if !cores.is_finite() || cores < 0.0 {
                return Err(invalid("must be a finite, non-negative number of cores"));
            }
            let millis = cores * 1000.0;
            if millis > u64::MAX as f64 {
                return Err(invalid("overflows u64 millicores"));
            }
            millis as u64
        };
        Ok(Self {
            raw: value.to_string(),
            millicores,
        })
    }
}

impl fmt::Display for Cpu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Serialize for Cpu {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Cpu {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

// =============================================================================
// Memory
// =============================================================================

/// A Kubernetes memory quantity — `"4Gi"`, `"512Mi"`, `"2k"`, `"4096"`.
///
/// Keeps the original spelling for pass-through to the Kubernetes API, and
/// exposes the parsed value in bytes for the Docker path and metrics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Memory {
    raw: String,
    bytes: u64,
}

impl Memory {
    /// The quantity exactly as it was written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The quantity in bytes.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl FromStr for Memory {
    type Err = QuantityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &'static str| QuantityParseError {
            kind: "memory",
            value: value.to_string(),
            reason,
        };
        let (digits, multiplier) = split_suffix(
            value,
            &[
                ("Ki", 1024),
                ("Mi", 1024 * 1024),
                ("Gi", 1024 * 1024 * 1024),
                ("Ti", 1024_u64.pow(4)),
                ("k", 1_000),
                ("M", 1_000_000),
                ("G", 1_000_000_000),
                ("T", 1_000_000_000_000),
            ],
        );
        let units: u64 = digits.parse().map_err(|_| invalid("not an integer"))?;
        let bytes = units
            .checked_mul(multiplier.unwrap_or(1))
            .ok_or_else(|| invalid("overflows u64 bytes"))?;
        Ok(Self {
            raw: value.to_string(),
            bytes,
        })
    }
}

impl fmt::Display for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Serialize for Memory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Memory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

/// Split the longest matching suffix off a quantity, returning the leading
/// digits and the suffix's scale. Suffixes are matched longest-first so `"Mi"`
/// wins over `"M"`.
fn split_suffix<'a>(value: &'a str, suffixes: &[(&str, u64)]) -> (&'a str, Option<u64>) {
    let mut best: Option<(&str, u64)> = None;
    for (suffix, scale) in suffixes {
        if value.ends_with(suffix) && best.is_none_or(|(seen, _)| suffix.len() > seen.len()) {
            best = Some((suffix, *scale));
        }
    }
    match best {
        Some((suffix, scale)) => (&value[..value.len() - suffix.len()], Some(scale)),
        None => (value, None),
    }
}

// =============================================================================
// StorageMedium
// =============================================================================

crate::string_enum!(
    /// Where a node-local sandbox volume is backed.
    StorageMedium {
        /// A Kubernetes `emptyDir`, capped by the paired size quantity.
        EmptyDir => "emptyDir",
        /// A node-local `hostPath` under `K8S_SANDBOX_CACHE_HOST_ROOT`. Kubernetes
        /// enforces no size cap; capacity is the node filesystem.
        Node => "node",
    }
);

// =============================================================================
// ResourceConfig
// =============================================================================

/// A container's CPU and memory for one direction (requests or limits).
///
/// Each field overrides the deployment default independently: a config naming
/// only `memory` keeps the configured CPU default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceQuantities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Cpu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
}

impl ResourceQuantities {
    pub fn is_empty(&self) -> bool {
        self.cpu.is_none() && self.memory.is_none()
    }
}

/// Resource and storage configuration for a sandbox, consumed by every runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceConfig {
    #[serde(default, skip_serializing_if = "ResourceQuantities::is_empty")]
    pub requests: ResourceQuantities,
    #[serde(default, skip_serializing_if = "ResourceQuantities::is_empty")]
    pub limits: ResourceQuantities,
    /// Workspace storage size. Sizes the per-task PVC when `pvc_enabled`, and
    /// the node-local emptyDir otherwise. Unset falls back to
    /// `K8S_SANDBOX_STORAGE_SIZE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<Memory>,
    /// Whether this sandbox gets a per-task PVC mounted at `/opt/moatless`.
    /// Unset falls back to `K8S_SANDBOX_DEFAULT_PVC`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pvc_enabled: Option<bool>,
    /// Whether this sandbox snapshots its workspace to S3. Unset falls back to
    /// `K8S_SANDBOX_DEFAULT_S3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_enabled: Option<bool>,
    /// Size cap for the dind sidecar's `/var/lib/docker` emptyDir. Only
    /// meaningful with `docker_storage_mode: EmptyDir`. Unset falls back to
    /// `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage: Option<Memory>,
    /// Where the dind sidecar's `/var/lib/docker` is backed. Unset means
    /// [`StorageMedium::Node`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage_mode: Option<StorageMedium>,
}

#[cfg(test)]
mod run_config_tests {
    use super::*;

    fn round_trip<T>(json: &str) -> T
    where
        T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let parsed: T = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            serde_json::to_string(&parsed).expect("serialize"),
            json,
            "serialization must reproduce the input spelling"
        );
        parsed
    }

    #[test]
    fn cpu_quantities_parse_to_millicores_and_keep_their_spelling() {
        for (json, millicores) in [
            (r#""250m""#, 250),
            (r#""1""#, 1000),
            (r#""2""#, 2000),
            (r#""0.5""#, 500),
            (r#""500000u""#, 500),
            (r#""1000000000n""#, 1000),
        ] {
            let cpu: Cpu = round_trip(json);
            assert_eq!(cpu.millicores(), millicores, "for {json}");
        }
    }

    #[test]
    fn a_malformed_cpu_quantity_fails_the_decode() {
        for json in [r#""garbage""#, r#""12x""#, r#""1.5m""#, r#""-1""#, r#""""#] {
            assert!(
                serde_json::from_str::<Cpu>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    #[test]
    fn memory_quantities_parse_to_bytes_and_keep_their_spelling() {
        for (json, bytes) in [
            (r#""128Mi""#, 128 * 1024 * 1024),
            (r#""1Gi""#, 1024 * 1024 * 1024),
            (r#""256000Ki""#, 256_000 * 1024),
            (r#""2k""#, 2000),
            (r#""5M""#, 5_000_000),
            (r#""3G""#, 3_000_000_000),
            (r#""4096""#, 4096),
        ] {
            let memory: Memory = round_trip(json);
            assert_eq!(memory.bytes(), bytes, "for {json}");
        }
    }

    #[test]
    fn a_malformed_memory_quantity_fails_the_decode() {
        for json in [r#""garbage""#, r#""1.5Gi""#, r#""10Gb""#, r#""""#] {
            assert!(
                serde_json::from_str::<Memory>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    #[test]
    fn storage_medium_maps_both_ways_and_rejects_anything_else() {
        for medium in StorageMedium::ALL {
            let json = serde_json::to_string(medium).expect("serialize");
            assert_eq!(json, format!("\"{}\"", medium.as_str()));
            assert_eq!(
                serde_json::from_str::<StorageMedium>(&json).expect("deserialize"),
                *medium
            );
            assert_eq!(medium.to_string(), medium.as_str());
        }
        for json in [r#""s3""#, r#""EMPTYDIR""#, r#""  node ""#, r#""emptydir""#] {
            assert!(
                serde_json::from_str::<StorageMedium>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    #[test]
    fn resource_config_round_trips_without_changing_its_json() {
        let json = r#"{"requests":{"cpu":"500m","memory":"1Gi"},"limits":{"cpu":"4","memory":"8Gi"},"storage":"20Gi","pvcEnabled":true,"s3Enabled":false,"dockerStorage":"30Gi","dockerStorageMode":"node"}"#;
        let config: ResourceConfig = round_trip(json);
        assert_eq!(config.requests.cpu.as_ref().map(Cpu::millicores), Some(500));
        assert_eq!(
            config.limits.memory.as_ref().map(Memory::bytes),
            Some(8 * 1024 * 1024 * 1024)
        );
        assert_eq!(config.docker_storage_mode, Some(StorageMedium::Node));
    }

    #[test]
    fn an_empty_resource_config_emits_nothing() {
        assert_eq!(
            serde_json::to_string(&ResourceConfig::default()).expect("serialize"),
            "{}"
        );
    }

    #[test]
    fn a_stale_key_is_ignored_rather_than_failing_the_decode() {
        // Rows written before a field was removed keep their keys in JSONB, and
        // committed `.moatless/` config files keep theirs until someone edits
        // them. Neither may break a start.
        let config: ResourceConfig = serde_json::from_str(
            r#"{"storage":"20Gi","buildCacheMode":"node","buildCacheScope":"task"}"#,
        )
        .expect("deserialize");
        assert_eq!(config.storage.as_ref().map(Memory::as_str), Some("20Gi"));
    }

    #[test]
    fn one_bad_quantity_fails_the_whole_resource_config() {
        assert!(serde_json::from_str::<ResourceConfig>(r#"{"requests":{"cpu":"lots"}}"#).is_err());
        assert!(
            serde_json::from_str::<ResourceConfig>(r#"{"dockerStorageMode":"tmpfs"}"#).is_err()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_provider_maps_persisted_and_environment_values() {
        let mut environment_keys = Vec::new();
        for provider in WorkspaceCredentialProvider::ALL {
            assert_eq!(provider.as_str().parse(), Ok(*provider));
            assert!(provider.env_key().ends_with("TOKEN") || provider.env_key().ends_with("KEY"));
            environment_keys.push(provider.env_key());
        }
        environment_keys.sort_unstable();
        environment_keys.dedup();
        assert_eq!(
            environment_keys.len(),
            WorkspaceCredentialProvider::ALL.len()
        );
    }

    /// The client reads this shape, and nothing on its side is vendored here.
    #[test]
    fn a_workspace_icon_serializes_as_the_clients_project_icon_override() {
        assert_eq!(
            serde_json::to_value(WorkspaceIcon::Lucide {
                name: "rocket".to_string(),
                color: "blue".to_string(),
            })
            .unwrap(),
            serde_json::json!({ "kind": "lucide", "name": "rocket", "color": "blue" })
        );
        assert_eq!(
            serde_json::to_value(WorkspaceIcon::Emoji {
                emoji: "🚀".to_string(),
            })
            .unwrap(),
            serde_json::json!({ "kind": "emoji", "emoji": "🚀" })
        );
    }

    #[test]
    fn proxy_clone_repository_id_reads_back_what_proxy_clone_url_wrote() {
        let base = "https://backend.example.com";
        let id = "6d0b4d9a-6d8e-4c0a-9b1a-0f2f0a1b2c3d";
        let url = proxy_clone_url(base, id);
        assert_eq!(proxy_clone_repository_id(base, &url), Some(id));
        assert_eq!(
            proxy_clone_repository_id("https://backend.example.com/", &url),
            Some(id)
        );
        assert_eq!(
            proxy_clone_repository_id(base, &format!("{url}.git")),
            Some(id)
        );
    }

    #[test]
    fn proxy_clone_repository_id_rejects_another_origin() {
        let base = "https://backend.example.com";
        assert_eq!(
            proxy_clone_repository_id(base, "https://github.com/owner/repo.git"),
            None
        );
        assert_eq!(
            proxy_clone_repository_id(base, "https://other.example.com/git/abc"),
            None
        );
        assert_eq!(
            proxy_clone_repository_id(base, &format!("{base}/git/")),
            None
        );
        assert_eq!(
            proxy_clone_repository_id(base, &format!("{base}/git/abc/info/refs")),
            None
        );
    }

    #[test]
    fn derive_mount_name_handles_common_url_shapes() {
        assert_eq!(
            derive_mount_name("https://github.com/owner/repo.git"),
            "repo"
        );
        assert_eq!(derive_mount_name("https://github.com/owner/repo"), "repo");
        assert_eq!(derive_mount_name("git@github.com:owner/repo.git"), "repo");
        assert_eq!(derive_mount_name("https://host/owner/repo/"), "repo");
        assert_eq!(
            derive_mount_name("https://gitlab.com/group/sub/my-service.git"),
            "my-service"
        );
    }

    #[test]
    fn derive_mount_name_falls_back_for_unusable_urls() {
        assert_eq!(derive_mount_name(""), "repo");
        assert_eq!(derive_mount_name("/"), "repo");
        assert_eq!(derive_mount_name("https://host/../"), "repo");
    }

    /// A mount name is joined onto `WORKSPACE_DIR`, so traversal and separators
    /// must not survive sanitization.
    #[test]
    fn sanitize_mount_name_strips_path_traversal() {
        assert_eq!(sanitize_mount_name("../../etc"), Some("..-..-etc".into()));
        assert_eq!(sanitize_mount_name("a/b"), Some("a-b".into()));
        assert_eq!(sanitize_mount_name(".."), None);
        assert_eq!(sanitize_mount_name("."), None);
        assert_eq!(sanitize_mount_name("   "), None);
        assert_eq!(sanitize_mount_name("/"), None);
    }

    #[test]
    fn sanitize_mount_name_keeps_ordinary_names() {
        assert_eq!(sanitize_mount_name("my-service"), Some("my-service".into()));
        assert_eq!(sanitize_mount_name("Api_2.0"), Some("Api_2.0".into()));
    }

    #[test]
    fn a_checkout_still_tells_an_older_sandbox_it_uses_no_ssh() {
        let checkout = RepoCheckout {
            clone_url: "https://github.com/owner/service.git".into(),
            branch: None,
            repository_id: "repo_1".into(),
            provider: "github".into(),
            mount_name: "service".into(),
            is_primary: true,
        };
        let json = serde_json::to_value(&checkout).unwrap();
        assert_eq!(json["usesSsh"], false);
        assert_eq!(json["cloneUrl"], "https://github.com/owner/service.git");
        assert_eq!(
            serde_json::from_value::<RepoCheckout>(json).unwrap(),
            checkout
        );
    }

    #[test]
    fn checkout_dir_joins_workspace_dir() {
        let checkout = RepoCheckout {
            clone_url: "https://github.com/owner/service.git".into(),
            branch: None,
            repository_id: "repo_1".into(),
            provider: "github".into(),
            mount_name: "service".into(),
            is_primary: true,
        };
        assert_eq!(
            checkout.dir("/opt/moatless/workspace"),
            "/opt/moatless/workspace/service"
        );
        assert_eq!(
            checkout.dir("/opt/moatless/workspace/"),
            "/opt/moatless/workspace/service"
        );
    }
}
