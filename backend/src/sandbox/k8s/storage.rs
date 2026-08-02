//! Storage backend selection for sandbox workspaces.
//!
//! A repository picks how its sandbox workspace (`/opt/moatless`) is persisted
//! via `resources.storage_mode`; when a repo leaves it unset, the
//! deployment-wide default (`K8S_SANDBOX_DEFAULT_STORAGE_MODE`) applies. Three
//! backends exist:
//!
//! - [`StorageBackend::Ebs`] (default) — a per-task EBS PVC, `ReadWriteOnce`,
//!   surviving pod restarts. Reliable but forces `strategy: Recreate` + a drain
//!   wait on every restart and carries a standing volume per task.
//! - [`StorageBackend::Ephemeral`] — a node-local `emptyDir`, no PVC. Fast, but
//!   nothing survives the pod (only transcripts persist, via their own S3 sync).
//! - [`StorageBackend::S3`] — a node-local `emptyDir` like `ephemeral`, plus the
//!   sandbox syncs **everything under `WORKSPACE_DIR`** to/from S3 (presigned
//!   direct transfer), giving persistence without an EBS attach/detach on the
//!   pod lifecycle. That is every repo checkout *with its `.git`*, plus loose
//!   files outside any repo; only gitignored paths inside a repo are skipped.
//!   See `sandbox/src/init/workspace_sync.rs` for the exact file set — this was
//!   a working-tree-only sync in an earlier design and is not one now.
//! - [`StorageBackend::Node`] — a best-effort node-local `hostPath`, no PVC. The
//!   workspace survives stop/start only when the replacement pod lands on the
//!   same node.
//!
//! `Ephemeral`, `S3`, and `Node` skip the PVC ([`StorageBackend::uses_pvc`] is
//! `false`). `S3` additionally enables after-turn workspace sync in the sandbox
//! ([`StorageBackend::syncs_sandbox`]).

use crate::sandbox::models::ProvisionPlan;

/// How a sandbox's workspace is persisted. Derived from `resources.storage_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    /// Per-task EBS PVC mounted at `/opt/moatless` (default).
    Ebs,
    /// Node-local `emptyDir`, no persistence.
    Ephemeral,
    /// Node-local `emptyDir` + git-working-tree sync to S3.
    S3,
    /// Node-local hostPath keyed by task id. Best-effort reuse on the same node.
    Node,
}

impl StorageBackend {
    /// Parse a single `storage_mode` string (case-insensitive, trimmed).
    /// Returns `None` for empty / unrecognized values.
    fn parse(mode: &str) -> Option<Self> {
        match mode.trim().to_ascii_lowercase().as_str() {
            "ebs" => Some(Self::Ebs),
            "ephemeral" => Some(Self::Ephemeral),
            "s3" => Some(Self::S3),
            "node" => Some(Self::Node),
            _ => None,
        }
    }

    /// Resolve the backend for a provisioning plan.
    ///
    /// A per-repo `resources.storage_mode` wins when set to a recognized value.
    /// Otherwise the deployment-wide `default_mode` applies (from
    /// `K8S_SANDBOX_DEFAULT_STORAGE_MODE`). If both are unset/unrecognized, falls
    /// back to [`StorageBackend::Ebs`] — the safe default that needs no S3.
    pub fn from_plan(plan: &ProvisionPlan, default_mode: &str) -> Self {
        plan.repo_config
            .resources
            .as_ref()
            .and_then(|r| r.storage_mode.as_deref())
            .and_then(Self::parse)
            .or_else(|| Self::parse(default_mode))
            .unwrap_or(Self::Ebs)
    }

    /// True only for [`StorageBackend::Ebs`] — the sole backend that provisions
    /// a PVC. Other backends mount node-local storage instead.
    pub fn uses_pvc(self) -> bool {
        matches!(self, Self::Ebs)
    }

    /// True only for [`StorageBackend::S3`] — the sandbox syncs workspaces after
    /// terminal agent turns and is handed `WORKSPACE_SYNC_ENABLED=true`.
    pub fn syncs_sandbox(self) -> bool {
        matches!(self, Self::S3)
    }

    /// Stable label value for metrics (`storage_backend` dimension).
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Self::Ebs => "ebs",
            Self::Ephemeral => "ephemeral",
            Self::S3 => "s3",
            Self::Node => "node",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::models::ResourceConfig;
    use crate::sandbox::models::RepoConfig;

    /// Build a minimal plan whose repo either sets `storage_mode` or leaves it
    /// unset (`None`), so default resolution can be exercised in isolation.
    fn plan_with_mode(mode: Option<&str>) -> ProvisionPlan {
        ProvisionPlan {
            repo_config: RepoConfig {
                name: "test-repo".to_string(),
                docker_image: None,
                servers: None,
                system_prompt: None,
                code_execution: None,
                preview_config: None,
                resources: mode.map(|m| ResourceConfig {
                    storage_mode: Some(m.to_string()),
                    docker_storage_mode: None,
                    ..Default::default()
                }),
                plugins: None,
                docker_access: None,
                docker_networks: None,
                sandbox_service_account: None,
                network_profile: None,
            },
            git_clone_url: "https://example.com/repo.git".to_string(),
            git_branch: None,
            git_provider: "github".to_string(),
            env_vars: None,
            repos: Vec::new(),
        }
    }

    #[test]
    fn repo_mode_overrides_default() {
        // A repo's explicit mode always wins, whatever the deployment default.
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(Some("ephemeral")), "s3"),
            StorageBackend::Ephemeral
        );
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(Some("ebs")), "s3"),
            StorageBackend::Ebs
        );
        // Case/space-insensitive.
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(Some("  S3 ")), "ebs"),
            StorageBackend::S3
        );
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(Some(" NODE ")), "s3"),
            StorageBackend::Node
        );
    }

    #[test]
    fn default_applies_when_repo_unset() {
        // No per-repo mode → the deployment default decides.
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(None), "s3"),
            StorageBackend::S3
        );
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(None), "ephemeral"),
            StorageBackend::Ephemeral
        );
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(None), "ebs"),
            StorageBackend::Ebs
        );
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(None), "node"),
            StorageBackend::Node
        );
    }

    #[test]
    fn unknown_values_fall_back_to_ebs() {
        // An unrecognized repo mode falls through to the default...
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(Some("bogus")), "s3"),
            StorageBackend::S3
        );
        // ...and an unrecognized default (or both unset) lands on safe EBS.
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(Some("bogus")), "also-bogus"),
            StorageBackend::Ebs
        );
        assert_eq!(
            StorageBackend::from_plan(&plan_with_mode(None), ""),
            StorageBackend::Ebs
        );
    }
}
