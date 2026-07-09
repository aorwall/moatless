//! Storage backend selection for sandbox workspaces.
//!
//! A repository picks how its sandbox workspace (`/opt/moatless`) is persisted
//! via `resources.storage_mode`. Three backends exist:
//!
//! - [`StorageBackend::Ebs`] (default) — a per-task EBS PVC, `ReadWriteOnce`,
//!   surviving pod restarts. Reliable but forces `strategy: Recreate` + a drain
//!   wait on every restart and carries a standing volume per task.
//! - [`StorageBackend::Ephemeral`] — a node-local `emptyDir`, no PVC. Fast, but
//!   nothing survives the pod (only transcripts persist, via their own S3 sync).
//! - [`StorageBackend::S3`] — a node-local `emptyDir` like `ephemeral`, plus the
//!   sandbox syncs the git working tree to/from S3 (presigned direct transfer),
//!   giving persistence without an EBS attach/detach on the pod lifecycle.
//!
//! Both `Ephemeral` and `S3` skip the PVC ([`StorageBackend::uses_pvc`] is
//! `false`); they differ only in whether the workspace-sync loop runs in the
//! sandbox ([`StorageBackend::syncs_workspace`]).

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
}

impl StorageBackend {
    /// Resolve the backend from `resources.storage_mode` (case-insensitive,
    /// trimmed). Unset / unknown values fall back to [`StorageBackend::Ebs`].
    pub fn from_plan(plan: &ProvisionPlan) -> Self {
        let mode = plan
            .repo_config
            .resources
            .as_ref()
            .and_then(|r| r.storage_mode.as_deref())
            .map(|m| m.trim().to_ascii_lowercase());
        match mode.as_deref() {
            Some("ephemeral") => Self::Ephemeral,
            Some("s3") => Self::S3,
            _ => Self::Ebs,
        }
    }

    /// True only for [`StorageBackend::Ebs`] — the sole backend that provisions
    /// a PVC. The other two mount a node-local `emptyDir` instead.
    pub fn uses_pvc(self) -> bool {
        matches!(self, Self::Ebs)
    }

    /// True only for [`StorageBackend::S3`] — the sandbox runs the workspace-sync
    /// loop and is handed `WORKSPACE_SYNC_ENABLED=true`.
    pub fn syncs_workspace(self) -> bool {
        matches!(self, Self::S3)
    }

    /// Stable label value for metrics (`storage_backend` dimension).
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Self::Ebs => "ebs",
            Self::Ephemeral => "ephemeral",
            Self::S3 => "s3",
        }
    }
}
