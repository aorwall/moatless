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
    let config = test_config();
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
}

/// Under `run_as_non_root` the `prepare-tmp` init container gets the whole
/// `workspace-data` volume, so the kubelet has not yet created the `tmp`
/// subPath the sandbox mounts at `/tmp`, and it runs before the sandbox does.
#[test]
fn run_as_non_root_prepares_tmp_before_the_sandbox_mounts_it() {
    let mut config = test_config();
    config.sandbox.run_as_non_root = true;
    let pod = build_pod(&config, "task-1", &test_plan(), None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();

    let init = spec.init_containers.as_ref().expect("init containers");
    assert_eq!(init.len(), 1);
    let prepare = &init[0];
    assert_eq!(prepare.name, "prepare-tmp");
    assert_eq!(prepare.image, spec.containers[0].image);
    let mounts = prepare.volume_mounts.as_ref().unwrap();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].name, "workspace-data");
    assert_eq!(mounts[0].mount_path, "/opt/moatless");
    assert_eq!(
        mounts[0].sub_path, None,
        "a subPath here would create tmp as root"
    );
    let command = prepare.command.as_ref().unwrap();
    assert!(command[2].contains("chmod 01777 tmp"));
    assert_eq!(command.last().map(String::as_str), Some("/opt/moatless"));

    let tmp_mount = spec.containers[0]
        .volume_mounts
        .as_ref()
        .unwrap()
        .iter()
        .find(|m| m.mount_path == "/tmp")
        .expect("sandbox mounts /tmp");
    assert_eq!(tmp_mount.name, "workspace-data");
    assert_eq!(tmp_mount.sub_path.as_deref(), Some("tmp"));
}

/// A root sandbox owns `/tmp` and chmods it itself, so it gets no init
/// container.
#[test]
fn run_as_root_adds_no_tmp_init_container() {
    let pod = build_pod(&test_config(), "task-1", &test_plan(), None, None).unwrap();
    assert!(pod.spec.unwrap().init_containers.is_none());
}

/// Runs [`super::containers::TMP_PREPARE`] the way the pod does, on a volume
/// whose `tmp` is missing, then on one whose `tmp` is the kubelet's `2777`.
#[cfg(unix)]
#[test]
fn tmp_prepare_script_leaves_a_sticky_tmp() {
    use std::os::unix::fs::PermissionsExt;

    let run = |root: &std::path::Path| {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(super::containers::TMP_PREPARE)
            .arg("prepare-tmp")
            .arg(root)
            .status()
            .expect("run sh");
        assert!(status.success());
        let mode = std::fs::metadata(root.join("tmp"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o1777, "got {mode:o}");
    };

    let volume = tempfile::tempdir().unwrap();
    std::fs::set_permissions(volume.path(), std::fs::Permissions::from_mode(0o2777)).unwrap();
    run(volume.path());

    std::fs::set_permissions(
        volume.path().join("tmp"),
        std::fs::Permissions::from_mode(0o2777),
    )
    .unwrap();
    std::fs::write(volume.path().join("tmp/leftover"), b"x").unwrap();
    run(volume.path());
    assert!(
        volume.path().join("tmp/leftover").exists(),
        "an owned tmp is kept"
    );
}
