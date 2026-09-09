//! Storage backend selection for sandbox workspaces.
//!
//! A sandbox workspace (`/opt/moatless`) is persisted by two **independent**
//! switches, each resolved from `resources.pvc_enabled` / `resources.s3_enabled`
//! with a per-deployment default when the repo leaves one unset:
//!
//! - **PVC** ([`StorageBackend::pvc`]) — the live copy: a per-task
//!   `ReadWriteOnce` PVC that survives pod restarts, forces `strategy: Recreate`
//!   plus a drain wait, and is deleted with the sandbox. Without it the
//!   workspace is a node-local `emptyDir` that dies with the pod.
//! - **S3** ([`StorageBackend::s3`]) — the copy that outlives the Deployment;
//!   `crates/sandbox/src/workspace_sync/` owns the file set.
//!
//! The pair decides how a start treats the volume it finds:
//!
//! | pvc | s3 | Volume | On start |
//! |---|---|---|---|
//! | on | on | PVC | trust the volume; pull from S3 only when it holds nothing usable |
//! | on | off | PVC | trust the volume; clone when empty |
//! | off | on | emptyDir | always pull from S3, then clone what the snapshot lacked |
//! | off | off | emptyDir | always clone |
//!
//! [`StorageBackend::restore_if_empty`] is that distinction: with a durable
//! volume the snapshot is a fallback; without one it is the only persistence
//! there is.

use crate::provisioning::ProvisionPlan;

/// How a sandbox's workspace is persisted — two independent switches, not a
/// choice between tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageBackend {
    /// A per-task PVC backs `/opt/moatless`. When `false`, a node-local
    /// `emptyDir` does.
    pub pvc: bool,
    /// The sandbox snapshots `WORKSPACE_DIR` to S3.
    pub s3: bool,
}

impl StorageBackend {
    /// Resolve both switches for a provisioning plan.
    ///
    /// Each switch is resolved **independently**: a repo that sets only one gets
    /// the deployment default for the other, so a storage-only override cannot
    /// silently turn the other switch off.
    pub fn from_plan(plan: &ProvisionPlan, default_pvc: bool, default_s3: bool) -> Self {
        let resources = plan.repo_config.resources.as_ref();
        Self {
            pvc: resources.and_then(|r| r.pvc_enabled).unwrap_or(default_pvc),
            s3: resources.and_then(|r| r.s3_enabled).unwrap_or(default_s3),
        }
    }

    /// Whether a PVC is provisioned and mounted for this sandbox.
    pub fn uses_pvc(self) -> bool {
        self.pvc
    }

    /// Whether the sandbox is handed `WORKSPACE_SYNC_ENABLED=true` and takes part
    /// in the pre-stop workspace push.
    pub fn syncs_sandbox(self) -> bool {
        self.s3
    }

    /// Whether the sandbox restores from S3 **only** into a volume holding
    /// nothing usable, rather than on every start.
    ///
    /// True exactly when both switches are on: the PVC is then the live copy and
    /// overwriting it with an older snapshot would lose work. With no PVC the
    /// `emptyDir` starts empty every time, so the restore is unconditional and
    /// this stays false.
    pub fn restore_if_empty(self) -> bool {
        self.pvc && self.s3
    }

    /// Stable label value for metrics (`storage_backend` dimension). Four values,
    /// one per combination, so cardinality is unchanged.
    pub fn as_metric_label(self) -> &'static str {
        match (self.pvc, self.s3) {
            (true, true) => "pvc+s3",
            (true, false) => "pvc",
            (false, true) => "s3",
            (false, false) => "none",
        }
    }
}

/// Build the S3 key for a task's workspace archive:
/// `<prefix>/<task_id>/workspace.tar`. The `<prefix>/<task_id>/` portion is the
/// same one the Kubernetes runtime removes, so uploads and cleanup stay aligned.
pub fn sandbox_key(prefix: &str, task_id: &str) -> String {
    format!("{}/{}/workspace.tar", prefix.trim_end_matches('/'), task_id)
}

/// Build the S3 key for the file mirror's manifest:
/// `<prefix>/<task_id>/manifest.json.gz`.
///
/// Presence of this object is what tells a restore the task has a mirror; its
/// absence sends the restore to [`sandbox_key`]'s tar.
pub fn manifest_key(prefix: &str, task_id: &str) -> String {
    format!(
        "{}/{}/manifest.json.gz",
        prefix.trim_end_matches('/'),
        task_id
    )
}

/// Build the prefix holding the task's `restic` repository:
/// `<prefix>/<task_id>/repo`.
///
/// Its own segment, so restic's `config`, `keys/`, `snapshots/`, `index/` and
/// `data/` cannot collide with the tar and the mirror that sit beside it under
/// the task prefix.
pub fn workspace_repository_prefix(prefix: &str, task_id: &str) -> String {
    format!("{}/{}/repo", prefix.trim_end_matches('/'), task_id)
}

