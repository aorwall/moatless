//! What Fargate admits and what it refuses.

use super::fixtures::*;
use super::*;

#[test]
fn fargate_pod_uses_s3_sync_and_guaranteed_resources() {
    let config = fargate_config();
    let plan = test_plan();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let spec = pod.spec.as_ref().unwrap();
    assert!(
        spec.containers
            .iter()
            .all(|container| container.name != "dind")
    );
    assert!(
        spec.volumes
            .as_ref()
            .unwrap()
            .iter()
            .all(|volume| volume.host_path.is_none() && volume.persistent_volume_claim.is_none())
    );

    for container in &spec.containers {
        let security = container.security_context.as_ref().unwrap();
        assert_ne!(security.privileged, Some(true));
        assert_eq!(security.allow_privilege_escalation, Some(false));
        let capabilities = security.capabilities.as_ref().unwrap();
        assert!(capabilities.add.is_none());
        assert_eq!(
            capabilities.drop.as_deref(),
            Some(["ALL".to_string()].as_slice())
        );
    }

    let sandbox = spec
        .containers
        .iter()
        .find(|container| container.name == "sandbox")
        .unwrap();
    assert!(sandbox.env.as_ref().unwrap().iter().any(|variable| {
        variable.name == "WORKSPACE_SYNC_ENABLED" && variable.value.as_deref() == Some("true")
    }));
    let resources = sandbox.resources.as_ref().unwrap();
    for name in ["cpu", "memory", "ephemeral-storage"] {
        assert_eq!(
            resources.requests.as_ref().unwrap().get(name),
            resources.limits.as_ref().unwrap().get(name)
        );
    }
    assert_eq!(
        resources
            .requests
            .as_ref()
            .unwrap()
            .get("ephemeral-storage")
            .map(|value| value.0.as_str()),
        Some("40Gi")
    );
}

#[test]
fn fargate_rejects_docker_access() {
    let config = fargate_config();
    let mut plan = test_plan();
    plan.repo_config.docker_access = Some(true);

    let error = build_pod(&config, "task-1", &plan, None, None).unwrap_err();
    assert!(error.contains("dockerAccess"));
}

#[test]
fn fargate_rejects_repo_storage_override() {
    use moatless_workspace_types::workspace::ResourceConfig;

    let config = fargate_config();
    let mut plan = test_plan();
    plan.repo_config.resources = Some(ResourceConfig {
        pvc_enabled: Some(true),
        ..Default::default()
    });

    let error = build_pod(&config, "task-1", &plan, None, None).unwrap_err();
    assert!(error.contains("pvcEnabled=false"));
}

#[test]
fn fargate_rejects_workspace_pvc() {
    let config = fargate_config();
    let plan = test_plan();

    let error = build_pod(&config, "task-1", &plan, Some("workspace-pvc"), None).unwrap_err();
    assert!(error.contains("volume claims"));
}

#[test]
fn fargate_rejects_per_repo_burstable_resources() {
    use moatless_workspace_types::workspace::ResourceConfig;

    let config = fargate_config();
    let mut plan = test_plan();
    plan.repo_config.resources = Some(ResourceConfig {
        requests: quantities(Some("1"), None),
        limits: quantities(Some("4"), None),
        ..Default::default()
    });

    let error = build_pod(&config, "task-1", &plan, None, None).unwrap_err();
    assert!(error.contains("equal cpu requests and limits"));
}
