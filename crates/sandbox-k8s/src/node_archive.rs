//! Node-local archive location tracking and strict deletion.
//!
//! Node-backed Docker and build-cache data outlive sandbox pods. A deterministic
//! task ConfigMap records every node that has hosted such data and the newest
//! lifecycle generation seen there. Starts prefer the newest recorded node that
//! still exists, and schedule anywhere once those nodes are gone — everything
//! tracked here is a cache, so losing it costs a rebuild, never the work itself.
//! Hard deletion runs one deterministic cleanup pod per surviving recorded node
//! before removing the ConfigMap.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{
    Capabilities, ConfigMap, Container, HostPathVolumeSource, Node, Pod, PodSecurityContext,
    PodSpec, SeccompProfile, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use sha2::{Digest, Sha256};
use tracing::info;

use super::client::K8sClient;
use super::naming::sanitize_k8s_name;
use moatless_core::config::Config;
use moatless_sandbox_control::lifecycle::runtime::{generation_from_label, OPERATION_GENERATION_LABEL};
use moatless_sandbox_control::lifecycle::RuntimeError;

const CLEANUP_MOUNT_PATH: &str = "/host-cache";
const CLEANUP_TIMEOUT: Duration = Duration::from_mins(5);
const RECORD_RETRIES: usize = 5;

pub(crate) type NodeRecords = BTreeMap<String, i64>;

/// The node cleanup obligation captured before runtime resources are removed.
pub(crate) struct RemovalPlan {
    pub nodes: Vec<String>,
}

fn runtime_error(context: &str, error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Infrastructure(format!("{context}: {error}"))
}

fn archive_config_map_name(task_id: &str) -> String {
    let prefix = "moatless-archive-";
    let sanitized = sanitize_k8s_name(task_id, 63 - prefix.len());
    format!("{prefix}{sanitized}")
}

fn merge_node_record(records: &mut NodeRecords, node_name: &str, generation: i64) {
    records
        .entry(node_name.to_string())
        .and_modify(|current| *current = (*current).max(generation))
        .or_insert(generation);
}

