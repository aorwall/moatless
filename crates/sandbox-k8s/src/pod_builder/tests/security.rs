//! The security contexts each container runs under, and what `run_as_non_root`
//! changes.

use super::fixtures::*;
use super::*;

/// Flag OFF (default): no pod-level securityContext, and the agent/server
/// containers keep `DAC_OVERRIDE` (the root-mode behaviour).
#[test]
fn run_as_non_root_off_keeps_root_behaviour() {
    let config = test_config(); // run_as_non_root defaults to false
    let plan = test_plan();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();
    assert!(
        spec.security_context.is_none(),
        "no pod securityContext when the flag is off"
    );

    for c in &spec.containers {
        let add = c
            .security_context
            .as_ref()
            .and_then(|s| s.capabilities.as_ref())
            .and_then(|caps| caps.add.as_ref())
            .expect("app container has added capabilities");
        assert!(
            add.iter().any(|cap| cap == "DAC_OVERRIDE"),
            "root mode keeps DAC_OVERRIDE on {}",
            c.name
        );
    }
}

/// Flag ON: pod-level securityContext runs containers as UID/GID 1000 with
/// fsGroup 1000 + `RuntimeDefault` seccomp.
///
/// Agent/server containers drop `DAC_OVERRIDE` (chown + `fsGroup: 1000` already
/// give ownership) but keep `NET_BIND_SERVICE`.
#[test]
fn run_as_non_root_on_sets_pod_security_context_and_drops_dac_override() {
    let mut config = test_config();
    config.sandbox.run_as_non_root = true;
    let plan = test_plan();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();

    let psc = spec
        .security_context
        .as_ref()
        .expect("pod securityContext present when the flag is on");
    assert_eq!(psc.run_as_user, Some(1000));
    assert_eq!(psc.run_as_group, Some(1000));
    assert_eq!(psc.run_as_non_root, Some(true));
    assert_eq!(psc.fs_group, Some(1000));
    // OnRootMismatch avoids a full recursive chown of the workspace PVC on
    // every mount (only walks when the volume root ownership is wrong).
    assert_eq!(
        psc.fs_group_change_policy.as_deref(),
        Some("OnRootMismatch")
    );
    assert_eq!(
        psc.seccomp_profile.as_ref().map(|s| s.type_.as_str()),
        Some("RuntimeDefault")
    );

    for c in &spec.containers {
        let caps = c
            .security_context
            .as_ref()
            .and_then(|s| s.capabilities.as_ref())
            .expect("app container has capabilities");
        let add = caps.add.as_ref().expect("added capabilities");
        assert!(
            !add.iter().any(|cap| cap == "DAC_OVERRIDE"),
            "non-root mode must drop DAC_OVERRIDE on {}",
            c.name
        );
        assert!(
            add.iter().any(|cap| cap == "NET_BIND_SERVICE"),
            "non-root mode keeps NET_BIND_SERVICE on {}",
            c.name
        );
    }
}

/// Even with the flag ON, the privileged `dind` sidecar keeps running as root
/// (explicit runAsUser 0 / runAsNonRoot false so the kubelet admits it under
/// the pod-level runAsNonRoot).
#[test]
fn run_as_non_root_on_keeps_dind_root() {
    let mut config = test_config();
    config.sandbox.run_as_non_root = true;
    let mut plan = test_plan();
    plan.repo_config.docker_access = Some(true);

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let dind = pod
        .spec
        .as_ref()
        .unwrap()
        .containers
        .iter()
        .find(|c| c.name == "dind")
        .expect("dind sidecar present");
    let sc = dind.security_context.as_ref().unwrap();
    assert_eq!(sc.privileged, Some(true));
    assert_eq!(sc.run_as_user, Some(0));
    assert_eq!(sc.run_as_non_root, Some(false));
}

/// Privileged containers carry no `seLinuxOptions`. The CRI drops the field for
/// privileged containers on an SELinux-enforcing node, so emitting one would
/// assert a guarantee the runtime does not honour.
#[test]
fn privileged_containers_have_no_selinux_options() {
    use moatless_core::config::SandboxCacheVolume;

    let mut config = test_config();
    config.sandbox.cache_volumes = vec![SandboxCacheVolume {
        name: "bun".into(),
        mount_path: "/bun-cache".into(),
        env: vec![("BUN_INSTALL_CACHE_DIR".into(), "/bun-cache".into())],
    }];
    let mut plan = test_plan();
    plan.repo_config.docker_access = Some(true);

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();

    let dind = spec
        .containers
        .iter()
        .find(|c| c.name == "dind")
        .expect("dind sidecar present");
    assert_eq!(
        dind.security_context
            .as_ref()
            .unwrap()
            .se_linux_options
            .as_ref(),
        None
    );

    let nvme = spec
        .init_containers
        .as_ref()
        .unwrap()
        .iter()
        .find(|c| c.name == "nvme-setup")
        .expect("nvme-setup init container present");
    assert_eq!(
        nvme.security_context
            .as_ref()
            .unwrap()
            .se_linux_options
            .as_ref(),
        None
    );
}

/// The `nvme-setup` init container keeps its privileged root context and
/// carries modest resource requests/limits.
#[test]
fn nvme_setup_init_container_is_root_and_has_resources() {
    use moatless_core::config::SandboxCacheVolume;

    let mut config = test_config();
    config.sandbox.run_as_non_root = true;
    config.sandbox.cache_volumes = vec![SandboxCacheVolume {
        name: "bun".into(),
        mount_path: "/bun-cache".into(),
        env: vec![("BUN_INSTALL_CACHE_DIR".into(), "/bun-cache".into())],
    }];
    let plan = test_plan();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let nvme = pod
        .spec
        .as_ref()
        .unwrap()
        .init_containers
        .as_ref()
        .unwrap()
        .iter()
        .find(|c| c.name == "nvme-setup")
        .expect("nvme-setup init container present");

    let sc = nvme.security_context.as_ref().unwrap();
    assert_eq!(sc.privileged, Some(true));
    assert_eq!(sc.run_as_user, Some(0));
    assert_eq!(sc.run_as_non_root, Some(false));

    let resources = nvme
        .resources
        .as_ref()
        .expect("nvme-setup must declare resources");
    assert!(
        resources
            .requests
            .as_ref()
            .is_some_and(|r| r.contains_key("cpu") && r.contains_key("memory"))
    );
    assert!(
        resources
            .limits
            .as_ref()
            .is_some_and(|l| l.contains_key("cpu") && l.contains_key("memory"))
    );
}
