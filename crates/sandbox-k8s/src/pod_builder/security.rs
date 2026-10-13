//! The security contexts the pod and its containers run under.

use super::*;

/// Container-level security context for the agent + server containers.
///
/// Both modes drop ALL capabilities. `DAC_OVERRIDE` is added back only when
/// running as root (UID 0), since `/home/moatless` is chowned to UID 1000 and
/// root needs it to write there during init. Fargate accepts drops but not
/// additions, so its preset adds none.
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
/// is enabled; `None` leaves containers running as root.
///
/// Runs containers as non-root `moatless` (UID/GID 1000) with `RuntimeDefault`
/// seccomp and `fsGroup: 1000`; the privileged `dind` sidecar overrides back to
/// root at the container level (see [`privileged_security_context`]).
pub(super) fn pod_security_context(config: &Config) -> Option<PodSecurityContext> {
    config.sandbox.run_as_non_root.then(|| PodSecurityContext {
        run_as_user: Some(1000),
        run_as_group: Some(1000),
        run_as_non_root: Some(true),
        fs_group: Some(1000),
        // Chown only when root ownership mismatches: otherwise the kubelet
        // walks the entire workspace PVC on every mount, adding large
        // pod-start latency. After the first mount ownership already
        // matches, so later mounts skip the walk.
        fs_group_change_policy: Some("OnRootMismatch".to_string()),
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// Privileged security context for the `dind` sidecar.
///
/// Pins `runAsUser: 0`/`runAsNonRoot: false` so it stays root (needed for
/// `dockerd`) even when the pod-level context enables `runAsNonRoot`, or the
/// kubelet would reject it; used only here, never by
/// [`app_security_context`]'s containers.
pub(super) fn privileged_security_context() -> SecurityContext {
    SecurityContext {
        privileged: Some(true),
        run_as_user: Some(0),
        run_as_non_root: Some(false),
        ..Default::default()
    }
}