fn records_from_config_map(config_map: &ConfigMap) -> Result<NodeRecords, RuntimeError> {
    config_map
        .data
        .as_ref()
        .map(|data| {
            data.iter()
                .map(|(node, generation)| {
                    generation
                        .parse::<i64>()
                        .map(|generation| (node.clone(), generation))
                        .map_err(|error| {
                            runtime_error(
                                "parse node archive metadata",
                                format!("{node}={generation}: {error}"),
                            )
                        })
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn config_map(task_id: &str, records: &NodeRecords) -> ConfigMap {
    ConfigMap {
        metadata: ObjectMeta {
            name: Some(archive_config_map_name(task_id)),
            labels: Some(BTreeMap::from([
                (
                    "app.kubernetes.io/name".to_string(),
                    "moatless-node-archive".to_string(),
                ),
                (
                    "app.kubernetes.io/component".to_string(),
                    "archive-metadata".to_string(),
                ),
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "moatless".to_string(),
                ),
                ("moatless.io/task-id".to_string(), task_id.to_string()),
            ])),
            ..Default::default()
        },
        data: Some(
            records
                .iter()
                .map(|(node, generation)| (node.clone(), generation.to_string()))
                .collect(),
        ),
        ..Default::default()
    }
}

pub(crate) fn pod_uses_node_archives(pod: &Pod) -> bool {
    pod_has_volume(pod, volume_holds_task_node_data)
}

fn pod_has_volume(pod: &Pod, predicate: fn(&Volume) -> bool) -> bool {
    pod.spec
        .as_ref()
        .and_then(|spec| spec.volumes.as_ref())
        .is_some_and(|volumes| volumes.iter().any(predicate))
}

pub(crate) fn deployment_uses_node_archives(deployment: &Deployment) -> bool {
    deployment_has_volume(deployment, volume_holds_task_node_data)
}

fn deployment_has_volume(deployment: &Deployment, predicate: fn(&Volume) -> bool) -> bool {
    deployment
        .spec
        .as_ref()
        .and_then(|spec| spec.template.spec.as_ref())
        .and_then(|spec| spec.volumes.as_ref())
        .is_some_and(|volumes| volumes.iter().any(predicate))
}

/// Task-scoped node-local trees owed a cleanup pod on hard deletion. The
/// workspace is not among them: it is a PVC or an emptyDir, never a hostPath.
fn volume_holds_task_node_data(volume: &Volume) -> bool {
    let Some(host_path) = volume.host_path.as_ref() else {
        return false;
    };
    match volume.name.as_str() {
        "docker-storage" => true,
        "build-cache" => host_path.path.contains("/build-cache/tasks/"),
        _ => false,
    }
}

/// Merge one observation without allowing an older watcher event to lower the
/// generation already recorded for the same node.
pub(crate) async fn record_node(
    k8s: &K8sClient,
    task_id: &str,
    node_name: &str,
    generation: i64,
) -> Result<(), RuntimeError> {
    let config_maps: Api<ConfigMap> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = archive_config_map_name(task_id);

    for _ in 0..RECORD_RETRIES {
        if let Some(mut current) = config_maps
            .get_opt(&name)
            .await
            .map_err(|error| runtime_error("read node archive metadata", error))?
        {
            let mut records = records_from_config_map(&current)?;
            merge_node_record(&mut records, node_name, generation);
            current.data = config_map(task_id, &records).data;
            match config_maps
                .replace(&name, &PostParams::default(), &current)
                .await
            {
                Ok(_) => return Ok(()),
                Err(kube::Error::Api(ref response)) if response.code == 409 => continue,
                Err(error) => {
                    return Err(runtime_error("update node archive metadata", error));
                }
            }
        }

        let mut records = NodeRecords::new();
        merge_node_record(&mut records, node_name, generation);
        match config_maps
            .create(&PostParams::default(), &config_map(task_id, &records))
            .await
        {
            Ok(_) => return Ok(()),
            Err(kube::Error::Api(ref response)) if response.code == 409 => continue,
            Err(error) => {
                return Err(runtime_error("create node archive metadata", error));
            }
        }
    }

    Err(RuntimeError::Infrastructure(format!(
        "node archive metadata for task {task_id} changed too frequently"
    )))
}

/// Best-effort watcher entry point. The caller decides how to log failures.
pub(crate) async fn record_observed_pod(
    k8s: &K8sClient,
    task_id: &str,
    pod: &Pod,
) -> Result<(), RuntimeError> {
    if !pod_uses_node_archives(pod) {
        return Ok(());
    }
    let Some(node_name) = pod
        .spec
        .as_ref()
        .and_then(|spec| spec.node_name.as_deref())
        .filter(|node| !node.is_empty())
    else {
        return Ok(());
    };
    let generation = pod
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| generation_from_label(labels.get(OPERATION_GENERATION_LABEL)))
        .map_or(0, moatless_sandbox_control::lifecycle::OperationGeneration::value);
    record_node(k8s, task_id, node_name, generation).await
}

async fn load_records(k8s: &K8sClient, task_id: &str) -> Result<Option<NodeRecords>, RuntimeError> {
    let config_maps: Api<ConfigMap> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    config_maps
        .get_opt(&archive_config_map_name(task_id))
        .await
        .map_err(|error| runtime_error("read node archive metadata", error))?
        .as_ref()
        .map(records_from_config_map)
        .transpose()
}

/// Synchronously capture every currently scheduled node-backed sandbox pod.
pub(crate) async fn record_current_pod_nodes(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<usize, RuntimeError> {
    let pods: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let selector = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));
    let current = pods
        .list(&selector)
        .await
        .map_err(|error| runtime_error("list current sandbox pods", error))?;
    let mut recorded = 0;
    for pod in &current.items {
        if pod_uses_node_archives(pod)
            && pod
                .spec
                .as_ref()
                .and_then(|spec| spec.node_name.as_ref())
                .is_some()
        {
            record_observed_pod(k8s, task_id, pod).await?;
            recorded += 1;
        }
    }
    Ok(recorded)
}

fn select_newest_surviving(
    records: &NodeRecords,
    surviving_nodes: &BTreeSet<String>,
) -> Option<String> {
    records
        .iter()
        .filter(|(node, _)| surviving_nodes.contains(*node))
        .max_by(|(node_a, generation_a), (node_b, generation_b)| {
            generation_a
                .cmp(generation_b)
                .then_with(|| node_a.cmp(node_b))
        })
        .map(|(node, _)| node.clone())
}

async fn cluster_node_names(
    k8s: &K8sClient,
    selector: Option<&str>,
) -> Result<BTreeSet<String>, RuntimeError> {
    let nodes: Api<Node> = Api::all(k8s.client.clone());
    let params = selector.map_or_else(ListParams::default, |selector| {
        ListParams::default().labels(selector)
    });
    nodes
        .list(&params)
        .await
        .map_err(|error| runtime_error("list Kubernetes nodes", error))
        .map(|list| {
            list.items
                .into_iter()
                .filter_map(|node| node.metadata.name)
                .collect()
        })
}

/// Resolve a retained placement. Everything tracked here is a cache, so a
/// vanished record set is not fatal: the caller schedules unpinned and pays a
/// cold rebuild rather than stranding the task on a node that no longer exists.
pub(crate) async fn retained_node(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<Option<String>, RuntimeError> {
    let records = load_records(k8s, task_id).await?;
    // Only a present record set justifies listing nodes.
    let surviving = match records {
        Some(_) => cluster_node_names(k8s, None).await?,
        None => BTreeSet::new(),
    };
    Ok(resolve_retained(records.as_ref(), &surviving, task_id))
}

fn resolve_retained(
    records: Option<&NodeRecords>,
    surviving: &BTreeSet<String>,
    task_id: &str,
) -> Option<String> {
    let node = select_newest_surviving(records?, surviving);
    if node.is_none() {
        info!(
            task_id,
            "Recorded archive nodes are gone; scheduling without a node pin"
        );
    }
    node
}

/// A destroyed node took its disk with it, so it owes no cleanup. Keeping it in
/// the plan pins a cleanup pod to a nonexistent node, which never schedules and
/// blocks removal forever.
fn surviving_cleanup_nodes(records: NodeRecords, surviving: &BTreeSet<String>) -> Vec<String> {
    records
        .into_keys()
        .filter(|node| surviving.contains(node))
        .collect()
}

fn node_selector(config: &Config) -> Result<Option<String>, RuntimeError> {
    let Some(json) = config.sandbox.node_selector_json.as_deref() else {
        return Ok(None);
    };
    let selector = serde_json::from_str::<BTreeMap<String, String>>(json)
        .map_err(|error| runtime_error("parse sandbox node selector for legacy cleanup", error))?;
    Ok((!selector.is_empty()).then(|| {
        selector
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(",")
    }))
}

/// Capture the current location before teardown and resolve the nodes owed a
/// hard-delete cleanup. Legacy node-backed tasks with no metadata fan out to
/// every node matching the sandbox selector.
pub(crate) async fn prepare_remove(
    k8s: &K8sClient,
    config: &Config,
    task_id: &str,
    deployment: Option<&Deployment>,
) -> Result<RemovalPlan, RuntimeError> {
    let metadata_before = load_records(k8s, task_id).await?;
    let recorded_current = record_current_pod_nodes(k8s, task_id).await?;
    let metadata_after = load_records(k8s, task_id).await?;

    if let Some(records) = metadata_after {
        let surviving = cluster_node_names(k8s, None).await?;
        return Ok(RemovalPlan {
            nodes: surviving_cleanup_nodes(records, &surviving),
        });
    }

    let runtime_is_definitely_non_node =
        deployment.is_some_and(|deployment| !deployment_uses_node_archives(deployment));
    if runtime_is_definitely_non_node && recorded_current == 0 && metadata_before.is_none() {
        return Ok(RemovalPlan { nodes: Vec::new() });
    }

    let selector = node_selector(config)?;
    let nodes = cluster_node_names(k8s, selector.as_deref())
        .await?
        .into_iter()
        .collect();
    Ok(RemovalPlan { nodes })
}

fn cleanup_pod_name(task_id: &str, node_name: &str) -> String {
    let digest = Sha256::digest(format!("{task_id}\0{node_name}"));
    let suffix = hex::encode(&digest[..6]);
    let prefix = "moatless-archive-cleanup-";
    let task_budget = 63 - prefix.len() - 1 - suffix.len();
    format!(
        "{prefix}{}-{suffix}",
        sanitize_k8s_name(task_id, task_budget)
    )
}

fn build_cleanup_pod(
    image: &str,
    image_pull_policy: &str,
    cache_host_root: &str,
    task_id: &str,
    node_name: &str,
) -> Pod {
    Pod {
        metadata: ObjectMeta {
            name: Some(cleanup_pod_name(task_id, node_name)),
            labels: Some(BTreeMap::from([
                (
                    "app.kubernetes.io/name".to_string(),
                    "moatless-archive-cleanup".to_string(),
                ),
                (
                    "app.kubernetes.io/component".to_string(),
                    "archive-cleanup".to_string(),
                ),
            ])),
            ..Default::default()
        },
        spec: Some(PodSpec {
            automount_service_account_token: Some(false),
            node_name: Some(node_name.to_string()),
            restart_policy: Some("Never".to_string()),
            security_context: Some(PodSecurityContext {
                seccomp_profile: Some(SeccompProfile {
                    type_: "RuntimeDefault".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            containers: vec![Container {
                name: "cleanup".to_string(),
                image: Some(image.to_string()),
                image_pull_policy: Some(image_pull_policy.to_string()),
                command: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
                args: Some(vec![
                    "rm -rf -- \"/host-cache/workspaces/$TASK_ID\" \"/host-cache/docker/$TASK_ID\" \"/host-cache/build-cache/tasks/$TASK_ID\"".to_string(),
                ]),
                env: Some(vec![k8s_openapi::api::core::v1::EnvVar {
                    name: "TASK_ID".to_string(),
                    value: Some(task_id.to_string()),
                    ..Default::default()
                }]),
                security_context: Some(SecurityContext {
                    allow_privilege_escalation: Some(false),
                    capabilities: Some(Capabilities {
                        // The task tree is owned by the sandbox user (UID
                        // 1000), so root needs only DAC_OVERRIDE to traverse
                        // and unlink it. Drop every broader default capability.
                        add: Some(vec!["DAC_OVERRIDE".to_string()]),
                        drop: Some(vec!["ALL".to_string()]),
                    }),
                    privileged: Some(false),
                    read_only_root_filesystem: Some(true),
                    run_as_non_root: Some(false),
                    run_as_user: Some(0),
                    ..Default::default()
                }),
                volume_mounts: Some(vec![VolumeMount {
                    name: "node-cache".to_string(),
                    mount_path: CLEANUP_MOUNT_PATH.to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            volumes: Some(vec![Volume {
                name: "node-cache".to_string(),
                host_path: Some(HostPathVolumeSource {
                    path: cache_host_root.to_string(),
                    type_: Some("DirectoryOrCreate".to_string()),
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn delete_cleanup_pod(
    pods: &Api<Pod>,
    task_id: &str,
    name: &str,
) -> Result<(), RuntimeError> {
    match pods.delete(name, &DeleteParams::default()).await {
        Ok(_) => {}
        Err(kube::Error::Api(ref response)) if response.code == 404 => {}
        Err(error) => return Err(runtime_error("delete archive cleanup pod", error)),
    }
    super::wait::wait_absent(
        task_id,
        "archive cleanup pod",
        Duration::from_mins(1),
        || async { pods.get_opt(name).await.map(|pod| pod.is_none()) },
    )
    .await
    .map_err(|error| runtime_error("await archive cleanup pod absence", error))
}

/// Run a deterministic, idempotent cleanup pod and observe both successful
/// completion and subsequent pod absence.
pub(crate) async fn cleanup_node(
    k8s: &K8sClient,
    config: &Config,
    task_id: &str,
    node_name: &str,
) -> Result<(), RuntimeError> {
    let pods: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = cleanup_pod_name(task_id, node_name);
    let pod = build_cleanup_pod(
        &config.sandbox.image,
        &config.sandbox.image_pull_policy,
        &config.sandbox.cache_host_root,
        task_id,
        node_name,
    );
    match pods.get_opt(&name).await {
        Ok(Some(_)) => {}
        Ok(None) => match pods.create(&PostParams::default(), &pod).await {
            Ok(_) => {
                info!(
                    task_id,
                    node_name,
                    pod_name = %name,
                    "Created node archive cleanup pod"
                );
            }
            Err(kube::Error::Api(ref response)) if response.code == 409 => {}
            Err(error) => return Err(runtime_error("create archive cleanup pod", error)),
        },
        Err(error) => return Err(runtime_error("read archive cleanup pod", error)),
    }

    let deadline = tokio::time::Instant::now() + CLEANUP_TIMEOUT;
    loop {
        let current = pods
            .get_opt(&name)
            .await
            .map_err(|error| runtime_error("observe archive cleanup pod", error))?;
        let Some(current) = current else {
            return Err(RuntimeError::Infrastructure(format!(
                "archive cleanup pod {name} vanished before successful completion"
            )));
        };
        match current
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
        {
            Some("Succeeded") => {
                delete_cleanup_pod(&pods, task_id, &name).await?;
                info!(task_id, node_name, "Deleted node-local task archives");
                return Ok(());
            }
            Some("Failed") => {
                let detail = current
                    .status
                    .as_ref()
                    .and_then(|status| status.message.as_deref())
                    .unwrap_or("unknown failure")
                    .to_string();
                delete_cleanup_pod(&pods, task_id, &name).await?;
                return Err(RuntimeError::Infrastructure(format!(
                    "archive cleanup pod failed on node {node_name}: {detail}"
                )));
            }
            _ if tokio::time::Instant::now() >= deadline => {
                delete_cleanup_pod(&pods, task_id, &name).await?;
                return Err(RuntimeError::Infrastructure(format!(
                    "timed out waiting for archive cleanup on node {node_name}"
                )));
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
}

pub(crate) async fn delete_metadata(k8s: &K8sClient, task_id: &str) -> Result<(), RuntimeError> {
    let config_maps: Api<ConfigMap> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = archive_config_map_name(task_id);
    match config_maps.delete(&name, &DeleteParams::default()).await {
        Ok(_) => {}
        Err(kube::Error::Api(ref response)) if response.code == 404 => return Ok(()),
        Err(error) => return Err(runtime_error("delete node archive metadata", error)),
    }
    super::wait::wait_absent(
        task_id,
        "node archive metadata",
        Duration::from_mins(1),
        || async { config_maps.get_opt(&name).await.map(|item| item.is_none()) },
    )
    .await
    .map_err(|error| runtime_error("await node archive metadata absence", error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_record_merge_keeps_latest_generation() {
        let mut records = NodeRecords::new();
        merge_node_record(&mut records, "node-b", 2);
        merge_node_record(&mut records, "node-a", 3);
        merge_node_record(&mut records, "node-b", 1);
        merge_node_record(&mut records, "node-b", 4);

        assert_eq!(
            records,
            BTreeMap::from([("node-a".to_string(), 3), ("node-b".to_string(), 4)])
        );
    }

    #[test]
    fn latest_recorded_surviving_node_is_selected() {
        let records = BTreeMap::from([
            ("node-a".to_string(), 1),
            ("node-b".to_string(), 3),
            ("node-c".to_string(), 2),
        ]);
        let surviving = BTreeSet::from(["node-a".to_string(), "node-c".to_string()]);

        assert_eq!(
            select_newest_surviving(&records, &surviving).as_deref(),
            Some("node-c")
        );
    }

    #[test]
    fn retained_node_selection_refuses_missing_nodes() {
        let records = BTreeMap::from([("gone-node".to_string(), 7)]);
        assert!(select_newest_surviving(&records, &BTreeSet::new()).is_none());
    }

    #[test]
    fn cleanup_pod_is_root_but_unprivileged_and_not_watched() {
        let pod = build_cleanup_pod(
            "sandbox:latest",
            "IfNotPresent",
            "/mnt/cache",
            "task-1",
            "node-a",
        );
        let spec = pod.spec.as_ref().expect("pod spec");
        let container = &spec.containers[0];
        let security = container.security_context.as_ref().expect("security");

        assert_eq!(spec.node_name.as_deref(), Some("node-a"));
        assert_eq!(spec.restart_policy.as_deref(), Some("Never"));
        assert_eq!(spec.automount_service_account_token, Some(false));
        assert_eq!(security.run_as_user, Some(0));
        assert_eq!(security.privileged, Some(false));
        assert_eq!(security.allow_privilege_escalation, Some(false));
        assert!(security
            .capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.drop.as_ref())
            .is_some_and(|drop| drop == &["ALL".to_string()]));
        assert!(security
            .capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.add.as_ref())
            .is_some_and(|add| add == &["DAC_OVERRIDE".to_string()]));
        assert!(!pod
            .metadata
            .labels
            .as_ref()
            .is_some_and(|labels| labels.contains_key("moatless.io/task-id")));
        assert!(container
            .args
            .as_ref()
            .is_some_and(|args| args[0].contains("/host-cache/build-cache/tasks/$TASK_ID")));
    }

    #[test]
    fn task_scoped_build_cache_uses_node_archives_but_shared_cache_does_not() {
        assert!(volume_holds_task_node_data(&Volume {
            name: "build-cache".to_string(),
            host_path: Some(HostPathVolumeSource {
                path: "/mnt/cache/build-cache/tasks/task-1".to_string(),
                type_: Some("DirectoryOrCreate".to_string()),
            }),
            ..Default::default()
        }));
        assert!(!volume_holds_task_node_data(&Volume {
            name: "build-cache".to_string(),
            host_path: Some(HostPathVolumeSource {
                path: "/mnt/cache/build-cache/shared".to_string(),
                type_: Some("DirectoryOrCreate".to_string()),
            }),
            ..Default::default()
        }));
    }

    fn host_path_volume(name: &str, path: &str) -> Volume {
        Volume {
            name: name.to_string(),
            host_path: Some(HostPathVolumeSource {
                path: path.to_string(),
                type_: Some("DirectoryOrCreate".to_string()),
            }),
            ..Default::default()
        }
    }

    fn deployment_with_volumes(volumes: Vec<Volume>) -> Deployment {
        Deployment {
            spec: Some(k8s_openapi::api::apps::v1::DeploymentSpec {
                template: k8s_openapi::api::core::v1::PodTemplateSpec {
                    spec: Some(PodSpec {
                        volumes: Some(volumes),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn task_scoped_build_cache_is_tracked() {
        let deployment = deployment_with_volumes(vec![host_path_volume(
            "build-cache",
            "/mnt/cache/build-cache/tasks/task-1",
        )]);

        assert!(deployment_uses_node_archives(&deployment));
    }

    #[test]
    fn docker_storage_is_tracked() {
        let deployment = deployment_with_volumes(vec![host_path_volume(
            "docker-storage",
            "/mnt/cache/docker/task-1",
        )]);

        assert!(deployment_uses_node_archives(&deployment));
    }

    /// The workspace is a PVC or an emptyDir now, so even a leftover hostPath
    /// under the old node tier's path must not pull the workspace back into
    /// node-archive tracking.
    #[test]
    fn a_workspace_volume_is_never_tracked() {
        for volume in [
            host_path_volume("workspace-data", "/mnt/cache/workspaces/task-1"),
            Volume {
                name: "workspace-data".to_string(),
                empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                ..Default::default()
            },
        ] {
            let deployment = deployment_with_volumes(vec![volume]);
            assert!(!deployment_uses_node_archives(&deployment));
        }
    }

    #[test]
    fn surviving_recorded_node_is_pinned() {
        let records = BTreeMap::from([("node-a".to_string(), 1)]);
        let surviving = BTreeSet::from(["node-a".to_string()]);

        assert_eq!(
            resolve_retained(Some(&records), &surviving, "task-1"),
            Some("node-a".to_string())
        );
    }

    /// A cache is rebuildable, so neither a vanished node nor missing metadata
    /// may block a start — both simply schedule unpinned.
    #[test]
    fn a_lost_node_schedules_unpinned_rather_than_failing() {
        let records = BTreeMap::from([("gone-node".to_string(), 1)]);
        let surviving = BTreeSet::from(["node-a".to_string()]);

        assert_eq!(resolve_retained(Some(&records), &surviving, "task-1"), None);
        assert_eq!(resolve_retained(None, &surviving, "task-1"), None);
    }

    #[test]
    fn destroyed_nodes_are_dropped_from_the_removal_plan() {
        let records = BTreeMap::from([("gone-node".to_string(), 2), ("live-node".to_string(), 1)]);
        let surviving = BTreeSet::from(["live-node".to_string()]);

        assert_eq!(
            surviving_cleanup_nodes(records, &surviving),
            vec!["live-node".to_string()]
        );
    }

    #[test]
    fn hard_delete_cleanup_pod_is_deterministic_for_retries() {
        let first = build_cleanup_pod(
            "sandbox:latest",
            "IfNotPresent",
            "/mnt/cache",
            "task-1",
            "node-a",
        );
        let retry = build_cleanup_pod(
            "sandbox:latest",
            "IfNotPresent",
            "/mnt/cache",
            "task-1",
            "node-a",
        );

        assert_eq!(first.metadata.name, retry.metadata.name);
        assert_eq!(first.spec, retry.spec);
    }
}
