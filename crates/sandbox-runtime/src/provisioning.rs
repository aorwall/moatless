//! The provision plan a runtime is handed: [`ProvisionRequest`]/[`ProvisionPlan`]
//! and the [`RepoConfig`] the backend sends.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use moatless_workspace_types::workspace::RepoCheckout;

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
    #[serde(default)]
    pub git_provider: String,
    /// Custom environment variables
    pub env_vars: Option<HashMap<String, String>>,
    /// Resolved sandbox checkouts, primary first; empty when no repository is configured.
    #[serde(default)]
    pub repos: Vec<RepoCheckout>,
}

/// Repository config for provisioning and sandbox environment
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoConfig {
    pub name: String,
    /// Per-repository sandbox image override
    pub docker_image: Option<String>,
    // --- Pass-through fields for REPO_CONFIG env var ---
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Code execution configuration (dockerImage, envVars, etc.)
    #[schema(value_type = Object)]
    pub code_execution: Option<serde_json::Value>,
    // --- K8s resource configuration (opt-in per-repo) ---
    /// Per-repo K8s resource requests/limits override for the sandbox.
    #[schema(value_type = moatless_workspace_types::schemas::workspace::ResourceConfig)]
    pub resources: Option<moatless_workspace_types::workspace::ResourceConfig>,
    // --- Plugins (resolved from the task owner's activation records) ---
    /// The plugins this task's owner has turned on, each with the skills they
    /// picked. The sandbox clones each one and removes the skills not named.
    pub plugins: Option<Vec<moatless_workspace_types::plugin::PluginDelivery>>,
    // --- Docker access (opt-in) ---
    /// Mount Docker socket inside sandbox container (default: false). Overrides
    /// global `DOCKER_ENABLE_SOCKET_MOUNT` when set.
    #[serde(default)]
    pub docker_access: Option<bool>,
    /// Additional Docker networks to connect sandbox to. Overrides global
    /// `DOCKER_EXTRA_NETWORKS` when set.
    #[serde(default)]
    pub docker_networks: Option<Vec<String>>,
    // --- Kubernetes ServiceAccount (opt-in per-repo) ---
    /// Per-repo Kubernetes `ServiceAccount` for the sandbox pod; overrides
    /// `K8S_SANDBOX_SERVICE_ACCOUNT` when set, ignored in Docker mode.
    ///
    /// Enables "shadow" dev sandboxes that run under an SA bound to the
    /// backend's RBAC.
    #[serde(default)]
    pub sandbox_service_account: Option<String>,
    // --- Network profile (opt-in per-repo) ---
    /// Per-repo network profile; sets label
    /// `moatless.io/network-profile=<value>` for a static `NetworkPolicy` to
    /// select. Ignored in Docker mode.
    ///
    /// Selects a policy only; grants nothing on its own.
    #[serde(default)]
    pub network_profile: Option<String>,
    // --- Node placement (opt-in per-workspace/per-loop) ---
    /// Extra node labels the sandbox pod must match, merged key by key over
    /// `K8S_SANDBOX_NODE_SELECTOR`: a global key named here is replaced, one
    /// left out survives. Selects platform-owned capacity; creates none.
    /// Ignored in Docker mode.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
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
}
