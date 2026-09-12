//! The security contexts the pod and its containers run under.

use super::*;

/// Container-level security context for the agent + server containers.
///
/// Both modes drop ALL capabilities and add back only what the workload needs.
///
/// - Non-root (`K8S_SANDBOX_RUN_AS_NON_ROOT=true`): the pod-level securityContext
///   (see [`pod_security_context`]) runs these containers as UID/GID 1000, and
///   the image chowns every runtime path to that user (plus `fsGroup: 1000` for
///   the mounted volumes). Ownership is therefore correct without `DAC_OVERRIDE`.
/// - Root (default, flag off): the container runs as UID 0 while `/home/moatless`
///   and friends are owned by UID 1000, so `DAC_OVERRIDE` is required for root to
///   write there (gh CLI config, Claude onboarding flag) during init.
///
/// `NET_BIND_SERVICE` is kept in standard mode so servers can bind privileged
/// ports if a repo configures one. Fargate accepts capability drops but not
/// capability additions, so its preset runs without added capabilities.
pub(super) fn app_security_context(config: &Config) -> SecurityContext {
    if config.sandbox.compute_mode.is_fargate() {
        return SecurityContext {
            allow_privilege_escalation: Some(false),
            capabilities: Some(Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                add: None,
            }),
            ..Default::default()
        };
    }
    let mut add = vec!["NET_BIND_SERVICE".to_string()];
    if !config.sandbox.run_as_non_root {
        add.push("DAC_OVERRIDE".to_string());
    }
    SecurityContext {
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".to_string()]),
            add: Some(add),
        }),
        ..Default::default()
    }
}

/// Pod-level securityContext, populated only when `K8S_SANDBOX_RUN_AS_NON_ROOT`
/// is enabled.
///
/// Runs the pod's containers as the non-root `moatless` user (UID/GID 1000) with
/// the `RuntimeDefault` seccomp profile, and sets `fsGroup: 1000` so mounted
/// volumes (workspace PVC/emptyDir, shared-data) are group-writable
/// by that user. The privileged `dind` sidecar and `nvme-setup` init container
/// override this back to root at the container level (see
/// [`privileged_security_context`]).
///
/// Returns `None` when the flag is off, leaving the pod at its previous behaviour
/// (containers run as root).
pub(super) fn pod_security_context(config: &Config) -> Option<PodSecurityContext> {
    config.sandbox.run_as_non_root.then(|| PodSecurityContext {
        run_as_user: Some(1000),
        run_as_group: Some(1000),
        run_as_non_root: Some(true),
        fs_group: Some(1000),
        // Only recursively chown/chmod a volume when its root doesn't already
        // have the right ownership. Without this the kubelet walks the ENTIRE
        // workspace PVC (node_modules, build caches, many small files) on every
        // mount, adding large pod-start latency on each start/restart. After the
        // first mount the root already matches, so the walk is skipped.
        fs_group_change_policy: Some("OnRootMismatch".to_string()),
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// Privileged security context for the `nvme-setup` init container and the `dind`
/// sidecar.
///
/// Required to `mkfs.ext4` / `mount` the raw instance-store NVMe device (init) and
/// to run `dockerd` (sidecar). Explicitly pins `runAsUser: 0` / `runAsNonRoot:
/// false` so these containers keep running as root even when the pod-level
/// securityContext enables `runAsNonRoot` — otherwise the kubelet would reject
/// them. Used ONLY by these containers — never by the agent/server containers,
/// which use [`app_security_context`].
///
/// These containers carry no `seLinuxOptions`. On an SELinux-enforcing node the
/// CRI drops that field for any privileged container, so setting it changes
/// nothing about the type the container actually gets.
pub(super) fn privileged_security_context() -> SecurityContext {
    SecurityContext {
        privileged: Some(true),
        run_as_user: Some(0),
        run_as_non_root: Some(false),
        ..Default::default()
    }
}
