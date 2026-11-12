//! The Docker-in-Docker sidecar: when it appears, what backs it, and what it
//! adds to the sandbox container.

use super::fixtures::*;
use super::*;

/// `dockerAccess: true` → a privileged `dind` sidecar + `docker-storage`
/// volume + `DOCKER_HOST`/`TESTCONTAINERS_RYUK_DISABLED` on the agent
/// container, while the agent container itself stays unprivileged.
#[test]
fn docker_access_adds_privileged_dind_sidecar_and_env() {
    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.docker_access = Some(true);

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();

    // A privileged `dind` sidecar using the configured image, running dockerd.
    let dind = spec
        .containers
        .iter()
        .find(|c| c.name == "dind")
        .expect("dind sidecar present when dockerAccess is set");
    assert_eq!(
        dind.security_context.as_ref().and_then(|s| s.privileged),
        Some(true),
        "dind must be privileged"
    );
    assert_eq!(dind.image.as_deref(), Some("docker:dind"));
    let args = dind.args.as_ref().unwrap().join(" ");
    assert!(args.contains("dockerd"), "dind runs dockerd, got: {args}");
    assert!(args.contains("--tls=false"), "dind runs without TLS");
    assert!(
        args.contains("--feature=containerd-snapshotter=false"),
        "dind must use overlay2, or an SELinux xattr fails every unpack: {args}"
    );
    let start = dind.command.as_ref().unwrap().join(" ");
    assert!(
        start.contains("context=") && start.contains("exec dockerd-entrypoint.sh \"$@\""),
        "dind must label its volumes mount, or seeding a VOLUME fails: {start}"
    );
    assert!(
        dind.volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .any(|m| m.name == "docker-storage" && m.mount_path == "/var/lib/docker"),
        "dind must mount docker-storage at /var/lib/docker"
    );

    // The default node-local docker-storage hostPath backs /var/lib/docker.
    let ds = spec
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .find(|v| v.name == "docker-storage")
        .expect("docker-storage volume present");
    assert!(ds.host_path.is_some(), "docker-storage must be a hostPath");
    assert!(
        ds.empty_dir.is_none(),
        "docker-storage must not be an emptyDir"
    );

    // The sandbox container is pointed at the dind sidecar and disables Ryuk.
    let sandbox = spec
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .unwrap();
    let env = sandbox.env.as_ref().unwrap();
    assert!(
        env.iter()
            .any(|e| e.name == "DOCKER_HOST" && e.value.as_deref() == Some("tcp://localhost:2375")),
        "sandbox must point DOCKER_HOST at the dind sidecar"
    );
    assert!(
        env.iter()
            .any(|e| e.name == "TESTCONTAINERS_RYUK_DISABLED"
                && e.value.as_deref() == Some("true")),
        "Ryuk must be disabled for the dind topology"
    );

    // The agent container itself stays hardened (not privileged).
    assert_ne!(
        sandbox.security_context.as_ref().and_then(|s| s.privileged),
        Some(true),
        "sandbox container must not be privileged"
    );
}

/// `resources.dockerStorage` overrides the global dind storage cap when the
/// repo explicitly selects the emptyDir Docker storage tier.
#[test]
fn docker_storage_override_sizes_dind_empty_dir() {
    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.docker_access = Some(true);
    plan.repo_config.resources = Some(moatless_workspace_types::workspace::ResourceConfig {
        docker_storage: Some(memory("50Gi")),
        docker_storage_mode: Some(StorageMedium::EmptyDir),
        ..Default::default()
    });

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let ds_limit = pod
        .spec
        .as_ref()
        .unwrap()
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .find(|v| v.name == "docker-storage")
        .and_then(|v| v.empty_dir.as_ref())
        .and_then(|e| e.size_limit.as_ref())
        .map(|q| q.0.as_str().to_string());
    assert_eq!(ds_limit.as_deref(), Some("50Gi"));

    // EmptyDir mode with no override → global default from config.
    plan.repo_config.resources = Some(moatless_workspace_types::workspace::ResourceConfig {
        docker_storage_mode: Some(StorageMedium::EmptyDir),
        ..Default::default()
    });
    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let ds_limit = pod
        .spec
        .as_ref()
        .unwrap()
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .find(|v| v.name == "docker-storage")
        .and_then(|v| v.empty_dir.as_ref())
        .and_then(|e| e.size_limit.as_ref())
        .map(|q| q.0.as_str().to_string());
    assert_eq!(
        ds_limit.as_deref(),
        Some(config.sandbox.docker_dind_storage_size.as_str())
    );
}

#[test]
fn docker_storage_remains_node_backed_without_archive_management() {
    let mut config = test_config();
    assert!(!config.sandbox.node_archive_enabled);
    config.sandbox.cache_host_root = "/mnt/cache".into();
    let mut plan = test_plan();
    plan.repo_config.docker_access = Some(true);
    plan.repo_config.resources = Some(moatless_workspace_types::workspace::ResourceConfig {
        docker_storage: Some(memory("50Gi")),
        ..Default::default()
    });

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let docker_storage = pod
        .spec
        .as_ref()
        .unwrap()
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .find(|v| v.name == "docker-storage")
        .expect("docker-storage volume present");
    let host_path = docker_storage
        .host_path
        .as_ref()
        .expect("default dind storage uses hostPath");
    assert_eq!(host_path.path, "/mnt/cache/docker/task-1");
    assert_eq!(host_path.type_.as_deref(), Some("DirectoryOrCreate"));
    assert!(
        docker_storage.empty_dir.is_none(),
        "node docker storage must not use emptyDir"
    );
}

/// Default (no `dockerAccess`) → hardened: no dind sidecar, no docker-storage
/// volume, no `DOCKER_HOST` on the agent container.
#[test]
fn no_docker_access_means_no_dind_sidecar() {
    let config = test_config();
    let plan = test_plan(); // docker_access defaults to None

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();

    assert!(
        spec.containers.iter().all(|c| c.name != "dind"),
        "no dind sidecar without dockerAccess"
    );
    assert!(
        spec.volumes
            .as_ref()
            .unwrap()
            .iter()
            .all(|v| v.name != "docker-storage"),
        "no docker-storage volume without dockerAccess"
    );
    let sandbox = spec
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .unwrap();
    assert!(
        sandbox
            .env
            .as_ref()
            .unwrap()
            .iter()
            .all(|e| e.name != "DOCKER_HOST"),
        "no DOCKER_HOST env without dockerAccess"
    );
}
