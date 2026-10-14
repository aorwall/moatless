//! The CPU, memory and size limits a container ends up with, and what a
//! per-repo override does and does not change.

use super::fixtures::*;
use super::*;

#[test]
fn sandbox_container_has_memory_request_and_limit() {
    let config = test_config();
    let plan = test_plan();

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    let resources = container
        .resources
        .as_ref()
        .expect("sandbox container must have resources populated");

    let requests = resources.requests.as_ref().expect("requests set");
    assert_eq!(requests.get("memory").map(|q| q.0.as_str()), Some("2Gi"));
    assert_eq!(requests.get("cpu").map(|q| q.0.as_str()), Some("1"));

    let limits = resources.limits.as_ref().expect("limits set");
    assert_eq!(limits.get("memory").map(|q| q.0.as_str()), Some("8Gi"));
    assert_eq!(limits.get("cpu").map(|q| q.0.as_str()), Some("4"));
}

/// The prompt's "Saved to:" line is rendered from `file_mount_path`, so the pod
/// has to be told the same path rather than falling back to a default of its
/// own.
#[test]
fn sandbox_container_takes_upload_dir_from_file_mount_path() {
    let mut config = test_config();
    config.sandbox.file_mount_path = "/opt/moatless/workspace/uploads".into();
    let plan = test_plan();

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();

    let env = container.env.as_ref().expect("sandbox container has env");
    let upload_dir = env
        .iter()
        .find(|var| var.name == "UPLOAD_DIR")
        .expect("UPLOAD_DIR is set on the sandbox container");
    assert_eq!(
        upload_dir.value.as_deref(),
        Some("/opt/moatless/workspace/uploads")
    );
}

#[test]
fn sandbox_container_honors_per_repo_resource_override() {
    use moatless_workspace_types::workspace::ResourceConfig;

    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.resources = Some(ResourceConfig {
        requests: quantities(Some("500m"), Some("4Gi")),
        limits: quantities(Some("6"), Some("16Gi")),
        ..Default::default()
    });

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(
        resource_quantities(&container),
        ("500m", "4Gi", "6", "16Gi")
    );
}

#[test]
fn sandbox_container_keeps_defaults_for_keys_the_override_omits() {
    use moatless_workspace_types::workspace::ResourceConfig;

    // A memory-only override must not drop the configured CPU request/limit.
    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.resources = Some(ResourceConfig {
        requests: quantities(None, Some("4Gi")),
        limits: quantities(None, Some("16Gi")),
        ..Default::default()
    });

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(resource_quantities(&container), ("1", "4Gi", "4", "16Gi"));
}

#[test]
fn sandbox_container_stays_bounded_with_a_storage_only_override() {
    use moatless_workspace_types::workspace::ResourceConfig;

    // `ResourceConfig` doubles as the storage config. A repo that sets only
    // storage fields must keep the global CPU/memory bounds — an empty
    // `resources` block makes the container BestEffort and first to be
    // evicted.
    let config = test_config();
    let mut plan = test_plan();
    plan.repo_config.resources = Some(ResourceConfig {
        storage: Some(memory("20Gi")),
        pvc_enabled: Some(false),
        ..Default::default()
    });

    let container = build_sandbox_container(&config, "task-1", &plan, None, None).unwrap();
    assert_eq!(resource_quantities(&container), ("1", "2Gi", "4", "8Gi"));
}

/// The shared-data emptyDir is always bounded by `shared_data_size`.
#[test]
fn shared_data_empty_dir_is_size_capped() {
    let mut config = test_config();
    config.sandbox.shared_data_size = "512Mi".into();
    let plan = test_plan();

    let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
    let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
    let shared = volumes
        .iter()
        .find(|v| v.name == "shared-data")
        .expect("shared-data volume present");
    assert_eq!(
        shared
            .empty_dir
            .as_ref()
            .and_then(|e| e.size_limit.as_ref())
            .map(|q| q.0.as_str()),
        Some("512Mi"),
        "shared-data emptyDir must be size-capped"
    );
}

/// The sandbox container's ephemeral-storage `(request, limit)`.
fn ephemeral_storage(config: &Config) -> (Option<String>, Option<String>) {
    let container = build_sandbox_container(config, "task-1", &test_plan(), None, None).unwrap();
    let resources = container.resources.unwrap();
    let get = |map: Option<BTreeMap<String, Quantity>>| {
        map.and_then(|map| map.get("ephemeral-storage").map(|q| q.0.clone()))
    };
    (get(resources.requests), get(resources.limits))
}

#[test]
fn no_ephemeral_storage_setting_writes_neither_request_nor_limit() {
    assert_eq!(ephemeral_storage(&test_config()), (None, None));
}

/// A request alone sets no limit, so a container over it is ranked first
/// for eviction under node pressure rather than evicted at a fixed ceiling.
#[test]
fn an_ephemeral_storage_request_sets_no_limit() {
    let mut config = test_config();
    config.sandbox.ephemeral_storage = Some("6Gi".into());
    assert_eq!(ephemeral_storage(&config), (Some("6Gi".into()), None));
}

#[test]
fn an_ephemeral_storage_limit_is_set_independently_of_the_request() {
    let mut config = test_config();
    config.sandbox.ephemeral_storage = Some("6Gi".into());
    config.sandbox.ephemeral_storage_limit = Some("50Gi".into());
    assert_eq!(
        ephemeral_storage(&config),
        (Some("6Gi".into()), Some("50Gi".into()))
    );
}

/// Fargate provisions pod-local disk from the request and wants equal
/// requests and limits, so the request doubles as the limit there, even over
/// an explicit limit.
#[test]
fn fargate_uses_the_ephemeral_storage_request_as_the_limit() {
    assert_eq!(
        ephemeral_storage(&fargate_config()),
        (Some("40Gi".into()), Some("40Gi".into()))
    );
    let mut config = fargate_config();
    config.sandbox.ephemeral_storage_limit = Some("20Gi".into());
    assert_eq!(
        ephemeral_storage(&config),
        (Some("40Gi".into()), Some("40Gi".into()))
    );
}
