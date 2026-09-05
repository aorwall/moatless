//! The provision plan a runtime is handed, and the per-server config it derives.
//!
//! [`ProvisionRequest`]/[`ProvisionPlan`] and [`RepoConfig`] are the plan the
//! backend sends; [`allocated_port`] and [`server_capabilities_from_config`]
//! read one server's config out of it.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tracing::warn;
use utoipa::ToSchema;

use moatless_types::sandbox::ServerCapabilities;
use moatless_types::workspace::{Port, RepoCheckout, ServerConfig};

/// Provision plan sent from TS backend
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionRequest {
    pub plan: ProvisionPlan,
    pub user_id: String,
}

/// Provision plan with all configuration needed to create a workspace
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionPlan {
    pub repo_config: RepoConfig,
    /// The primary repository's clone URL. Mirrors `repos[0]`, which is always
    /// the primary checkout; kept as its own field for the `GIT_CLONE_URL` env.
    pub git_clone_url: String,
    /// The primary repository's branch. Mirrors `repos[0].branch`.
    pub git_branch: Option<String>,
    /// The primary repository's git provider (e.g. "github", "gitlab", "gitness")
    #[serde(default = "default_git_provider")]
    pub git_provider: String,
    /// Custom environment variables
    pub env_vars: Option<HashMap<String, String>>,
    /// Every repository the task's workspace places in the sandbox, primary
    /// first. Empty only when `git_clone_url` is the sole checkout.
    #[serde(default)]
    pub repos: Vec<RepoCheckout>,
}

fn default_git_provider() -> String {
    "github".to_string()
}

/// Repository config for provisioning and sandbox environment
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoConfig {
    pub name: String,
    /// Per-repository sandbox image override
    pub docker_image: Option<String>,
    /// Servers with allocated ports
    #[schema(value_type = Vec<moatless_api_schemas::workspace::ServerConfig>)]
    pub servers: Option<Vec<ServerConfig>>,
    // --- Pass-through fields for REPO_CONFIG env var ---
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Code execution configuration (dockerImage, envVars, etc.)
    #[schema(value_type = Object)]
    pub code_execution: Option<serde_json::Value>,
    /// Preview server configuration
    #[schema(value_type = Object)]
    pub preview_config: Option<serde_json::Value>,
    // --- K8s resource configuration (opt-in per-repo) ---
    /// Per-repo K8s resource requests/limits override for server containers.
    #[schema(value_type = moatless_api_schemas::workspace::ResourceConfig)]
    pub resources: Option<moatless_types::workspace::ResourceConfig>,
    // --- Plugins (resolved from the task owner's activation records) ---
    /// The plugins this task's owner has turned on, each with the skills they
    /// picked. The sandbox clones each one and removes the skills not named.
    pub plugins: Option<Vec<moatless_api_schemas::plugin::PluginDelivery>>,
    // --- Docker access (opt-in) ---
    /// Mount Docker socket inside sandbox container (default: false).
    /// Overrides global `DOCKER_ENABLE_SOCKET_MOUNT` when set.
    #[serde(default)]
    pub docker_access: Option<bool>,
    /// Additional Docker networks to connect sandbox to.
    /// Overrides global `DOCKER_EXTRA_NETWORKS` when set.
    #[serde(default)]
    pub docker_networks: Option<Vec<String>>,
    // --- Kubernetes ServiceAccount (opt-in per-repo) ---
    /// Per-repo Kubernetes `ServiceAccount` for the sandbox pod. Overrides the
    /// global default (`K8S_SANDBOX_SERVICE_ACCOUNT`) when set. Ignored in Docker
    /// mode. Enables "shadow" dev sandboxes that run under an SA bound to the
    /// backend's RBAC.
    #[serde(default)]
    pub sandbox_service_account: Option<String>,
    // --- Network profile (opt-in per-repo) ---
    /// Per-repo network profile. When set, the sandbox pod carries the label
    /// `moatless.io/network-profile=<value>`, which a static (GitOps-owned)
    /// `NetworkPolicy` can select to govern the pod's egress/ingress. Selects a
    /// policy only; grants nothing on its own. Ignored in Docker mode.
    #[serde(default)]
    pub network_profile: Option<String>,
}

/// A live override of one preview server's start command and env.
///
/// Whole-state, not a delta: callers always send the complete set to the
/// sandbox, which never merges.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerOverride {
    /// Replaces the server's configured start command. `None` = use the
    /// workspace's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,
    /// Env entries exported after `server.env`, so they win over it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Keys to remove — the only way to drop a variable the container spec
    /// injected, since sourcing an env file can add but never remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unset_env: Vec<String>,
}

impl ServerOverride {
    /// True when nothing is overridden, so the entry can be dropped from the
    /// persisted map rather than stored empty.
    pub fn is_empty(&self) -> bool {
        self.start_command.is_none() && self.env.is_empty() && self.unset_env.is_empty()
    }
}

/// Derive capabilities from a `ServerConfig`.
pub fn server_capabilities_from_config(config: &ServerConfig) -> ServerCapabilities {
    ServerCapabilities {
        address_bar: config.enable_inspector,
        inspector: config.enable_inspector,
        reload: true,
        open_external: true,
        viewport: true,
        console: true,
    }
}

/// The port a runtime can act on for this server, or `None` with a log.
///
/// Every server in a provision plan has been through `assign_missing_ports`, so
/// a server without a port here means the plan was assembled some other way. A
/// runtime can neither start nor observe such a server, so it is skipped rather
/// than given an invented port.
pub fn allocated_port(server: &ServerConfig, task_id: &str) -> Option<u16> {
    if server.port.is_none() {
        warn!(
            task_id,
            server = %server.name,
            "Skipping a server with no allocated port"
        );
    }
    server.port.map(Port::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provision_request_deserializes() {
        let json = r#"{
            "plan": {
                "repoConfig": {
                    "name": "test-repo"
                },
                "gitCloneUrl": "https://github.com/test/repo.git"
            },
            "userId": "user-1"
        }"#;
        let req: ProvisionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.user_id, "user-1");
        assert_eq!(req.plan.repo_config.name, "test-repo");
        assert_eq!(req.plan.git_clone_url, "https://github.com/test/repo.git");
        assert!(req.plan.git_branch.is_none());
    }

    #[test]
    fn capabilities_with_inspector_enabled() {
        let config = ServerConfig {
            name: "preview".to_string(),
            enable_inspector: true,
            ..Default::default()
        };
        let caps = server_capabilities_from_config(&config);
        assert!(caps.inspector);
        assert!(caps.address_bar);
        assert!(caps.reload);
        assert!(caps.viewport);
        assert!(caps.console);
    }

    #[test]
    fn capabilities_with_inspector_disabled() {
        let config = ServerConfig {
            name: "files".to_string(),
            enable_inspector: false,
            ..Default::default()
        };
        let caps = server_capabilities_from_config(&config);
        assert!(!caps.inspector);
        assert!(!caps.address_bar);
        assert!(caps.reload);
        assert!(caps.viewport);
        assert!(caps.console);
    }
}
