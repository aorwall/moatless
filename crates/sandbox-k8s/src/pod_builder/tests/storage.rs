//! Which volume a workspace lands on, how it is sized, and what the cache
//! volumes wire up.

use super::fixtures::*;
use super::*;

#[test]
fn pvc_switch_off_mounts_capped_workspace_empty_dir() {
    use moatless_workspace_types::workspace::ResourceConfig;

    // No PVC → workspace-data is a node-local emptyDir sized from the repo's
    // resources.storage, mounted at /opt/moatless everywhere.
    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.resources = Some(ResourceConfig {
        storage: Some(memory("7Gi")),
        pvc_enabled: Some(false),
        s3_enabled: Some(false),
        ..Default::default()
    });

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
    let ws = volumes
        .iter()
        .find(|v| v.name == "workspace-data")
        .expect("workspace-data volume present");
    let empty = ws
        .empty_dir
        .as_ref()
        .expect("workspace-data is an emptyDir when the PVC switch is off");
    // Node disk, not RAM: medium must be unset (None/empty).
    assert!(
        empty.medium.as_deref().unwrap_or("").is_empty(),
        "a PVC-less workspace must live on node disk, not memory"
    );
    assert_eq!(empty.size_limit.as_ref().map(|q| q.0.as_str()), Some("7Gi"));

    // No PVC volume should be present.
    assert!(
        volumes.iter().all(|v| v.persistent_volume_claim.is_none()),
        "the PVC switch is off — no PVC may be referenced"
    );

    // The sandbox container mounts the workspace emptyDir at /opt/moatless and /tmp.
    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    let mounts = container.volume_mounts.as_ref().unwrap();
    assert!(
        mounts
            .iter()
            .any(|m| m.name == "workspace-data" && m.mount_path == "/opt/moatless"),
        "sandbox must mount workspace-data at /opt/moatless"
    );
    assert!(
        mounts.iter().any(|m| m.name == "workspace-data"
            && m.mount_path == "/tmp"
            && m.sub_path.as_deref() == Some("tmp")),
        "/tmp must be a subPath of workspace-data"
    );
    assert!(
        mounts
            .iter()
            .any(|m| m.name == "run-moatless" && m.mount_path == "/run/moatless"),
        "credential-broker socket dir must be a writable mount for non-root"
    );

    // Server containers share the same workspace volume.
    let servers = build_server_containers(&config, "task-1", &plan).unwrap();
    let web = servers
        .iter()
        .find(|c| c.name == "web-server")
        .expect("web-server present");
    assert!(
        web.volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .any(|m| m.name == "workspace-data" && m.mount_path == "/opt/moatless"),
        "server containers must share workspace-data"
    );
}

/// The two switches are read independently, and neither ever produces a
/// hostPath: the removed `node` workspace tier has no replacement.
#[test]
fn storage_switches_pick_volume_and_sync_env() {
    use moatless_workspace_types::workspace::ResourceConfig;

    let config = test_config();
    for (pvc, s3) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut plan = test_plan();
        plan.repo_config.resources = Some(ResourceConfig {
            pvc_enabled: Some(pvc),
            s3_enabled: Some(s3),
            ..Default::default()
        });
        // A PVC name is only handed to `build_pod` when the switch asked for
        // one — the provider resolves it the same way.
        let pvc_name = pvc.then_some("pvc-task-1");

        let pod = build_pod(&config, "task-1", &plan, pvc_name, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let ws = volumes
            .iter()
            .find(|v| v.name == "workspace-data")
            .expect("workspace-data volume present");
        assert!(
            ws.host_path.is_none(),
            "no switch combination may mount the workspace as a hostPath ({pvc}/{s3})"
        );
        assert_eq!(
            ws.persistent_volume_claim.is_some(),
            pvc,
            "the pvc switch decides the volume kind ({pvc}/{s3})"
        );
        assert_eq!(
            ws.empty_dir.is_some(),
            !pvc,
            "without a PVC the workspace is an emptyDir ({pvc}/{s3})"
        );
        assert!(
            pod.spec
                .as_ref()
                .and_then(|s| s.init_containers.as_ref())
                .is_none_or(|c| c.iter().all(|c| c.name != "node-sandbox-setup")),
            "the node workspace setup container is gone ({pvc}/{s3})"
        );

        let container = build_sandbox_container(&config, "task-1", &plan, None, pvc_name).unwrap();
        assert_eq!(
            env_value(&container, "WORKSPACE_SYNC_ENABLED"),
            s3.then_some("true"),
            "the s3 switch decides the sync env ({pvc}/{s3})"
        );
        assert_eq!(
            env_value(&container, "WORKSPACE_RESTORE_IF_EMPTY"),
            (pvc && s3).then_some("true"),
            "restore is conditional only when a durable volume backs the snapshot ({pvc}/{s3})"
        );
    }
}

/// A pvc+s3 plan that ends up on an emptyDir — PVC creation degraded, or the
/// attach-failure fallback rebuilt without one — must behave like the
/// emptyDir+S3 tier.
///
/// Claiming durability on an ephemeral volume would drop the periodic upload
/// backstop and skip the boot restore — exactly backwards for the pod that
/// needs both most.
#[test]
fn a_pvc_plan_without_a_pvc_is_not_treated_as_durable() {
    use moatless_workspace_types::workspace::ResourceConfig;

    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.resources = Some(ResourceConfig {
        pvc_enabled: Some(true),
        s3_enabled: Some(true),
        ..Default::default()
    });

    let degraded = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(
        env_value(&degraded, "WORKSPACE_SYNC_ENABLED"),
        Some("true"),
        "S3 stays on — it is the only persistence this pod has left"
    );
    assert_eq!(
        env_value(&degraded, "WORKSPACE_RESTORE_IF_EMPTY"),
        None,
        "an emptyDir is not durable, whatever the plan asked for"
    );

    // The same plan *with* its claim is the durable tier.
    let durable =
        build_sandbox_container(&config, "task-1", &plan, None, Some("pvc-task-1")).unwrap();
    assert_eq!(
        env_value(&durable, "WORKSPACE_RESTORE_IF_EMPTY"),
        Some("true")
    );
}

#[test]
fn pvc_less_workspace_size_falls_back_to_global_default() {
    // No per-repo storage override → emptyDir uses the global storage_size.
    let mut config = test_config();
    config.sandbox.storage_size = "3Gi".into();
    let plan = test_plan();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
    let ws = volumes
        .iter()
        .find(|v| v.name == "workspace-data")
        .expect("workspace-data volume present");
    assert_eq!(
        ws.empty_dir
            .as_ref()
            .and_then(|e| e.size_limit.as_ref())
            .map(|q| q.0.as_str()),
        Some("3Gi")
    );
}

/// Hardened default: a sandbox that opts into nothing gets no init container
/// and no hostPath volume. Only `dockerAccess` adds either.
#[test]
fn default_sandbox_pod_has_no_init_or_hostpath() {
    let config = test_config();
    let plan = test_plan();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();
    assert!(spec.init_containers.is_none(), "no init containers");
    assert!(
        spec.volumes
            .as_ref()
            .unwrap()
            .iter()
            .all(|v| v.host_path.is_none()),
        "no hostPath volumes"
    );
}