/// Build the S3 prefix a `restic` repository's snapshots sit under:
/// `<prefix>/<task_id>/repo/snapshots/`.
///
/// Any object here is what tells a restore the task has something to restore,
/// the way [`manifest_key`] answers it for the mirror. It takes a listing rather
/// than a `head_object` because a snapshot's id is its content hash, and the
/// sibling `repo/config` answers the wrong question: `restic::prune` opens the
/// repository, which creates it, so a task stopped before its first successful
/// snapshot holds a config and nothing behind it.
pub fn workspace_repository_snapshots_prefix(prefix: &str, task_id: &str) -> String {
    format!(
        "{}/snapshots/",
        workspace_repository_prefix(prefix, task_id)
    )
}

/// The path-safety rule for a mirrored file and the root every snapshot records
/// under, both owned by the wire contract the sandbox and the backend speak —
/// see [`sandbox_proto::is_safe_workspace_path`] and
/// [`sandbox_proto::SNAPSHOT_ROOT`].
pub use sandbox_proto::{SNAPSHOT_ROOT, is_safe_workspace_path};

/// Build the S3 key for one mirrored workspace file:
/// `<prefix>/<task_id>/files/<workspace-relative-path>`.
///
/// The `files/` segment keeps the per-file objects from colliding with the
/// archive and the manifest, which sit beside it under the task prefix.
pub fn workspace_object_key(prefix: &str, task_id: &str, path: &str) -> String {
    format!(
        "{}/{}/files/{}",
        prefix.trim_end_matches('/'),
        task_id,
        path.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provisioning::RepoConfig;
    use moatless_types::workspace::ResourceConfig;

    /// Build a minimal plan whose repo sets each switch independently, so
    /// per-field default resolution can be exercised in isolation.
    fn plan_with(pvc: Option<bool>, s3: Option<bool>) -> ProvisionPlan {
        ProvisionPlan {
            repo_config: RepoConfig {
                name: "test-repo".to_string(),
                docker_image: None,
                servers: None,
                system_prompt: None,
                code_execution: None,
                preview_config: None,
                resources: (pvc.is_some() || s3.is_some()).then(|| ResourceConfig {
                    pvc_enabled: pvc,
                    s3_enabled: s3,
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
    fn repo_switches_override_defaults() {
        let backend = StorageBackend::from_plan(&plan_with(Some(false), Some(true)), true, false);
        assert!(!backend.pvc);
        assert!(backend.s3);
    }

    #[test]
    fn defaults_apply_when_repo_unset() {
        assert_eq!(
            StorageBackend::from_plan(&plan_with(None, None), true, true),
            StorageBackend {
                pvc: true,
                s3: true
            }
        );
        assert_eq!(
            StorageBackend::from_plan(&plan_with(None, None), false, false),
            StorageBackend {
                pvc: false,
                s3: false
            }
        );
    }

    /// The switches resolve per field: setting one must not drag the other away
    /// from the deployment default.
    #[test]
    fn one_switch_set_leaves_the_other_on_the_default() {
        let backend = StorageBackend::from_plan(&plan_with(Some(true), None), false, true);
        assert!(backend.pvc, "repo-set switch wins");
        assert!(backend.s3, "unset switch keeps the deployment default");

        let backend = StorageBackend::from_plan(&plan_with(None, Some(false)), true, true);
        assert!(backend.pvc, "unset switch keeps the deployment default");
        assert!(!backend.s3, "repo-set switch wins");
    }

    #[test]
    fn capabilities_follow_the_two_switches() {
        let both = StorageBackend {
            pvc: true,
            s3: true,
        };
        assert!(both.uses_pvc() && both.syncs_sandbox() && both.restore_if_empty());

        let pvc_only = StorageBackend {
            pvc: true,
            s3: false,
        };
        assert!(pvc_only.uses_pvc());
        assert!(!pvc_only.syncs_sandbox());
        assert!(!pvc_only.restore_if_empty());

        let s3_only = StorageBackend {
            pvc: false,
            s3: true,
        };
        assert!(!s3_only.uses_pvc());
        assert!(s3_only.syncs_sandbox());
        assert!(
            !s3_only.restore_if_empty(),
            "an emptyDir starts empty every time, so its restore is unconditional"
        );

        let neither = StorageBackend {
            pvc: false,
            s3: false,
        };
        assert!(!neither.uses_pvc() && !neither.syncs_sandbox() && !neither.restore_if_empty());
    }

    #[test]
    fn metric_labels_are_distinct_per_combination() {
        assert_eq!(
            StorageBackend {
                pvc: true,
                s3: true
            }
            .as_metric_label(),
            "pvc+s3"
        );
        assert_eq!(
            StorageBackend {
                pvc: true,
                s3: false
            }
            .as_metric_label(),
            "pvc"
        );
        assert_eq!(
            StorageBackend {
                pvc: false,
                s3: true
            }
            .as_metric_label(),
            "s3"
        );
        assert_eq!(
            StorageBackend {
                pvc: false,
                s3: false
            }
            .as_metric_label(),
            "none"
        );
    }
}
