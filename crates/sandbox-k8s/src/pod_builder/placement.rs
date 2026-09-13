//! Where a pod runs and what it runs from: image, size, node pinning, and the
//! backing store the Docker daemon gets.

use super::*;

/// Resolve the sandbox image for a plan: per-repo override → default sandbox
/// image.
///
/// An empty override string is treated as unset; this is the single source of
/// truth for the image a sandbox runs.
pub fn resolve_sandbox_image(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .docker_image
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| config.sandbox.image.clone())
}

/// Resolve the sandbox storage size for the PVC-less (emptyDir) case: a
/// per-repo `resources.storage` override wins over the global
/// `K8S_SANDBOX_STORAGE_SIZE`. One knob sizes both — the PVC reads the same
/// value in `pvc.rs`.
pub(super) fn resolve_sandbox_size(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.storage.as_ref())
        .map_or_else(|| config.sandbox.storage_size.clone(), Memory::to_string)
}

/// Express a retained-node placement so the scheduler still runs.
///
/// `spec.nodeName` binds directly and skips scheduling, so a node that can't
/// fit the pod rejects it at admission and the `ReplicaSet` respawns forever;
/// affinity leaves an unschedulable pod `Pending` instead. Only a cache (dind's
/// data root) pins here, so the preference is soft: prefer the recorded node,
/// settle for another and pay a cold rebuild.
pub(crate) fn retained_node_affinity(node_name: &str) -> Affinity {
    let term = NodeSelectorTerm {
        match_fields: Some(vec![NodeSelectorRequirement {
            key: "metadata.name".to_string(),
            operator: "In".to_string(),
            values: Some(vec![node_name.to_string()]),
        }]),
        ..Default::default()
    };
    Affinity {
        node_affinity: Some(NodeAffinity {
            preferred_during_scheduling_ignored_during_execution: Some(vec![
                PreferredSchedulingTerm {
                    weight: 100,
                    preference: term,
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Place a freshly built pod on its retained node.
pub(crate) fn pin_pod_to_node(pod: &mut Pod, node_name: &str) -> Result<(), SandboxError> {
    pod.spec
        .as_mut()
        .ok_or_else(|| SandboxError::PodBuildError("Pod has no spec".to_string()))?
        .affinity = Some(retained_node_affinity(node_name));
    Ok(())
}

pub(super) fn node_docker_host_path(config: &Config, task_id: &str) -> String {
    let root = config.sandbox.cache_host_root.trim_end_matches('/');
    format!("{root}/docker/{task_id}")
}

/// Where the dind sidecar's `/var/lib/docker` lives. Unset means
/// [`StorageMedium::Node`].
pub(super) fn resolve_dind_storage_backend(plan: &ProvisionPlan) -> StorageMedium {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.docker_storage_mode)
        .unwrap_or(StorageMedium::Node)
}

/// Resolve the `docker-storage` emptyDir cap for the dind sidecar: a per-repo
/// `resources.dockerStorage` override wins over the global
/// `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE`.
///
/// Exceeding the cap gets the pod evicted by the kubelet (wiping dind data), so
/// DB-heavy repos need headroom.
pub(super) fn resolve_dind_storage_size(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.docker_storage.as_ref())
        .map_or_else(
            || config.sandbox.docker_dind_storage_size.clone(),
            Memory::to_string,
        )
}

/// Parent directory of the package-cache root (e.g. `/mnt` for `/mnt/cache`).
///
/// `nvme-setup` bind-mounts this parent with bidirectional propagation so the
/// NVMe it mounts at `<parent>/<basename>` is visible to the sibling
/// `pkg-cache` hostPath mount; falls back to `/` if the root has no parent
/// component.
pub(super) fn cache_host_parent(root: &str) -> &str {
    let trimmed = root.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => "/",
    }
}
